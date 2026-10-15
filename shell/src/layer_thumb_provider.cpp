// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header.

#include "layer_thumb_provider.h"

#include "document_controller.h"

namespace appname::shell {

QPixmap LayerThumbProvider::requestPixmap(const QString &id, QSize *size,
                                          const QSize &requestedSize)
{
    // id = "<layerId>@<infoRevision>"; the revision is cache-bust only.
    const QString layerId = id.section(QLatin1Char('@'), 0, 0);
    if (layerId.isEmpty() || !m_controller)
        return {};
    const QImage img = m_controller->layerThumbImage(layerId);
    if (img.isNull())
        return {};
    const QSize target = requestedSize.isValid() && !requestedSize.isEmpty()
        ? requestedSize
        : QSize(28, 22);
    const QPixmap pm = QPixmap::fromImage(
        img.scaled(target, Qt::KeepAspectRatio, Qt::SmoothTransformation));
    if (size)
        *size = pm.size();
    return pm;
}

} // namespace appname::shell
