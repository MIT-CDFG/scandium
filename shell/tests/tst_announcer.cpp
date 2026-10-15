// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.3 live-region policy [NORMATIVE] + keyboard-scripts.yaml
// global rule 7. Pins, against a REAL event loop (no clock mocks):
//   1. an idle announce emits synchronously (settle-to-speech latency 0);
//   2. rate limit: one region flush per 500 ms, timer PRECISE, and the
//      follow-up lands inside the KS 700 ms observation bound;
//   3. latest-wins coalescing per source (the held-] contract);
//   4. FIFO across sources — a second source is never starved by the
//      first, and never jumps the queue;
//   5. uncataloged MessageIds are REFUSED, never key-echoed into speech
//      (the P2-D2 class, mechanized);
//   6. golden resolutions of the three cataloged a11y.layers.* rows
//      against the REAL checked-in en.json (the §49.1/KS-8 wordings) —
//      catalog drift on the spoken strings reds here mechanically;
//   7. the dispatch subject resolves a real QAccessibleInterface
//      (review D1 MAJOR: platform bridges silently drop announcement
//      events whose object resolves none — a bare QObject does; the
//      shell's QQuickWindow must). Needs a QGuiApplication, so this
//      test runs on the offscreen platform, not GUILESS.

#include "announcer.h"
#include "i18n_catalog.h"

#include <QAccessible>
#include <QFile>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QSet>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

namespace {

QString readAll(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly))
        return QString();
    return QString::fromUtf8(f.readAll());
}

Catalog realEnCatalog()
{
    const QString en =
        readAll(QStringLiteral(APPNAME_REPO_I18N_DIR "/en.json"));
    return Catalog::fromTexts({{QStringLiteral("en"), en}});
}

Catalog syntheticCatalog()
{
    // Announcement-mechanics ids, deliberately synthetic: these tests pin
    // TIMING/ORDER semantics, not wording.
    const QString json = QStringLiteral(
        "{ \"t.one\": \"one {v}\", \"t.two\": \"two {v}\","
        "  \"t.three\": \"three\" }");
    return Catalog::fromTexts({{QStringLiteral("t"), json}});
}

qint64 logMs(const QVariantList &log, int i)
{
    return log.at(i).toMap().value(QStringLiteral("ms")).toLongLong();
}

// ND2 (delta review; desk-prioritized): App Nap / machine sleep freezes
// the process while the monotonic clock keeps counting — observed as a
// 930 s "delta" on this very machine. A wall-clock-absolute UPPER bound
// then reds with a known cause and no defect. Policy: timing tests
// detect gross suspension (an anomaly no scheduler produces) and re-run
// ONCE; every NEVER-EARLY lower bound stays hard on every attempt
// (suspension only inflates time — it cannot make a timer fire early).
constexpr qint64 SuspensionMs = 5000;

QString logText(const QVariantList &log, int i)
{
    return log.at(i).toMap().value(QStringLiteral("text")).toString();
}

// Drops // line comments before the call-site scan below. Without this,
// a call site QUOTED IN PROSE would enter the derived population — and
// undo()'s own explanatory block quotes setStatusFromResult repeatedly.
// LIMIT: this is a lexer, not a parser. It does not understand block
// comments, #if-ed out regions, or a "//" occurring inside a string
// literal (which it would mis-strip). Measured on the file it is
// pointed at: 0 block-comment openers, 0 string literals containing
// "//" — so it is exact there today, and the case's own site-count pin
// below is what notices if that stops being true.
QString stripLineComments(const QString &src)
{
    QStringList out;
    const QStringList lines = src.split(QLatin1Char('\n'));
    out.reserve(lines.size());
    for (const QString &line : lines) {
        const int i = line.indexOf(QStringLiteral("//"));
        out << (i < 0 ? line : line.left(i));
    }
    return out.join(QLatin1Char('\n'));
}

} // namespace

class TstAnnouncer : public QObject {
    Q_OBJECT
private slots:
    void idleAnnounceIsImmediate();
    void rateLimitAndKsBound();
    void latestWinsPerSource();
    void fifoAcrossSources();
    void refusesUncatalogedIds();
    void goldenLayersRowsAgainstRealCatalog();
    void subjectResolvesAccessibleInterface();
    void jsonlSinkWritesRows();
    void handWrittenStatusKindsUncatalogedSetIsExactlyUndoRedo();
};

void TstAnnouncer::idleAnnounceIsImmediate()
{
    Catalog cat = syntheticCatalog();
    Announcer a(&cat);
    QSignalSpy spy(&a, &Announcer::announced);
    a.announce(QStringLiteral("s"), QStringLiteral("t.three"));
    QCOMPARE(spy.count(), 1); // synchronous — no queue on an idle region
    QVERIFY2(logMs(a.announcementLog(), 0) < 100,
             "idle announce must not wait for a window");
}

void TstAnnouncer::rateLimitAndKsBound()
{
    Catalog cat = syntheticCatalog();
    for (int attempt = 0; attempt < 2; ++attempt) {
        Announcer a(&cat);
        QSignalSpy spy(&a, &Announcer::announced);
        a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
                   {{QStringLiteral("v"), 1}});
        QCOMPARE(spy.count(), 1);
        // Second announce lands inside the window: must NOT emit now...
        a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
                   {{QStringLiteral("v"), 2}});
        QCOMPARE(spy.count(), 1);
        // ...and must emit within the KS 700 ms bound (rule 7: 500 ms
        // rate limit + margin). PreciseTimer: never early.
        const bool flushed = spy.wait(700);
        qint64 delta = -1;
        if (spy.count() == 2) {
            const QVariantList log = a.announcementLog();
            delta = logMs(log, 1) - logMs(log, 0);
        }
        // Suspension shapes (ND2): flush observed but grossly late, or
        // the wall-clock wait expired around a frozen process. Retry
        // once; a second anomaly fails hard.
        if (attempt == 0
            && (delta > SuspensionMs || (!flushed && spy.count() < 2))) {
            qWarning("suspension suspected (flushed=%d delta=%lld) — "
                     "one retry (ND2)",
                     flushed, static_cast<long long>(delta));
            continue;
        }
        QCOMPARE(spy.count(), 2);
        QVERIFY2(delta >= 490, qPrintable(QStringLiteral(
            "flush must respect the 500 ms window (delta %1)").arg(delta)));
        QVERIFY2(delta <= 700, qPrintable(QStringLiteral(
            "flush must land inside the KS bound (delta %1)").arg(delta)));
        return;
    }
}

void TstAnnouncer::latestWinsPerSource()
{
    Catalog cat = syntheticCatalog();
    for (int attempt = 0; attempt < 2; ++attempt) {
        Announcer a(&cat);
        QSignalSpy spy(&a, &Announcer::announced);
        a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
                   {{QStringLiteral("v"), QStringLiteral("first")}});
        // Three rapid updates behind the window — exactly ONE coalesced
        // flush, carrying the LAST payload (a held ] announces the
        // final brush size, not every step).
        for (const char *v : {"a", "b", "final"})
            a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
                       {{QStringLiteral("v"), QString::fromLatin1(v)}});
        const bool flushed = spy.wait(700);
        // Suspension shapes (ND2): a freeze inside the announce burst
        // expires the window mid-burst (an intermediate value emits
        // immediately → extra flush), or the wait dies around a frozen
        // process. Retry once; hard on the second attempt.
        if (attempt == 0
            && (!flushed || spy.count() != 2
                || logText(a.announcementLog(), 1)
                       != QStringLiteral("one final"))) {
            qWarning("suspension suspected (flushed=%d count=%lld) — "
                     "one retry (ND2)",
                     flushed, static_cast<long long>(spy.count()));
            continue;
        }
        QCOMPARE(spy.count(), 2);
        QCOMPARE(logText(a.announcementLog(), 1),
                 QStringLiteral("one final"));
        // No third flush pending (suspension-safe: a freeze cannot
        // conjure a flush that was never queued).
        QVERIFY(!spy.wait(600));
        QCOMPARE(spy.count(), 2);
        return;
    }
}

void TstAnnouncer::fifoAcrossSources()
{
    Catalog cat = syntheticCatalog();
    for (int attempt = 0; attempt < 2; ++attempt) {
        Announcer a(&cat);
        QSignalSpy spy(&a, &Announcer::announced);
        a.announce(QStringLiteral("s1"), QStringLiteral("t.one"),
                   {{QStringLiteral("v"), 0}});
        // s2 goes pending first, then s1 updates: flush order must be
        // s2 (first pending) then s1 — the chatty source cannot starve
        // the other, and the newcomer cannot jump the queue.
        a.announce(QStringLiteral("s2"), QStringLiteral("t.two"),
                   {{QStringLiteral("v"), QStringLiteral("s2-final")}});
        a.announce(QStringLiteral("s1"), QStringLiteral("t.one"),
                   {{QStringLiteral("v"), QStringLiteral("s1-final")}});
        if (spy.count() < 2)
            spy.wait(700);
        if (spy.count() < 3)
            spy.wait(700);
        // Suspension shape (ND2): waits dying around a frozen process.
        // Retry once; hard on the second attempt. (Payload ORDER stays
        // hard on every attempt below — a freeze inflates spacing but
        // cannot reorder the FIFO or rewrite latest-wins payloads.)
        if (attempt == 0 && spy.count() != 3) {
            qWarning("suspension suspected (count=%lld) — one retry (ND2)",
                     static_cast<long long>(spy.count()));
            continue;
        }
        QCOMPARE(spy.count(), 3);
        QCOMPARE(logText(a.announcementLog(), 1),
                 QStringLiteral("two s2-final"));
        QCOMPARE(logText(a.announcementLog(), 2),
                 QStringLiteral("one s1-final"));
        // Spacing between the two queued flushes also honors the window
        // (lower bound only — suspension-safe, stays hard).
        const QVariantList log = a.announcementLog();
        QVERIFY(logMs(log, 2) - logMs(log, 1) >= 490);
        return;
    }
}

void TstAnnouncer::refusesUncatalogedIds()
{
    Catalog cat = syntheticCatalog();
    Announcer a(&cat);
    QSignalSpy spy(&a, &Announcer::announced);
    a.announce(QStringLiteral("s"), QStringLiteral("a11y.not.a.row"),
               {{QStringLiteral("name"), QStringLiteral("Sky")}});
    QCOMPARE(spy.count(), 0);
    QCOMPARE(a.refusedCount(), 1);
    QVERIFY(a.announcementLog().isEmpty());
    // An empty catalog refuses everything — silence, never key-echo.
    Catalog empty = Catalog::fromTexts({});
    Announcer b(&empty);
    QSignalSpy spyB(&b, &Announcer::announced);
    b.announce(QStringLiteral("s"), QStringLiteral("t.three"));
    QCOMPARE(spyB.count(), 0);
    QCOMPARE(b.refusedCount(), 1);
}

void TstAnnouncer::goldenLayersRowsAgainstRealCatalog()
{
    Catalog cat = realEnCatalog();
    QVERIFY2(cat.hasEntries(), "real i18n/en.json unreadable");
    Announcer a(&cat);
    QSignalSpy spy(&a, &Announcer::announced);
    a.announce(QStringLiteral("layers"), QStringLiteral("a11y.layers.hidden"),
               {{QStringLiteral("name"), QStringLiteral("Sky")}});
    QCOMPARE(spy.count(), 1);
    QCOMPARE(logText(a.announcementLog(), 0),
             QStringLiteral("Layer 'Sky' hidden"));
    // Fresh announcers: golden pins are about wording, not the window.
    Announcer b(&cat);
    b.announce(QStringLiteral("layers"), QStringLiteral("a11y.layers.renamed"),
               {{QStringLiteral("name"), QStringLiteral("Sky")}});
    QCOMPARE(logText(b.announcementLog(), 0),
             QStringLiteral("Layer renamed to 'Sky'")); // the KS-8 golden
    Announcer c(&cat);
    c.announce(QStringLiteral("layers"), QStringLiteral("a11y.layers.moved"),
               {{QStringLiteral("index"), 2},
                {QStringLiteral("name"), QStringLiteral("Sky")}});
    QCOMPARE(logText(c.announcementLog(), 0),
             QStringLiteral("Layer 'Sky' moved to position 2"));
    // Wave B: the KS goldens for the rows the 74fd5e3 batch cataloged —
    // wording drift on any spoken string reds here mechanically.
    struct GoldenRow {
        const char *id;
        QVariantMap args;
        const char *expect;
    };
    const GoldenRow rows[] = {
        {"a11y.canvas.selection_bounds",
         {{QStringLiteral("width"), 640}, {QStringLiteral("height"), 400}},
         "Selection: 640 by 400 pixels"}, // KS-1 s4 golden
        {"a11y.canvas.selection_cleared", {}, "Selection cleared"},
        {"a11y.canvas.crop_bounds",
         {{QStringLiteral("width"), 320}, {QStringLiteral("height"), 200}},
         "Crop: 320 by 200 pixels"},
        {"a11y.history.undone",
         {{QStringLiteral("label"), QStringLiteral("Fill")}},
         "Undone: Fill"}, // KS-1 s8 golden
        {"a11y.history.redone",
         {{QStringLiteral("label"), QStringLiteral("Fill")}},
         "Redone: Fill"},
        {"a11y.file.saved", {}, "Document saved"}, // §31.3 golden
        {"a11y.job.completed",
         {{QStringLiteral("label"), QStringLiteral("out.png")}},
         "out.png finished"},
        {"a11y.tool.selected",
         {{QStringLiteral("name"), QStringLiteral("Crop")}},
         "Crop"}, // §49.1: the bare tool name
        {"a11y.tool.brush_size", {{QStringLiteral("size"), 64}},
         "Brush size 64 pixels"}, // §31.3 golden
        {"a11y.zoom.level", {{QStringLiteral("value"), 100}},
         "Zoom 100 percent"},
    };
    for (const GoldenRow &row : rows) {
        Announcer g(&cat);
        g.announce(QStringLiteral("g"), QLatin1String(row.id), row.args);
        QVERIFY2(g.announcementLog().size() == 1,
                 qPrintable(QStringLiteral("row not announced (uncataloged?): %1")
                                .arg(QLatin1String(row.id))));
        QCOMPARE(logText(g.announcementLog(), 0),
                 QString::fromLatin1(row.expect));
    }
}

// ND1 intercept plumbing: QAccessible::UpdateHandler is a bare function
// pointer, so the capture rides file statics.
namespace {
int g_announceEvents = 0;
QObject *g_lastEventObject = nullptr;
bool g_lastPolitenessPolite = false;

void captureUpdate(QAccessibleEvent *event)
{
    if (event->type() != QAccessible::Announcement)
        return;
    ++g_announceEvents;
    g_lastEventObject = event->object();
    auto *ann = static_cast<QAccessibleAnnouncementEvent *>(event);
    g_lastPolitenessPolite =
        ann->politeness() == QAccessible::AnnouncementPoliteness::Polite;
}
} // namespace

void TstAnnouncer::subjectResolvesAccessibleInterface()
{
    // The D1 invariant: the object announcements dispatch on MUST
    // resolve an accessible interface, or cocoa/AT-SPI drop the event
    // before it reaches any screen reader.
    QQuickWindow window;
    QVERIFY2(QAccessible::queryAccessibleInterface(&window) != nullptr,
             "QQuickWindow must resolve a QAccessibleInterface — the "
             "announcement dispatch subject depends on it (D1)");
    Catalog cat = syntheticCatalog();
    Announcer a(&cat);
    // The bare-QObject fallback resolves none — which is exactly why
    // main.cpp hands the window over.
    QVERIFY(QAccessible::queryAccessibleInterface(&a) == nullptr);
    a.setSubject(&window);
    QSignalSpy spy(&a, &Announcer::announced);
    // ND1 (delta review): the signal/log surface alone cannot red a
    // dispatch regression back to `this` — intercept the actual event
    // and pin its object AND politeness (the §31.3 never-assertive rule,
    // mechanized at the event level).
    g_announceEvents = 0;
    g_lastEventObject = nullptr;
    g_lastPolitenessPolite = false;
    const auto prior = QAccessible::installUpdateHandler(captureUpdate);
    a.announce(QStringLiteral("s"), QStringLiteral("t.three"));
    QAccessible::installUpdateHandler(prior);
    QCOMPARE(spy.count(), 1); // subject-dispatch path stays observable
    QCOMPARE(g_announceEvents, 1);
    QVERIFY2(g_lastEventObject == &window,
             "announcement event must ride the SUBJECT (a bare-QObject "
             "dispatch is silently dropped by every platform bridge)");
    QVERIFY2(g_lastPolitenessPolite,
             "the live region is POLITE, never assertive (§31.3)");
}

void TstAnnouncer::jsonlSinkWritesRows()
{
    // A11Y-5 hook A: APPNAME_A11Y_LOG appends one JSONL row per emit —
    // the external harness door. Env is read at CONSTRUCTION.
    QTemporaryDir dir;
    QVERIFY(dir.isValid());
    const QString sink = dir.filePath(QStringLiteral("a11y.jsonl"));
    qputenv("APPNAME_A11Y_LOG", sink.toLocal8Bit());
    Catalog cat = syntheticCatalog();
    Announcer a(&cat);
    qunsetenv("APPNAME_A11Y_LOG"); // scope the env to this announcer
    a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
               {{QStringLiteral("v"), QStringLiteral("row1")}});
    QFile f(sink);
    QVERIFY2(f.open(QIODevice::ReadOnly), "sink file not created");
    const QList<QByteArray> lines = f.readAll().trimmed().split('\n');
    QCOMPARE(lines.size(), 1);
    QJsonParseError err;
    const QJsonDocument doc = QJsonDocument::fromJson(lines.at(0), &err);
    QVERIFY2(err.error == QJsonParseError::NoError, "row is not valid JSON");
    const QJsonObject row = doc.object();
    QCOMPARE(row.value(QLatin1String("source")).toString(),
             QStringLiteral("s"));
    QCOMPARE(row.value(QLatin1String("id")).toString(),
             QStringLiteral("t.one"));
    QCOMPARE(row.value(QLatin1String("text")).toString(),
             QStringLiteral("one row1"));
    QVERIFY(row.contains(QLatin1String("ms")));
    // A refused id must write NOTHING (the door mirrors speech;
    // refusal is synchronous and pre-window, so this is immediate).
    a.announce(QStringLiteral("s"), QStringLiteral("not.a.row"));
    QFile fr(sink);
    QVERIFY(fr.open(QIODevice::ReadOnly));
    QCOMPARE(fr.readAll().trimmed().split('\n').size(), 1);
    fr.close();
    // Review D4: the sink must APPEND (a truncating sink would have
    // passed the single-row shape) and must write only EMITS — a
    // coalesced burst lands as ONE row carrying the LATEST payload,
    // with the suppressed intermediate never written.
    QSignalSpy spy(&a, &Announcer::announced);
    a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
               {{QStringLiteral("v"), QStringLiteral("suppressed")}});
    a.announce(QStringLiteral("s"), QStringLiteral("t.one"),
               {{QStringLiteral("v"), QStringLiteral("kept")}});
    QVERIFY(spy.wait(700));
    QFile f2(sink);
    QVERIFY(f2.open(QIODevice::ReadOnly));
    const QByteArray all = f2.readAll();
    const QList<QByteArray> rows2 = all.trimmed().split('\n');
    QCOMPARE(rows2.size(), 2); // append, not truncate
    QVERIFY2(!all.contains("suppressed"),
             "a coalescing-suppressed intermediate was written — the "
             "sink must mirror speech (emits only)");
    QVERIFY(rows2.at(1).contains("kept"));
    // Env is read at CONSTRUCTION: an announcer built after qunsetenv
    // has no sink — its emit must not touch the file.
    Announcer b(&cat);
    b.announce(QStringLiteral("s"), QStringLiteral("t.three"));
    QFile f3(sink);
    QVERIFY(f3.open(QIODevice::ReadOnly));
    QCOMPARE(f3.readAll().trimmed().split('\n').size(), 2);
}

// C-179 MECHANICAL PIN — the CATALOG side of the undo/redo label frame.
// This case does NOT repair the defect; it pins the invariant the defect
// sits on, so the most likely WRONG repair reds here instead of shipping.
//
// document_controller.cpp undo()/redo() (:329/:355) hand-write the kinds
// "undo"/"redo" into setStatusFromResult, which renders
// tr("history." + kind) into ui.status.applied — the visible status bar
// AND Main.qml's Accessible.description, where an uncataloged key is
// SPOKEN as a raw id (this file's announce path is gated by
// Catalog::has(); the description binding is not gated at all). Neither
// literal is a registered command kind, so on the APPNAME_ERR_IO arm the
// user reads, and a reader speaks, "history.undo". That arm is dead only
// because history spilling is opt-in and the cabi has never opted in —
// Engine::undo -> revive_history_step -> IoReadFailed is a real IO arm,
// and appname-cli calls set_history_limits on the same engine and DOES
// spill. One such call added to the cabi makes the key-echo live.
//
// The tempting repair when someone first meets that raw id is to mint
// history.undo / history.redo rows: the id disappears and the defect —
// a label built from a kind the engine never validated — ships. THIS
// CASE FAILS ON THAT REPAIR. Its positive control (history.doc.new: the
// other hand-written setStatusFromResult literal, :203/:209, which IS a
// registered kind and IS cataloged in all four locales) is asserted in
// the same case and aborts it first, so an unreadable or empty catalog
// — where has() answers false for every key — cannot make the absence
// assertions below pass vacuously.
//
// ---- C-204: DERIVED, NOT ENUMERATED ----
// The C-179 form of this case named "undo" and "redo" in a fixed array.
// That is the defect O-INT 6 found in a pre-check that enumerates rows
// nobody applies, and it applies here unchanged: A GUARD ENUMERATED
// AGAINST A FIXED NAMED LIST CANNOT FAIL ON A CASE ADDED AFTER IT. A
// third hand-written literal reaching setStatusFromResult would have
// passed this pin in silence — measured, not supposed: with a fifth
// call site added, the enumerated form returned 11 passed / 0 failed.
//
// So the population is now DERIVED. The case parses the
// setStatusFromResult call sites out of the checked-in controller
// source, partitions the kinds it finds by whether this catalog carries
// their history.<kind> row, and asserts the UNCATALOGED side equals
// {undo, redo} as a SET. A new hand-written kind lands in that set and
// reds, named.
//
// ★ STATED PARSE LIMITS — read these before trusting a pass. This case
// reads TEXT, not a compiled AST, and it sees exactly one shape: a
// string literal written AT the call site, optionally wrapped in
// QStringLiteral/QLatin1String/QByteArrayLiteral. It does NOT see
//   * a kind held in a variable or member  — 2 such sites exist today
//     (applyCommand's `kind`, stroke commit's `m_strokeKind`), and they
//     are the door the known-open history.history.goto label comes
//     through;
//   * a kind assembled by concatenation or built by a macro;
//   * a kind reaching the function through any callee other than these;
//   * anything in a block comment or an #if-ed out region (the
//     line-comment stripper above is a lexer, not a preprocessor).
// The two site counts pinned at the END of this case are what keep that
// boundary from widening unnoticed; they are the honest statement of
// reach, not a claim of totality.
void TstAnnouncer::handWrittenStatusKindsUncatalogedSetIsExactlyUndoRedo()
{
    const QString src = stripLineComments(
        readAll(QStringLiteral(APPNAME_DOCUMENT_CONTROLLER_CPP)));
    QVERIFY2(!src.isEmpty(),
             "document_controller.cpp is unreadable at "
             APPNAME_DOCUMENT_CONTROLLER_CPP ". The population below is "
             "DERIVED from that file; with no text there is nothing to "
             "derive and every assertion in this case would be vacuous.");

    // A CALL site. The definition is the one occurrence preceded by
    // "::" (DocumentController::setStatusFromResult), so the lookbehind
    // drops it without naming a line number that would rot.
    const QRegularExpression siteRe(
        QStringLiteral("(?<!::)\\bsetStatusFromResult\\s*\\("));
    // A call site whose FIRST argument is a written-out string literal,
    // with or without a Qt wrapper. `setStatusFromResult(kind, ...)`
    // does not match: the only thing permitted between "(" and the
    // quote is whitespace and a wrapper token.
    const QRegularExpression literalRe(QStringLiteral(
        "(?<!::)\\bsetStatusFromResult\\s*\\(\\s*"
        "(?:(?:QStringLiteral|QLatin1String|QByteArrayLiteral)\\s*\\(\\s*)?"
        "\"([^\"]*)\""));

    int siteCount = 0;
    for (auto it = siteRe.globalMatch(src); it.hasNext(); it.next())
        ++siteCount;

    QSet<QString> derivedKinds;
    QStringList derivedSites; // "kind@line", for failure messages
    int literalSiteCount = 0;
    for (auto it = literalRe.globalMatch(src); it.hasNext();) {
        const QRegularExpressionMatch m = it.next();
        const QString kind = m.captured(1);
        derivedKinds.insert(kind);
        derivedSites << QStringLiteral("%1@%2").arg(
            kind, QString::number(QStringView(src).left(m.capturedStart(1))
                                      .count(QLatin1Char('\n'))
                                  + 1));
        ++literalSiteCount;
    }

    // NON-VACUITY OF THE DERIVATION ITSELF. A parse that silently
    // matched nothing (renamed function, reshaped call sites, a regex
    // broken by a Qt upgrade) would otherwise hand every assertion
    // below an empty set to agree with.
    QVERIFY2(literalSiteCount > 0,
             "DERIVATION EMPTY: parsed 0 hand-written setStatusFromResult "
             "literals out of document_controller.cpp. Either the call "
             "sites changed shape or this case's regex stopped matching. "
             "Do not read the verdict below as a pass — fix the parse.");
    QVERIFY2(derivedKinds.contains(QStringLiteral("doc.new")),
             qPrintable(
                 QStringLiteral(
                     "DERIVATION CONTROL FAILED: \"doc.new\" is no longer among "
                     "the parsed literals (%1). That kind is this case's known-"
                     "cataloged anchor; if the parse cannot see it, the parse is "
                     "not seeing the call sites this case is about.")
                     .arg(derivedSites.join(QStringLiteral(", ")))));

    const char *const locales[] = {"en", "es", "ja", "pseudo"};
    int checked = 0;
    for (const char *loc : locales) {
        const QString lang = QString::fromLatin1(loc);
        const QString path = QStringLiteral(APPNAME_REPO_I18N_DIR "/")
                             + lang + QStringLiteral(".json");
        const Catalog cat = Catalog::fromTexts({{lang, readAll(path)}});
        QVERIFY2(cat.hasEntries(),
                 qPrintable(QStringLiteral("checked-in catalog unreadable: %1")
                                .arg(path)));

        // POSITIVE CONTROL — inside the assertion, not beside it.
        QVERIFY2(
            cat.has(QStringLiteral("history.doc.new")),
            qPrintable(
                QStringLiteral(
                    "POSITIVE CONTROL FAILED for %1: history.doc.new is not in this "
                    "catalog, so the absence assertions in this case prove NOTHING "
                    "(an empty or unreadable catalog answers has() false for every "
                    "key, which would look like a pass). Restore that row — or, if it "
                    "was renamed deliberately, re-point this control at another "
                    "history.<registered kind> row — before reading any verdict below.")
                    .arg(path)));

        // THE PARTITION. Split the DERIVED population by whether this
        // catalog carries its history.<kind> row, and compare the
        // uncataloged side against the known set as a SET.
        QStringList uncataloged;
        for (const QString &kind : derivedKinds)
            if (!cat.has(QStringLiteral("history.") + kind))
                uncataloged << kind;
        uncataloged.sort();
        const QStringList known{QStringLiteral("redo"), QStringLiteral("undo")};

        QVERIFY2(
            uncataloged == known,
            qPrintable(
                QStringLiteral(
                    "HAND-WRITTEN STATUS KINDS CHANGED against %1.\n"
                    "  derived call sites : %2\n"
                    "  uncataloged now    : {%3}\n"
                    "  known/accepted     : {%4}\n"
                    "\n"
                    "READ WHICH DIRECTION MOVED.\n"
                    "\n"
                    "(a) A KIND WAS ADDED. Some call site now hand-writes a kind "
                    "with no history.<kind> row, so setStatusFromResult's "
                    "APPNAME_ERR_IO arm will render tr(\"history.<kind>\") — which "
                    "KEY-ECHOES the raw id into the status line and, through "
                    "Main.qml's ungated Accessible.description binding, into what "
                    "a screen reader speaks off the canvas. This case exists for "
                    "exactly this event. DO NOT silence it by minting a "
                    "history.<kind> row: if the engine never registered that kind, "
                    "the row names no command and only hides the defect. Fix the "
                    "CALL SITE — take the kind from the receipt as undo()'s ok arm "
                    "does (r.json[\"kind\"]), or route the error arm to "
                    "setStatusFromResult's rejected frame, which needs no history.* "
                    "label at all.\n"
                    "\n"
                    "(b) A ROW WAS MINTED for undo/redo. That is the WRONG REPAIR "
                    "the C-179 form of this case was written to catch, and it is "
                    "still wrong: neither literal is a registered command kind, so "
                    "the row names nothing the engine can accept. Remove it and fix "
                    "the call site as above.\n"
                    "\n"
                    "(c) A CALL SITE WAS REMOVED. If undo()/redo() genuinely stopped "
                    "hand-writing their kinds, that is the real fix — narrow `known` "
                    "in this case to match, and say so in the commit.")
                    .arg(path, derivedSites.join(QStringLiteral(", ")),
                         uncataloged.join(QStringLiteral(", ")),
                         known.join(QStringLiteral(", ")))));
        ++checked;
    }
    // Non-vacuity of the LOOP: four canonical catalogs, four visits. A
    // catalog dropped from the set must red here, not silently shrink
    // the guard.
    QCOMPARE(checked, 4);

    // ---- THE GUARD'S STATED REACH, PINNED RATHER THAN DESCRIBED ----
    // The partition above only sees kinds written as literals AT the
    // call site. Two call sites pass a variable instead and are
    // invisible to it: applyCommand's `kind` parameter and stroke
    // commit's `m_strokeKind`. That blind spot is not hypothetical —
    // historyGoto() reaches setStatusFromResult through applyCommand
    // with the REGISTERED kind "history.goto", whose label key
    // "history.history.goto" is uncataloged in all four locales TODAY.
    // That is a known, tracked, separately-owned defect, and this case
    // deliberately does not fire on it (a guard that reds on a
    // known-open item is a broken build, not a guard).
    //
    // These two counts are what stop the exclusion from quietly
    // widening: convert a literal site to a variable and the population
    // shrinks with no partition failure, which is the enumeration
    // problem returning by another door. Neither number is a defect
    // claim — if a legitimate new call site moves one, re-measure the
    // reach, update the count, and state the new blind spot.
    QCOMPARE(literalSiteCount, 4);
    QCOMPARE(siteCount - literalSiteCount, 2);
}

// Offscreen QGuiApplication (not GUILESS): pin 7 needs a QQuickWindow,
// and the platform must be forced BEFORE the app constructs.
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QGuiApplication app(argc, argv);
    TstAnnouncer tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_announcer.moc"
