// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan P-ROUND item 3 (operator directive: layer thumbnails, filed
// as ABI-growth request 7 rather than faked client-side) — GRANTED as
// appname_doc_read_layer in the ABI 1.4 block. This provider serves
// image://layerthumb/<layerId>@<infoRevision> from the per-layer content
// readback; the revision suffix busts Qt's per-URL cache exactly when
// info truth moves. CONTENT, not contribution (a thumbnail does not fade
// with layer opacity — header contract). Typed refusals from the 1.4
// service matrix (adjustment layers, kinds without a composite path)
// yield NO pixmap — the panel keeps its kind glyphs, never a faked
// preview. Downscaling is presentation-side scaling of readback pixels,
// the same class as the canvas zoom path.

#pragma once

#include <QQuickImageProvider>

namespace appname::shell {

class DocumentController;

// Pixmap-type providers are called on the GUI thread, so calling into
// the controller directly is safe. The QML engine owns the provider; in
// main() the engine is declared after (destroyed before) the controller,
// so the raw pointer cannot dangle.
class LayerThumbProvider : public QQuickImageProvider {
public:
    explicit LayerThumbProvider(DocumentController *controller)
        : QQuickImageProvider(QQuickImageProvider::Pixmap)
        , m_controller(controller)
    {
    }

    QPixmap requestPixmap(const QString &id, QSize *size,
                          const QSize &requestedSize) override;

private:
    DocumentController *m_controller;
};

} // namespace appname::shell
