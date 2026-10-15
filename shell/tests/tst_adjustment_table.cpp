// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// THE ADJUSTMENT PARAM TABLE PIN (v1.0.1 item 4 class (a)).
//
// AdjustmentParamDialog.qml hand-writes bounds for the three served
// adjustment kinds because `param_schema()` is a Rust trait method with
// ZERO hits in the cabi crate — the shell cannot read the engine's
// schema at runtime. This test is why that is a CONVENIENCE gap and not
// a correctness one: it parses BOTH sides and compares them, the same
// arrangement tst_filter_table uses, which caught three drifted filters
// in review MAJOR-1.
//
// ONE DIFFERENCE FROM THAT PRECEDENT, and it is load-bearing: its row
// regex captures numbers as `[0-9.]+`, which cannot match a NEGATIVE.
// The filter corpus happens to have no negative bounds so it never
// mattered there. Adjustments have several (-1, -20, -0.5), and a regex
// that silently fails to match a row leaves an empty vector the compare
// loop never enters — the pin would disarm without going red. Hence
// `-?` below AND the same row-count guards the filter pin carries.

#include <QFile>
#include <QHash>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <QTest>

namespace {

struct Row {
    QString name;
    double min = 0, max = 0, def = 0;
    bool integer = false;
};

QString readAll(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly))
        return {};
    return QString::fromUtf8(f.readAll());
}

QHash<QString, QVector<Row>> parseQmlTable(const QString &qml)
{
    QHash<QString, QVector<Row>> out;
    static const QRegularExpression opRe(QStringLiteral(
        "\"(adjustment\\.[a-z_]+)\":\\s*\\[(.*?)\\n        \\]"),
        QRegularExpression::DotMatchesEverythingOption);
    // `-?` on every number: see the header note.
    static const QRegularExpression rowRe(QStringLiteral(
        // [a-zA-Z], not [a-z_]: a SECOND character-class assumption in my
        // own parser, the same family as the `[0-9.]+` that could not
        // match a negative. The scalar ops happen to use all-lowercase
        // param names (brightness, contrast, legacy, exposure, offset,
        // gamma, level), so it never mattered — until hue_saturation
        // arrived with colorizeHue/colorizeSat/colorizeLightness and the
        // parser silently saw ONE row of four. A regex that cannot match
        // a legal name does not fail; it under-counts.
        "\\{\\s*name:\\s*\"([a-zA-Z_]+)\"[^}]*?"
        "min:\\s*(-?[0-9.]+)[^}]*?max:\\s*(-?[0-9.]+)[^}]*?"
        "def:\\s*(-?[0-9.]+)[^}]*?integer:\\s*(true|false)"),
        QRegularExpression::DotMatchesEverythingOption);
    auto oit = opRe.globalMatch(qml);
    while (oit.hasNext()) {
        const auto om = oit.next();
        QVector<Row> rows;
        auto rit = rowRe.globalMatch(om.captured(2));
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
        out.insert(om.captured(1), rows);
    }
    return out;
}

// Pull each op's registered JSON Schema out of the engine source.
//
// CORRECTION TO THIS COMMENT'S FIRST VERSION, kept because the wrong
// reason is the dangerous part. It said the structured ops "simply do
// not match here" because they are MULTI-LINE. They are not multi-line.
// They are delimited `r##"…"##` instead of `r#"…"#`, and the reason is
// mechanical: their JSON contains `"#/$defs/tuple"`, and the `"#`
// sequence inside it would TERMINATE a single-hash raw string, so Rust
// required a second hash. My regex asked for exactly one.
//
// The exclusion was therefore ACCIDENTALLY correct — right answer, wrong
// mechanism — and a reader trusting the stated reason would have
// concluded that adding a structured op to the dialog was safe because
// its schema "cannot be parsed". `r#+` now matches both, so the parser
// sees all eight ops and the dialog's contents decide what is checked.
QHash<QString, QJsonObject> parseEngineSchemas(const QString &rs)
{
    QHash<QString, QJsonObject> out;
    static const QRegularExpression schemaRe(QStringLiteral(
        "r(#+)\"(\\{\"\\$id\":\"https://appname\\.org/spec/adjustment/"
        "([a-z_]+)\\.v1\\.json\".*?)\"\\1"));
    auto it = schemaRe.globalMatch(rs);
    while (it.hasNext()) {
        const auto m = it.next();
        const QJsonDocument d =
            QJsonDocument::fromJson(m.captured(2).toUtf8());
        if (d.isObject())
            out.insert(QStringLiteral("adjustment.") + m.captured(3),
                       d.object());
    }
    return out;
}

} // namespace

class TstAdjustmentTable : public QObject
{
    Q_OBJECT
private slots:
    void qmlTableMatchesTheEngineSchemas();
    void everyRegisteredOpIsAccountedFor();
    void nothingIsAppliedUNTILApplyIsPressed();
};

void TstAdjustmentTable::qmlTableMatchesTheEngineSchemas()
{
    const QString qml = readAll(QStringLiteral(APPNAME_ADJUST_DIALOG_QML));
    QVERIFY2(!qml.isEmpty(), "AdjustmentParamDialog.qml unreadable");
    const auto table = parseQmlTable(qml);
    // FOUR, not three: the flat-row regex also matches `groupedFlat`'s
    // hue_saturation entry, which is correct — those four top-level
    // params are flat rows and belong in this comparison exactly like the
    // scalar ops' rows do.
    QCOMPARE(table.size(), 4);

    // VACUITY GUARDS, the filter pin's shape: an op key can match while
    // its ROW regex silently fails, leaving an empty vector the compare
    // loop never enters. Assert every op parses at least one row AND the
    // total is the known 11 (brightness_contrast 3 + exposure 3 +
    // threshold 1 + hue_saturation's four flat colorize rows).
    int totalRows = 0;
    for (auto it = table.constBegin(); it != table.constEnd(); ++it) {
        QVERIFY2(!it.value().isEmpty(),
                 qPrintable(QStringLiteral("%1: op key matched but ZERO rows "
                                           "parsed — the pin would silently "
                                           "disarm").arg(it.key())));
        totalRows += it.value().size();
    }
    QCOMPARE(totalRows, 11);

    const QString rs = readAll(QStringLiteral(APPNAME_ADJUST_OPS_RS));
    QVERIFY2(!rs.isEmpty(), "appname-adjust/src/ops.rs unreadable");
    const auto schemas = parseEngineSchemas(rs);
    // EXACTLY EIGHT, not "at least three". The loose bound was what let a
    // regex that could only see FIVE of them pass unnoticed: `r#"` matched
    // the scalar ops and missed levels/curves/hue_saturation, which are
    // `r##"`-delimited because their JSON contains `"#/$defs/…`. A
    // widened regex with a loose assertion is a fix with no evidence it
    // did anything, so the number is pinned to the engine's real count.
    QCOMPARE(schemas.size(), 8);

    // GROUPED OPS, BOTH OF THEM, FROM ONE LOOP. The dialog declares each
    // op's tuple ONCE and the engine declares it once too, as a $defs
    // entry referenced by every channel — so the comparison resolves the
    // $ref rather than walking the channel keys, which would report N
    // passes for one fact.
    //
    // hue_saturation joined this loop at class (c) instead of getting a
    // copy of the levels block. C-421 filed it as needing a
    // purpose-built editor; measured, it is seven bands x the same three
    // bounded numbers plus four flat top-level params — the levels shape
    // with a different tuple. A second copied block here would have been
    // the same mistake the dialog avoided by sharing its delegate.
    struct Grouped { const char *op; const char *defName; const char *qmlProp;
                     int channels; int tupleSize; };
    static const Grouped kGrouped[] = {
        { "adjustment.levels",         "tuple", "levelsTuple: [", 4, 5 },
        { "adjustment.hue_saturation", "band",  "hueSatBand: [",  7, 3 },
    };
    for (const Grouped &g : kGrouped) {
        const QJsonObject sch = schemas.value(QLatin1String(g.op));
        QVERIFY2(!sch.isEmpty(),
                 qPrintable(QStringLiteral("%1 schema not parsed from the engine")
                                .arg(QLatin1String(g.op))));
        const QJsonObject tuple =
            sch.value(QStringLiteral("$defs")).toObject()
               .value(QLatin1String(g.defName)).toObject()
               .value(QStringLiteral("properties")).toObject();
        QCOMPARE(tuple.size(), g.tupleSize);
        // Count the $ref channels specifically: hue_saturation's
        // top-level ALSO holds four flat scalars, so "properties.size()"
        // is not the channel count for every grouped op.
        const QJsonObject top = sch.value(QStringLiteral("properties")).toObject();
        int refChannels = 0;
        for (const QString &c : top.keys())
            if (top.value(c).toObject().contains(QStringLiteral("$ref"))) {
                QCOMPARE(top.value(c).toObject().value(QStringLiteral("$ref")).toString(),
                         QStringLiteral("#/$defs/") + QLatin1String(g.defName));
                ++refChannels;
            }
        QCOMPARE(refChannels, g.channels);

        static const QRegularExpression tupleRowRe(QStringLiteral(
            "\\{\\s*name:\\s*\"([a-zA-Z]+)\"[^}]*?"
            "min:\\s*(-?[0-9.]+)[^}]*?max:\\s*(-?[0-9.]+)[^}]*?"
            "def:\\s*(-?[0-9.]+)"),
            QRegularExpression::DotMatchesEverythingOption);
        const int tStart = qml.indexOf(QLatin1String(g.qmlProp));
        QVERIFY2(tStart > 0,
                 qPrintable(QStringLiteral("%1 block not found in the dialog")
                                .arg(QLatin1String(g.qmlProp))));
        const int tEnd = qml.indexOf(QStringLiteral("\n    ]"), tStart);
        QVERIFY2(tEnd > tStart, "tuple block not terminated");
        const QString block = qml.mid(tStart, tEnd - tStart);
        auto tit = tupleRowRe.globalMatch(block);
        int seen = 0;
        while (tit.hasNext()) {
            const auto tm = tit.next();
            const QString name = tm.captured(1);
            QVERIFY2(tuple.contains(name),
                     qPrintable(QStringLiteral("%1.%2 is in the dialog and NOT "
                                               "in the engine tuple")
                                    .arg(QLatin1String(g.op), name)));
            const QJsonObject pj = tuple.value(name).toObject();
            QCOMPARE(tm.captured(2).toDouble(), pj.value(QStringLiteral("minimum")).toDouble());
            QCOMPARE(tm.captured(3).toDouble(), pj.value(QStringLiteral("maximum")).toDouble());
            QCOMPARE(tm.captured(4).toDouble(), pj.value(QStringLiteral("default")).toDouble());
            ++seen;
        }
        // Vacuity guard: a knocked-out row regex leaves `seen` at 0 and
        // the loop above asserts nothing.
        QCOMPARE(seen, g.tupleSize);
    }


    // GROUPED OPS: the dialog declares the tuple ONCE and the engine
    // declares it once too, as $defs/tuple referenced by every channel.
    // So the comparison resolves the $ref rather than walking the four
    // channel keys — checking the same five bounds four times would
    // report four passes for one fact.
    {
        const QString qmlAll = qml;
        const QJsonObject lv = schemas.value(QStringLiteral("adjustment.levels"));
        QVERIFY2(!lv.isEmpty(), "levels schema not parsed from the engine");
        const QJsonObject defs = lv.value(QStringLiteral("$defs")).toObject();
        const QJsonObject tuple =
            defs.value(QStringLiteral("tuple")).toObject()
                .value(QStringLiteral("properties")).toObject();
        QCOMPARE(tuple.size(), 5);
        // the four channels all point at that one tuple
        const QJsonObject chans = lv.value(QStringLiteral("properties")).toObject();
        QCOMPARE(chans.size(), 4);
        for (const QString &c : chans.keys())
            QCOMPARE(chans.value(c).toObject().value(QStringLiteral("$ref")).toString(),
                     QStringLiteral("#/$defs/tuple"));
        // …and the dialog's shared tuple matches it, row for row.
        static const QRegularExpression tupleRowRe(QStringLiteral(
            "\\{\\s*name:\\s*\"([a-zA-Z]+)\"[^}]*?"
            "min:\\s*(-?[0-9.]+)[^}]*?max:\\s*(-?[0-9.]+)[^}]*?"
            "def:\\s*(-?[0-9.]+)"),
            QRegularExpression::DotMatchesEverythingOption);
        const int tupleStart = qmlAll.indexOf(QStringLiteral("levelsTuple: ["));
        QVERIFY2(tupleStart > 0, "levelsTuple block not found in the dialog");
        const int tupleEnd = qmlAll.indexOf(QStringLiteral("\n    ]"), tupleStart);
        QVERIFY2(tupleEnd > tupleStart, "levelsTuple block not terminated");
        const QString block = qmlAll.mid(tupleStart, tupleEnd - tupleStart);
        auto tit = tupleRowRe.globalMatch(block);
        int seen = 0;
        while (tit.hasNext()) {
            const auto tm = tit.next();
            const QString name = tm.captured(1);
            QVERIFY2(tuple.contains(name),
                     qPrintable(QStringLiteral("levels.%1 is in the dialog and "
                                               "NOT in the engine tuple").arg(name)));
            const QJsonObject pj = tuple.value(name).toObject();
            QCOMPARE(tm.captured(2).toDouble(), pj.value(QStringLiteral("minimum")).toDouble());
            QCOMPARE(tm.captured(3).toDouble(), pj.value(QStringLiteral("maximum")).toDouble());
            QCOMPARE(tm.captured(4).toDouble(), pj.value(QStringLiteral("default")).toDouble());
            ++seen;
        }
        // The vacuity guard again: a knocked-out row regex would leave
        // `seen` at 0 and the loop above would assert nothing.
        QCOMPARE(seen, 5);
    }

    // CURVES: the numbers that matter are the POINT-COUNT limits and the
    // coordinate range, because the payload is a variable-length list
    // rather than a fixed tuple. The editor hard-codes minPoints/
    // maxPoints and clamps to [0..1]; all three come from the engine and
    // all three are pinned here.
    {
        const QJsonObject cur = schemas.value(QStringLiteral("adjustment.curves"));
        QVERIFY2(!cur.isEmpty(), "curves schema not parsed from the engine");
        const QJsonObject pts =
            cur.value(QStringLiteral("$defs")).toObject()
               .value(QStringLiteral("pts")).toObject();
        const int engMin = pts.value(QStringLiteral("minItems")).toInt();
        const int engMax = pts.value(QStringLiteral("maxItems")).toInt();
        const QJsonObject coord =
            pts.value(QStringLiteral("items")).toObject()
               .value(QStringLiteral("items")).toObject();
        const QString ed = readAll(QStringLiteral(APPNAME_CURVE_EDITOR_QML));
        QVERIFY2(!ed.isEmpty(), "CurveEditor.qml unreadable");
        auto intAfter = [&ed](const QString &prop) {
            static const QString pat = QStringLiteral("property int %1: ");
            const int at = ed.indexOf(pat.arg(prop));
            if (at < 0) return -1;
            return ed.mid(at + pat.arg(prop).size(), 4).split(QLatin1Char('\n'))
                     .first().trimmed().toInt();
        };
        QCOMPARE(intAfter(QStringLiteral("minPoints")), engMin);
        QCOMPARE(intAfter(QStringLiteral("maxPoints")), engMax);
        // the coordinate clamp
        QCOMPARE(coord.value(QStringLiteral("minimum")).toDouble(), 0.0);
        QCOMPARE(coord.value(QStringLiteral("maximum")).toDouble(), 1.0);
        QVERIFY2(ed.contains(QStringLiteral("Math.max(0, Math.min(1,")),
                 "CurveEditor does not clamp coordinates to the engine's [0..1]");
        // four channels, each a pts
        const QJsonObject top = cur.value(QStringLiteral("properties")).toObject();
        QCOMPARE(top.size(), 4);
        for (const QString &c : top.keys())
            QCOMPARE(top.value(c).toObject().value(QStringLiteral("$ref")).toString(),
                     QStringLiteral("#/$defs/pts"));
    }

    for (auto it = table.constBegin(); it != table.constEnd(); ++it) {
        const QString op = it.key();
        QVERIFY2(schemas.contains(op),
                 qPrintable(QStringLiteral("%1 is in the dialog and NOT in the "
                                           "engine registry").arg(op)));
        const QJsonObject props =
            schemas.value(op).value(QStringLiteral("properties")).toObject();
        for (const Row &r : it.value()) {
            QVERIFY2(props.contains(r.name),
                     qPrintable(QStringLiteral("%1.%2 is in the dialog and NOT "
                                               "in the engine schema")
                                    .arg(op, r.name)));
            const QJsonObject p = props.value(r.name).toObject();
            // A boolean param has no numeric bounds to compare; its row
            // carries placeholders and the `bool: true` flag is what the
            // dialog reads. Everything else must match the engine EXACTLY.
            if (p.value(QStringLiteral("type")).toString()
                == QLatin1String("boolean"))
                continue;
            QCOMPARE(r.min, p.value(QStringLiteral("minimum")).toDouble());
            QCOMPARE(r.max, p.value(QStringLiteral("maximum")).toDouble());
            QCOMPARE(r.def, p.value(QStringLiteral("default")).toDouble());
        }
        // …and no engine param is MISSING from the dialog, which the loop
        // above cannot see: it walks the dialog's rows, so a param the
        // engine gained would be silently unoffered.
        //
        // Count only the NON-$ref properties. hue_saturation's top level
        // holds seven $ref bands beside its four flat params, and those
        // bands are checked by the grouped loop above — counting them
        // here would demand the flat table carry rows it must not have.
        int flatProps = 0;
        for (const QString &k : props.keys())
            if (!props.value(k).toObject().contains(QStringLiteral("$ref")))
                ++flatProps;
        QCOMPARE(it.value().size(), flatProps);
    }
}

void TstAdjustmentTable::everyRegisteredOpIsAccountedFor()
{
    // THIS CASE REPLACED an absence check that had nothing left to
    // assert. It listed curves, levels and hue_saturation as ops the
    // scalar dialog must NOT contain; levels left at class (b),
    // hue_saturation at (c), and curves at (c) with its own editor — so
    // the list emptied one op at a time until the case was vacuous.
    //
    // A guard whose subject dissolves should be REPLACED, not deleted:
    // what actually matters now is that no registered op is silently
    // unoffered. Every one of the engine's eight must be in the scalar
    // table, the grouped table, the curve table, or on the short list of
    // ops that deliberately have no dialog — and that last list is
    // spelled out so adding a ninth op forces a decision instead of
    // defaulting to silence.
    const QString qml = readAll(QStringLiteral(APPNAME_ADJUST_DIALOG_QML));
    const QString rs = readAll(QStringLiteral(APPNAME_ADJUST_OPS_RS));
    const auto schemas = parseEngineSchemas(rs);
    QCOMPARE(schemas.size(), 8);

    // invert has ZERO properties — a dialog with no rows would be chrome
    // pretending there is a decision. posterize is a single integer that
    // no menu offers yet; it is named here so its absence is a recorded
    // choice rather than an oversight.
    const QSet<QString> deliberatelyNoDialog = {
        QStringLiteral("adjustment.invert"),
        QStringLiteral("adjustment.posterize")
    };
    QStringList unoffered;
    for (const QString &op : schemas.keys()) {
        if (deliberatelyNoDialog.contains(op))
            continue;
        if (!qml.contains(op))
            unoffered << op;
    }
    QVERIFY2(unoffered.isEmpty(),
             qPrintable(QStringLiteral("registered ops the dialog never offers "
                                       "and that are not on the no-dialog list: %1")
                            .arg(unoffered.join(QStringLiteral(", ")))));
}

// ADDED C-757 — B-03's "cancel-restores" is TRUE BY CONSTRUCTION, and this
// pins the CONSTRUCTION rather than the clause. The dialog mutates the
// document in exactly one place: the Apply button. Cancel closes and nothing
// needs restoring because nothing was applied.
//
// WHY PIN A PROPERTY THAT IS TRIVIALLY TRUE. B-03 also asks for a LIVE
// PREVIEW, which this dialog does not have and cannot get from the shell
// alone (measured C-757: 0 preview/dry-run doors in crates/appname-cabi/src
// against 38 no_mangle and 10 apply_command in the same 5342 lines, and 0 of
// 103 frozen command kinds). The day a preview arrives, "cancel restores"
// stops being free and becomes a REAL restore path that someone must build.
// This pin is what makes that day loud instead of silent — the same shape as
// the refusalToast roster row whose premise is checked beside it (C-754).
void TstAdjustmentTable::nothingIsAppliedUNTILApplyIsPressed()
{
    const QString qml = readAll(QStringLiteral(APPNAME_ADJUST_DIALOG_QML));
    QVERIFY2(!qml.isEmpty(), "AdjustmentParamDialog.qml unreadable");

    // Comment-stripped, because the header discusses the submission at
    // length and a raw contains() would match the PROSE about it — the
    // needle failure this desk hit in C-711/C-712 and pinned in C-747.
    const QString code = QString(qml).remove(
        QRegularExpression(QStringLiteral("//[^\n]*")));
    QVERIFY2(code.contains(QLatin1String("AppI18n.tr")),
             "control failed: comment-stripping removed the code itself");

    // EXACTLY ONE document mutation, and it is the adjustment-layer add.
    QCOMPARE(code.count(QLatin1String("AppDoc.addAdjustmentLayer")), 1);

    // AND NO OTHER MUTATING DOOR. A live preview would almost certainly
    // arrive as one of these, applied as the user drags — which is what
    // OPERATOR-PRIORITY.md forbids ("OK commits the whole stack as ONE
    // command"; do not apply-as-you-toggle) and what would silently void
    // the cancel-restores clause.
    for (const char *door : {"AppDoc.applyCommand", "AppDoc.setMaskProp",
                             "AppDoc.setLayerProps", "AppDoc.addLayer",
                             "AppDoc.cutLayer", "AppDoc.deleteLayerContent"}) {
        QVERIFY2(!code.contains(QLatin1String(door)),
                 qPrintable(QStringLiteral("a second mutation door appeared in the "
                                           "adjustment dialog: ")
                            + QLatin1String(door)));
    }

    // CANCEL CLOSES AND DOES NOTHING ELSE. Anchored on the catalogued id so
    // a renamed button reds here rather than drifting past a positional test.
    static const QRegularExpression cancelRe(QStringLiteral(
        "Button\\s*\\{[^}]*ui\\.dialog\\.cancel[^}]*\\}"));
    const auto cm = cancelRe.match(code);
    QVERIFY2(cm.hasMatch(), "the Cancel button block was not found");
    QVERIFY2(cm.captured(0).contains(QLatin1String("root.close()")),
             "Cancel no longer simply closes");
    QVERIFY2(!cm.captured(0).contains(QLatin1String("AppDoc")),
             "Cancel touches the document — it must have nothing to undo");
}

QTEST_MAIN(TstAdjustmentTable)
#include "tst_adjustment_table.moc"
