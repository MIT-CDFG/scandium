// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.6/§10.2 (appname.command.v1 envelope, client-generated ids,
// replay determinism), §22.1 (color wire object); contracts/commands/
// {doc.new,brush.stroke,eraser.stroke}.yaml. TESTER-added edge coverage
// beyond tst_envelope_builder.cpp: exact compact-JSON byte goldens (key
// order + number formatting are part of the deterministic wire), sizePx at
// contract extremes and beyond (the builder serializes faithfully; range
// enforcement is the engine's stage 3), non-finite doubles (Qt serializes
// them as JSON null — pinned), color component precision across the whole
// 8-bit picker range, seed masking at u64 extremes, exact pinned
// deterministic-UUID sequences (independent splitmix64 derivation), and
// version/variant bits over thousands of counter values.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSet>
#include <QtTest>

#include <cmath>
#include <limits>

using namespace appname::shell;

namespace {

QJsonObject parse(const QByteArray &bytes)
{
    return QJsonDocument::fromJson(bytes).object();
}

StrokeParams brushParams()
{
    StrokeParams p;
    p.kind = QStringLiteral("brush.stroke");
    p.targetLayerId = QStringLiteral("00000000-0001-4000-8000-000000000003");
    p.samplesAssetId = QStringLiteral("00000000-0001-4000-8000-000000000006");
    p.sizePx = 24.0;
    p.hardness = 0.25; // item 4: a non-default override pins the wire spelling
    p.color = QColor(255, 0, 0);
    p.seed = 7;
    return p;
}

} // namespace

class TstEnvelopeBuilderEdges : public QObject {
    Q_OBJECT

private slots:
    // Exact wire bytes: QJsonObject serializes with sorted keys and
    // shortest number spellings, so the envelope byte stream for fixed
    // inputs is a deterministic constant — pinned in full (D-48 witness:
    // identical inputs => identical envelope bytes).
    void compactJsonByteGoldens()
    {
        const QByteArray brush = buildStrokeEnvelope(
            QStringLiteral("00000000-0001-4000-8000-000000000001"), brushParams());
        QCOMPARE(QString::fromUtf8(brush),
                 QStringLiteral(
                     "{\"id\":\"00000000-0001-4000-8000-000000000001\","
                     "\"kind\":\"brush.stroke\","
                     "\"params\":{\"blend\":\"normal\","
                     "\"brushPreset\":\"7efe96cd-64b6-5e4d-835c-5dd050705583\","
                     "\"color\":{\"comps\":[1,0,0,0],\"model\":\"rgb\"},"
                     "\"flow\":1,\"hardness\":0.25,\"opacity\":1,\"seed\":7,\"sizePx\":24},"
                     "\"samples\":\"00000000-0001-4000-8000-000000000006\","
                     "\"schema\":\"appname.command.v1\","
                     "\"target\":{\"layerId\":\"00000000-0001-4000-8000-000000000003\"}}"));

        const QByteArray docNew = buildDocNewEnvelope(
            QStringLiteral("00000001-0000-4000-8000-000000000001"), 640, 400,
            QStringLiteral("white"));
        QCOMPARE(QString::fromUtf8(docNew),
                 QStringLiteral(
                     "{\"id\":\"00000001-0000-4000-8000-000000000001\","
                     "\"kind\":\"doc.new\","
                     "\"params\":{\"background\":\"white\",\"height\":400,\"width\":640},"
                     "\"schema\":\"appname.command.v1\"}"));

        // Same inputs twice => identical bytes.
        QCOMPARE(buildStrokeEnvelope(
                     QStringLiteral("00000000-0001-4000-8000-000000000001"), brushParams()),
                 brush);
    }

    // Top-level and params key sets, exactly. doc.new carries NO target
    // and NO samples (contract: target ignored for doc.new; the builder
    // omits it, matching the validMinimal example). additionalProperties
    // is false in every contract, so a single stray key is a stage-3
    // failure — the exact key inventory is load-bearing.
    void keySetInventories()
    {
        const QJsonObject docNew = parse(buildDocNewEnvelope(
            QStringLiteral("00000001-0000-4000-8000-000000000001"), 640, 400,
            QStringLiteral("white")));
        QCOMPARE(docNew.keys(),
                 QStringList({QStringLiteral("id"), QStringLiteral("kind"),
                              QStringLiteral("params"), QStringLiteral("schema")}));
        QCOMPARE(docNew.value("params").toObject().keys(),
                 QStringList({QStringLiteral("background"), QStringLiteral("height"),
                              QStringLiteral("width")}));

        const QJsonObject brush = parse(buildStrokeEnvelope(
            QStringLiteral("00000000-0001-4000-8000-000000000001"), brushParams()));
        QCOMPARE(brush.keys(),
                 QStringList({QStringLiteral("id"), QStringLiteral("kind"),
                              QStringLiteral("params"), QStringLiteral("samples"),
                              QStringLiteral("schema"), QStringLiteral("target")}));
        QCOMPARE(brush.value("params").toObject().keys(),
                 QStringList({QStringLiteral("blend"), QStringLiteral("brushPreset"),
                              QStringLiteral("color"), QStringLiteral("flow"),
                              QStringLiteral("hardness"), QStringLiteral("opacity"),
                              QStringLiteral("seed"), QStringLiteral("sizePx")}));
        QCOMPARE(brush.value("target").toObject().keys(),
                 QStringList({QStringLiteral("layerId")}));
        QCOMPARE(brush.value("params").toObject().value("color").toObject().keys(),
                 QStringList({QStringLiteral("comps"), QStringLiteral("model")}));

        StrokeParams e = brushParams();
        e.kind = QStringLiteral("eraser.stroke");
        const QJsonObject eraser = parse(buildStrokeEnvelope(
            QStringLiteral("00000000-0002-4000-8000-000000000001"), e));
        QCOMPARE(eraser.value("params").toObject().keys(),
                 QStringList({QStringLiteral("brushPreset"), QStringLiteral("flow"),
                              QStringLiteral("opacity"), QStringLiteral("seed"),
                              QStringLiteral("sizePx")}));

        // Any kind other than brush.stroke takes the color-less shape; the
        // kind string itself is a faithful passthrough.
        StrokeParams other = brushParams();
        other.kind = QStringLiteral("smudge.stroke");
        const QJsonObject smudge = parse(buildStrokeEnvelope(
            QStringLiteral("00000000-0003-4000-8000-000000000001"), other));
        QCOMPARE(smudge.value("kind").toString(), QStringLiteral("smudge.stroke"));
        QVERIFY(!smudge.value("params").toObject().contains("color"));
        QVERIFY(!smudge.value("params").toObject().contains("blend"));
        QVERIFY(!smudge.value("params").toObject().contains("hardness"));

        // A caller-supplied preset overrides the placeholder faithfully.
        StrokeParams preset = brushParams();
        preset.brushPreset = QStringLiteral("00000000-0009-4000-8000-000000000009");
        const QJsonObject withPreset = parse(buildStrokeEnvelope(
            QStringLiteral("00000000-0004-4000-8000-000000000001"), preset));
        QCOMPARE(withPreset.value("params").toObject().value("brushPreset").toString(),
                 preset.brushPreset);
    }

    // sizePx at the contract extremes round-trips bit-exactly; values
    // OUTSIDE the contract range serialize faithfully too — the builder is
    // a serializer, not a validator (range rejection is the engine's
    // stage 3, error.schema.out_of_range). Pinned so any future silent
    // clamping shows up as a diff.
    void sizePxExtremesAndFaithfulOutOfRange_data()
    {
        QTest::addColumn<double>("sizePx");
        QTest::newRow("contract-min-0.1") << 0.1;
        QTest::newRow("contract-max-10000") << 10000.0;
        QTest::newRow("below-min") << 0.05;
        QTest::newRow("above-max") << 20000.0;
        QTest::newRow("negative") << -5.0;
        QTest::newRow("nonrepresentable-third") << (1.0 / 3.0);
    }

    void sizePxExtremesAndFaithfulOutOfRange()
    {
        QFETCH(double, sizePx);
        StrokeParams p = brushParams();
        p.sizePx = sizePx;
        p.opacity = 1.5;  // out of 0..=1 — faithful
        p.flow = -0.25;   // out of 0..=1 — faithful
        const QJsonObject params =
            parse(buildStrokeEnvelope(QStringLiteral("00000000-0005-4000-8000-000000000001"), p))
                .value("params")
                .toObject();
        QCOMPARE(params.value("sizePx").toDouble(), sizePx); // exact double round-trip
        QCOMPARE(params.value("opacity").toDouble(), 1.5);
        QCOMPARE(params.value("flow").toDouble(), -0.25);
    }

    // Non-finite doubles: Qt's JSON layer stores NaN/±Inf as null, so a
    // NaN sizePx leaves the builder as "sizePx":null — a silent value
    // alteration pinned here (implementation report item: the shell must
    // keep non-finite values out upstream; the engine sees a wrong-typed
    // field, not the number the shell thought it sent).
    void nonFiniteDoublesSerializeAsJsonNull()
    {
        StrokeParams p = brushParams();
        p.sizePx = std::nan("");
        p.opacity = std::numeric_limits<double>::infinity();
        const QJsonObject params =
            parse(buildStrokeEnvelope(QStringLiteral("00000000-0006-4000-8000-000000000001"), p))
                .value("params")
                .toObject();
        QVERIFY(params.contains("sizePx"));
        QVERIFY(params.value("sizePx").isNull());
        QVERIFY(params.value("opacity").isNull());
    }

    // §22.1 color: every 8-bit picker value round-trips through the JSON
    // wire back to its exact component double, and re-quantizes to the
    // original integer. comps[3] is 0.0 for rgb even when the QColor
    // carries alpha (alpha is not part of the brush color contract).
    void colorComponentPrecisionAcrossPickerRange()
    {
        for (int v = 0; v <= 255; ++v) {
            StrokeParams p = brushParams();
            p.color = QColor(v, 255 - v, v / 2);
            const QJsonArray comps =
                parse(buildStrokeEnvelope(
                          QStringLiteral("00000000-0007-4000-8000-000000000001"), p))
                    .value("params")
                    .toObject()
                    .value("color")
                    .toObject()
                    .value("comps")
                    .toArray();
            QCOMPARE(comps.size(), 4);
            QCOMPARE(comps.at(0).toDouble(), double(p.color.redF()));
            QCOMPARE(comps.at(1).toDouble(), double(p.color.greenF()));
            QCOMPARE(comps.at(2).toDouble(), double(p.color.blueF()));
            QCOMPARE(qRound(comps.at(0).toDouble() * 255.0), v);
            QCOMPARE(qRound(comps.at(1).toDouble() * 255.0), 255 - v);
            QCOMPARE(qRound(comps.at(2).toDouble() * 255.0), v / 2);
        }
        StrokeParams withAlpha = brushParams();
        withAlpha.color = QColor(10, 20, 30, 128); // alpha silently dropped
        const QJsonArray comps =
            parse(buildStrokeEnvelope(
                      QStringLiteral("00000000-0008-4000-8000-000000000001"), withAlpha))
                .value("params")
                .toObject()
                .value("color")
                .toObject()
                .value("comps")
                .toArray();
        QCOMPARE(comps.at(3).toDouble(), 0.0);
    }

    // Seed masking at u64 extremes: the builder records seed & 0x7fff...
    // as a JSON integer. Values above i64::MAX are silently CHANGED in the
    // recorded command (implementation report item — replay reads the
    // recorded seed, so live-vs-replay can diverge if live dabs ever use
    // the unmasked value). Pinned: max, 2^63 (masks to 0), 2^63-1, small.
    void seedMaskingAtU64Extremes()
    {
        auto seedOf = [](quint64 seed) {
            StrokeParams p;
            p.kind = QStringLiteral("eraser.stroke");
            p.targetLayerId = QStringLiteral("00000000-0002-4000-8000-000000000003");
            p.samplesAssetId = QStringLiteral("00000000-0002-4000-8000-000000000006");
            p.seed = seed;
            return buildStrokeEnvelope(
                QStringLiteral("00000000-0002-4000-8000-000000000001"), p);
        };
        const QByteArray atMax = seedOf(std::numeric_limits<quint64>::max());
        QCOMPARE(parse(atMax).value("params").toObject().value("seed").toInteger(),
                 Q_INT64_C(9223372036854775807));
        QVERIFY(atMax.contains("\"seed\":9223372036854775807")); // full width on the wire
        QCOMPARE(parse(seedOf(Q_UINT64_C(0x8000000000000000)))
                     .value("params").toObject().value("seed").toInteger(),
                 Q_INT64_C(0)); // masked to zero — the silent change
        QCOMPARE(parse(seedOf(Q_UINT64_C(0x7fffffffffffffff)))
                     .value("params").toObject().value("seed").toInteger(),
                 Q_INT64_C(9223372036854775807)); // in-range value unchanged
        QCOMPARE(parse(seedOf(7)).value("params").toObject().value("seed").toInteger(),
                 Q_INT64_C(7));
    }

    // Deterministic UuidSource: exact pinned sequences. Expected strings
    // derived INDEPENDENTLY from the documented construction (splitmix64
    // over (seed, counter), v4/variant bit packing) — not copied from the
    // implementation's output. 0xA99AA9E5EED is the tour seed (main.cpp),
    // so this doubles as the tour's replay-determinism witness.
    void deterministicUuidSequencePinned()
    {
        UuidSource tour(Q_UINT64_C(0xA99AA9E5EED));
        QCOMPARE(tour.next(), QStringLiteral("675b3b5f-86d8-410f-95aa-5c21aeba67fd"));
        QCOMPARE(tour.next(), QStringLiteral("53734b06-0f90-4ac9-a968-93cb3503cd07"));
        QCOMPARE(tour.next(), QStringLiteral("a3e55a2a-41f1-4503-a229-5ca477cc453d"));

        UuidSource zero(Q_UINT64_C(0));
        QCOMPARE(zero.next(), QStringLiteral("e220a839-7b1d-4daf-9cc6-0547776902ba"));

        UuidSource top(std::numeric_limits<quint64>::max());
        QCOMPARE(top.next(), QStringLiteral("e4d97177-1b65-4c20-ae4e-8f3d5fc98118"));

        UuidSource fortyTwo(Q_UINT64_C(42));
        QCOMPARE(fortyTwo.next(), QStringLiteral("bdd73226-2feb-4e95-ac2d-ecf4775147c6"));
        QCOMPARE(fortyTwo.next(), QStringLiteral("ba69ec90-eb4f-4f88-aad8-71ed1dd59b56"));
    }

    // Version/variant bits hold for every counter value over thousands of
    // iterations; the sequence has no collisions and replays identically
    // from a second source with the same seed.
    void uuidBitsHoldOverManyCounters()
    {
        UuidSource a(Q_UINT64_C(42)), b(Q_UINT64_C(42));
        QSet<QString> seen;
        for (int i = 0; i < 5000; ++i) {
            const QString u = a.next();
            QCOMPARE(b.next(), u); // bit-identical replay
            QCOMPARE(u.size(), 36);
            QCOMPARE(u.at(14), QLatin1Char('4')); // version nibble
            const QChar variant = u.at(19);
            QVERIFY2(variant == QLatin1Char('8') || variant == QLatin1Char('9')
                         || variant == QLatin1Char('a') || variant == QLatin1Char('b'),
                     qPrintable(QStringLiteral("counter %1: variant char %2").arg(i).arg(variant)));
            QVERIFY2(!seen.contains(u),
                     qPrintable(QStringLiteral("collision at counter %1").arg(i)));
            seen.insert(u);
        }
    }

    // doc.new is a faithful serializer as well: out-of-contract dimensions
    // and background spellings pass through unaltered (stage 3 is the
    // engine's job; the dialog is the shell-side gate).
    void docNewFaithfulSerialization()
    {
        const QJsonObject params = parse(buildDocNewEnvelope(
            QStringLiteral("00000001-0000-4000-8000-000000000002"), -5, 0,
            QStringLiteral("purple")))
                                       .value("params")
                                       .toObject();
        QCOMPARE(params.value("width").toInt(), -5);
        QCOMPARE(params.value("height").toInt(), 0);
        QCOMPARE(params.value("background").toString(), QStringLiteral("purple"));
    }
};

QTEST_GUILESS_MAIN(TstEnvelopeBuilderEdges)
#include "tst_envelope_builder_edges.moc"
