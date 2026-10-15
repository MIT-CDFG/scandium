// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §27 (strings/i18n pipeline); plan §2.2 "Qt/QML shell v1" row
// ("i18n from day 1 ... reads the same JSON catalog format the CLI uses").
// Grammar and semantics ported from crates/appname-cli/src/i18n.rs (same
// authorship, same license): flat `{ "id": "template" }` catalogs (or the
// map nested under "messages"), `_meta.plural_rules` of `one_other` /
// `other_only`, `{name}` placeholders, ICU-subset cardinal plurals
// `{name, plural, one {...} other {...}}` with `#` as the number,
// unconsumed args appended as ` name=value`, malformed blocks left
// verbatim, key-echo fallback for uncataloged ids.
//
// Resolution (this shell, per its §2.2 row): `APPNAME_I18N_DIR` ->
// executable-adjacent `i18n/` -> key-echo. NO embedded catalog: the shell
// key-echoes until O-I18N-A11Y's catalogs land — visibly untranslated
// keeps hardcoded English out (preamble-honesty by construction).
// Language: `--lang` (caller) > `APPNAME_LANG` > "en". Lookup chain:
// requested locale -> en -> key-echo; the catalog that supplied a template
// pluralizes with ITS OWN rule.
//
// v1 limitation (documented): the language is fixed at process start; the
// runtime-switch surface arrives with O-I18N-A11Y's audit wave.

#pragma once

#include <QHash>
#include <QObject>
#include <QString>
#include <QVariantMap>
#include <QVector>

namespace appname::shell {

class Catalog {
public:
    // `--lang` value (empty = unset) > APPNAME_LANG > "en".
    static QString resolveLang(const QString &flagValue);

    // Load the lookup chain for `lang` from `APPNAME_I18N_DIR`, then the
    // executable-adjacent `i18n/` directory. Missing catalogs are not an
    // error (key-echo keeps the UI honest).
    static Catalog load(const QString &lang);

    // Build from in-memory catalog texts (tests; empty chain = key-echo).
    static Catalog fromTexts(const QVector<QPair<QString, QString>> &langAndJson);

    // Translate `key` with ordered args; key-echo fallback appends
    // ` name=value` for every arg, visibly untranslated.
    QString tr(const QString &key, const QVector<QPair<QString, QString>> &args = {}) const;

    // Whether `key` resolves without key-echo (any catalog in the chain
    // carries it). The announcer's refusal gate (§31.3).
    bool has(const QString &key) const;

    bool hasEntries() const { return !m_chain.isEmpty(); }

private:
    enum class Rule { OneOther, OtherOnly };
    struct Entry {
        Rule rule = Rule::OneOther;
        QHash<QString, QString> map;
    };

    static Rule defaultRule(const QString &lang);
    static bool parseCatalog(const QString &lang, const QByteArray &text, Entry *out);
    static bool loadLangFile(const QString &lang, Entry *out);
    static QString expand(const QString &tmpl,
                          const QVector<QPair<QString, QString>> &args,
                          Rule rule);
    static QString expandPlurals(const QString &tmpl,
                                 const QVector<QPair<QString, QString>> &args,
                                 Rule rule,
                                 QVector<QString> *consumed);

    QVector<Entry> m_chain;

    friend class CatalogQml;
};

// The QML face of the catalog: every visible string in the shell goes
// through `AppI18n.tr(...)` (QML) or `Catalog::tr` (C++). Args from QML
// ride a QVariantMap; its deterministic (alphabetical) key order is the
// append order for unconsumed args — a documented, deliberate divergence
// from the CLI's caller-given order (QML has no ordered map literal).
class CatalogQml : public QObject {
    Q_OBJECT
public:
    explicit CatalogQml(Catalog catalog, QObject *parent = nullptr);

    Q_INVOKABLE QString tr(const QString &key, const QVariantMap &args = QVariantMap()) const;

    const Catalog &catalog() const { return m_catalog; }

private:
    Catalog m_catalog;
};

} // namespace appname::shell
