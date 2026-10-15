// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: v1.0.1 items 2+3 support; contract in units.h. The preset px
// pins (tst_units): A4 2480×3508, A5 1748×2480, Letter 2550×3300 (all
// @300dpi), HD 1920×1080, 4K UHD 3840×2160, Square Social 1080×1080.

#include "units.h"

#include <QVariantMap>
#include <QtDebug>
#include <QtMath>

namespace appname::shell {

namespace {

constexpr double kCmPerInch = 2.54;
constexpr double kMmPerInch = 25.4;

const Preset kPresets[] = {
    // Paper @300dpi (physical dims are the truth; px is derived).
    {"ui.preset.a4", 210.0, 297.0, Unit::Mm, 300.0},
    {"ui.preset.a5", 148.0, 210.0, Unit::Mm, 300.0},
    {"ui.preset.letter", 8.5, 11.0, Unit::In, 300.0},
    // Screen (px truth; no dpi component).
    {"ui.preset.hd", 1920.0, 1080.0, Unit::Px, 0.0},
    {"ui.preset.uhd4k", 3840.0, 2160.0, Unit::Px, 0.0},
    {"ui.preset.square_social", 1080.0, 1080.0, Unit::Px, 0.0},
};

} // namespace

double pxFromUnit(double value, Unit unit, double dpi)
{
    if (unit == Unit::Px)
        return value;
    if (dpi <= 0.0)
        return 0.0; // degenerate dpi: visibly wrong, never garbage
    switch (unit) {
    case Unit::In:
        return value * dpi;
    case Unit::Cm:
        return value * dpi / kCmPerInch;
    case Unit::Mm:
        return value * dpi / kMmPerInch;
    case Unit::Px:
        break;
    }
    return value;
}

double unitFromPx(double px, Unit unit, double dpi)
{
    if (unit == Unit::Px)
        return px;
    if (dpi <= 0.0)
        return 0.0;
    switch (unit) {
    case Unit::In:
        return px / dpi;
    case Unit::Cm:
        return px / dpi * kCmPerInch;
    case Unit::Mm:
        return px / dpi * kMmPerInch;
    case Unit::Px:
        break;
    }
    return px;
}

Unit unitFromKey(const QString &key)
{
    if (key == QLatin1String("px"))
        return Unit::Px;
    if (key == QLatin1String("in"))
        return Unit::In;
    if (key == QLatin1String("cm"))
        return Unit::Cm;
    if (key == QLatin1String("mm"))
        return Unit::Mm;
    qWarning() << "units: unknown unit key" << key << "- reading as px";
    return Unit::Px;
}

QString unitKeyOf(Unit unit)
{
    switch (unit) {
    case Unit::Px:
        return QStringLiteral("px");
    case Unit::In:
        return QStringLiteral("in");
    case Unit::Cm:
        return QStringLiteral("cm");
    case Unit::Mm:
        return QStringLiteral("mm");
    }
    return QStringLiteral("px");
}

const Preset *presetTable(int *count)
{
    if (count)
        *count = int(sizeof(kPresets) / sizeof(kPresets[0]));
    return kPresets;
}

UnitsQml::UnitsQml(QObject *parent)
    : QObject(parent)
{
}

double UnitsQml::pxFromUnit(double value, const QString &unitKey,
                            double dpi) const
{
    return appname::shell::pxFromUnit(value, unitFromKey(unitKey), dpi);
}

double UnitsQml::unitFromPx(double px, const QString &unitKey,
                            double dpi) const
{
    return appname::shell::unitFromPx(px, unitFromKey(unitKey), dpi);
}

QVariantList UnitsQml::presets() const
{
    QVariantList out;
    int count = 0;
    const Preset *table = presetTable(&count);
    out.reserve(count);
    for (int i = 0; i < count; ++i) {
        const Preset &p = table[i];
        QVariantMap m;
        m.insert(QStringLiteral("nameKey"), QString::fromLatin1(p.nameKey));
        m.insert(QStringLiteral("unit"), unitKeyOf(p.unit));
        m.insert(QStringLiteral("width"), p.width);
        m.insert(QStringLiteral("height"), p.height);
        m.insert(QStringLiteral("dpi"), p.dpi);
        m.insert(QStringLiteral("widthPx"),
                 qRound(appname::shell::pxFromUnit(p.width, p.unit, p.dpi)));
        m.insert(QStringLiteral("heightPx"),
                 qRound(appname::shell::pxFromUnit(p.height, p.unit, p.dpi)));
        out.append(m);
    }
    return out;
}

} // namespace appname::shell
