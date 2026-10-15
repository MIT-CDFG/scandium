// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

#include "license_texts.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>

namespace appname::shell {

namespace {

QVariantMap entryFor(const QDir &root, const QString &rel)
{
    const QFileInfo fi(root.filePath(rel));
    QVariantMap m;
    m.insert(QStringLiteral("name"), fi.fileName());
    m.insert(QStringLiteral("rel"), rel);
    m.insert(QStringLiteral("bytes"), static_cast<qlonglong>(fi.size()));
    return m;
}

} // namespace

LicenseTexts::LicenseTexts(const QString &dir, QObject *parent)
    : QObject(parent), m_dir(dir)
{
    if (m_dir.isEmpty())
        return;                       // no folder resolved: `available` stays false
    const QDir root(m_dir);
    if (!root.exists())
        return;

    // ONE LEVEL of nesting, deliberately, because that is the shape packaging
    // makes: top-level notice files beside LICENSES/. A recursive walk would
    // also sweep in anything a future staging step drops here, and this list
    // is shown to a user as "the texts we ship" — it should mirror the
    // staging contract, not whatever happens to be on disk.
    for (const QString &f : root.entryList(QDir::Files, QDir::Name))
        m_entries.append(entryFor(root, f));
    for (const QString &d : root.entryList(QDir::Dirs | QDir::NoDotAndDotDot, QDir::Name)) {
        const QDir sub(root.filePath(d));
        for (const QString &f : sub.entryList(QDir::Files, QDir::Name))
            m_entries.append(entryFor(root, d + QLatin1Char('/') + f));
    }
}

QString LicenseTexts::readText(const QString &rel, bool *truncated) const
{
    if (truncated)
        *truncated = false;

    // MEMBERSHIP FIRST. `rel` arrives from QML; only paths this object itself
    // enumerated are readable, so a crafted "../../etc/passwd" is not a path
    // traversal but simply not an entry.
    bool known = false;
    for (const QVariant &e : m_entries) {
        if (e.toMap().value(QStringLiteral("rel")).toString() == rel) {
            known = true;
            break;
        }
    }
    if (!known)
        return {};

    QFile f(QDir(m_dir).filePath(rel));
    if (!f.open(QIODevice::ReadOnly))
        return {};
    QByteArray head = f.read(kMaxRenderBytes);
    const bool cut = f.pos() < f.size();
    if (truncated)
        *truncated = cut;

    if (cut) {
        // WHERE THE CUT LANDS IS NOT COSMETIC IN A LICENCE TEXT. `read(n)`
        // ends at an arbitrary byte: measured against the shipped notices
        // aggregate it lands mid-clause ("...or additions\n   to that Work
        // or "), and on a file with different content it can land INSIDE a
        // multi-byte UTF-8 sequence, which fromUtf8 renders as a
        // replacement character — a compliance surface showing mojibake.
        //
        // Ending at the last LINE boundary fixes both at once: '\n' is
        // ASCII, so it is always a character boundary too. Measured cost on
        // the real file: 19 bytes.
        const int nl = head.lastIndexOf('\n');
        if (nl >= 0) {
            head.truncate(nl + 1);
        } else {
            // A single line longer than the cap — no boundary to fall back
            // to, so drop a DANGLING partial sequence and nothing more. The
            // length check is what keeps a COMPLETE trailing character from
            // being eaten as if it were partial.
            int i = head.size() - 1;
            int cont = 0;
            while (i >= 0 && (static_cast<unsigned char>(head.at(i)) & 0xC0) == 0x80
                   && cont < 3) {
                --i;
                ++cont;
            }
            if (i >= 0) {
                const unsigned char lead = static_cast<unsigned char>(head.at(i));
                int need = 0;
                if ((lead & 0x80) == 0)         need = 0;
                else if ((lead & 0xE0) == 0xC0) need = 1;
                else if ((lead & 0xF0) == 0xE0) need = 2;
                else if ((lead & 0xF8) == 0xF0) need = 3;
                if (need != cont)
                    head.truncate(i);
            }
        }
    }
    return QString::fromUtf8(head);
}

QVariantMap LicenseTexts::read(const QString &rel) const
{
    bool tr = false;
    const QString text = readText(rel, &tr);
    QVariantMap m;
    m.insert(QStringLiteral("text"), text);
    m.insert(QStringLiteral("truncated"), tr);
    return m;
}

} // namespace appname::shell
