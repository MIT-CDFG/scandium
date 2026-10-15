// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.3 live-region policy [NORMATIVE]; keyboard-scripts.yaml
// global rule 7 (700 ms observation bound). Mechanics documented in
// announcer.h. The AT bridge is QAccessibleAnnouncementEvent (Qt ≥ 6.8),
// politeness POLITE always — §31.3: "never `assertive`; nothing here is
// an emergency". Headless/offscreen runs dispatch the event into a null
// bridge harmlessly; the announced() signal + ring log stay the
// observable truth for tests either way.

#include "announcer.h"

#include <QAccessible>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>

namespace appname::shell {

namespace {

// QVariantMap -> ordered args, alphabetical by key — the same documented
// order CatalogQml::tr uses (QMap iteration is sorted), so announcement
// resolution and visible-string resolution can never disagree on
// unconsumed-arg ordering.
QVector<QPair<QString, QString>> toOrderedArgs(const QVariantMap &args)
{
    QVector<QPair<QString, QString>> out;
    out.reserve(args.size());
    for (auto it = args.constBegin(); it != args.constEnd(); ++it)
        out.append({it.key(), it.value().toString()});
    return out;
}

} // namespace

Announcer::Announcer(const Catalog *catalog, QObject *parent)
    : QObject(parent)
    , m_catalog(catalog)
    , m_sinkPath(QString::fromLocal8Bit(qgetenv("APPNAME_A11Y_LOG")))
{
    m_clock.start();
    m_timer.setSingleShot(true);
    // The 500 ms window is a NORMATIVE contract with a 200 ms margin to
    // the KS bound — coarse timers (5% slack) would eat 25 ms of it for
    // no reason.
    m_timer.setTimerType(Qt::PreciseTimer);
    connect(&m_timer, &QTimer::timeout, this, &Announcer::flushDue);
}

void Announcer::announce(const QString &source, const QString &messageId,
                         const QVariantMap &args)
{
    if (!m_catalog || !m_catalog->has(messageId)) {
        // Refuse, never key-echo (P2-D2 mechanized): Catalog::tr's
        // key-echo fallback keeps VISIBLE strings honest by being
        // visibly untranslated, but a spoken raw id is noise a
        // screen-reader user cannot even see to report.
        ++m_refused;
        qWarning("announcer: refused uncataloged MessageId '%s'",
                 qPrintable(messageId));
        return;
    }
    const qint64 now = m_clock.elapsed();
    if (m_pending.isEmpty() && now - m_lastFlushMs >= RateLimitMs) {
        m_lastFlushMs = now;
        emitNow(source, messageId, args);
        return;
    }
    // Inside the rate-limit window (or behind queued sources): coalesce
    // latest-wins per source; a source announces from its slot in the
    // FIFO order it first went pending.
    if (!m_pending.contains(source))
        m_order.append(source);
    m_pending.insert(source, {messageId, args});
    if (!m_timer.isActive()) {
        const qint64 wait = qMax<qint64>(0, m_lastFlushMs + RateLimitMs - now);
        m_timer.start(int(wait));
    }
}

void Announcer::flushDue()
{
    if (m_order.isEmpty())
        return;
    const QString source = m_order.takeFirst();
    const Pending p = m_pending.take(source);
    m_lastFlushMs = m_clock.elapsed();
    emitNow(source, p.messageId, p.args);
    if (!m_order.isEmpty())
        m_timer.start(int(RateLimitMs));
}

void Announcer::emitNow(const QString &source, const QString &messageId,
                        const QVariantMap &args)
{
    const QString text = m_catalog->tr(messageId, toOrderedArgs(args));

    // Dispatch on the subject (the root window): platform bridges drop
    // events whose object resolves no QAccessibleInterface (D1). The
    // `this` fallback keeps headless unit runs observable via the
    // signal/log surface only.
    QAccessibleAnnouncementEvent event(m_subject ? m_subject : this, text);
    event.setPoliteness(QAccessible::AnnouncementPoliteness::Polite);
    QAccessible::updateAccessibility(&event);

    QVariantMap entry;
    entry.insert(QStringLiteral("source"), source);
    entry.insert(QStringLiteral("id"), messageId);
    entry.insert(QStringLiteral("text"), text);
    entry.insert(QStringLiteral("ms"), m_clock.elapsed());
    m_log.append(entry);

    // A11Y-5 hook A: the external harness door — one compact JSONL row
    // per emit, appended immediately (a crashed run keeps its prefix).
    if (!m_sinkPath.isEmpty()) {
        QFile f(m_sinkPath);
        if (f.open(QIODevice::WriteOnly | QIODevice::Append)) {
            f.write(QJsonDocument(QJsonObject::fromVariantMap(entry))
                        .toJson(QJsonDocument::Compact));
            f.write("\n");
        }
    }

    emit announced(source, messageId, text);
}

} // namespace appname::shell
