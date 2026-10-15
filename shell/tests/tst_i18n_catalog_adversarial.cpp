// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §27 (strings/i18n pipeline); grammar authority
// crates/appname-cli/src/i18n.rs (the shell loader must not drift from the
// CLI loader). TESTER-added adversarial coverage beyond
// tst_i18n_catalog.cpp: pathological brace nesting, non-numeric and
// whitespace-wrapped plural arg values, missing-`other` and `=1` selector
// rejection, huge templates, malformed/empty catalog files and the chain
// fall-through they trigger, `messages` shadowing, unknown
// `_meta.plural_rules` values, per-file fall-through from APPNAME_I18N_DIR
// to the exe-adjacent directory, key-echo format stability, and
// placeholder replace-order semantics.
//
// NOTE: JSON fixtures are escaped ordinary string literals, NOT raw
// strings — this Qt build's moc (6.11.1) mis-lexes raw strings with
// brace/quote interplay and silently drops the test class (documented in
// tst_i18n_catalog.cpp; same workaround here).

#include "i18n_catalog.h"

#include <QDir>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

namespace {

QVector<QPair<QString, QString>> args(std::initializer_list<QPair<QString, QString>> list)
{
    return QVector<QPair<QString, QString>>(list);
}

Catalog enCatalog(const QString &json)
{
    return Catalog::fromTexts({{QStringLiteral("en"), json}});
}

} // namespace

class TstI18nCatalogAdversarial : public QObject {
    Q_OBJECT

private slots:
    // Braces nested inside plural branch bodies: the branch body is
    // substituted verbatim (no recursive plural pass), then the ordinary
    // placeholder pass resolves {x} inside it.
    void nestedPlaceholderInsidePluralBranch()
    {
        const Catalog c = enCatalog(QStringLiteral(
            "{\"k\": \"{n, plural, one {a {x} b} other {c {x} d}}\"}"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("1")},
                            {QStringLiteral("x"), QStringLiteral("Z")}})),
                 QStringLiteral("a Z b"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("2")},
                            {QStringLiteral("x"), QStringLiteral("Z")}})),
                 QStringLiteral("c Z d"));
    }

    // A nested plural inside a plural branch is NOT re-expanded (no
    // recursion — parity with the CLI): the inner block lands verbatim in
    // the output and its arg is appended as unconsumed.
    void nestedPluralIsNotRecursivelyExpanded()
    {
        const Catalog c = enCatalog(QStringLiteral(
            "{\"k\": \"{n, plural, other {{m, plural, other {z}}}}\"}"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("2")},
                            {QStringLiteral("m"), QStringLiteral("3")}})),
                 QStringLiteral("{m, plural, other {z}} m=3"));
    }

    // Unbalanced plural block (missing final close): emitted verbatim from
    // the opening brace; the plural arg stays unconsumed and appends.
    void unbalancedPluralBlockEmittedVerbatim()
    {
        const Catalog c = enCatalog(QStringLiteral(
            "{\"k\": \"{n, plural, other {x}\"}"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("{n, plural, other {x} n=1"));
    }

    // Pathological brace runs terminate and stay verbatim: deep balanced
    // nesting is not a plural (no comma) and a long open-brace run is
    // unbalanced.
    void pathologicalBraceRunsTerminate()
    {
        QString deep;
        for (int i = 0; i < 40; ++i)
            deep += QLatin1Char('{');
        deep += QLatin1Char('x');
        for (int i = 0; i < 40; ++i)
            deep += QLatin1Char('}');
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral("{\"deep\": \"%1\", \"open\": \"%2\"}")
                  .arg(deep, QString(500, QLatin1Char('{')))}});
        QCOMPARE(c.tr(QStringLiteral("deep")), deep);
        QCOMPARE(c.tr(QStringLiteral("open")), QString(500, QLatin1Char('{')));
    }

    // Plural arg values that are not plain numbers. Non-numeric values
    // select `other` and are still consumed (# takes the raw string) —
    // parity with the CLI, including the strict-whitespace rule
    // (f64::from_str rejects surrounding whitespace; the shell now
    // matches).
    void pluralArgValueNotANumber()
    {
        const Catalog c = enCatalog(QStringLiteral(
            "{\"k\": \"{n, plural, one {# layer} other {# layers}}\"}"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("abc")}})),
                 QStringLiteral("abc layers"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("1.0")}})),
                 QStringLiteral("1.0 layer"));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral("1e0")}})),
                 QStringLiteral("1e0 layer"));
        // Whitespace-wrapped numbers now parse strictly like the CLI
        // authority (Rust f64::from_str rejects " 1 "): category `other`,
        // raw string consumed by `#`. Was the tester's pinned divergence;
        // fixed shell-side, this is the parity regression witness.
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("n"), QStringLiteral(" 1 ")}})),
                 QStringLiteral(" 1  layers"));
        // Absent arg: branch body verbatim, `#` NOT substituted, nothing
        // consumed, nothing appended.
        QCOMPARE(c.tr(QStringLiteral("k")), QStringLiteral("# layers"));
    }

    // A plural without an `other` branch is not a plural block: verbatim +
    // unconsumed append. Same for ICU exact-match `=1` selectors (the
    // subset supports `one`/`other` only).
    void missingOtherAndExactSelectorsRejectBlock()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral("{\"noother\": \"{n, plural, one {x}}\","
                             " \"exact\": \"{n, plural, =1 {x} other {y}}\"}")}});
        QCOMPARE(c.tr(QStringLiteral("noother"),
                      args({{QStringLiteral("n"), QStringLiteral("2")}})),
                 QStringLiteral("{n, plural, one {x}} n=2"));
        QCOMPARE(c.tr(QStringLiteral("exact"),
                      args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("{n, plural, =1 {x} other {y}} n=1"));
    }

    // Huge template: linear scan must complete promptly and interpolate at
    // the far end (perf smoke: whole test file budget < 5 s).
    void hugeTemplateCompletesAndInterpolates()
    {
        const QString big = QString(200000, QLatin1Char('a'));
        const Catalog c = enCatalog(
            QStringLiteral("{\"k\": \"%1{x}\"}").arg(big));
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("x"), QStringLiteral("Z")}})),
                 big + QStringLiteral("Z"));
    }

    // Malformed catalog texts contribute NO chain entry (empty file, JSON
    // array, JSON null); an empty JSON object is a valid catalog with no
    // keys. Templates mapping to the empty string beat key-echo.
    void malformedAndEmptyCatalogTexts()
    {
        QCOMPARE(Catalog::fromTexts({{QStringLiteral("en"), QString()}}).hasEntries(),
                 false);
        QCOMPARE(Catalog::fromTexts({{QStringLiteral("en"), QStringLiteral("[1,2]")}})
                     .hasEntries(),
                 false);
        QCOMPARE(Catalog::fromTexts({{QStringLiteral("en"), QStringLiteral("null")}})
                     .hasEntries(),
                 false);
        const Catalog empty = enCatalog(QStringLiteral("{}"));
        QVERIFY(empty.hasEntries());
        QCOMPARE(empty.tr(QStringLiteral("k")), QStringLiteral("k")); // echo
        const Catalog blank = enCatalog(QStringLiteral("{\"k\": \"\"}"));
        QCOMPARE(blank.tr(QStringLiteral("k")), QString()); // catalogued empty wins
    }

    // A "messages" object shadows ALL top-level keys; a non-object
    // "messages" value leaves the flat map in force and is itself a
    // template. Parity with the CLI's parse_catalog.
    void messagesNestingShadowsTopLevel()
    {
        const Catalog nested = enCatalog(QStringLiteral(
            "{\"top\": \"TOP\", \"messages\": {\"a\": \"A\"}}"));
        QCOMPARE(nested.tr(QStringLiteral("a")), QStringLiteral("A"));
        QCOMPARE(nested.tr(QStringLiteral("top")), QStringLiteral("top")); // echo

        const Catalog flat = enCatalog(QStringLiteral(
            "{\"messages\": \"hi\", \"a\": \"A\"}"));
        QCOMPARE(flat.tr(QStringLiteral("a")), QStringLiteral("A"));
        QCOMPARE(flat.tr(QStringLiteral("messages")), QStringLiteral("hi"));
    }

    // Unknown _meta.plural_rules values fall to the language default
    // (en -> one_other, ja -> other_only); _meta of the wrong type too.
    void metaPluralRulesUnknownValueFallsToLangDefault()
    {
        const QString tmpl =
            QStringLiteral("\"k\": \"{n, plural, one {one} other {other}}\"");
        const Catalog enBogus = enCatalog(
            QStringLiteral("{\"_meta\": {\"plural_rules\": \"bogus\"}, %1}").arg(tmpl));
        QCOMPARE(enBogus.tr(QStringLiteral("k"),
                            args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("one"));
        const Catalog jaBogus = Catalog::fromTexts(
            {{QStringLiteral("ja"),
              QStringLiteral("{\"_meta\": {\"plural_rules\": \"bogus\"}, %1}").arg(tmpl)}});
        QCOMPARE(jaBogus.tr(QStringLiteral("k"),
                            args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("other"));
        const Catalog enMetaString = enCatalog(
            QStringLiteral("{\"_meta\": \"other_only\", %1}").arg(tmpl));
        QCOMPARE(enMetaString.tr(QStringLiteral("k"),
                                 args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("one")); // non-object _meta ignored
        // And the reverse of the builder's ja case: an en catalog may
        // legitimately declare other_only.
        const Catalog enOtherOnly = enCatalog(
            QStringLiteral("{\"_meta\": {\"plural_rules\": \"other_only\"}, %1}").arg(tmpl));
        QCOMPARE(enOtherOnly.tr(QStringLiteral("k"),
                                args({{QStringLiteral("n"), QStringLiteral("1")}})),
                 QStringLiteral("other"));
    }

    // Requested-locale file present but malformed JSON: the loader must
    // skip it and the chain must fall to en cleanly.
    void malformedLocaleFileFallsThroughToEn()
    {
        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        {
            QFile es(dir.filePath(QStringLiteral("es.json")));
            QVERIFY(es.open(QIODevice::WriteOnly));
            es.write("{ this is not json ");
        }
        {
            QFile en(dir.filePath(QStringLiteral("en.json")));
            QVERIFY(en.open(QIODevice::WriteOnly));
            en.write("{\"tst.chain\": \"from-en\"}");
        }
        qputenv("APPNAME_I18N_DIR", dir.path().toUtf8());
        const Catalog c = Catalog::load(QStringLiteral("es"));
        qunsetenv("APPNAME_I18N_DIR");
        QVERIFY(c.hasEntries());
        QCOMPARE(c.tr(QStringLiteral("tst.chain")), QStringLiteral("from-en"));
    }

    // APPNAME_I18N_DIR holding only the requested locale: the missing en
    // file must fall through PER-FILE to the exe-adjacent i18n/ directory
    // (staged catalogs in the build dir). The witness key is picked
    // dynamically from the real exe-adjacent en.json so this does not pin
    // catalog content.
    void envDirMissingLocaleFallsThroughPerFile()
    {
        const QString exeEnPath =
            QDir(QCoreApplication::applicationDirPath())
                .filePath(QStringLiteral("i18n/en.json"));
        QFile exeEn(exeEnPath);
        if (!exeEn.open(QIODevice::ReadOnly))
            QSKIP("no exe-adjacent i18n/en.json staged next to the test binary");
        const QJsonDocument doc = QJsonDocument::fromJson(exeEn.readAll());
        QVERIFY(doc.isObject());
        QString witnessKey;
        const QJsonObject root = doc.object();
        const QJsonObject map = root.value(QLatin1String("messages")).isObject()
            ? root.value(QLatin1String("messages")).toObject()
            : root;
        for (auto it = map.constBegin(); it != map.constEnd(); ++it) {
            if (it.key() != QLatin1String("_meta") && it.value().isString()
                && it.value().toString() != it.key()) {
                witnessKey = it.key();
                break;
            }
        }
        if (witnessKey.isEmpty())
            QSKIP("exe-adjacent en.json has no usable witness key");

        QTemporaryDir dir;
        QVERIFY(dir.isValid());
        {
            QFile es(dir.filePath(QStringLiteral("es.json")));
            QVERIFY(es.open(QIODevice::WriteOnly));
            es.write("{\"tst.unique.only_es\": \"desde-es\"}");
        }
        qputenv("APPNAME_I18N_DIR", dir.path().toUtf8());
        const Catalog c = Catalog::load(QStringLiteral("es"));
        qunsetenv("APPNAME_I18N_DIR");
        // es came from the env dir...
        QCOMPARE(c.tr(QStringLiteral("tst.unique.only_es")), QStringLiteral("desde-es"));
        // ...and en fell through to the exe-adjacent staged catalog: the
        // witness key resolves to a template, not a key-echo.
        QVERIFY(c.tr(witnessKey) != witnessKey);
    }

    // Key-echo append format is a stability contract (status bar + logs
    // parse it): space-separated name=value in caller order, empty values
    // and spaces inside values verbatim, empty key echoes empty.
    void keyEchoArgAppendFormatStability()
    {
        const Catalog c = Catalog::fromTexts({});
        QCOMPARE(c.tr(QStringLiteral("k"),
                      args({{QStringLiteral("b"), QString()},
                            {QStringLiteral("a"), QStringLiteral("x y")},
                            {QStringLiteral("u"), QString::fromUtf8("日本")}})),
                 QString::fromUtf8("k b= a=x y u=日本"));
        QCOMPARE(c.tr(QString(), args({{QStringLiteral("a"), QStringLiteral("1")}})),
                 QStringLiteral(" a=1"));
        QCOMPARE(c.tr(QString()), QString());
    }

    // Placeholder replacement order semantics: values containing brace
    // syntax are re-scanned by LATER placeholder replacements (documented
    // injection order — identical algorithm to the CLI, so parity holds).
    // Every occurrence of a placeholder is replaced; consumption is
    // per-arg, not per-occurrence.
    void placeholderReplaceOrderAndAllOccurrences()
    {
        const Catalog c = Catalog::fromTexts(
            {{QStringLiteral("en"),
              QStringLiteral("{\"inj\": \"A {a} B {b}\", \"dup\": \"{a}+{a}\"}")}});
        QCOMPARE(c.tr(QStringLiteral("inj"),
                      args({{QStringLiteral("a"), QStringLiteral("{b}")},
                            {QStringLiteral("b"), QStringLiteral("V")}})),
                 QStringLiteral("A V B V"));
        QCOMPARE(c.tr(QStringLiteral("dup"),
                      args({{QStringLiteral("a"), QStringLiteral("x")}})),
                 QStringLiteral("x+x"));
    }

    // QML face: numeric QVariant args stringify before plural evaluation
    // (int 1 -> "1" -> category one; double 3.0 -> "3" -> other).
    void qmlFacePluralWithNumericVariants()
    {
        CatalogQml qml(enCatalog(QStringLiteral(
            "{\"k\": \"{count, plural, one {# layer} other {# layers}}\"}")));
        QVariantMap one;
        one.insert(QStringLiteral("count"), 1);
        QCOMPARE(qml.tr(QStringLiteral("k"), one), QStringLiteral("1 layer"));
        QVariantMap three;
        three.insert(QStringLiteral("count"), 3.0);
        QCOMPARE(qml.tr(QStringLiteral("k"), three), QStringLiteral("3 layers"));
    }

    // resolveLang's answer becomes a PATH — loadLangFile builds
    // `<dir>/<lang>.json` from it — so a tag that is not a bare token can walk
    // out of the catalog directory. Robustness, NOT a privilege boundary: this
    // is a local app and whoever passes --lang already owns those files. What
    // is pinned here is that a malformed tag cannot silently become a path.
    void resolveLangRejectsShapesThatCouldLeaveTheCatalogDirectory()
    {
        // ★ ACCEPTS FIRST, AND THEY ARE THE LOAD-BEARING HALF. A guard that
        // rejected EVERYTHING would satisfy every rejection case below while
        // making the app monolingual. These pin that a well-formed tag comes
        // back UNCHANGED, so over-rejection reds here rather than shipping.
        // ★ "en" IS DELIBERATELY FIRST AND DELIBERATELY CANNOT DISCRIMINATE:
        // it is also the FALLBACK value, so under a guard that rejected
        // everything this one row would still pass — its expected value and
        // the failure value coincide. Measured, not reasoned: mutating the
        // guard to `return false` reds at "es" (line below), never at "en".
        // It stays as documentation that the fallback is "en", and the rows
        // after it are what actually hold the accept half up. Do not trim this
        // list to its first element.
        const QStringList wellFormed{
            QStringLiteral("en"),    QStringLiteral("es"),
            QStringLiteral("ja"),    QStringLiteral("pseudo"),
            QStringLiteral("pt-BR"), QStringLiteral("zh_Hans"),
        };
        for (const QString &tag : wellFormed) {
            QCOMPARE(Catalog::resolveLang(tag), tag);
        }

        // Rejected SHAPES. Each would otherwise reach QDir::filePath.
        const QStringList malformed{
            QStringLiteral("../../etc/passwd"),
            QStringLiteral("../en"),
            QStringLiteral("a/b"),
            QStringLiteral("en.json"),
            QStringLiteral("."),
            QStringLiteral(".."),
            QStringLiteral("e n"),
            QStringLiteral("-en"),
            QStringLiteral("1en"),
        };
        for (const QString &tag : malformed) {
            // ANNOUNCED, NOT SILENT — and asserted, not asserted in prose.
            // ignoreMessage fails the case if the warning never arrives, so
            // this pins the honesty of the fallback, not just its value.
            QTest::ignoreMessage(
                QtWarningMsg,
                qPrintable(QStringLiteral("i18n: --lang value \"%1\" is not a "
                                          "well-formed language tag; falling "
                                          "back to \"en\"")
                               .arg(tag)));
            QCOMPARE(Catalog::resolveLang(tag), QStringLiteral("en"));
        }

        // The environment path is the same door and gets the same guard. It is
        // reached only when the flag is empty, which is why the flag argument
        // is empty here rather than absent.
        const QByteArray saved = qgetenv("APPNAME_LANG");
        const bool had = qEnvironmentVariableIsSet("APPNAME_LANG");
        qputenv("APPNAME_LANG", QByteArray("../../etc/passwd"));
        QTest::ignoreMessage(QtWarningMsg,
                             "i18n: APPNAME_LANG value \"../../etc/passwd\" is "
                             "not a well-formed language tag; falling back to "
                             "\"en\"");
        QCOMPARE(Catalog::resolveLang(QString()), QStringLiteral("en"));
        // ...and a well-formed env value still survives, so the env arm is not
        // simply dead.
        qputenv("APPNAME_LANG", QByteArray("ja"));
        QCOMPARE(Catalog::resolveLang(QString()), QStringLiteral("ja"));
        if (had)
            qputenv("APPNAME_LANG", saved);
        else
            qunsetenv("APPNAME_LANG");
    }
};

QTEST_GUILESS_MAIN(TstI18nCatalogAdversarial)
#include "tst_i18n_catalog_adversarial.moc"
