// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts transform.perspective / transform.warp yaml. Pins:
// the quad's TL,TR,BR,BL wire order, the warp preset shape, and the
// §40.6 mesh cross-field ((3c+1)*(3r+1) point count) enforced by
// construction — a mismatch yields the EMPTY object, never a
// silently-wrong envelope.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QTest>

using namespace appname::shell;

class TstTransformBuilders : public QObject
{
    Q_OBJECT

private slots:
    void quadCarriesCornersInOrder();
    void presetShape();
    void meshCountEnforced();
    void meshValidLattice();
    void puppetOptionalFieldsOmitted();
    void puppetEmptyPinsRefused();
};

void TstTransformBuilders::quadCarriesCornersInOrder()
{
    const QPointF quad[4] = {QPointF(0, 0), QPointF(320, 4),
                             QPointF(316, 200), QPointF(2, 196)};
    const QJsonObject p = buildPerspectiveQuadParams(quad);
    const QJsonArray q = p.value(QLatin1String("quad")).toArray();
    QCOMPARE(q.size(), 4);
    // TL,TR,BR,BL — the contract example's winding.
    QCOMPARE(q.at(1).toArray().at(0).toDouble(), 320.0);
    QCOMPARE(q.at(1).toArray().at(1).toDouble(), 4.0);
    QCOMPARE(q.at(3).toArray().at(0).toDouble(), 2.0);
    QCOMPARE(q.at(3).toArray().at(1).toDouble(), 196.0);
}

void TstTransformBuilders::presetShape()
{
    const QJsonObject p =
        buildWarpPresetParams(QStringLiteral("arc"), 35.0);
    const QJsonObject preset = p.value(QLatin1String("preset")).toObject();
    QCOMPARE(preset.value(QLatin1String("kind")).toString(),
             QStringLiteral("arc"));
    QCOMPARE(preset.value(QLatin1String("bendPct")).toDouble(), 35.0);
    QVERIFY(!p.contains(QLatin1String("mesh"))); // arms never mix
}

void TstTransformBuilders::meshCountEnforced()
{
    // 1x1 lattice needs (3+1)*(3+1) = 16 points; 15 must refuse.
    QVector<QPointF> pts(15, QPointF(0, 0));
    QVERIFY(buildWarpMeshParams(1, 1, pts).isEmpty());
    pts.append(QPointF(0, 0));
    QVERIFY(!buildWarpMeshParams(1, 1, pts).isEmpty());
    QVERIFY(buildWarpMeshParams(0, 1, pts).isEmpty()); // bounds guard
}

void TstTransformBuilders::meshValidLattice()
{
    // 4x4 grid: (13)*(13) = 169 points, row-major carried verbatim.
    QVector<QPointF> pts;
    for (int i = 0; i < 169; ++i)
        pts.append(QPointF(i, i * 2));
    const QJsonObject p = buildWarpMeshParams(4, 4, pts);
    const QJsonObject mesh = p.value(QLatin1String("mesh")).toObject();
    QCOMPARE(mesh.value(QLatin1String("cols")).toInt(), 4);
    QCOMPARE(mesh.value(QLatin1String("rows")).toInt(), 4);
    const QJsonArray arr = mesh.value(QLatin1String("points")).toArray();
    QCOMPARE(arr.size(), 169);
    QCOMPARE(arr.at(168).toArray().at(1).toDouble(), 336.0);
}

void TstTransformBuilders::puppetOptionalFieldsOmitted()
{
    // Engine ruling (kind_impls.rs): rotationDeg ABSENT = free rotation
    // -- must be OMITTED, never null; depth 0 = the default -- omitted.
    PuppetPin plain;
    plain.pos = QPointF(300, 200);
    PuppetPin rotated;
    rotated.pos = QPointF(420, 380);
    rotated.hasRotation = true;
    rotated.rotationDeg = 30.0;
    rotated.depth = 1;
    const QJsonObject p =
        buildPuppetParams({plain, rotated});
    const QJsonArray pins = p.value(QLatin1String("pins")).toArray();
    QCOMPARE(pins.size(), 2);
    const QJsonObject p0 = pins.at(0).toObject();
    QVERIFY(!p0.contains(QLatin1String("rotationDeg")));
    QVERIFY(!p0.contains(QLatin1String("depth")));
    const QJsonObject p1 = pins.at(1).toObject();
    QCOMPARE(p1.value(QLatin1String("rotationDeg")).toDouble(), 30.0);
    QCOMPARE(p1.value(QLatin1String("depth")).toInt(), 1);
    // Sentinel knobs omitted = engine defaults never re-stated.
    QVERIFY(!p.contains(QLatin1String("density")));
    QVERIFY(!p.contains(QLatin1String("expansionPx")));
    QVERIFY(!p.contains(QLatin1String("rigidity")));
    const QJsonObject full = buildPuppetParams(
        {plain}, QStringLiteral("more"), 8, 0.5);
    QCOMPARE(full.value(QLatin1String("density")).toString(),
             QStringLiteral("more"));
    QCOMPARE(full.value(QLatin1String("expansionPx")).toInt(), 8);
    QCOMPARE(full.value(QLatin1String("rigidity")).toDouble(), 0.5);
}

void TstTransformBuilders::puppetEmptyPinsRefused()
{
    QVERIFY(buildPuppetParams({}).isEmpty()); // len >= 1 contract floor
}

QTEST_GUILESS_MAIN(TstTransformBuilders)
#include "tst_transform_builders.moc"
