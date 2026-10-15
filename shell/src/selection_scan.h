// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.3 (a11y.canvas.selection_bounds — "Selection: {width} by
// {height} pixels", announced on settle from the applied receipt). Pure
// PRESENTATION-side geometry over readback bytes (readSelectionImage's
// Grayscale8 coverage masks): no engine math, no document mutation — the
// bounding box of nonzero coverage, accumulated strip-by-strip so every
// single read stays under the 2^26-px readback cap (the flagged
// unchunked-full-doc-read class is avoided by construction here).

#pragma once

#include <QImage>
#include <QRect>

namespace appname::shell {

// Union `running` with the bounding box of nonzero pixels in `strip`
// (Grayscale8; rows offset by `yOffset` in document space). A null
// `running` starts fresh; returns null if both are empty.
QRect accumulateMaskBounds(const QImage &strip, int yOffset,
                           const QRect &running);

// The RGBA8888 twin over the 1.4 per-layer readback door's delivery
// (readLayer: straight alpha): the bounding box of ALPHA-PRESENT pixels
// (alpha != 0). Alpha presence is encoding-robust — the color channels'
// transfer encoding never moves a zero/nonzero alpha across the
// threshold. G-3: the perspective session's entry-time source-bounds
// derivation scans with this (canvas_view.cpp beginPerspectiveFrame).
QRect accumulateAlphaBounds(const QImage &strip, int yOffset,
                            const QRect &running);

} // namespace appname::shell
