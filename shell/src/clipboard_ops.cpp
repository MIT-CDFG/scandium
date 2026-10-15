// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header.

#include "clipboard_ops.h"

namespace appname::shell {

bool applySelectionCoverage(QImage *rgba, const QImage &mask)
{
    if (!rgba || rgba->isNull() || mask.isNull()
        || rgba->format() != QImage::Format_RGBA8888
        || mask.format() != QImage::Format_Grayscale8
        || rgba->size() != mask.size())
        return false;
    for (int y = 0; y < rgba->height(); ++y) {
        uchar *px = rgba->scanLine(y);
        const uchar *cov = mask.constScanLine(y);
        for (int x = 0; x < rgba->width(); ++x) {
            // Straight alpha: ONLY the alpha byte scales (§13.1 coverage);
            // color bytes stay verbatim so fully-covered pixels round-trip
            // bit-exact.
            const uint a = px[x * 4 + 3];
            px[x * 4 + 3] = static_cast<uchar>((a * cov[x] + 127) / 255);
        }
    }
    return true;
}

} // namespace appname::shell
