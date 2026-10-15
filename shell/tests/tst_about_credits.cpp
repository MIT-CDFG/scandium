// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09's ACCEPTANCE CLAUSE IS "About shows all; MATCHES THIRD-PARTY-NOTICES",
// and until now nothing checked the second half. The dialog could lose a
// credit, or the notices file could gain an obligation, and every suite
// would stay green — the failure mode is silent in BOTH directions, which
// is exactly the shape C-706 cost this desk a ten-tick debt to learn.
//
// WHAT THIS PINS, AND WHY IT IS NOT A RENDER TEST. The rendering is
// already covered by the catalog: every credit is an AppI18n.tr() of an id
// whose absence key-echoes visibly. What NOTHING covered is the SET —
// which obligations the dialog carries and which it deliberately omits.
// That is a relationship between two files, so the test reads both.
//
// THE OMISSIONS ARE PINNED AS HARD AS THE INCLUSIONS, because an
// attribution surface is judged by what it leaves out:
//
//   rgb2spec  is recorded, and belongs to the LUT BAKE TOOL's help/About —
//             a DIFFERENT BINARY. The ROADMAP's B-09 line names it; the
//             row's own acceptance clause makes the notices file the
//             authority, and the notices file places it elsewhere.
//   cgats21   has a minted catalog id and its record sits under "Pending —
//             vendor drop not landed yet". Crediting it would attest to a
//             component that is not in the product.
//   spectral.js is named by the ROADMAP row as one of B-09's five
//             obligations and its record carries NO `additional surface`
//             line AT ALL — measured C-749, with rgb2spec's own such line
//             as the positive control that the needle can see one. MIT's
//             notice duty is discharged by SHIPPING THE TEXT (it is in the
//             staged notices aggregate the licences viewer reads), not by
//             an About credit. Pinned here because the ROADMAP is what the
//             next desk reads, and adding the credit would ATTEST TO AN
//             OBLIGATION THAT DOES NOT EXIST — the opposite error to
//             C-731's, and the same surface.
//   wasmtime  is named by the ROADMAP row and is in NOTHING the shell
//             ships: appname-cabi is the sole consumer of appname-script
//             and takes it default-features=false, disabling wasm-plugins,
//             the only thing pulling dep:wasmtime. Confirmed at the binary
//             (0 wasmtime/cranelift symbols in libappname_cabi.dylib) and
//             by O-BUILD's no-jit entitlements, which select on exactly
//             that symbol absence.
//
// PROSE COUPLING, STATED RATHER THAN HIDDEN: the notices file's obligation
// lines are English sentences, so the shell-About discriminator here is a
// text match ("About" present, "LUT bake tool" absent). That is fragile by
// construction. It is kept because a check that fails LOUDLY when the
// notices wording changes is worth more than no check — the failure is a
// prompt to re-read the obligations, which is the right response either
// way. It must never be "fixed" by loosening it into always passing.

// ADDED C-747 — WORD 3's FALLBACK, PINNED. The Help > Open Licenses Folder
// item is what ships if B-09's display half misses 09-16 18:00 ET, and it
// had ZERO coverage: its id resolved (measured) but nothing held it against
// a rename, a broken visible gate, or a handler pointed elsewhere. No global
// tr-id sweep exists in this tree — every slice pins its own — so it is
// pinned here, beside the About surface it belongs to.
//
// THE NAIVE COMMENT STRIPPER ABOVE MUST NOT TOUCH Main.qml. It removes
// //[^\n]* and the fallback's handler reads Qt.openUrlExternally("file://"
// + dir) — stripping would eat the line from "file: onward and the needle
// would then measure a mutilated source. So the MenuItem block is extracted
// by BRACE MATCHING and asserted RAW, with a guard that fails if a comment
// ever enters the block (the a11y lint's single-quote-guard pattern: force a
// conscious extension rather than a silent false match).

#include "i18n_catalog.h"

#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <QString>
#include <QStringList>
#include <QtTest>

using namespace appname::shell;

namespace {

QString readAll(const QString &path, bool *ok)
{
    QFile f(path);
    *ok = f.open(QIODevice::ReadOnly);
    return *ok ? QString::fromUtf8(f.readAll()) : QString();
}

// The ids the dialog actually renders, read from its Repeater model —
// the SOURCE, never a transcription of it.
QStringList dialogCreditIds(const QString &qml)
{
    static const QRegularExpression re(
        QStringLiteral(R"(model:\s*\[([^\]]*)\])"));
    const auto m = re.match(qml);
    if (!m.hasMatch())
        return {};
    QStringList out;
    // Custom raw-string delimiter: the pattern itself contains `)"`, which
    // closes a plain R"(...)" early. Cost one build; recorded so the next
    // regex-in-a-raw-string does not.
    static const QRegularExpression item(QStringLiteral(R"RX("([a-z0-9_]+)")RX"));
    auto it = item.globalMatch(m.captured(1));
    while (it.hasNext())
        out << it.next().captured(1);
    return out;
}

// Obligation lines that name THIS shell's About surface. The LUT bake
// tool's line also says "About" and is not ours.
QStringList shellAboutObligations(const QString &notices)
{
    QStringList out;
    const auto lines = notices.split(QLatin1Char('\n'));
    for (const QString &l : lines) {
        // NO `additional surface` PRECONDITION (C-745). It was here, and it
        // could not see an obligation phrased any other way: FreeType's duty
        // reads "required acknowledgment, carried verbatim per the record's
        // obligations" and is invisible to that needle. Measured: 10 lines
        // say "additional surface", 1 says "required acknowledgment", and
        // the two populations do not overlap.
        //
        // FreeType is correctly NOT a credit here — its line never names the
        // About surface, and build_dmg.sh states THIRD-PARTY-NOTICES.md
        // carries it, which ships in the bundle. But that was luck, not
        // coverage: had it said "in the About dialog", the old needle would
        // have missed a REQUIRED ACKNOWLEDGMENT and this test would have
        // certified the omission.
        //
        // The surviving discriminator is the one that was always doing the
        // work: the line must NAME the About surface, and must not be the
        // LUT bake tool's. Verified this change keeps the answer at 3.
        if (!l.trimmed().startsWith(QLatin1Char('-')))
            continue;
        if (!l.contains(QLatin1String("About")))
            continue;
        if (l.contains(QLatin1String("LUT bake tool")))
            continue;
        out << l;
    }
    return out;
}

// A31 (C-754): brace-walking QML without blanking string literals MIS-NESTS.
// Main.qml carries ELEVEN literals containing a brace — "{tool} is not wired
// to the canvas yet", ", {}, {}, " — and a lone `}` in one of them closes the
// walk early, silently returning a TRUNCATED block that every needle below
// would then measure. Blank a COPY: the two share offsets, so the extent is
// found in the blanked text and the text is returned from the original,
// which the raw needles require.
QString blankStrings(const QString &in)
{
    QString out = in;
    bool inString = false;
    for (int i = 0; i < out.size(); ++i) {
        const QChar c = out.at(i);
        if (inString) {
            if (c == QLatin1Char('\\') && i + 1 < out.size()) {
                out[i + 1] = QLatin1Char(' ');
                ++i;
                continue;
            }
            if (c == QLatin1Char('"')) { inString = false; continue; }
            out[i] = QLatin1Char(' ');
        } else if (c == QLatin1Char('"')) {
            inString = true;
        }
    }
    return out;
}

// The MenuItem block owning `objectName`, from its opening `MenuItem {` to
// the matching close brace. Bounding it is the whole point: the comment
// ABOVE this particular item names AppLicensesDir, so an unbounded
// contains() would match the prose explaining the gate rather than the gate.
QString menuItemBlock(const QString &src, const QString &objectName)
{
    const int idAt = src.indexOf(QStringLiteral("objectName: \"") + objectName
                                 + QStringLiteral("\""));
    if (idAt < 0)
        return QString();
    const int open = src.lastIndexOf(QStringLiteral("MenuItem {"), idAt);
    if (open < 0)
        return QString();
    const QString blanked = blankStrings(src);
    int brace = blanked.indexOf(QLatin1Char('{'), open);
    int depth = 0;
    for (int i = brace; i < blanked.size(); ++i) {
        if (blanked.at(i) == QLatin1Char('{')) ++depth;
        else if (blanked.at(i) == QLatin1Char('}')) {
            if (--depth == 0)
                return src.mid(open, i - open + 1);   // ORIGINAL text
        }
    }
    return QString();
}

} // namespace

class TstAboutCredits : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void dialogCarriesExactlyTheRecordedShellObligations();
    void deliberateOmissionsStayOmitted();
    void theHelpFallbackIsDeclaredGuardedAndCatalogDriven();
    void theLicenceSurfaceIdsRESOLVEAndAFabricatedOneDoesNot();
    void everyCreditIdTheDialogEMITSHasAStringAtAll();
    void theAboutFrameCANNOTCarryTheBuildCommit();

private:
    QString m_qml;
    QString m_notices;
    QString m_main;
    Catalog m_catalog = Catalog::fromTexts({});
};

void TstAboutCredits::initTestCase()
{
    bool ok = false;
    m_qml = readAll(QStringLiteral(APPNAME_SHELL_DIR "/qml/dialogs/AboutDialog.qml"), &ok);
    QVERIFY2(ok, "AboutDialog.qml unreadable");
    m_notices = readAll(QStringLiteral(APPNAME_REPO_ROOT "/THIRD-PARTY-NOTICES.md"), &ok);
    QVERIFY2(ok, "THIRD-PARTY-NOTICES.md unreadable");

    // Controls: both reads must have produced real content, so a later
    // zero is a MEASUREMENT and not an empty file (rule 2 — a zero over an
    // empty population is void, not green).
    QVERIFY2(m_qml.size() > 500, "AboutDialog.qml suspiciously small");
    QVERIFY2(m_notices.size() > 100000, "notices file suspiciously small");

    m_main = readAll(QStringLiteral(APPNAME_SHELL_DIR "/qml/Main.qml"), &ok);
    QVERIFY2(ok, "Main.qml unreadable");
    QVERIFY2(m_main.size() > 50000, "Main.qml suspiciously small");

    // canonical + seed, merged, canonical winning on collision — exactly
    // what merge_catalogs.py stages. Both ids checked below live ONLY in
    // one of the two halves, so a single-source catalog would key-echo one
    // of them and the check would pass against a catalog no user sees.
    QFile can(QStringLiteral(APPNAME_REPO_I18N_DIR "/en.json"));
    QVERIFY2(can.open(QIODevice::ReadOnly), "canonical en.json unreadable");
    QJsonObject merged = QJsonDocument::fromJson(can.readAll()).object();
    QFile seed(QStringLiteral(APPNAME_SHELL_DIR "/i18n/ui.en.json"));
    QVERIFY2(seed.open(QIODevice::ReadOnly), "shell seed unreadable");
    const QJsonObject s = QJsonDocument::fromJson(seed.readAll()).object();
    for (auto it = s.begin(); it != s.end(); ++it)
        if (!merged.contains(it.key()))
            merged.insert(it.key(), it.value());
    m_catalog = Catalog::fromTexts(
        {{QStringLiteral("en"), QString::fromUtf8(QJsonDocument(merged).toJson())}});
}

void TstAboutCredits::dialogCarriesExactlyTheRecordedShellObligations()
{
    const QStringList ids = dialogCreditIds(m_qml);
    QVERIFY2(!ids.isEmpty(), "no Repeater credit model found in AboutDialog.qml");

    const QStringList obligations = shellAboutObligations(m_notices);

    // The COUNTS must agree. If the notices file gains a shell-About
    // obligation, this reds until the dialog carries it.
    QCOMPARE(ids.size(), obligations.size());

    // And each id must be NAMED by one of them. Counting alone would pass
    // a dialog that carried three credits for the wrong three components.
    const QMap<QString, QString> component = {
        {QStringLiteral("libmypaint"), QStringLiteral("libmypaint")},
        {QStringLiteral("lucide_icons"), QStringLiteral("ucide")},
        {QStringLiteral("lensfun_database"), QStringLiteral("lensfun")},
    };
    for (const QString &id : ids) {
        QVERIFY2(component.contains(id),
                 qPrintable(QStringLiteral("credit id not in the recorded set: ") + id));
        const QString needle = component.value(id);
        bool found = false;
        for (const QString &o : obligations)
            if (o.contains(needle)) { found = true; break; }
        QVERIFY2(found, qPrintable(
            QStringLiteral("no shell-About obligation names ") + needle));
    }
}

void TstAboutCredits::deliberateOmissionsStayOmitted()
{
    const QString code = QString(m_qml).remove(
        QRegularExpression(QStringLiteral("//[^\n]*")));

    // Comment-stripped, because all three ARE discussed in the header and
    // a naive contains() would match the prose explaining their absence —
    // the needle failure this desk hit twice in two ticks (C-711, C-712).
    for (const char *forbidden : {"cgats21", "rgb2spec", "wasmtime", "spectral"}) {
        QVERIFY2(!code.contains(QLatin1String(forbidden)),
                 qPrintable(QStringLiteral("deliberately-omitted component appears in CODE: ")
                            + QLatin1String(forbidden)));
    }

    // Positive control: the needle CAN see the code it is searching, so
    // the three zeros above are measurements rather than a broken probe.
    QVERIFY2(code.contains(QLatin1String("libmypaint")),
             "control failed: comment-stripped code lost a credit it should carry");
}

void TstAboutCredits::theHelpFallbackIsDeclaredGuardedAndCatalogDriven()
{
    // A31 CONTROL, on a fixture, because the real Help block happens to
    // carry no brace-in-literal today and would prove nothing: a LONE `}`
    // inside a literal closes an unblanked walk early. Balanced ones like
    // "{tool}" would not have shown this at all — which is why the fixture
    // is unbalanced on purpose.
    {
        const QString fx = QStringLiteral(
            "Menu {\n"
            "  MenuItem {\n"
            "    objectName: \"fx\"\n"
            "    text: \"a } brace\"\n"
            "    onTriggered: theMarker()\n"
            "  }\n"
            "}\n");
        const QString got = menuItemBlock(fx, QStringLiteral("fx"));
        QVERIFY2(got.contains(QLatin1String("theMarker")),
                 "a lone brace inside a string literal truncated the block");
    }

    const QString block = menuItemBlock(m_main, QStringLiteral("helpOpenLicensesItem"));
    QVERIFY2(!block.isEmpty(), "helpOpenLicensesItem: no MenuItem block found in Main.qml");

    // THE EXTRACTOR MUST BOUND, NOT JUST FIND. A "block" that is really the
    // whole file would satisfy every needle below and prove nothing, so the
    // sibling item is pulled in the same run: it must come back non-empty
    // and must NOT contain this item's id.
    const QString sibling = menuItemBlock(m_main, QStringLiteral("helpAboutItem"));
    QVERIFY2(!sibling.isEmpty(), "control failed: sibling helpAboutItem block not found");
    QVERIFY2(!sibling.contains(QLatin1String("open_licenses")),
             "control failed: the extractor is not bounding the block");

    // RAW-TEXT NEEDLES, so a comment inside the block would be indistinguishable
    // from code. Force a conscious decision instead of a silent false match.
    for (const QString &line : block.split(QLatin1Char('\n')))
        QVERIFY2(!line.trimmed().startsWith(QLatin1String("//")),
                 qPrintable(QStringLiteral("a comment entered the fallback's MenuItem block; "
                                           "the needles here are raw-text: ") + line.trimmed()));

    QVERIFY2(block.contains(QStringLiteral("AppI18n.tr(\"ui.menu.help.open_licenses\")")),
             "the fallback's caption is not catalog-driven");
    // The gate, and the guard that keeps it from throwing where there is no
    // host: an unguarded AppLicensesDir threw ReferenceError in three Main.qml
    // harnesses at once when AppBuildCommit shipped without the same guard.
    QVERIFY2(block.contains(QLatin1String("typeof AppLicensesDir === \"string\"")),
             "AppLicensesDir is read without the typeof guard");
    QVERIFY2(block.contains(QLatin1String("visible: dir !== \"\"")),
             "the fallback is not gated on the folder resolving");
    // And it must OPEN THE LICENCES FOLDER, not something adjacent.
    QVERIFY2(block.contains(QLatin1String("Qt.openUrlExternally")),
             "the fallback does not open anything");
    QVERIFY2(block.contains(QLatin1String("+ dir")),
             "the fallback opens something other than the resolved licences dir");

    // LIMIT STATED: this pins the DECLARATION, not the render, and it does
    // not drive the item. Triggering would call Qt.openUrlExternally for
    // real — a Finder window on whoever runs the suite — and a test must not
    // fire that side effect to prove a string is wired.
}

void TstAboutCredits::theLicenceSurfaceIdsRESOLVEAndAFabricatedOneDoesNot()
{
    // Catalog::tr KEY-ECHOES on a miss and the echo is non-empty and truthy
    // (C10), so "it returned something" is never the check — "it returned
    // something OTHER THAN THE KEY" is.
    for (const char *id : {"ui.menu.help.open_licenses",
                           "ui.menu.help.about",
                           "ui.dialog.about.third_party_notices"}) {
        const QString key = QLatin1String(id);
        QVERIFY2(m_catalog.tr(key) != key,
                 qPrintable(QStringLiteral("merged catalog key-echoed ") + key));
    }

    // THE NEGATIVE CONTROL, pointed at the failure that would actually
    // occur: a renamed or dropped id. If a fabricated id ALSO came back
    // resolved, the three passes above would be a property of a blind
    // instrument rather than a measurement.
    const QString fake = QStringLiteral("ui.menu.help.open_licenses_NOT_A_REAL_ID");
    QCOMPARE(m_catalog.tr(fake), fake);
}

// ADDED C-793. The neighbour above checks THREE HARDCODED ids and the credit
// ids are not among them. These are the ones that matter most and were the
// ones nothing watched: they are built by CONCATENATION —
// AboutDialog.qml emits tr("ui.dialog.about.credit." + modelData) over the
// model's rows — so a literal grep for emitted ids returns
// "ui.dialog.about.credit." and nothing after the dot, and every instrument
// that works that way is blind to all three.
//
// ★★★★★ AND THE i18n AUDIT IS ONE OF THOSE INSTRUMENTS, MEASURED RATHER THAN
// ASSUMED (C-793). i18n/tools/audit.py expands family prefixes, but its own
// docstring says the expansion "is bounded by the en catalog ∩ (registry ∪
// MINTED), so RED-2 is TAUTOLOGICAL for family ids" — an id ABSENT from
// canonical en is never counted as emitted and therefore can never be
// reported missing. Controlled by deleting a PRESENT sibling
// (ui.dialog.about.credit.lucide_icons) from canonical en and re-running:
// RED-2's list did not change, while RED-3 fired on the cross-locale key-set
// difference. So a family member missing from ONE locale is caught by RED-3,
// and a family member missing from ALL FOUR is caught by NEITHER — which is
// exactly the state of ui.dialog.about.credit.libmypaint today.
//
// WHAT THIS CASE CAN AND CANNOT DO. It asserts every emitted credit id has a
// string in the SHIPPING en catalog (canonical ∪ seed, merged as
// merge_catalogs.py stages it), derived from the dialog's own model so a
// fourth credit row cannot arrive without one. It does NOT close the es/ja/
// pseudo gap: the seed is English-only, so a seed-only id key-echoes in the
// other three locales, and that is RED-2's open item (Q-D52) rather than a
// thing a shell test can assert.
void TstAboutCredits::everyCreditIdTheDialogEMITSHasAStringAtAll()
{
    const QStringList rows = dialogCreditIds(m_qml);
    QVERIFY2(!rows.isEmpty(), "no credit model found — every check below "
                              "would be vacuously true");

    // Re-read both halves so the SPLIT can be reported. initTestCase merges
    // them and the merge is what ships; which half a string came from is what
    // decides whether the OTHER THREE LOCALES will see it.
    QFile can(QStringLiteral(APPNAME_REPO_I18N_DIR "/en.json"));
    QVERIFY(can.open(QIODevice::ReadOnly));
    const QJsonObject canon = QJsonDocument::fromJson(can.readAll()).object();
    QFile sd(QStringLiteral(APPNAME_SHELL_DIR "/i18n/ui.en.json"));
    QVERIFY(sd.open(QIODevice::ReadOnly));
    const QJsonObject seed = QJsonDocument::fromJson(sd.readAll()).object();

    QStringList echoed, canonicalOnes, seedOnly, neither;
    for (const QString &row : rows) {
        const QString id = QStringLiteral("ui.dialog.about.credit.") + row;
        // C10: tr KEY-ECHOES on a miss and the echo is non-empty and truthy,
        // so "returned something" is never the check.
        if (m_catalog.tr(id) == id)
            echoed << id;
        // THREE buckets, not two. My first cut wrote
        // `(canon.contains(id) ? canonicalOnes : seedOnly)`, which files an id
        // present in NEITHER register under "seed-only" -- so the mutation that
        // DELETED the seed string still reported "1 seed-only", and the
        // sum-equals-rows assertion below was true by construction. Caught by
        // running the mutation rather than by reading the code.
        if (canon.contains(id))        canonicalOnes << id;
        else if (seed.contains(id))    seedOnly << id;
        else                           neither << id;
    }
    qInfo().noquote()
        << QStringLiteral("CREDIT IDS: %1 emitted — %2 canonical, %3 seed-only, "
                          "%4 in NEITHER register (the seed is English-ONLY, so "
                          "seed-only ids key-echo in es/ja/pseudo — RED-2's open "
                          "item)")
               .arg(rows.size()).arg(canonicalOnes.size()).arg(seedOnly.size())
               .arg(neither.size());

    QVERIFY2(echoed.isEmpty(),
             qPrintable(QStringLiteral("credit ids with NO STRING in the "
                                       "shipping en catalog: ")
                        + echoed.join(QStringLiteral(", "))));
    QVERIFY2(neither.isEmpty(),
             qPrintable(QStringLiteral("credit ids in NEITHER the canonical "
                                       "catalog nor the shell seed: ")
                        + neither.join(QStringLiteral(", "))));

    // NEGATIVE CONTROL, pointed at the failure that would actually occur: a
    // renamed model row. If a fabricated credit id ALSO resolved, the passes
    // above would be a property of a blind instrument.
    const QString fake =
        QStringLiteral("ui.dialog.about.credit.NOT_A_REAL_COMPONENT");
    QCOMPARE(m_catalog.tr(fake), fake);
}

// ADDED C-760 — PINNING THE PREMISE OF A GOLDEN, not the golden.
//
// AboutDialog renders "Build <sha>", which changes on EVERY COMMIT. The tour
// frame 39-about.png therefore BLANKS that row before capturing; without the
// blanking the golden can never match a fresh run again, and one commit after
// any re-pin it breaks tour_ok's artifact arm PERMANENTLY (C-759, caught by
// looking at the first capture).
//
// THE GATE CANNOT NOTICE THIS ITSELF. tour_ok runs A and B from the SAME
// binary, so a build-volatile field is identical between them: two runs
// agreeing proves stability WITHIN a build and says nothing ACROSS builds --
// and across builds is where a golden lives. Measured across a real commit
// transition, 40 of 40 frames were byte-identical, so this is the only
// exposure in the tree; that is exactly why the one line holding it deserves
// a pin rather than trust.
void TstAboutCredits::theAboutFrameCANNOTCarryTheBuildCommit()
{
    bool ok = false;
    const QString tour = readAll(QStringLiteral(APPNAME_SHELL_DIR "/src/tour.cpp"), &ok);
    QVERIFY2(ok, "tour.cpp unreadable");
    QVERIFY2(tour.size() > 10000, "tour.cpp suspiciously small");

    const int cap = tour.indexOf(QLatin1String("39-about.png"));
    QVERIFY2(cap > 0, "the About frame's capture site is gone -- if the frame "
                      "was removed, remove this pin deliberately");
    const int blank = tour.lastIndexOf(
        QLatin1String("setProperty(\"buildCommit\", QString())"), cap);
    QVERIFY2(blank > 0,
             "the About tour frame no longer blanks the build commit: the "
             "golden will hold a sha and can never match a fresh run again");
    // ORDER IS THE WHOLE POINT -- blanking AFTER the capture would read as
    // present and do nothing. lastIndexOf(..., cap) already bounds it above,
    // so this asserts they are the SAME block rather than a distant echo.
    QVERIFY2(cap - blank < 1500,
             "the blanking is far from the capture -- is it the same step?");

    // AND THE MECHANISM IT RELIES ON: the row hides when the value is empty.
    // Blanking a property whose row rendered regardless would leave "Build "
    // in the frame and be silently useless.
    const QString qml = readAll(
        QStringLiteral(APPNAME_SHELL_DIR "/qml/dialogs/AboutDialog.qml"), &ok);
    QVERIFY2(ok, "AboutDialog.qml unreadable");
    const QString code = QString(qml).remove(
        QRegularExpression(QStringLiteral("//[^\n]*")));
    QVERIFY2(code.contains(QLatin1String("visible: root.buildCommit !== \"\"")),
             "the commit row no longer hides on an empty value, so blanking it "
             "for the tour no longer removes it from the frame");
}

QTEST_MAIN(TstAboutCredits)
#include "tst_about_credits.moc"
