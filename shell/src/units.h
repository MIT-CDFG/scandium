// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: v1.0.1 operator feedback items 2+3 (reports/O-UI-log.md "v1.0.1
// OPERATOR FEEDBACK BATCH 1"). Pure PRESENTATION-side unit math for the
// New Document and Resize dialogs: px/in/cm/mm conversions through a
// caller-supplied dpi, plus the new-document preset table as data. NO
// engine math — presets and units are dialog sugar over the UNCHANGED
// doc.new / doc.resize integer-px doors; the rounded px the dialogs
// display is exactly the px they submit.
//
// Conversions (in = px/dpi; cm = in * 2.54; mm = cm * 10; px identity).
// dpi > 0 is the physical-arm precondition — the dialogs gate it (the
// New Document DPI SpinBox floors at 1; Resize disables physical units
// while its ppi field is empty/invalid). A degenerate dpi <= 0 collapses
// the physical arms to 0.0, deliberately: 0 px is visibly wrong and
// unsubmittable (doors floor at 1), where a NaN would render as garbage.

#pragma once

#include <QObject>
#include <QString>
#include <QVariantList>

namespace appname::shell {

enum class Unit { Px, In, Cm, Mm };

// Value in `unit` -> pixels at `dpi`.
double pxFromUnit(double value, Unit unit, double dpi);

// Pixels at `dpi` -> value in `unit`.
double unitFromPx(double px, Unit unit, double dpi);

// The ui.unit.* id suffixes ("px"/"in"/"cm"/"mm") <-> Unit. An unknown
// key warns and reads as Px (a shell-side typo, never user input).
Unit unitFromKey(const QString &key);
QString unitKeyOf(Unit unit);

// One new-document preset: physical presets carry their natural-unit
// dims + the dpi they are specified at (A4/A5/Letter @300); screen
// presets are px with dpi 0 = "no dpi component" (selecting one leaves
// the dialog's DPI field untouched).
struct Preset {
    const char *nameKey; // cataloged ui.preset.* id (MINTED.md batch)
    double width;        // in `unit`
    double height;
    Unit unit;
    double dpi;          // 0 = none (px presets)
};

// The fixed table (Custom is the dialog's own row 0, not data).
const Preset *presetTable(int *count);

// QML face (context property AppUnits): the conversions as Q_INVOKABLEs
// keyed by the ui.unit.* suffix strings, plus the preset table as a
// QVariantList of maps {nameKey, unit, width, height, dpi, widthPx,
// heightPx} — widthPx/heightPx are the ROUNDED ints the dialog both
// displays and submits.
class UnitsQml : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantList presets READ presets CONSTANT)
public:
    explicit UnitsQml(QObject *parent = nullptr);

    Q_INVOKABLE double pxFromUnit(double value, const QString &unitKey,
                                  double dpi) const;
    Q_INVOKABLE double unitFromPx(double px, const QString &unitKey,
                                  double dpi) const;

    QVariantList presets() const;
};

} // namespace appname::shell
