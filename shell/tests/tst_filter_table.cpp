// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: reviewer-specified mechanical pin (A11Y-1 review MAJOR-1 — the
// sigmaPx drift class RECURRED as a hand-transcribed param table three
// filters stale). This test parses FilterParamDialog.qml's `tables`
// block AND the engine corpus sources' REGISTERED schema strings, and
// diffs name/min/max/default/integer-typing per filter — the QML table
// can no longer drift silently from engine truth. Paths are injected by
// the build (checked-in sources, not copies — the tst_theme_contrast
// pattern).

#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <QTest>

namespace {

QString readAll(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly | QIODevice::Text))
        return {};
    return QString::fromUtf8(f.readAll());
}

struct Row {
    QString name;
    double min = 0;
    double max = 0;
    double def = 0;
    bool integer = false;
};

// Parse the QML `tables` object: per filter id, the row list.
QHash<QString, QVector<Row>> parseQmlTable(const QString &qml)
{
    QHash<QString, QVector<Row>> out;
    static const QRegularExpression filterRe(QStringLiteral(
        "\"(filter\\.[a-z_]+)\":\\s*\\[(.*?)\\]"),
        QRegularExpression::DotMatchesEverythingOption);
    static const QRegularExpression rowRe(QStringLiteral(
        "\\{\\s*name:\\s*\"([a-z_]+)\"[^}]*?"
        "min:\\s*([0-9.]+)[^}]*?max:\\s*([0-9.]+)[^}]*?"
        "def:\\s*([0-9.]+)[^}]*?integer:\\s*(true|false)"),
        QRegularExpression::DotMatchesEverythingOption);
    auto fit = filterRe.globalMatch(qml);
    while (fit.hasNext()) {
        const auto fm = fit.next();
        QVector<Row> rows;
        auto rit = rowRe.globalMatch(fm.captured(2));
        while (rit.hasNext()) {
            const auto rm = rit.next();
            Row r;
            r.name = rm.captured(1);
            r.min = rm.captured(2).toDouble();
            r.max = rm.captured(3).toDouble();
            r.def = rm.captured(4).toDouble();
            r.integer = rm.captured(5) == QLatin1String("true");
            rows.append(r);
        }
        out.insert(fm.captured(1), rows);
    }
    return out;
}

// Extract every registered schema JSON string from a corpus .rs source
// (the r#"{"$id":...}"# raw strings) keyed by the schema $id's filter
// stem.
QHash<QString, QJsonObject> parseCorpusSchemas(const QStringList &sources)
{
    QHash<QString, QJsonObject> out;
    static const QRegularExpression schemaRe(QStringLiteral(
        "r#\"(\\{\"\\$id\":\"https://appname\\.org/spec/filter/"
        "([a-z_]+)\\.v1\\.json\".*?\\})\"#"),
        QRegularExpression::DotMatchesEverythingOption);
    for (const QString &src : sources) {
        auto it = schemaRe.globalMatch(src);
        while (it.hasNext()) {
            const auto m = it.next();
            const QJsonDocument doc =
                QJsonDocument::fromJson(m.captured(1).toUtf8());
            if (doc.isObject())
                out.insert(QStringLiteral("filter.") + m.captured(2),
                           doc.object());
        }
    }
    return out;
}

} // namespace

class TstFilterTable : public QObject
{
    Q_OBJECT

private slots:
    void tableMatchesRegisteredSchemas();
};

void TstFilterTable::tableMatchesRegisteredSchemas()
{
    const QString qml = readAll(QStringLiteral(APPNAME_FILTER_DIALOG_QML));
    QVERIFY2(!qml.isEmpty(), "FilterParamDialog.qml unreadable");
    const auto table = parseQmlTable(qml);
    QVERIFY2(table.size() == 6,
             qPrintable(QStringLiteral("expected 6 filters in the QML table, parsed %1")
                            .arg(table.size())));
    // Row-level empty-parse guard (review round 2, probe C): the filter
    // key can match while its ROW regex silently fails (a renamed or
    // reordered field), leaving an empty vector the per-row loop never
    // enters — the pin would disarm without going red. Assert every
    // filter parses at least one row AND the total is the known 8
    // (1+1+1+1+1+3); any knockout now reds loudly instead of skipping.
    int totalRows = 0;
    for (auto it = table.constBegin(); it != table.constEnd(); ++it) {
        QVERIFY2(!it.value().isEmpty(),
                 qPrintable(QStringLiteral("%1: filter key matched but ZERO rows parsed "
                                           "(row regex knockout — pin would silently disarm)")
                                .arg(it.key())));
        totalRows += it.value().size();
    }
    QVERIFY2(totalRows == 8,
             qPrintable(QStringLiteral("expected 8 param rows across the table, parsed %1")
                            .arg(totalRows)));

    QStringList sources;
    for (const QString &f :
         {QStringLiteral("gaussian.rs"), QStringLiteral("box_blur.rs"),
          QStringLiteral("median.rs"), QStringLiteral("sharpen.rs"),
          QStringLiteral("noise.rs")}) {
        const QString src =
            readAll(QStringLiteral(APPNAME_FILTER_CORPUS_DIR) + QLatin1Char('/') + f);
        QVERIFY2(!src.isEmpty(), qPrintable(f + QStringLiteral(" unreadable")));
        sources.append(src);
    }
    const auto schemas = parseCorpusSchemas(sources);

    for (auto it = table.constBegin(); it != table.constEnd(); ++it) {
        QVERIFY2(schemas.contains(it.key()),
                 qPrintable(QStringLiteral("no registered schema found for %1")
                                .arg(it.key())));
        const QJsonObject props =
            schemas.value(it.key()).value(QLatin1String("properties")).toObject();
        for (const Row &row : it.value()) {
            QVERIFY2(props.contains(row.name),
                     qPrintable(QStringLiteral("%1: param %2 not in the registered schema (dead name)")
                                    .arg(it.key(), row.name)));
            const QJsonObject p = props.value(row.name).toObject();
            const auto chk = [&](const char *field, double qmlVal) {
                QVERIFY2(qFuzzyCompare(1.0 + p.value(QLatin1String(field)).toDouble(),
                                       1.0 + qmlVal),
                         qPrintable(QStringLiteral("%1.%2 %3: qml %4 != schema %5")
                                        .arg(it.key(), row.name,
                                             QLatin1String(field))
                                        .arg(qmlVal)
                                        .arg(p.value(QLatin1String(field)).toDouble())));
            };
            // Minimum: the UI floor may TIGHTEN (>=) for spinbox
            // granularity (the reviewer-sanctioned add_noise 0.01 case:
            // 0.001 is unrepresentable at x100 scale) — but NEVER loosen
            // below the schema (an out-of-range emission). Maximum and
            // default are EXACT: a lowered max is a client-owned wall
            // (the engine owns walls), and the default-omission rule is
            // a lie unless def == the engine default.
            const double schemaMin =
                p.value(QLatin1String("minimum")).toDouble();
            QVERIFY2(row.min >= schemaMin - 1e-12,
                     qPrintable(QStringLiteral("%1.%2 minimum: qml %3 LOOSER than schema %4")
                                    .arg(it.key(), row.name)
                                    .arg(row.min)
                                    .arg(schemaMin)));
            chk("maximum", row.max);
            chk("default", row.def);
            const bool schemaInt =
                p.value(QLatin1String("type")).toString() == QLatin1String("integer");
            QVERIFY2(schemaInt == row.integer,
                     qPrintable(QStringLiteral("%1.%2: integer flag qml %3 != schema type %4")
                                    .arg(it.key(), row.name)
                                    .arg(row.integer)
                                    .arg(p.value(QLatin1String("type")).toString())));
        }
    }
}

QTEST_GUILESS_MAIN(TstFilterTable)
#include "tst_filter_table.moc"
