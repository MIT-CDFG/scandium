// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// CLICK-FOCUS PARITY (Windows-port W-14 / W-15 / W-17, operator WORD 31 (5)).
//
// Qt Quick Controls 6.11.1 gives every QQuickAbstractButton a platform focus
// policy in QQuickAbstractButtonPrivate::init():
//
//     #ifdef Q_OS_MACOS
//         q->setFocusPolicy(Qt::TabFocus);
//     #else
//         q->setFocusPolicy(Qt::StrongFocus);
//     #endif
//
// so on Windows a POINTER CLICK on a Button / ToolButton / CheckBox / Switch
// moves keyboard focus onto it, while on macOS it does not. The shell's
// accessibility design was recorded against the macOS convention
// (Main.qml focusInsideToolbar(): "A pointer click on a palette slot does NOT
// move focus into the pane (macOS button click-focus convention)"), and four
// Windows ctest reds follow from the difference: the palette click moves focus
// into the pane (W-14), which then suppresses the tool announcement (W-15),
// and a toolbar click takes focus off the canvas so its frame differs (W-17).
//
// This filter restores the macOS convention on Windows: immediately before a
// mouse press is delivered to a QQuickWindow, any visible button under the
// pointer whose policy is still Qt's non-macOS default (StrongFocus) is
// demoted to TabFocus, the exact macOS value. Tab / F6 keyboard traversal is
// unchanged (TabFocus keeps the button on the Tab ring, and the app already
// sets TabFocusAllControls). Policies set explicitly to anything other than
// StrongFocus are left alone. The demotion is lazy, so delegates and popups
// created after startup are covered when first pressed.
//
// The class compiles on every platform so it can be exercised from macOS;
// installClickFocusParity() installs it ONLY on Windows, so the frozen macOS
// tree's behaviour is unchanged. Not covered: an accessibility "press" action
// (QQuickAbstractButton::click() also focuses on ClickFocus) — no press event
// passes through the filter for it.
#pragma once

#include <QEvent>
#include <QGuiApplication>
#include <QMouseEvent>
#include <QObject>
#include <QPointF>
#include <QQuickItem>
#include <QQuickWindow>

class ClickFocusParity : public QObject
{
public:
    using QObject::QObject;

    // Demote every visible StrongFocus button under scenePos (whole subtree,
    // so overlay-hosted popups and unclipped children are reached). Returns
    // how many were demoted; exposed for tests.
    static int demoteUnder(QQuickItem *item, const QPointF &scenePos)
    {
        if (!item || !item->isVisible())
            return 0;
        int n = 0;
        if (item->inherits("QQuickAbstractButton")
            && item->focusPolicy() == Qt::StrongFocus
            && item->contains(item->mapFromScene(scenePos))) {
            item->setFocusPolicy(Qt::TabFocus);
            ++n;
        }
        const auto children = item->childItems();
        for (QQuickItem *c : children)
            n += demoteUnder(c, scenePos);
        return n;
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (event->type() == QEvent::MouseButtonPress) {
            if (auto *window = qobject_cast<QQuickWindow *>(watched)) {
                const auto *me = static_cast<QMouseEvent *>(event);
                demoteUnder(window->contentItem(), me->scenePosition());
            }
        }
        return false; // never consumes the event
    }
};

// Called once, right after the QGuiApplication exists — by main.cpp and by
// every test main() that mirrors main.cpp's setup.
inline void installClickFocusParity(QGuiApplication &app)
{
#ifdef Q_OS_WIN
    app.installEventFilter(new ClickFocusParity(&app));
#else
    Q_UNUSED(app);
#endif
}
