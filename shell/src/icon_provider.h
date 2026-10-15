// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: P-ROUND item 2 (operator directive) + deps/lucide.yaml
// currentColor election — the vendored SVGs stay VERBATIM on disk; this
// provider substitutes currentColor IN MEMORY at render time and
// rasterizes via QtSvg. QML tints via icon.color (the provider renders
// black; Qt's icon machinery colorizes by alpha), so one rendered asset
// serves every theme state. Pure presentation — no document pixels.

#pragma once

#include <QQuickImageProvider>

namespace appname::shell {

class LucideIconProvider : public QQuickImageProvider {
public:
    LucideIconProvider()
        : QQuickImageProvider(QQuickImageProvider::Pixmap)
    {
    }

    // id = the vendored glyph basename (e.g. "move", "eye-off").
    QPixmap requestPixmap(const QString &id, QSize *size,
                          const QSize &requestedSize) override;
};

} // namespace appname::shell
