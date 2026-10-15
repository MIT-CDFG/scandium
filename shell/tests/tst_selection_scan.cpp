// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.3 selection-bounds support (selection_scan.h). Pins the
// pure strip accumulator on synthetic Grayscale8 masks: empty strips,
// single pixels, edges, faint (value 1) coverage, cross-strip unions
// (the 2^26-cap chunking shape), odd widths (stride ≠ width — scanLine
// must be the row source, not flat indexing), and running-rect identity.

#include "selection_scan.h"

#include <QtTest>

using namespace appname::shell;

namespace {

QImage grayImage(int w, int h, uchar fill = 0)
{
    QImage img(w, h, QImage::Format_Grayscale8);
    img.fill(fill);
    return img;
}

} // namespace

class TstSelectionScan : public QObject {
    Q_OBJECT
private slots:
    void emptyStripKeepsRunning();
    void singlePixelBounds();
    void faintCoverageCounts();
    void crossStripUnion();
    void oddWidthStrideSafe();
};

void TstSelectionScan::emptyStripKeepsRunning()
{
    const QImage strip = grayImage(64, 16);
    QCOMPARE(accumulateMaskBounds(strip, 0, QRect()), QRect());
    const QRect running(3, 4, 5, 6);
    QCOMPARE(accumulateMaskBounds(strip, 100, running), running);
}

void TstSelectionScan::singlePixelBounds()
{
    QImage strip = grayImage(64, 16);
    strip.setPixelColor(63, 15, QColor(255, 255, 255));
    QCOMPARE(accumulateMaskBounds(strip, 0, QRect()), QRect(63, 15, 1, 1));
    // yOffset shifts document-space rows.
    QCOMPARE(accumulateMaskBounds(strip, 200, QRect()),
             QRect(63, 215, 1, 1));
}

void TstSelectionScan::faintCoverageCounts()
{
    // Fractional coverage (anti-aliased selection edges) is value 1..254
    // — nonzero IS selected coverage; the bounds must include it.
    QImage strip = grayImage(8, 8);
    strip.setPixelColor(2, 3, QColor(1, 1, 1));
    strip.setPixelColor(5, 6, QColor(254, 254, 254));
    QCOMPARE(accumulateMaskBounds(strip, 0, QRect()), QRect(2, 3, 4, 4));
}

void TstSelectionScan::crossStripUnion()
{
    // The KS-2 shape: a 400x160 selection contract-centered on 640x400,
    // scanned in two strips split mid-selection.
    QImage top = grayImage(640, 200);
    QImage bottom = grayImage(640, 200);
    for (int y = 120; y < 200; ++y)
        for (int x = 120; x < 520; ++x)
            top.setPixelColor(x, y, QColor(255, 255, 255));
    for (int y = 0; y < 80; ++y)
        for (int x = 120; x < 520; ++x)
            bottom.setPixelColor(x, y, QColor(255, 255, 255));
    QRect bounds = accumulateMaskBounds(top, 0, QRect());
    bounds = accumulateMaskBounds(bottom, 200, bounds);
    QCOMPARE(bounds, QRect(120, 120, 400, 160));
}

void TstSelectionScan::oddWidthStrideSafe()
{
    // Odd width forces bytesPerLine != width (4-byte alignment): flat
    // indexing would smear rows; scanLine-based scanning must not.
    QImage strip = grayImage(37, 3);
    strip.setPixelColor(36, 1, QColor(255, 255, 255));
    QCOMPARE(accumulateMaskBounds(strip, 0, QRect()), QRect(36, 1, 1, 1));
}

QTEST_GUILESS_MAIN(TstSelectionScan)
#include "tst_selection_scan.moc"
