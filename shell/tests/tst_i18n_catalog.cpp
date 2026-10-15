// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §27; parity with the CLI loader's grammar
// (crates/appname-cli/src/i18n.rs — these cases mirror its test suite so
// the two loaders cannot drift silently).
//
// NOTE (build workaround, flagged in the lane log): the JSON fixtures are
// escaped ordinary string literals, NOT raw strings. This Qt build's moc
// (6.11.1) mis-lexes raw string literals whose content has brace/quote
// interplay (empirically: unbalanced braces, and adjacent-concatenated raw
// strings) and then silently drops the test class ("No relevant classes
// found"), which surfaces as a missing-vtable link error. Ordinary
// literals lex correctly.

#include "i18n_catalog.h"

#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

namespace {

QVector<QPair<QString, QString>> args(std::initializer_list<QPair<QString, QString>> list)
{
    return QVector<QPair<QString, QString>>(list);
}

} // namespace

class TstI18nCatalog : public QObject {
    Q_OBJECT

private slots:
    void langResolutionFlagWinsOverEnvWinsOverDefault()
    {
        qputenv("APPNAME_LANG", "ja");
        QCOMPARE(Catalog::resolveLang(QStringLiteral("es")), QStringLiteral("es"));
        QCOMPARE(Catalog::resolveLang(QString()), QStringLiteral("ja"));
        QCOMPARE(Catalog::resolveLang(QStringLiteral("  ")), QStringLiteral("ja"));
        qputenv("APPNAME_LANG", " ");
        QCOMPARE(Catalog::resolveLang(QString()), QStringLiteral("en"));
        qunsetenv("APPNAME_LANG");
        QCOMPARE(Catalog::resolveLang(QString()), QStringLiteral("en"));
    }

    void placeholderInterpolationAndUnconsumedAppend()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral("{\"k\": \"Error {code}: {message}\"}")}});
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("code"), QStringLiteral("error.x")},
                            {QStringLiteral("message"), QStringLiteral("boom")},
                            {QStringLiteral("id"), QStringLiteral("u-1")}})),
                 QStringLiteral("Error error.x: boom id=u-1"));
    }

    void pluralOneOtherRule()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral(
                  "{\"k\": \"{count, plural, one {# layer} other {# layers}}\"}")}});
        QCOMPARE(c.tr(QStringLiteral("k"), args({{QStringLiteral("count"), QStringLiteral("1")}})),
                 QStringLiteral("1 layer"));
        QCOMPARE(c.tr(QStringLiteral("k"), args({{QStringLiteral("count"), QStringLiteral("3")}})),
                 QStringLiteral("3 layers"));
        QCOMPARE(c.tr(QStringLiteral("k"), args({{QStringLiteral("count"), QStringLiteral("0")}})),
                 QStringLiteral("0 layers"));
    }

    void pluralOtherOnlyRuleFromMeta()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("ja"),
              QString::fromUtf8("{\"_meta\": {\"plural_rules\": \"other_only\"},"
                                "\"k\": \"{count, plural, other {レイヤー # 枚}}\"}")}});
        QCOMPARE(c.tr(QStringLiteral("k"), args({{QStringLiteral("count"), QStringLiteral("1")}})),
                 QString::fromUtf8("レイヤー 1 枚"));
    }

    void enFallbackTemplateKeepsEnPluralRule()
    {
        // ja catalog lacks the key; the en fallback pluralizes with en's
        // one_other rule even though the requested locale is ja.
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("ja"),
              QStringLiteral("{\"_meta\": {\"plural_rules\": \"other_only\"}}")},
             {QStringLiteral("en"),
              QStringLiteral("{\"k\": \"{n, plural, one {# item} other {# items}}\"}")}});
        QCOMPARE(c.tr(QStringLiteral("k"), args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("1 item"));
    }

    void malformedBlocksStayVisible()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral(
                  "{\"k\": \"{n, plural, few {x} other {y}}\", \"u\": \"broken {n\"}")}});
        // Unsupported selector: whole block verbatim, arg appended.
        QCOMPARE(c.tr(QStringLiteral("k"), args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("{n, plural, few {x} other {y}} n=1"));
        // Unbalanced brace: emitted as-is.
        QCOMPARE(c.tr(QStringLiteral("u")), QStringLiteral("broken {n"));
    }

    void keyEchoWhenUncataloged()
    {
        const Catalog c = Catalog::fromTexts({});
        QCOMPARE(c.tr(QStringLiteral("ui.status.applied"),
                      args({{QStringLiteral("label"), QStringLiteral("Brush Stroke")}})),
                 QStringLiteral("ui.status.applied label=Brush Stroke"));
    }

    void messagesNestingAndMetaSkip()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral("{\"messages\": {\"a\": \"A\", \"_meta\": \"not a string\"}}")}});
        QCOMPARE(c.tr(QStringLiteral("a")), QStringLiteral("A"));
        QCOMPARE(c.tr(QStringLiteral("_meta")), QStringLiteral("_meta")); // echo, never a template
    }

    void envDirResolutionWinsAndChainPrefersRequestedLocale()
    {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        {
            QFile es(dir.filePath(QStringLiteral("es.json")));
            QVERIFY(es.open(QIODevice::WriteOnly));
            es.write("{\"greet\": \"Hola {name}\"}");
        }
        {
            QFile en(dir.filePath(QStringLiteral("en.json")));
            QVERIFY(en.open(QIODevice::WriteOnly));
            en.write("{\"greet\": \"Hello {name}\", \"only_en\": \"fallback\"}");
        }
        qputenv("APPNAME_I18N_DIR", dir.path().toUtf8());
        const Catalog c = Catalog::load(QStringLiteral("es"));
        qunsetenv("APPNAME_I18N_DIR");
        QVERIFY(c.hasEntries());
        QCOMPARE(c.tr(QStringLiteral("greet"),
                      args({{QStringLiteral("name"), QStringLiteral("mundo")}})),
                 QStringLiteral("Hola mundo"));
        QCOMPARE(c.tr(QStringLiteral("only_en")), QStringLiteral("fallback"));
        QCOMPARE(c.tr(QStringLiteral("missing")), QStringLiteral("missing"));
    }

    void qmlFaceUsesAlphabeticalArgOrder()
    {
        CatalogQml qml(Catalog::fromTexts({}));
        QVariantMap map;
        map.insert(QStringLiteral("b"), 2);
        map.insert(QStringLiteral("a"), 1);
        // Key-echo appends QVariantMap args in its deterministic
        // (alphabetical) order — documented divergence from the CLI.
        QCOMPARE(qml.tr(QStringLiteral("k"), map), QStringLiteral("k a=1 b=2"));
    }
};

QTEST_GUILESS_MAIN(TstI18nCatalog)
#include "tst_i18n_catalog.moc"
