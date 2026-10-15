// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.3 live-region policy [NORMATIVE] — ONE polite live region
// carries every state-change announcement (never assertive); rate-limited
// to one per 500 ms with latest-wins coalescing per source; announcements
// fire on settle from engine receipts, never optimistically; focus-driven
// speech (names/roles/values, §31.1) is the platform's job and is never
// duplicated here. keyboard-scripts.yaml global rule 7 binds the
// observable latency: the expected a11y.* MessageId within 700 ms of the
// step (500 ms rate limit + margin) — the worst case here is one full
// rate-limit window, 500 ms < 700 ms.

#pragma once

#include "i18n_catalog.h"

#include <QElapsedTimer>
#include <QHash>
#include <QObject>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QVector>

namespace appname::shell {

// The shell's single polite live region. QML face: AppA11y.announce(...).
// C++ callers (DocumentController receipts) use the same entry point.
class Announcer : public QObject {
    Q_OBJECT
public:
    explicit Announcer(const Catalog *catalog, QObject *parent = nullptr);

    // Queue one announcement. `messageId` MUST resolve through the catalog:
    // an uncataloged id is REFUSED (counted + warned) — key-echoing a raw
    // id into speech is the P2-D2 regression class, and silence is the
    // honest failure mode. `source` scopes latest-wins coalescing (a held
    // brush-size key announces its final value once); distinct sources
    // queue FIFO by first-pending time so none is starved.
    Q_INVOKABLE void announce(const QString &source, const QString &messageId,
                              const QVariantMap &args = QVariantMap());

    // Test/harness evidence surface: every emitted announcement as
    // {source, id, text, ms}, ms monotonic since construction. The
    // EXTERNAL door (A11Y-5 hook A): APPNAME_A11Y_LOG=<path> appends
    // one JSONL row per emit — append-per-emit, so a crashed run keeps
    // its prefix; the KS harness measures rule 7's 700 ms bound as
    // input-event -> row ms.
    Q_INVOKABLE QVariantList announcementLog() const { return m_log; }
    int refusedCount() const { return m_refused; }

    // The object announcement events are dispatched ON — it MUST resolve
    // a valid QAccessibleInterface or every platform bridge silently
    // drops the event (review A11Y-3 D1 MAJOR: cocoa's
    // notifyAccessibilityUpdate and AT-SPI's announcement path both
    // early-return on a null/invalid interface; a bare QObject like this
    // one resolves none). The shell hands over the root QQuickWindow
    // after engine load. Null = fall back to `this` (headless unit runs;
    // the signal/log surface still observes).
    void setSubject(QObject *subject) { m_subject = subject; }

signals:
    void announced(const QString &source, const QString &messageId,
                   const QString &text);

private:
    void flushDue();
    void emitNow(const QString &source, const QString &messageId,
                 const QVariantMap &args);

    struct Pending {
        QString messageId;
        QVariantMap args;
    };

    static constexpr qint64 RateLimitMs = 500;

    const Catalog *m_catalog; // not owned
    QObject *m_subject = nullptr; // not owned; see setSubject
    QString m_sinkPath; // APPNAME_A11Y_LOG (empty = no sink)
    QElapsedTimer m_clock;
    qint64 m_lastFlushMs = -RateLimitMs; // first announce emits immediately
    QHash<QString, Pending> m_pending;   // per-source latest
    QVector<QString> m_order;            // FIFO by first-pending time
    QTimer m_timer;
    QVariantList m_log;
    int m_refused = 0;
};

} // namespace appname::shell
