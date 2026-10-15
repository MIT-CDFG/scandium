// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: v1.0.1 items 2+3 (units.h). Pins the pure conversion math and
// the preset table EXACTLY: the six preset px pairs the operator named
// (A4 2480×3508 / A5 1748×2480 / Letter 2550×3300 @300dpi; HD 1920×1080;
// 4K UHD 3840×2160; Square Social 1080×1080), round-trips across units
// and dpi values, dpi edge cases (72/300/9600, the degenerate <=0 arm),
// mm↔px exactness at the A4 values, and the QVariantList face the QML
// dialogs bind (same rounded ints the doors receive).

#include "units.h"

#include <QtTest>

using namespace appname::shell;

namespace {

struct PresetPin {
    const char *nameKey;
    int widthPx;
    int heightPx;
};

// The operator's table, verbatim.
const PresetPin kPins[] = {
    {"ui.preset.a4", 2480, 3508},
    {"ui.preset.a5", 1748, 2480},
    {"ui.preset.letter", 2550, 3300},
    {"ui.preset.hd", 1920, 1080},
    {"ui.preset.uhd4k", 3840, 2160},
    {"ui.preset.square_social", 1080, 1080},
};

} // namespace

class TstUnits : public QObject {
    Q_OBJECT
private slots:
    void pxIdentity();
    void inchConversions();
    void cmMmConversions();
    void mmExactnessAtA4();
    void roundTrips();
    void degenerateDpi();
    void unitKeys();
    void presetTablePins();
    void qmlFaceParity();
    void qmlPresetList();
};

void TstUnits::pxIdentity()
{
    // px never touches dpi — identity at ANY dpi, including garbage.
    QCOMPARE(pxFromUnit(1024.0, Unit::Px, 300.0), 1024.0);
    QCOMPARE(pxFromUnit(1024.0, Unit::Px, 0.0), 1024.0);
    QCOMPARE(pxFromUnit(1024.0, Unit::Px, -5.0), 1024.0);
    QCOMPARE(unitFromPx(768.0, Unit::Px, 72.0), 768.0);
    QCOMPARE(unitFromPx(768.0, Unit::Px, 0.0), 768.0);
}

void TstUnits::inchConversions()
{
    QCOMPARE(pxFromUnit(1.0, Unit::In, 300.0), 300.0);
    QCOMPARE(pxFromUnit(1.0, Unit::In, 72.0), 72.0);
    QCOMPARE(pxFromUnit(8.5, Unit::In, 300.0), 2550.0);
    QCOMPARE(pxFromUnit(11.0, Unit::In, 300.0), 3300.0);
    QCOMPARE(unitFromPx(300.0, Unit::In, 300.0), 1.0);
    QCOMPARE(unitFromPx(144.0, Unit::In, 72.0), 2.0);
    // 9600 is the New Document DPI SpinBox ceiling.
    QCOMPARE(pxFromUnit(1.0, Unit::In, 9600.0), 9600.0);
    QCOMPARE(unitFromPx(9600.0, Unit::In, 9600.0), 1.0);
}

void TstUnits::cmMmConversions()
{
    // cm = in * 2.54; mm = cm * 10 — one inch of pixels reads back as
    // exactly 2.54 cm / 25.4 mm (same-literal division is exact).
    QCOMPARE(unitFromPx(300.0, Unit::Cm, 300.0), 2.54);
    QCOMPARE(unitFromPx(300.0, Unit::Mm, 300.0), 25.4);
    QCOMPARE(pxFromUnit(2.54, Unit::Cm, 300.0), 300.0);
    QCOMPARE(pxFromUnit(25.4, Unit::Mm, 300.0), 300.0);
    // The two physical arms agree with each other: 1 cm == 10 mm.
    QCOMPARE(pxFromUnit(1.0, Unit::Cm, 300.0),
             pxFromUnit(10.0, Unit::Mm, 300.0));
}

void TstUnits::mmExactnessAtA4()
{
    // The A4 pins land on the operator's ints at 300dpi...
    QCOMPARE(qRound(pxFromUnit(210.0, Unit::Mm, 300.0)), 2480);
    QCOMPARE(qRound(pxFromUnit(297.0, Unit::Mm, 300.0)), 3508);
    // ...and at 72dpi on the PostScript-point A4 (595×842 — the
    // familiar PDF page box; a wrong mm/in factor misses these).
    QCOMPARE(qRound(pxFromUnit(210.0, Unit::Mm, 72.0)), 595);
    QCOMPARE(qRound(pxFromUnit(297.0, Unit::Mm, 72.0)), 842);
}

void TstUnits::roundTrips()
{
    const Unit units[] = {Unit::Px, Unit::In, Unit::Cm, Unit::Mm};
    const double dpis[] = {72.0, 96.0, 300.0, 600.0, 9600.0};
    const double values[] = {0.001, 1.0, 8.5, 210.0, 1000000.0};
    for (Unit u : units)
        for (double dpi : dpis)
            for (double v : values) {
                // QCOMPARE on doubles is qFuzzyCompare — relative.
                QCOMPARE(unitFromPx(pxFromUnit(v, u, dpi), u, dpi), v);
                QCOMPARE(pxFromUnit(unitFromPx(v, u, dpi), u, dpi), v);
            }
}

void TstUnits::degenerateDpi()
{
    // Documented degenerate arm: physical units at dpi <= 0 collapse to
    // 0.0 (visibly wrong and unsubmittable — the doors floor at 1px)
    // instead of propagating a NaN into the dialogs' computed-px row.
    QCOMPARE(pxFromUnit(210.0, Unit::Mm, 0.0), 0.0);
    QCOMPARE(pxFromUnit(8.5, Unit::In, -300.0), 0.0);
    QCOMPARE(unitFromPx(2480.0, Unit::Mm, 0.0), 0.0);
}

void TstUnits::unitKeys()
{
    // The ui.unit.* suffixes are the wire between QML and the helper.
    QCOMPARE(unitFromKey(QStringLiteral("px")), Unit::Px);
    QCOMPARE(unitFromKey(QStringLiteral("in")), Unit::In);
    QCOMPARE(unitFromKey(QStringLiteral("cm")), Unit::Cm);
    QCOMPARE(unitFromKey(QStringLiteral("mm")), Unit::Mm);
    QCOMPARE(unitKeyOf(Unit::Px), QStringLiteral("px"));
    QCOMPARE(unitKeyOf(Unit::In), QStringLiteral("in"));
    QCOMPARE(unitKeyOf(Unit::Cm), QStringLiteral("cm"));
    QCOMPARE(unitKeyOf(Unit::Mm), QStringLiteral("mm"));
    // Round-trip over the enum.
    for (Unit u : {Unit::Px, Unit::In, Unit::Cm, Unit::Mm})
        QCOMPARE(unitFromKey(unitKeyOf(u)), u);
}

void TstUnits::presetTablePins()
{
    int count = 0;
    const Preset *table = presetTable(&count);
    QCOMPARE(count, 6);
    for (int i = 0; i < count; ++i) {
        const Preset &p = table[i];
        QCOMPARE(QString::fromLatin1(p.nameKey),
                 QString::fromLatin1(kPins[i].nameKey));
        QCOMPARE(qRound(pxFromUnit(p.width, p.unit, p.dpi)),
                 kPins[i].widthPx);
        QCOMPARE(qRound(pxFromUnit(p.height, p.unit, p.dpi)),
                 kPins[i].heightPx);
        // Physical presets carry a real dpi; screen presets carry none
        // (0 = "leave the dialog's DPI field untouched").
        if (p.unit == Unit::Px)
            QCOMPARE(p.dpi, 0.0);
        else
            QCOMPARE(p.dpi, 300.0);
    }
}

void TstUnits::qmlFaceParity()
{
    const UnitsQml qml;
    QCOMPARE(qml.pxFromUnit(210.0, QStringLiteral("mm"), 300.0),
             pxFromUnit(210.0, Unit::Mm, 300.0));
    QCOMPARE(qml.unitFromPx(2550.0, QStringLiteral("in"), 300.0),
             unitFromPx(2550.0, Unit::In, 300.0));
    QCOMPARE(qml.pxFromUnit(1024.0, QStringLiteral("px"), 0.0), 1024.0);
}

void TstUnits::qmlPresetList()
{
    const UnitsQml qml;
    const QVariantList list = qml.presets();
    QCOMPARE(list.size(), 6);
    for (int i = 0; i < list.size(); ++i) {
        const QVariantMap m = list.at(i).toMap();
        QCOMPARE(m.value(QStringLiteral("nameKey")).toString(),
                 QString::fromLatin1(kPins[i].nameKey));
        // The rounded ints the dialog displays AND submits.
        QCOMPARE(m.value(QStringLiteral("widthPx")).toInt(),
                 kPins[i].widthPx);
        QCOMPARE(m.value(QStringLiteral("heightPx")).toInt(),
                 kPins[i].heightPx);
        // The natural-unit dims reproduce the px pins through the same
        // helper the dialog calls (the list is self-consistent).
        QCOMPARE(qRound(qml.pxFromUnit(
                     m.value(QStringLiteral("width")).toDouble(),
                     m.value(QStringLiteral("unit")).toString(),
                     m.value(QStringLiteral("dpi")).toDouble())),
                 kPins[i].widthPx);
    }
    // Spot pins: A4 is mm@300, HD is px with no dpi component.
    const QVariantMap a4 = list.at(0).toMap();
    QCOMPARE(a4.value(QStringLiteral("unit")).toString(),
             QStringLiteral("mm"));
    QCOMPARE(a4.value(QStringLiteral("width")).toDouble(), 210.0);
    QCOMPARE(a4.value(QStringLiteral("height")).toDouble(), 297.0);
    QCOMPARE(a4.value(QStringLiteral("dpi")).toDouble(), 300.0);
    const QVariantMap hd = list.at(3).toMap();
    QCOMPARE(hd.value(QStringLiteral("unit")).toString(),
             QStringLiteral("px"));
    QCOMPARE(hd.value(QStringLiteral("dpi")).toDouble(), 0.0);
}

QTEST_GUILESS_MAIN(TstUnits)
#include "tst_units.moc"
