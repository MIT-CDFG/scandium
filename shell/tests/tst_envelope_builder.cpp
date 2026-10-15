// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.6/§10.2; contracts/commands/{doc.new,brush.stroke,
// eraser.stroke}.yaml — the envelopes the shell emits must match the
// contract examples' shape: schema/id/kind/target/params/samples, samples
// as an assetId STRING (never inline), color as the §22.1 wire object,
// eraser without color/blend.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <QtTest>

using namespace appname::shell;

namespace {

const QRegularExpression kUuidV4(
    QStringLiteral("^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"));

QJsonObject parse(const QByteArray &bytes)
{
    const QJsonDocument doc = QJsonDocument::fromJson(bytes);
    return doc.object();
}

} // namespace

class TstEnvelopeBuilder : public QObject {
    Q_OBJECT

private slots:
    void docNewEnvelopeShape()
    {
        const QJsonObject env = parse(buildDocNewEnvelope(
            QStringLiteral("00000001-0000-4000-8000-000000000001"), 640, 400,
            QStringLiteral("white")));
        QCOMPARE(env.value("schema").toString(), QStringLiteral("appname.command.v1"));
        QCOMPARE(env.value("kind").toString(), QStringLiteral("doc.new"));
        QCOMPARE(env.value("id").toString(),
                 QStringLiteral("00000001-0000-4000-8000-000000000001"));
        const QJsonObject params = env.value("params").toObject();
        QCOMPARE(params.value("width").toInt(), 640);
        QCOMPARE(params.value("height").toInt(), 400);
        QCOMPARE(params.value("background").toString(), QStringLiteral("white"));
    }

    void brushStrokeEnvelopeShape()
    {
        StrokeParams p;
        p.kind = QStringLiteral("brush.stroke");
        p.targetLayerId = QStringLiteral("00000000-0001-4000-8000-000000000003");
        p.samplesAssetId = QStringLiteral("00000000-0001-4000-8000-000000000006");
        p.sizePx = 24.0;
        p.hardness = 0.85; // item 4: non-default override must ride verbatim
        p.color = QColor(255, 0, 0);
        p.seed = 7;
        const QJsonObject env =
            parse(buildStrokeEnvelope(QStringLiteral("00000000-0001-4000-8000-000000000001"), p));

        QCOMPARE(env.value("schema").toString(), QStringLiteral("appname.command.v1"));
        QCOMPARE(env.value("kind").toString(), QStringLiteral("brush.stroke"));
        QCOMPARE(env.value("target").toObject().value("layerId").toString(), p.targetLayerId);

        // §9.6 rule 2: samples is the assetId string, NEVER inline points.
        QVERIFY(env.value("samples").isString());
        QCOMPARE(env.value("samples").toString(), p.samplesAssetId);

        const QJsonObject params = env.value("params").toObject();
        QCOMPARE(params.value("brushPreset").toString(), kDefaultBrushPreset);
        QCOMPARE(params.value("sizePx").toDouble(), 24.0);
        QCOMPARE(params.value("hardness").toDouble(), 0.85);
        QCOMPARE(params.value("opacity").toDouble(), 1.0);
        QCOMPARE(params.value("flow").toDouble(), 1.0);
        QCOMPARE(params.value("blend").toString(), QStringLiteral("normal"));
        QCOMPARE(params.value("seed").toInt(), 7);

        // §22.1 color wire shape: model + exactly 4 comps.
        const QJsonObject color = params.value("color").toObject();
        QCOMPARE(color.value("model").toString(), QStringLiteral("rgb"));
        const QJsonArray comps = color.value("comps").toArray();
        QCOMPARE(comps.size(), 4);
        QCOMPARE(comps.at(0).toDouble(), 1.0);
        QCOMPARE(comps.at(3).toDouble(), 0.0);
    }

    void eraserStrokeHasNoColorOrBlend()
    {
        StrokeParams p;
        p.kind = QStringLiteral("eraser.stroke");
        p.targetLayerId = QStringLiteral("00000000-0002-4000-8000-000000000003");
        p.samplesAssetId = QStringLiteral("00000000-0002-4000-8000-000000000006");
        const QJsonObject env = parse(buildStrokeEnvelope(
            QStringLiteral("00000000-0002-4000-8000-000000000001"), p));
        QCOMPARE(env.value("kind").toString(), QStringLiteral("eraser.stroke"));
        const QJsonObject params = env.value("params").toObject();
        // additionalProperties: false — extra params are a stage-3 schema
        // violation, so the eraser envelope must not carry color/blend,
        // nor hardness (item 4: eraser's contract is un-extended).
        QVERIFY(!params.contains("color"));
        QVERIFY(!params.contains("blend"));
        QVERIFY(!params.contains("hardness"));
        QVERIFY(params.contains("brushPreset"));
        QVERIFY(params.contains("seed"));
    }

    // Tier-1 row 5 (C-666): the sample-scope emit is KIND-GUARDED and
    // TRUE-ONLY — engine REGISTERED schemas ground every arm here
    // (registry_gen.rs:765/777; the sigmaPx lesson).
    void sampleAllLayersEmitsKindGuardedTrueOnly()
    {
        StrokeParams p;
        p.targetLayerId = QStringLiteral("00000000-0005-4000-8000-000000000003");
        p.samplesAssetId = QStringLiteral("00000000-0005-4000-8000-000000000006");

        // (1) spot heal + flag: the boolean rides ALONE — no source key
        // (heal's allOf[1]: mode "spot" FORBIDS source/offset, and the
        // flag is a top-level param, not the source arm).
        p.kind = QStringLiteral("heal.stroke");
        p.mode = QStringLiteral("spot");
        p.sampleAllLayers = true;
        QJsonObject params =
            parse(buildStrokeEnvelope(QStringLiteral("00000000-0005-4000-8000-000000000001"), p))
                .value("params").toObject();
        QCOMPARE(params.value("sampleAllLayers").toBool(), true);
        QVERIFY2(!params.contains("source"),
                 "spot heal must never grow a source key — the schema forbids it");

        // (2) heal WITHOUT the flag: the key is OMITTED (schema default
        // false — the default-omission rule; a stated false would
        // re-state the engine's default).
        p.sampleAllLayers = false;
        params = parse(buildStrokeEnvelope(
                           QStringLiteral("00000000-0005-4000-8000-000000000002"), p))
                     .value("params").toObject();
        QVERIFY(!params.contains("sampleAllLayers"));

        // (3) clone + flag: the §45.2 "all" scope travels as the PAIR —
        // source:"composite" AND sampleAllLayers:true. The boolean alone
        // would mirror the spec's own inexpressible composite+false row.
        p.kind = QStringLiteral("clone.stroke");
        p.mode.clear();
        p.sampleAllLayers = true;
        params = parse(buildStrokeEnvelope(
                           QStringLiteral("00000000-0005-4000-8000-000000000003"), p))
                     .value("params").toObject();
        QCOMPARE(params.value("sampleAllLayers").toBool(), true);
        QCOMPARE(params.value("source").toString(), QStringLiteral("composite"));

        // (4) THE GUARD ITSELF: a stray true on a kind whose schema lacks
        // the key reaches no wire (additionalProperties:false = stage-3
        // violation). This is the arm that lets callers pass the shared
        // property unconditionally.
        p.kind = QStringLiteral("brush.stroke");
        p.sampleAllLayers = true;
        params = parse(buildStrokeEnvelope(
                           QStringLiteral("00000000-0005-4000-8000-000000000004"), p))
                     .value("params").toObject();
        QVERIFY2(!params.contains("sampleAllLayers"),
                 "the kind guard leaked the flag onto brush.stroke");
        QVERIFY(!params.contains("source"));
    }

    // B-02, AND IT IS THE ARM C-678's FINDING ASKED FOR. clone.stroke's
    // required set is [brushPreset, source, offset] UNCONDITIONALLY
    // (clone.stroke.yaml:31). Row 5 emitted `source` only INSIDE the
    // sampleAllLayers branch and `offset` nowhere, so a dispatched clone
    // was missing one or two required params depending on a boolean —
    // latent only because clone never dispatched. Both flag states are
    // asserted here, which is what makes the guard a guard: a fix that
    // only satisfied the true-arm would pass a one-state test.
    void cloneAlwaysCarriesItsRequiredSourceAndOffset()
    {
        StrokeParams p;
        p.kind = QStringLiteral("clone.stroke");
        p.targetLayerId = QStringLiteral("00000000-0005-4000-8000-000000000003");
        p.samplesAssetId = QStringLiteral("00000000-0005-4000-8000-000000000006");
        p.cloneOffset = QPointF(-12.0, 34.5);

        // (1) FLAG OFF — the arm that was broken. source is the §45.2
        // current-layer form {layerId}, NOT composite, and offset rides.
        p.sampleAllLayers = false;
        QJsonObject params =
            parse(buildStrokeEnvelope(QStringLiteral("00000000-0005-4000-8000-000000000011"), p))
                .value("params").toObject();
        QVERIFY2(params.contains("source"),
                 "clone without the flag emitted NO source — the required set is "
                 "[brushPreset, source, offset] unconditionally");
        QCOMPARE(params.value("source").toObject().value("layerId").toString(),
                 p.targetLayerId);
        QVERIFY2(!params.contains("sampleAllLayers"),
                 "default-omission: a stated false re-states the engine's default");
        QVERIFY2(params.contains("offset"), "clone emitted no offset");
        QCOMPARE(params.value("offset").toArray().at(0).toDouble(), -12.0);
        QCOMPARE(params.value("offset").toArray().at(1).toDouble(), 34.5);

        // (2) FLAG ON — the §45.2 PAIR, and offset still rides.
        p.sampleAllLayers = true;
        params = parse(buildStrokeEnvelope(
                           QStringLiteral("00000000-0005-4000-8000-000000000012"), p))
                     .value("params").toObject();
        QCOMPARE(params.value("source").toString(), QStringLiteral("composite"));
        QCOMPARE(params.value("sampleAllLayers").toBool(), true);
        QCOMPARE(params.value("offset").toArray().at(1).toDouble(), 34.5);

        // (3) THE GUARD, and it is what stops this fix leaking: `offset`
        // is meaningless off clone and additionalProperties:false makes a
        // stray one a stage-3 rejection. A non-zero offset on a NON-clone
        // kind must reach no wire — asserted with the same value that
        // rides above, so the check cannot pass by the value being empty.
        p.kind = QStringLiteral("brush.stroke");
        params = parse(buildStrokeEnvelope(
                           QStringLiteral("00000000-0005-4000-8000-000000000013"), p))
                     .value("params").toObject();
        QVERIFY2(!params.contains("offset"),
                 "offset leaked onto brush.stroke — stage-3 rejection");
        QVERIFY(!params.contains("source"));
    }

    void deterministicUuidSourceIsStableAndV4Shaped()
    {
        UuidSource a(42), b(42), c(43);
        const QString a1 = a.next();
        const QString a2 = a.next();
        QCOMPARE(a1, b.next());
        QCOMPARE(a2, b.next());
        QVERIFY(a1 != a2);
        QVERIFY(a1 != c.next());
        QVERIFY(kUuidV4.match(a1).hasMatch());
        QVERIFY(kUuidV4.match(a2).hasMatch());
    }

    void randomUuidSourceIsV4Shaped()
    {
        UuidSource s;
        QVERIFY(kUuidV4.match(s.next()).hasMatch());
    }

    // ---- Pro-Wave row 1: adjustment.add + op.set_mask param shapes ----
    // Pinned against contracts/commands/adjustment.add.yaml and
    // op.set_mask.yaml. Both contracts set additionalProperties:false, so
    // the EXACT key set is the contract, not a detail: a stray key is a
    // stage-3 schema violation and a missing required key is
    // error.schema.missing_field. These tests therefore assert the key set
    // exactly (keys().size()), not merely that the required keys appear.

    void adjustmentAddParamsMatchContractKeySet()
    {
        const QJsonObject p = buildAdjustmentAddParams(
            QStringLiteral("adjustment.invert"),
            QStringLiteral("22222222-0000-4000-8000-0000000000a1"),
            QStringLiteral("33333333-0000-4000-8000-0000000000b1"));
        // required: [opKind, newLayerId, newOpId, params]
        QCOMPARE(p.value("opKind").toString(), QStringLiteral("adjustment.invert"));
        QCOMPARE(p.value("newLayerId").toString(),
                 QStringLiteral("22222222-0000-4000-8000-0000000000a1"));
        QCOMPARE(p.value("newOpId").toString(),
                 QStringLiteral("33333333-0000-4000-8000-0000000000b1"));
        // `params` is REQUIRED — present even when the adjustment takes
        // none. An omitted key here is missing_field, not a default.
        QVERIFY(p.contains(QStringLiteral("params")));
        QVERIFY(p.value("params").isObject());
        QVERIFY(p.value("params").toObject().isEmpty());
        // additionalProperties:false — exactly these four, nothing else.
        QCOMPARE(p.keys().size(), 4);
    }

    void adjustmentAddCarriesOpParamsVerbatim()
    {
        QJsonObject opParams;
        opParams.insert(QStringLiteral("levels"), 4);
        const QJsonObject p = buildAdjustmentAddParams(
            QStringLiteral("adjustment.posterize"),
            QStringLiteral("22222222-0000-4000-8000-0000000000a2"),
            QStringLiteral("33333333-0000-4000-8000-0000000000b2"), opParams);
        // The adjustment's own params are the caller's contract duty and
        // ride verbatim (the standing builder rule) — the builder must not
        // filter, rename, or default them.
        QCOMPARE(p.value("params").toObject(), opParams);
        QCOMPARE(p.keys().size(), 4);
    }

    void opSetMaskEmitsOnlyTheFromArm()
    {
        const QJsonObject p = buildOpSetMaskFromSelectionParams(
            QStringLiteral("33333333-0000-4000-8000-0000000000b1"));
        QCOMPARE(p.value("opId").toString(),
                 QStringLiteral("33333333-0000-4000-8000-0000000000b1"));
        QCOMPARE(p.value("from").toString(), QStringLiteral("selection"));
        // The §40.6 stage-3 oneOf is enforced BY CONSTRUCTION: maskId can
        // never appear alongside `from`, so the contract's
        // both-mask-id-and-from invalid case is unreachable from the shell.
        QVERIFY(!p.contains(QStringLiteral("maskId")));
        QCOMPARE(p.keys().size(), 2);
    }
};

QTEST_GUILESS_MAIN(TstEnvelopeBuilder)
#include "tst_envelope_builder.moc"
