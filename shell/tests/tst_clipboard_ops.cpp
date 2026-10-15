// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §26.1 rule 1 + §40.5 + §13.1 — the Cut pairing's app-side
// coverage mask. Pins: full coverage is the identity (bit-exact
// round-trip), zero coverage drops alpha only (color bytes verbatim),
// fractional coverage scales with round-half-up exactly as edit.cut
// scales removed alpha, and shape/format mismatches refuse without
// touching the image.

#include "clipboard_ops.h"

#include <QTest>

using appname::shell::applySelectionCoverage;

namespace {

QImage rgbaFixture(int w, int h)
{
    QImage img(w, h, QImage::Format_RGBA8888);
    for (int y = 0; y < h; ++y) {
        uchar *px = img.scanLine(y);
        for (int x = 0; x < w; ++x) {
            px[x * 4 + 0] = static_cast<uchar>(10 + x);
            px[x * 4 + 1] = static_cast<uchar>(20 + y);
            px[x * 4 + 2] = static_cast<uchar>(30 + x + y);
            px[x * 4 + 3] = static_cast<uchar>(200);
        }
    }
    return img;
}

QImage maskFixture(int w, int h, uchar value)
{
    QImage m(w, h, QImage::Format_Grayscale8);
    m.fill(value);
    return m;
}

} // namespace

class TstClipboardOps : public QObject
{
    Q_OBJECT

private slots:
    void fullCoverageIsIdentity();
    void zeroCoverageDropsAlphaOnly();
    void fractionalCoverageScalesRounded();
    void gradientMaskAppliesPerPixel();
    void dimensionMismatchRefusesUntouched();
    void formatMismatchRefusesUntouched();
};

void TstClipboardOps::fullCoverageIsIdentity()
{
    QImage img = rgbaFixture(5, 4);
    const QImage before = img;
    QVERIFY(applySelectionCoverage(&img, maskFixture(5, 4, 255)));
    QCOMPARE(img, before); // (a*255+127)/255 == a for every a
}

void TstClipboardOps::zeroCoverageDropsAlphaOnly()
{
    QImage img = rgbaFixture(3, 3);
    const QImage before = img;
    QVERIFY(applySelectionCoverage(&img, maskFixture(3, 3, 0)));
    for (int y = 0; y < 3; ++y) {
        const uchar *px = img.constScanLine(y);
        const uchar *was = before.constScanLine(y);
        for (int x = 0; x < 3; ++x) {
            QCOMPARE(px[x * 4 + 0], was[x * 4 + 0]);
            QCOMPARE(px[x * 4 + 1], was[x * 4 + 1]);
            QCOMPARE(px[x * 4 + 2], was[x * 4 + 2]);
            QCOMPARE(px[x * 4 + 3], uchar(0));
        }
    }
}

void TstClipboardOps::fractionalCoverageScalesRounded()
{
    QImage img = rgbaFixture(2, 1); // alpha 200 everywhere
    QVERIFY(applySelectionCoverage(&img, maskFixture(2, 1, 128)));
    // (200*128 + 127) / 255 = 100 (round-half-up integer form)
    QCOMPARE(img.constScanLine(0)[3], uchar(100));
    QCOMPARE(img.constScanLine(0)[7], uchar(100));
}

void TstClipboardOps::gradientMaskAppliesPerPixel()
{
    // Reviewer pin (Wave A re-review): every other row uses a UNIFORM
    // mask, which an x/y-transposed or constant-index coverage lookup
    // would still pass. A non-uniform mask pins the per-pixel (x,y)
    // correspondence: cov varies along BOTH axes, checked cell by cell
    // against the same (a*cov+127)/255 form the engine pairing uses.
    QImage img = rgbaFixture(4, 3); // alpha 200 everywhere
    QImage mask(4, 3, QImage::Format_Grayscale8);
    for (int y = 0; y < 3; ++y) {
        uchar *row = mask.scanLine(y);
        for (int x = 0; x < 4; ++x)
            row[x] = static_cast<uchar>(60 * x + 17 * y); // distinct per cell
    }
    QVERIFY(applySelectionCoverage(&img, mask));
    for (int y = 0; y < 3; ++y) {
        const uchar *px = img.constScanLine(y);
        for (int x = 0; x < 4; ++x) {
            const uint cov = 60u * x + 17u * y;
            const uchar expect = static_cast<uchar>((200u * cov + 127u) / 255u);
            QVERIFY2(px[x * 4 + 3] == expect,
                     qPrintable(QStringLiteral("alpha at (%1,%2): %3 != expected %4 (cov %5)")
                                    .arg(x).arg(y)
                                    .arg(px[x * 4 + 3]).arg(expect).arg(cov)));
        }
    }
}

void TstClipboardOps::dimensionMismatchRefusesUntouched()
{
    QImage img = rgbaFixture(4, 4);
    const QImage before = img;
    QVERIFY(!applySelectionCoverage(&img, maskFixture(3, 4, 0)));
    QCOMPARE(img, before);
}

void TstClipboardOps::formatMismatchRefusesUntouched()
{
    QImage rgb32 = rgbaFixture(2, 2).convertToFormat(QImage::Format_RGB32);
    const QImage before = rgb32;
    QVERIFY(!applySelectionCoverage(&rgb32, maskFixture(2, 2, 0)));
    QCOMPARE(rgb32, before);

    QImage img = rgbaFixture(2, 2);
    const QImage before2 = img;
    const QImage badMask =
        maskFixture(2, 2, 0).convertToFormat(QImage::Format_RGBA8888);
    QVERIFY(!applySelectionCoverage(&img, badMask));
    QCOMPARE(img, before2);
}

QTEST_GUILESS_MAIN(TstClipboardOps)
#include "tst_clipboard_ops.moc"
