// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan P-ROUND P6 (W05 recovery prompt) — reviewer-added invariant
// pin (P6 slice review, 2026-08-16), tst_theme_contrast precedent. The
// W05 dialog's honesty rests on a seam no other check covers: the i18n
// audit proves every EMITTED id is cataloged in every locale with
// locale-consistent placeholders (RED-2/3/4), but nothing proves the
// CALLER passes exactly the arg names the templates consume — a renamed
// {placeholder} (catalog side or QML side) would render a literal
// "{count}" or an unconsumed " count=7" append in the crash prompt and
// stay green everywhere else. This test parses the AppI18n.tr call
// sites OUT OF the checked-in RecoveryDialog.qml mechanically (no
// duplicated id list — editing the dialog re-checks automatically), pins
// the reviewed call-site contract (7 dialog.recovery.* ids +
// ui.dialog.cancel; {count} on unsaved_commands, {title}/{commands} on
// discard_confirm), and renders every call site through the real Catalog
// against each checked-in locale (en/es/ja/pseudo), asserting: no
// key-echo, no unexpanded {placeholder} residue, every sentinel arg
// consumed by the template (no ` name=value` append).

#include "i18n_catalog.h"

#include <QFile>
#include <QRegularExpression>
#include <QTest>

using namespace appname::shell;

namespace {

struct CallSite {
    QString id;
    QStringList argNames; // sorted
};

QByteArray readAll(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly))
        return {};
    return f.readAll();
}

// Extract every `AppI18n.tr(...)` call from QML source: quote-aware
// balanced-paren scan (arg objects nest calls like substring(...) and
// contain "/" string literals), first quoted string = the MessageId,
// then every `"name":` object key = an arg name.
QVector<CallSite> extractTrCalls(const QString &qml)
{
    QVector<CallSite> out;
    const QString marker = QStringLiteral("AppI18n.tr(");
    int from = 0;
    while (true) {
        const int at = qml.indexOf(marker, from);
        if (at < 0)
            break;
        int i = at + marker.size();
        int depth = 1;
        bool inString = false;
        QString body;
        for (; i < qml.size() && depth > 0; ++i) {
            const QChar c = qml.at(i);
            if (inString) {
                if (c == QLatin1Char('\\')) {
                    body.append(c);
                    if (i + 1 < qml.size())
                        body.append(qml.at(++i));
                    continue;
                }
                if (c == QLatin1Char('"'))
                    inString = false;
            } else {
                if (c == QLatin1Char('"'))
                    inString = true;
                else if (c == QLatin1Char('('))
                    ++depth;
                else if (c == QLatin1Char(')')) {
                    --depth;
                    if (depth == 0)
                        break;
                }
            }
            body.append(c);
        }
        from = i;
        CallSite site;
        const QRegularExpression idRe(QStringLiteral("^\\s*\"([^\"]+)\""));
        const QRegularExpressionMatch idM = idRe.match(body);
        if (!idM.hasMatch())
            continue; // family-constructed or non-literal id: not this dialog
        site.id = idM.captured(1);
        // Arg names: quoted object keys in the remainder after the id.
        const QString rest = body.mid(idM.capturedLength());
        const QRegularExpression argRe(QStringLiteral("\"([A-Za-z_][A-Za-z0-9_]*)\"\\s*:"));
        QRegularExpressionMatchIterator it = argRe.globalMatch(rest);
        while (it.hasNext())
            site.argNames.append(it.next().captured(1));
        site.argNames.sort();
        out.append(site);
    }
    return out;
}

} // namespace

class TstRecoveryDialogCatalog : public QObject {
    Q_OBJECT

private slots:
    void initTestCase()
    {
        const QByteArray qml = readAll(QStringLiteral(APPNAME_RECOVERY_DIALOG_QML_PATH));
        QVERIFY2(!qml.isEmpty(), "RecoveryDialog.qml unreadable");
        m_sites = extractTrCalls(QString::fromUtf8(qml));
        QVERIFY2(!m_sites.isEmpty(), "no AppI18n.tr call sites parsed");
        for (const char *lang : {"en", "es", "ja", "pseudo"}) {
            const QString path = QStringLiteral(APPNAME_REPO_I18N_DIR "/")
                + QLatin1String(lang) + QStringLiteral(".json");
            const QByteArray text = readAll(path);
            QVERIFY2(!text.isEmpty(), qPrintable(QStringLiteral("catalog unreadable: %1").arg(path)));
            m_locales.append({QLatin1String(lang), QString::fromUtf8(text)});
        }
    }

    // The reviewed contract: exactly these ids with exactly these args.
    void reviewedCallSiteContractHolds()
    {
        QHash<QString, QStringList> expected;
        expected.insert(QStringLiteral("dialog.recovery.title"), {});
        expected.insert(QStringLiteral("dialog.recovery.unsaved_commands"),
                        {QStringLiteral("count")});
        expected.insert(QStringLiteral("dialog.recovery.discard_confirm"),
                        {QStringLiteral("commands"), QStringLiteral("title")});
        expected.insert(QStringLiteral("dialog.recovery.not_now"), {});
        expected.insert(QStringLiteral("dialog.recovery.discard"), {});
        expected.insert(QStringLiteral("ui.dialog.cancel"), {});
        expected.insert(QStringLiteral("dialog.recovery.discard_permanently"), {});
        expected.insert(QStringLiteral("dialog.recovery.restore"), {});

        QHash<QString, QStringList> parsed;
        for (const CallSite &s : std::as_const(m_sites)) {
            QVERIFY2(!parsed.contains(s.id) || parsed.value(s.id) == s.argNames,
                     qPrintable(QStringLiteral("id used twice with differing args: %1").arg(s.id)));
            parsed.insert(s.id, s.argNames);
        }
        for (auto it = expected.constBegin(); it != expected.constEnd(); ++it) {
            QVERIFY2(parsed.contains(it.key()),
                     qPrintable(QStringLiteral("reviewed id missing from dialog: %1").arg(it.key())));
            QCOMPARE(parsed.value(it.key()), it.value());
        }
    }

    // Every call site renders clean in every checked-in locale.
    void everyCallSiteRendersCleanInEveryLocale()
    {
        for (const auto &locale : std::as_const(m_locales)) {
            const Catalog c = Catalog::fromTexts({{locale.first, locale.second}});
            QVERIFY(c.hasEntries());
            for (const CallSite &s : std::as_const(m_sites)) {
                QVector<QPair<QString, QString>> args;
                QStringList sentinels;
                for (const QString &name : s.argNames) {
                    const QString sentinel = QStringLiteral("XSENT_") + name + QStringLiteral("_X");
                    args.append({name, sentinel});
                    sentinels.append(sentinel);
                }
                const QString rendered = c.tr(s.id, args);
                const QString ctx = QStringLiteral("[%1] %2 -> \"%3\"")
                                        .arg(locale.first, s.id, rendered);
                // Key-echo = the id is not in this locale's catalog.
                QVERIFY2(!rendered.startsWith(s.id), qPrintable(ctx + QStringLiteral(" (key-echo)")));
                // A leftover brace = a template placeholder the caller
                // does not supply (renders literally in the crash prompt).
                QVERIFY2(!rendered.contains(QLatin1Char('{')),
                         qPrintable(ctx + QStringLiteral(" (unexpanded placeholder)")));
                for (const QString &sentinel : std::as_const(sentinels)) {
                    // Sentinel present: the arg reached the surface.
                    QVERIFY2(rendered.contains(sentinel),
                             qPrintable(ctx + QStringLiteral(" (arg dropped: %1)").arg(sentinel)));
                    // `name=value` residue = the template never consumed
                    // the arg (caller/catalog placeholder-name mismatch).
                    QVERIFY2(!rendered.contains(QLatin1Char('=') + sentinel),
                             qPrintable(ctx + QStringLiteral(" (unconsumed arg: %1)").arg(sentinel)));
                }
            }
        }
    }

private:
    QVector<CallSite> m_sites;
    QVector<QPair<QString, QString>> m_locales; // lang -> raw json text
};

QTEST_GUILESS_MAIN(TstRecoveryDialogCatalog)
#include "tst_recovery_dialog_catalog.moc"
