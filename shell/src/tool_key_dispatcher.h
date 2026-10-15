// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 11e — THE TOOL LETTER-KEY CHORD DISPATCHER (§32.3 group
// cycling; Appendix C tool letters; ES:16446 spring-load; ES:16452
// single-key suppression; ES:16453 auto-repeat policy).
//
// WHY A REAL KEY-EVENT DISPATCHER (the key-layer AMENDMENT, lane log):
// the key layer landed press-only QML Shortcut objects plus the
// dispatchToolChord funnel. A Shortcut fires on press only, so it can
// see neither RELEASE (spring-load needs hold-vs-tap) nor auto-repeat
// identity, and it cannot consult a capture gate at event time. This
// class is the migration: the eight letter Shortcuts LEAVE the shortcut
// map (deleted in Main.qml) and the ruled letter table, the ES:16452
// guard consult and the ES:16446 capture gate are hoisted here, with
// dispatchToolChord kept as the single tool-writing call site (the
// landed hand-off note names that call site as the (e) ruling's
// input-source anchor).
//
// ROUTING ORDER, MEASURED (Qt 6): for a KeyPress the shortcut map runs
// FIRST (QWindowSystemInterface::handleShortcutEvent), and only a key
// no shortcut claimed is then sent to the window — where this filter
// sees it. So the dispatcher can never shadow F6/Shift+F6
// (Qt.ApplicationShortcut), the Ctrl/Cmd chord table, or a
// theme-aliased StandardKey binding SET: those resolve ahead of it by
// construction. What the filter CAN out-race is the focused item's own
// claim on plain letters (combo type-ahead, text editors), because item
// delivery happens after the window filter. Preserved deliberately by
// re-running Qt's OWN arbitration: a ShortcutOverride probe to the
// focus object, yielded to exactly as the shortcut map yields. Measured
// consequence, matching both recorded probes: an OPEN ComboBox popup
// claims plain letters (type-ahead keeps working) while a CLOSED combo
// does not (its type-ahead stays bypassed).

#pragma once

#include <QElapsedTimer>
#include <QObject>
#include <QPointer>
#include <QStringList>
#include <QVector>

QT_BEGIN_NAMESPACE
class QKeyEvent;
class QQuickItem;
class QQuickWindow;
QT_END_NAMESPACE

namespace appname::shell {

class CanvasView;

// One ruled letter's cycle ring: the member wire ids in the palette
// model's DECLARED order (ToolPalette.qml groups). Rings are
// letter-LOCAL (desk ruling (a)) — a letter's ring is the member set
// Appendix C's letter id denotes, not the whole §32.3 palette group.
struct ToolRing {
    Qt::Key key;
    const char *letter;
    QStringList members;
};

class ToolKeyDispatcher : public QObject {
    Q_OBJECT
public:
    explicit ToolKeyDispatcher(QObject *parent = nullptr);

    // Install on the loaded root window: resolves the QML root (the
    // funnel + guard live there) and the CanvasView (the R3 capture
    // predicate), then filters the window's key events. main.cpp and
    // every Main.qml test harness call exactly this.
    void attach(QQuickWindow *window);

    // The ruled table of record. THE source pin
    // (tst_chord_table::singleKeySuppressionInRenameField) parses the
    // .cpp rows; this is its runtime companion.
    static const QVector<ToolRing> &ringTable();
    // The X action chord (Appendix C tool.swap_colors) — no ring.
    static Qt::Key swapColorsKey();
    // Every sequence this dispatcher claims, as portable text
    // ("M", "Shift+M", …) — the binding-sweep set of record.
    static QStringList ruledSequences();

    // Ring state readback for the net (never a QML shadow flag: this IS
    // the state the dispatcher steps).
    Q_INVOKABLE QStringList ringMembers(const QString &letter) const;
    Q_INVOKABLE QString ringCurrent(const QString &letter) const;

    // ES:16446 hold-vs-tap boundary, in ms. Named so the net asserts
    // against the spec number rather than a literal.
    static constexpr int SpringHoldMs = 300;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    bool isRuledKey(const QKeyEvent *ke, int *ringIndex, bool *isSwap) const;
    bool focusClaimsKey(const QKeyEvent *ke) const;
    bool textEditingGuardEngaged() const;
    bool captureInFlight() const;
    QString canvasTool() const;
    void dispatchRing(int ringIndex);
    void dispatchSwap();
    void writeTool(const QString &toolId);
    void syncRings();
    void handleRelease(const QKeyEvent *ke);

    QPointer<QQuickWindow> m_window;
    QPointer<QQuickItem> m_canvasItem;
    QPointer<CanvasView> m_canvas;
    QVector<int> m_ringCurrent; // per ring, index into its members

    // ES:16446 spring arm: the press that is still held.
    struct SpringArm {
        bool armed = false;
        int key = 0;
        QString priorTool;
    };
    SpringArm m_spring;
    QElapsedTimer m_springClock;
};

} // namespace appname::shell
