// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: font_meta.h — the ABI 1.6 door's host-derived metadata triple
// from a bounds-checked SFNT 'name'-table read. Pins: a synthetic
// in-memory face parses (Windows UTF-16BE preferred over Mac Roman),
// the family+postscript load-bearing pair (missing either refuses),
// the Regular style fallback, and truncation/garbage refusal at every
// bound (the parser never guesses).

#include "font_meta.h"

#include <QTest>

using appname::shell::parseFontNames;

namespace {

void putU16(QByteArray &b, quint16 v)
{
    b.append(char(v >> 8));
    b.append(char(v & 0xFF));
}

void putU32(QByteArray &b, quint32 v)
{
    putU16(b, quint16(v >> 16));
    putU16(b, quint16(v & 0xFFFF));
}

QByteArray utf16be(const QString &s)
{
    QByteArray out;
    for (QChar c : s)
        putU16(out, c.unicode());
    return out;
}

// Minimal single-table SFNT: offset table + one 'name' table record +
// the name table with the given (platform, nameID, value) entries.
QByteArray syntheticFont(
    const QVector<std::tuple<quint16, quint16, QByteArray>> &names)
{
    QByteArray nameTable;
    putU16(nameTable, 0); // format
    putU16(nameTable, quint16(names.size()));
    const qsizetype headerLen = 6 + qsizetype(names.size()) * 12;
    putU16(nameTable, quint16(headerLen)); // stringOffset
    QByteArray strings;
    for (const auto &[platform, nameId, value] : names) {
        putU16(nameTable, platform);
        putU16(nameTable, platform == 3 ? 1 : 0); // encoding
        putU16(nameTable, platform == 3 ? 0x0409 : 0); // language
        putU16(nameTable, nameId);
        putU16(nameTable, quint16(value.size()));
        putU16(nameTable, quint16(strings.size()));
        strings.append(value);
    }
    nameTable.append(strings);

    QByteArray font;
    putU32(font, 0x00010000); // sfnt version
    putU16(font, 1);          // numTables
    putU16(font, 0);          // searchRange (unused by the parser)
    putU16(font, 0);
    putU16(font, 0);
    putU32(font, 0x6E616D65); // 'name'
    putU32(font, 0);          // checksum
    putU32(font, 28);         // offset (12 + 16)
    putU32(font, quint32(nameTable.size()));
    font.append(nameTable);
    return font;
}

} // namespace

class TstFontMeta : public QObject
{
    Q_OBJECT

private slots:
    void windowsNamesPreferred();
    void macFallbackAndRegularStyle();
    void loadBearingPairRequired();
    void garbageRefused();
    void adversarialBoundsRefused();
    void seededFuzzNeverEscapes();
};

void TstFontMeta::windowsNamesPreferred()
{
    const QByteArray font = syntheticFont({
        {1, 1, QByteArray("MacFamily")},
        {3, 1, utf16be(QStringLiteral("WinFamily"))},
        {3, 2, utf16be(QStringLiteral("Bold"))},
        {3, 6, utf16be(QStringLiteral("WinFamily-Bold"))},
    });
    QString family;
    QString style;
    QString ps;
    QVERIFY(parseFontNames(font, &family, &style, &ps));
    QCOMPARE(family, QStringLiteral("WinFamily")); // 3 beats 1
    QCOMPARE(style, QStringLiteral("Bold"));
    QCOMPARE(ps, QStringLiteral("WinFamily-Bold"));
}

void TstFontMeta::macFallbackAndRegularStyle()
{
    // Mac-only names, no style record: family/ps parse, style falls
    // back to Regular.
    const QByteArray font = syntheticFont({
        {1, 1, QByteArray("OnlyMac")},
        {1, 6, QByteArray("OnlyMac-PS")},
    });
    QString family;
    QString style;
    QString ps;
    QVERIFY(parseFontNames(font, &family, &style, &ps));
    QCOMPARE(family, QStringLiteral("OnlyMac"));
    QCOMPARE(style, QStringLiteral("Regular"));
    QCOMPARE(ps, QStringLiteral("OnlyMac-PS"));
}

void TstFontMeta::loadBearingPairRequired()
{
    // Family present, postscript missing -> refuse (and vice versa).
    QVERIFY(!parseFontNames(
        syntheticFont({{3, 1, utf16be(QStringLiteral("F"))}}), nullptr,
        nullptr, nullptr));
    QVERIFY(!parseFontNames(
        syntheticFont({{3, 6, utf16be(QStringLiteral("F-PS"))}}), nullptr,
        nullptr, nullptr));
}

void TstFontMeta::garbageRefused()
{
    QVERIFY(!parseFontNames(QByteArray(), nullptr, nullptr, nullptr));
    QVERIFY(!parseFontNames(QByteArray("hello"), nullptr, nullptr, nullptr));
    QVERIFY(!parseFontNames(QByteArray("ttcf1234"), nullptr, nullptr,
                            nullptr)); // collections out of scope
    // Truncated mid-table: header promises more than the bytes hold.
    QByteArray good = syntheticFont({
        {3, 1, utf16be(QStringLiteral("F"))},
        {3, 6, utf16be(QStringLiteral("F-PS"))},
    });
    QVERIFY(!parseFontNames(good.left(good.size() / 2), nullptr, nullptr,
                            nullptr));
}

void TstFontMeta::adversarialBoundsRefused()
{
    // Reviewer rows (font-feed review): direct adversarial shapes the
    // committed rows reach only incidentally.
    QByteArray good = syntheticFont({
        {3, 1, utf16be(QStringLiteral("F"))},
        {3, 6, utf16be(QStringLiteral("F-PS"))},
    });

    // numTables lying large: header promises a table directory far past
    // the bytes.
    QByteArray lyingTables = good;
    lyingTables[4] = char(0xFF);
    lyingTables[5] = char(0xFF);
    QVERIFY(!parseFontNames(lyingTables, nullptr, nullptr, nullptr));

    // stringOffset past the end: every value read lands out of bounds ->
    // no usable names -> load-bearing refusal (no crash, no escape).
    QByteArray farStrings = good;
    const qsizetype nameOff = 28; // single-table synthetic layout
    farStrings[int(nameOff + 4)] = char(0x7F);
    farStrings[int(nameOff + 5)] = char(0xFF);
    QVERIFY(!parseFontNames(farStrings, nullptr, nullptr, nullptr));

    // Zero-length values: records parse, values skip -> refusal.
    QVERIFY(!parseFontNames(syntheticFont({
        {3, 1, QByteArray()},
        {3, 6, QByteArray()},
    }), nullptr, nullptr, nullptr));
}

void TstFontMeta::seededFuzzNeverEscapes()
{
    // Deterministic xorshift churn: random buffers, and random
    // corruptions of a valid face, must refuse or parse — never crash
    // or read out of bounds (ASan-era guard; seeds fixed for
    // reproducibility).
    quint64 s = 0x9E3779B97F4A7C15ULL;
    auto next = [&s]() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        return s;
    };
    for (int round = 0; round < 300; ++round) {
        const int len = int(next() % 512);
        QByteArray junk(len, 0);
        for (int i = 0; i < len; ++i)
            junk[i] = char(next() & 0xFF);
        parseFontNames(junk, nullptr, nullptr, nullptr); // outcome free; no escape
    }
    const QByteArray base = syntheticFont({
        {3, 1, utf16be(QStringLiteral("Fuzz"))},
        {3, 2, utf16be(QStringLiteral("Bold"))},
        {3, 6, utf16be(QStringLiteral("Fuzz-Bold"))},
    });
    for (int round = 0; round < 300; ++round) {
        QByteArray mut = base;
        const int flips = 1 + int(next() % 8);
        for (int f = 0; f < flips; ++f)
            mut[int(next() % quint64(mut.size()))] = char(next() & 0xFF);
        QString fam;
        QString sty;
        QString ps;
        parseFontNames(mut, &fam, &sty, &ps); // outcome free; no escape
    }
    QVERIFY(true); // reaching here without crash IS the assertion
}

QTEST_GUILESS_MAIN(TstFontMeta)
#include "tst_font_meta.moc"
