// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: contracts text.add/text.edit/text.set_style yaml (slice T
// builders). Pins: Unicode SCALAR counting (ranges count scalars, not
// UTF-16 units — QString::size() over-counts astral chars, the R14
// lesson class), the §40.7 layout cross-fields enforced by
// construction, the §23.1 run-model invariant (empty string = EMPTY
// runs array, never a zero-length run), and the whole-string range
// shape.

#include "envelope_builder.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QTest>

using namespace appname::shell;

class TstTextBuilders : public QObject
{
    Q_OBJECT

private slots:
    void scalarCount_data();
    void scalarCount();
    void scalarCountAdversarial_data();
    void scalarCountAdversarial();
    void pointParamsCrossFields();
    void paragraphParamsCrossFields();
    void styleCarriedVerbatimAndOmittedWhenEmpty();
    void emptyStringIsEmptyRuns();
    void singleRunCarriesText();
    void wholeRangeCountsScalars();
};

void TstTextBuilders::scalarCount_data()
{
    QTest::addColumn<QString>("text");
    QTest::addColumn<int>("expected");

    QTest::newRow("empty") << QString() << 0;
    QTest::newRow("ascii") << QStringLiteral("abc") << 3;
    // U+1D11E MUSICAL SYMBOL G CLEF: ONE scalar, TWO UTF-16 units —
    // QString::size() would say 2; the range family rule says 1.
    QTest::newRow("astral") << QString::fromUcs4(U"\U0001D11E") << 1;
    QTest::newRow("astral-embedded")
        << (QStringLiteral("a") + QString::fromUcs4(U"\U0001D11E")
            + QStringLiteral("b"))
        << 3;
    // e + COMBINING ACUTE: two SCALARS (grapheme clustering is a
    // display concern, not the range unit).
    QTest::newRow("combining")
        << QString::fromUcs4(U"é") << 2;
}

void TstTextBuilders::scalarCountAdversarial_data()
{
    QTest::addColumn<QString>("text");
    QTest::addColumn<int>("expected");

    // Reviewer rows (slice T review): the header claims unpaired
    // surrogates count as ONE scalar each (they round-trip as
    // replacement scalars on the UTF-8 wire — still one scalar). Pin
    // the adversarial shapes the walk must not mis-step on; the test
    // body also cross-checks against Qt's own decoder (toUcs4() maps
    // each unpaired surrogate to one U+FFFD).
    QString loneHigh; loneHigh.append(QChar(0xD834));
    QTest::newRow("lone-high-surrogate") << loneHigh << 1;
    QString loneLow; loneLow.append(QChar(0xDD1E));
    QTest::newRow("lone-low-surrogate") << loneLow << 1;
    QString highAtEnd = QStringLiteral("ab"); highAtEnd.append(QChar(0xD834));
    QTest::newRow("high-surrogate-at-end") << highAtEnd << 3;
    QString lowThenHigh; lowThenHigh.append(QChar(0xDD1E)); lowThenHigh.append(QChar(0xD834));
    QTest::newRow("reversed-pair") << lowThenHigh << 2;
    QString highThenPair; highThenPair.append(QChar(0xD834));
    highThenPair.append(QChar(0xD834)); highThenPair.append(QChar(0xDD1E));
    QTest::newRow("lone-high-then-valid-pair") << highThenPair << 2;
}

void TstTextBuilders::scalarCountAdversarial()
{
    QFETCH(QString, text);
    QFETCH(int, expected);
    QCOMPARE(unicodeScalarCount(text), expected);
    // Oracle cross-check (reviewer): Qt's own scalar decoding agrees on
    // every unpaired-surrogate shape.
    QCOMPARE(qsizetype(unicodeScalarCount(text)), text.toUcs4().size());
}

void TstTextBuilders::scalarCount()
{
    QFETCH(QString, text);
    QFETCH(int, expected);
    QCOMPARE(unicodeScalarCount(text), expected);
    QCOMPARE(buildWholeStringRange(text).last().toInt(), expected);
    // Oracle cross-check (reviewer): Qt's decoder agrees on the
    // committed rows too.
    QCOMPARE(qsizetype(unicodeScalarCount(text)), text.toUcs4().size());
}

void TstTextBuilders::pointParamsCrossFields()
{
    const QJsonObject p = buildTextAddPointParams(
        QPointF(12.5, 40.0), QStringLiteral("hi"),
        QStringLiteral("00000000-0000-4000-8000-000000000001"));
    QCOMPARE(p.value(QLatin1String("layout")).toString(),
             QStringLiteral("point"));
    const QJsonArray origin = p.value(QLatin1String("origin")).toArray();
    QCOMPARE(origin.size(), 2);
    QCOMPARE(origin.at(0).toDouble(), 12.5);
    QCOMPARE(origin.at(1).toDouble(), 40.0);
    QVERIFY(!p.contains(QLatin1String("rect"))); // cross-field: point never carries rect
    QCOMPARE(p.value(QLatin1String("text")).toString(), QStringLiteral("hi"));
    QVERIFY(p.contains(QLatin1String("newLayerId"))); // §9.6 client id
}

void TstTextBuilders::paragraphParamsCrossFields()
{
    const QJsonObject p = buildTextAddParagraphParams(
        QRect(3, 4, 100, 60), QString(),
        QStringLiteral("00000000-0000-4000-8000-000000000002"));
    QCOMPARE(p.value(QLatin1String("layout")).toString(),
             QStringLiteral("paragraph"));
    const QJsonObject r = p.value(QLatin1String("rect")).toObject();
    QCOMPARE(r.value(QLatin1String("x")).toInt(), 3);
    QCOMPARE(r.value(QLatin1String("y")).toInt(), 4);
    QCOMPARE(r.value(QLatin1String("w")).toInt(), 100);
    QCOMPARE(r.value(QLatin1String("h")).toInt(), 60);
    QVERIFY(!p.contains(QLatin1String("origin"))); // cross-field: paragraph never carries origin
}

void TstTextBuilders::styleCarriedVerbatimAndOmittedWhenEmpty()
{
    // 11d Phase 1: the optional styledRunStyle argument — carried
    // VERBATIM when non-empty, ABSENT from the wire when empty (§2.8:
    // never serialized null/empty; the default keeps every pre-11d call
    // site and the tour byte-identical). Both builders share the rule.
    QJsonObject style;
    style.insert(QLatin1String("size"), 301.0);
    const QJsonObject pt = buildTextAddPointParams(
        QPointF(1.0, 2.0), QString(),
        QStringLiteral("00000000-0000-4000-8000-000000000003"), style);
    QCOMPARE(pt.value(QLatin1String("style")).toObject()
                 .value(QLatin1String("size")).toDouble(),
             301.0);
    const QJsonObject para = buildTextAddParagraphParams(
        QRect(0, 0, 10, 10), QString(),
        QStringLiteral("00000000-0000-4000-8000-000000000004"), style);
    QCOMPARE(para.value(QLatin1String("style")).toObject()
                 .value(QLatin1String("size")).toDouble(),
             301.0);
    // Empty style (the default argument) = the pre-11d param shape,
    // field-for-field: byte-compatibility is load-bearing (the tour and
    // every default interactive submission ride this arm).
    const QJsonObject bare = buildTextAddPointParams(
        QPointF(1.0, 2.0), QString(),
        QStringLiteral("00000000-0000-4000-8000-000000000003"));
    QVERIFY(!bare.contains(QLatin1String("style")));
    QJsonObject viaEmpty = buildTextAddPointParams(
        QPointF(1.0, 2.0), QString(),
        QStringLiteral("00000000-0000-4000-8000-000000000003"),
        QJsonObject());
    QCOMPARE(viaEmpty, bare);
}

void TstTextBuilders::emptyStringIsEmptyRuns()
{
    // §23.1: zero-length runs forbidden — the empty string is the empty
    // ARRAY, not [{text:""}].
    QCOMPARE(buildSingleRunReplacement(QString()).size(), 0);
}

void TstTextBuilders::singleRunCarriesText()
{
    const QJsonArray runs =
        buildSingleRunReplacement(QStringLiteral("Appname"));
    QCOMPARE(runs.size(), 1);
    QCOMPARE(runs.first().toObject().value(QLatin1String("text")).toString(),
             QStringLiteral("Appname"));
}

void TstTextBuilders::wholeRangeCountsScalars()
{
    const QJsonArray range = buildWholeStringRange(
        QStringLiteral("ab") + QString::fromUcs4(U"\U0001D11E"));
    QCOMPARE(range.first().toInt(), 0);
    QCOMPARE(range.last().toInt(), 3); // 3 scalars, 4 UTF-16 units
}

QTEST_GUILESS_MAIN(TstTextBuilders)
#include "tst_text_builders.moc"
