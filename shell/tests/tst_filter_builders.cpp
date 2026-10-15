// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts filter.apply.yaml + the engine's liquify registry
// citizen (corpus/liquify.rs — the §16.3 field payload as the wire
// params, recorded pin). Pins: the filterId+params carriage, fade-pass
// omission at sentinels, and the liquify field-shape cross-field
// (disp count == 2*nx*ny; pitch in {4,8,16,32}) enforced by
// construction — mismatches yield the EMPTY object.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QTest>

using namespace appname::shell;

class TstFilterBuilders : public QObject
{
    Q_OBJECT

private slots:
    void applyCarriesIdAndParams();
    void fadePassOmittedAtSentinels();
    void liquifyFieldShapeEnforced();
    void liquifyValidField();
};

void TstFilterBuilders::applyCarriesIdAndParams()
{
    // `radius` is the ENGINE's real gaussian param (corpus/gaussian.rs;
    // the contract example's `sigmaPx` is a dead name the engine
    // silently ignores — review D1, drift filed upstream).
    QJsonObject fp;
    fp.insert(QLatin1String("radius"), 18.0);
    const QJsonObject p = buildFilterApplyParams(
        QStringLiteral("filter.gaussian_blur"), fp);
    QCOMPARE(p.value(QLatin1String("filterId")).toString(),
             QStringLiteral("filter.gaussian_blur"));
    QCOMPARE(p.value(QLatin1String("params")).toObject()
                 .value(QLatin1String("radius")).toDouble(), 18.0);
    QVERIFY(buildFilterApplyParams(QString(), fp).isEmpty());
}

void TstFilterBuilders::fadePassOmittedAtSentinels()
{
    const QJsonObject bare = buildFilterApplyParams(
        QStringLiteral("filter.median"), QJsonObject());
    QVERIFY(!bare.contains(QLatin1String("blend")));
    QVERIFY(!bare.contains(QLatin1String("opacity")));
    const QJsonObject faded = buildFilterApplyParams(
        QStringLiteral("filter.median"), QJsonObject(),
        QStringLiteral("luminosity"), 0.8);
    QCOMPARE(faded.value(QLatin1String("blend")).toString(),
             QStringLiteral("luminosity"));
    QCOMPARE(faded.value(QLatin1String("opacity")).toDouble(), 0.8);
}

void TstFilterBuilders::liquifyFieldShapeEnforced()
{
    QVector<double> disp(2 * 3 * 2, 0.0); // 3x2 nodes -> 12 numbers
    QVERIFY(!buildLiquifyFieldParams(16, QPointF(0, 0), 3, 2, disp).isEmpty());
    disp.removeLast();
    QVERIFY(buildLiquifyFieldParams(16, QPointF(0, 0), 3, 2, disp).isEmpty());
    disp.append(0.0);
    QVERIFY(buildLiquifyFieldParams(15, QPointF(0, 0), 3, 2, disp).isEmpty());
    QVERIFY(buildLiquifyFieldParams(16, QPointF(0, 0), 0, 2, disp).isEmpty());
}

void TstFilterBuilders::liquifyValidField()
{
    QVector<double> disp;
    for (int i = 0; i < 12; ++i)
        disp.append(double(i));
    const QJsonObject p =
        buildLiquifyFieldParams(32, QPointF(8, 4), 3, 2, disp);
    QCOMPARE(p.value(QLatin1String("meshPitch")).toInt(), 32);
    const QJsonArray origin = p.value(QLatin1String("origin")).toArray();
    QCOMPARE(origin.at(0).toDouble(), 8.0);
    const QJsonArray size = p.value(QLatin1String("size")).toArray();
    QCOMPARE(size.at(0).toInt(), 3);
    QCOMPARE(size.at(1).toInt(), 2);
    const QJsonArray d = p.value(QLatin1String("disp")).toArray();
    QCOMPARE(d.size(), 12);
    QCOMPARE(d.at(11).toDouble(), 11.0); // row-major carried verbatim
}

QTEST_GUILESS_MAIN(TstFilterBuilders)
#include "tst_filter_builders.moc"
