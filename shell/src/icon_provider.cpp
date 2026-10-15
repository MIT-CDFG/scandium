// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header.

#include "icon_provider.h"

#include <QFile>
#include <QPainter>
#include <QSvgRenderer>

namespace appname::shell {

QPixmap LucideIconProvider::requestPixmap(const QString &id, QSize *size,
                                          const QSize &requestedSize)
{
    // Basenames only — reject any path shape that could escape the
    // vendored resource directory.
    if (id.contains(QLatin1Char('/')) || id.contains(QLatin1String("..")))
        return {};
    QFile f(QStringLiteral(":/icons/lucide/") + id + QStringLiteral(".svg"));
    if (!f.open(QIODevice::ReadOnly))
        return {};
    QByteArray svg = f.readAll();
    // Render-time substitution (deps/lucide.yaml election: files verbatim,
    // recolor at render). Black + QML icon.color tinting-by-alpha.
    svg.replace("currentColor", "#000000");
    QSvgRenderer renderer(svg);
    if (!renderer.isValid())
        return {};
    const QSize target = requestedSize.isValid() && !requestedSize.isEmpty()
        ? requestedSize
        : QSize(24, 24); // the set's native grid
    QPixmap pm(target);
    pm.fill(Qt::transparent);
    QPainter p(&pm);
    renderer.render(&p, QRectF(QPointF(0, 0), QSizeF(target)));
    p.end();
    if (size)
        *size = target;
    return pm;
}

} // namespace appname::shell
