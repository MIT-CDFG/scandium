// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §27; grammar ported from crates/appname-cli/src/i18n.rs (see
// header). Behavior parity with the CLI loader is what the unit tests pin.

#include "i18n_catalog.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QRegularExpression>

namespace appname::shell {

namespace {

// Index of the matching close brace for the open brace at `open`, or -1.
int matchingBrace(const QString &s, int open)
{
    int depth = 0;
    for (int i = open; i < s.size(); ++i) {
        const QChar ch = s.at(i);
        if (ch == QLatin1Char('{')) {
            ++depth;
        } else if (ch == QLatin1Char('}')) {
            --depth;
            if (depth == 0)
                return i;
        }
    }
    return -1;
}

// `"name, plural, one {..} other {..}"` -> name + (selector, body) list.
// Mirrors i18n.rs parse_plural: only `one`/`other` selectors, an `other`
// branch is required, the arg name is ASCII alphanumeric/underscore.
bool parsePlural(const QString &inner,
                 QString *name,
                 QVector<QPair<QString, QString>> *branches)
{
    const int firstComma = inner.indexOf(QLatin1Char(','));
    if (firstComma < 0)
        return false;
    const int secondComma = inner.indexOf(QLatin1Char(','), firstComma + 1);
    if (secondComma < 0)
        return false;
    const QString head = inner.left(firstComma).trimmed();
    const QString kw = inner.mid(firstComma + 1, secondComma - firstComma - 1).trimmed();
    if (kw != QLatin1String("plural"))
        return false;
    if (head.isEmpty())
        return false;
    for (const QChar c : head) {
        const char16_t u = c.unicode();
        const bool ok = (u >= u'a' && u <= u'z') || (u >= u'A' && u <= u'Z')
            || (u >= u'0' && u <= u'9') || u == u'_';
        if (!ok)
            return false;
    }
    const QString branchesSrc = inner.mid(secondComma + 1);
    int i = 0;
    while (i < branchesSrc.size()) {
        while (i < branchesSrc.size() && branchesSrc.at(i).isSpace())
            ++i;
        if (i >= branchesSrc.size())
            break;
        const int selStart = i;
        while (i < branchesSrc.size() && branchesSrc.at(i) != QLatin1Char('{'))
            ++i;
        if (i >= branchesSrc.size())
            return false; // selector without branch
        const QString selector = branchesSrc.mid(selStart, i - selStart).trimmed();
        if (selector != QLatin1String("one") && selector != QLatin1String("other"))
            return false; // unsupported selector
        const int end = matchingBrace(branchesSrc, i);
        if (end < 0)
            return false;
        branches->append({selector, branchesSrc.mid(i + 1, end - i - 1)});
        i = end + 1;
    }
    bool hasOther = false;
    for (const auto &b : *branches)
        if (b.first == QLatin1String("other"))
            hasOther = true;
    if (!hasOther)
        return false; // an ICU plural requires an `other` branch
    *name = head;
    return true;
}

// A resolved language tag reaches the filesystem as `<dir>/<lang>.json`
// (loadLangFile), so it must be a bare TOKEN: no separators, no dots, nothing
// that can walk out of the catalog directory. Deliberately PERMISSIVE about
// WHICH tags exist — adding a locale must never require editing this guard, and
// a guard that has to be edited to ship a language is a guard that gets deleted
// — and strict only about the SHAPE that could leave the directory.
bool isWellFormedLangTag(const QString &lang)
{
    static const QRegularExpression re(
        QStringLiteral("\\A[A-Za-z][A-Za-z0-9]*(?:[-_][A-Za-z0-9]+)*\\z"));
    return lang.size() <= 35 && re.match(lang).hasMatch();
}

} // namespace

Catalog::Rule Catalog::defaultRule(const QString &lang)
{
    return lang == QLatin1String("ja") ? Rule::OtherOnly : Rule::OneOther;
}

QString Catalog::resolveLang(const QString &flagValue)
{
    // Both sources are UNTRUSTED SHAPE, not untrusted authority: this is a
    // local app and whoever passes --lang already owns the files on disk, so
    // this is a ROBUSTNESS guard, not a privilege boundary. What it prevents is
    // a malformed tag silently becoming a path.
    const QString flag = flagValue.trimmed();
    if (!flag.isEmpty()) {
        if (isWellFormedLangTag(flag))
            return flag;
        // Announced, never silent: a request that cannot be honoured says so.
        qWarning("i18n: --lang value \"%s\" is not a well-formed language tag; "
                 "falling back to \"en\"",
                 qUtf8Printable(flag));
        return QStringLiteral("en");
    }
    const QString env = QString::fromUtf8(qgetenv("APPNAME_LANG")).trimmed();
    if (!env.isEmpty()) {
        if (isWellFormedLangTag(env))
            return env;
        qWarning("i18n: APPNAME_LANG value \"%s\" is not a well-formed language "
                 "tag; falling back to \"en\"",
                 qUtf8Printable(env));
        return QStringLiteral("en");
    }
    return QStringLiteral("en");
}

bool Catalog::parseCatalog(const QString &lang, const QByteArray &text, Entry *out)
{
    QJsonParseError err;
    const QJsonDocument doc = QJsonDocument::fromJson(text, &err);
    if (err.error != QJsonParseError::NoError || !doc.isObject())
        return false;
    const QJsonObject root = doc.object();

    Entry entry;
    entry.rule = defaultRule(lang);
    const QJsonValue meta = root.value(QLatin1String("_meta"));
    if (meta.isObject()) {
        const QString rules = meta.toObject().value(QLatin1String("plural_rules")).toString();
        if (rules == QLatin1String("one_other"))
            entry.rule = Rule::OneOther;
        else if (rules == QLatin1String("other_only"))
            entry.rule = Rule::OtherOnly;
    }

    QJsonObject map = root;
    const QJsonValue messages = root.value(QLatin1String("messages"));
    if (messages.isObject())
        map = messages.toObject();

    for (auto it = map.constBegin(); it != map.constEnd(); ++it) {
        if (it.key() == QLatin1String("_meta"))
            continue;
        if (it.value().isString())
            entry.map.insert(it.key(), it.value().toString());
    }
    *out = entry;
    return true;
}

bool Catalog::loadLangFile(const QString &lang, Entry *out)
{
    QVector<QString> dirs;
    const QByteArray envDir = qgetenv("APPNAME_I18N_DIR");
    if (!envDir.isEmpty())
        dirs.append(QString::fromUtf8(envDir));
    // Executable-adjacent i18n/ (mirrors the CLI's exe-adjacent step).
    if (QCoreApplication::instance())
        dirs.append(QDir(QCoreApplication::applicationDirPath()).filePath(QStringLiteral("i18n")));
    for (const QString &dir : dirs) {
        QFile f(QDir(dir).filePath(lang + QStringLiteral(".json")));
        if (!f.open(QIODevice::ReadOnly))
            continue;
        if (parseCatalog(lang, f.readAll(), out))
            return true;
    }
    return false;
}

Catalog Catalog::load(const QString &lang)
{
    Catalog c;
    Entry entry;
    if (lang != QLatin1String("en") && loadLangFile(lang, &entry))
        c.m_chain.append(entry);
    Entry en;
    if (loadLangFile(QStringLiteral("en"), &en))
        c.m_chain.append(en);
    return c;
}

Catalog Catalog::fromTexts(const QVector<QPair<QString, QString>> &langAndJson)
{
    Catalog c;
    for (const auto &pair : langAndJson) {
        Entry entry;
        if (parseCatalog(pair.first, pair.second.toUtf8(), &entry))
            c.m_chain.append(entry);
    }
    return c;
}

bool Catalog::has(const QString &key) const
{
    // True iff some catalog in the chain carries the key — i.e. tr()
    // would NOT key-echo. The announcer refuses ids this returns false
    // for (§31.3: a spoken raw id is not "visibly untranslated", it is
    // unreportable noise).
    for (const Entry &entry : m_chain)
        if (entry.map.contains(key))
            return true;
    return false;
}

QString Catalog::tr(const QString &key, const QVector<QPair<QString, QString>> &args) const
{
    for (const Entry &entry : m_chain) {
        const auto it = entry.map.constFind(key);
        if (it != entry.map.constEnd())
            return expand(it.value(), args, entry.rule);
    }
    // Key-echo: the id itself + all args, visibly untranslated.
    QString out = key;
    for (const auto &arg : args) {
        out += QLatin1Char(' ');
        out += arg.first;
        out += QLatin1Char('=');
        out += arg.second;
    }
    return out;
}

QString Catalog::expand(const QString &tmpl,
                        const QVector<QPair<QString, QString>> &args,
                        Rule rule)
{
    QVector<QString> consumed;
    QString text = expandPlurals(tmpl, args, rule, &consumed);
    for (const auto &arg : args) {
        const QString needle = QLatin1Char('{') + arg.first + QLatin1Char('}');
        if (text.contains(needle)) {
            text.replace(needle, arg.second);
            consumed.append(arg.first);
        }
    }
    for (const auto &arg : args) {
        if (!consumed.contains(arg.first)) {
            text += QLatin1Char(' ');
            text += arg.first;
            text += QLatin1Char('=');
            text += arg.second;
        }
    }
    return text;
}

QString Catalog::expandPlurals(const QString &tmpl,
                               const QVector<QPair<QString, QString>> &args,
                               Rule rule,
                               QVector<QString> *consumed)
{
    QString out;
    int i = 0;
    while (i < tmpl.size()) {
        if (tmpl.at(i) != QLatin1Char('{')) {
            out += tmpl.at(i);
            ++i;
            continue;
        }
        const int end = matchingBrace(tmpl, i);
        if (end < 0) {
            out += tmpl.mid(i); // unbalanced: emit the rest verbatim
            break;
        }
        const QString inner = tmpl.mid(i + 1, end - i - 1);
        QString name;
        QVector<QPair<QString, QString>> branches;
        if (parsePlural(inner, &name, &branches)) {
            QString value;
            bool haveValue = false;
            for (const auto &arg : args) {
                if (arg.first == name) {
                    value = arg.second;
                    haveValue = true;
                    break;
                }
            }
            QString category = QStringLiteral("other");
            if (haveValue) {
                bool numOk = false;
                const double n = value.toDouble(&numOk);
                // Parity with the CLI authority (i18n.rs, f64::from_str):
                // Rust rejects surrounding whitespace where QString::
                // toDouble trims it — a whitespace-wrapped arg must select
                // the same branch in shell and CLI (tester T5).
                if (numOk && value != value.trimmed())
                    numOk = false;
                if (numOk)
                    category = (rule == Rule::OneOther && n == 1.0) ? QStringLiteral("one")
                                                                    : QStringLiteral("other");
            }
            const QPair<QString, QString> *branch = nullptr;
            for (const auto &b : branches)
                if (b.first == category) {
                    branch = &b;
                    break;
                }
            if (!branch) {
                for (const auto &b : branches)
                    if (b.first == QLatin1String("other")) {
                        branch = &b;
                        break;
                    }
            }
            if (branch && haveValue) {
                consumed->append(name);
                QString body = branch->second;
                body.replace(QLatin1Char('#'), value);
                out += body;
            } else if (branch) {
                out += branch->second;
            } else {
                out += QLatin1Char('{');
                out += inner;
                out += QLatin1Char('}');
            }
        } else {
            // Not a plural block ({name} placeholder or plain braces):
            // keep verbatim for the placeholder pass.
            out += QLatin1Char('{');
            out += inner;
            out += QLatin1Char('}');
        }
        i = end + 1;
    }
    return out;
}

CatalogQml::CatalogQml(Catalog catalog, QObject *parent)
    : QObject(parent)
    , m_catalog(std::move(catalog))
{
}

QString CatalogQml::tr(const QString &key, const QVariantMap &args) const
{
    QVector<QPair<QString, QString>> pairs;
    pairs.reserve(args.size());
    for (auto it = args.constBegin(); it != args.constEnd(); ++it)
        pairs.append({it.key(), it.value().toString()});
    return m_catalog.tr(key, pairs);
}

} // namespace appname::shell
