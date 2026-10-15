// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.3 selection-bounds announcement support; contract in
// selection_scan.h. Each scanner is pinned to its door's delivery format
// (Grayscale8 for readSelectionImage coverage, RGBA8888 for readLayer
// content); any other format is a caller bug surfaced loudly.

#include "selection_scan.h"

namespace appname::shell {

QRect accumulateMaskBounds(const QImage &strip, int yOffset,
                           const QRect &running)
{
    Q_ASSERT(strip.format() == QImage::Format_Grayscale8);
    QRect out = running;
    for (int y = 0; y < strip.height(); ++y) {
        const uchar *line = strip.constScanLine(y);
        int x0 = -1;
        int x1 = -1;
        for (int x = 0; x < strip.width(); ++x) {
            if (line[x]) {
                if (x0 < 0)
                    x0 = x;
                x1 = x;
            }
        }
        if (x0 >= 0) {
            const QRect rowRect(x0, yOffset + y, x1 - x0 + 1, 1);
            out = out.isNull() ? rowRect : out.united(rowRect);
        }
    }
    return out;
}

QRect accumulateAlphaBounds(const QImage &strip, int yOffset,
                            const QRect &running)
{
    Q_ASSERT(strip.format() == QImage::Format_RGBA8888);
    QRect out = running;
    for (int y = 0; y < strip.height(); ++y) {
        const uchar *line = strip.constScanLine(y);
        int x0 = -1;
        int x1 = -1;
        for (int x = 0; x < strip.width(); ++x) {
            if (line[x * 4 + 3]) { // straight-alpha byte (delivery contract)
                if (x0 < 0)
                    x0 = x;
                x1 = x;
            }
        }
        if (x0 >= 0) {
            const QRect rowRect(x0, yOffset + y, x1 - x0 + 1, 1);
            out = out.isNull() ? rowRect : out.united(rowRect);
        }
    }
    return out;
}

} // namespace appname::shell
