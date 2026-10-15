// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09's display half, C++ layer. The fixture MIRRORS WHAT PACKAGING STAGES
// rather than pointing at the repo root: build_dmg.sh copies NOTICE_FILES
// (LICENSE, LICENSE-SDK, THIRD-PARTY-NOTICES.md) to the top level and the
// LICENSES/ directory in whole, so the shipped folder holds 3 + 8 = 11.
// Pointing at the repo root would sweep in README and everything else and
// the count would mean nothing.
//
// THE CONTROL THAT MATTERS IS THE FLAT ONE: a plain entryList over the same
// folder finds 3 of 11 and looks like it worked. That asymmetry is the whole
// reason this class walks one level down, and the test asserts BOTH numbers
// so the walk cannot silently regress to the flat behaviour.

#include "license_texts.h"

#include <QDir>
#include <QFile>
#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

class TstLicenseTexts : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void theStagingContractIsStillEIGHTTextsAndELEVENEntries();
    void theWalkFindsEVERYStagedTextAndAFlatListingWouldFindThree();
    void readTextRefusesAnythingItDidNotEnumerate();
    void anOversizeTextIsCappedAndSaysSo();
    void anAbsentFolderIsUnavailableNotEmptyLying();
    void aCappedReadIsAnEXACTBYTEPREFIXAndEndsAtALineBoundary();
    void aCutThatWOULDSplitAMultiByteCharacterDoesNot();
    void noStagedLicenceTextIsZEROBYTES();

private:
    QTemporaryDir m_dir;
    QString m_staged;
    int m_nTexts = 0;       // MEASURED in initTestCase, PINNED in its own test
};

void TstLicenseTexts::initTestCase()
{
    QVERIFY(m_dir.isValid());
    m_staged = m_dir.filePath(QStringLiteral("licenses"));
    QVERIFY(QDir().mkpath(m_staged));
    QVERIFY(QDir().mkpath(m_staged + QStringLiteral("/LICENSES")));

    const QString repo = QStringLiteral(APPNAME_REPO_ROOT);
    for (const char *f : {"LICENSE", "LICENSE-SDK", "THIRD-PARTY-NOTICES.md"}) {
        const QString src = QDir(repo).filePath(QLatin1String(f));
        QVERIFY2(QFile::exists(src), qPrintable(QStringLiteral("missing notice source: ") + src));
        QVERIFY(QFile::copy(src, QDir(m_staged).filePath(QLatin1String(f))));
    }
    const QDir spdx(QDir(repo).filePath(QStringLiteral("LICENSES")));
    const auto texts = spdx.entryList(QDir::Files, QDir::Name);
    // ★ MEASURED HERE, PINNED IN ITS OWN TEST BELOW (C-851). This line was
    // `QCOMPARE(texts.size(), 8)` and a QtTest initTestCase failure ABORTS THE
    // WHOLE SUITE: staging one extra text took this suite from 8 passed to
    // "1 passed, 1 failed" and tst_licenses_dialog from 13 to the same --
    // twenty-one checks offline at the moment the licence surface changes
    // most, none of them ever run against the NEW set. The pin is deliberate
    // and it stays; what moved is where it fires from.
    m_nTexts = texts.size();
    QVERIFY2(m_nTexts > 0, "LICENSES/ staged nothing -- every count below "
                           "would be vacuously true");
    for (const QString &t : texts)
        QVERIFY(QFile::copy(spdx.filePath(t),
                            QDir(m_staged + QStringLiteral("/LICENSES")).filePath(t)));
}

void TstLicenseTexts::theStagingContractIsStillEIGHTTextsAndELEVENEntries()
{
    // THE TRIPWIRE. The staging set must not move in silence -- and when it
    // does move, THIS is the one test that should say so while every other
    // check in both suites keeps running against the new set.
    QCOMPARE(m_nTexts, 8);
    const LicenseTexts lt(m_staged);
    QCOMPARE(lt.entries().size(), 11);   // the three notice files + the eight
}

void TstLicenseTexts::theWalkFindsEVERYStagedTextAndAFlatListingWouldFindThree()
{
    const LicenseTexts lt(m_staged);
    QVERIFY(lt.available());
    QCOMPARE(lt.entries().size(), 3 + m_nTexts);

    // THE CONTROL: the flat listing this class exists to not be.
    QCOMPARE(QDir(m_staged).entryList(QDir::Files).size(), 3);

    // And the nested ones carry their subdirectory in `rel`, or `readText`
    // could never find them again.
    int nested = 0;
    for (const QVariant &e : lt.entries())
        if (e.toMap().value(QStringLiteral("rel")).toString().startsWith(QStringLiteral("LICENSES/")))
            ++nested;
    QCOMPARE(nested, m_nTexts);
}

// ★ THE OPERATOR RULED THIS A FAILING ASSERTION: WORD 8 of 2026-09-14 18:48 ET,
//   answer (2) -- "the one assertion that notices becomes the one that fails".
//
// WHAT IT GUARDS. A staged file that reads ZERO BYTES is enumerated like any
// other, so the list names it and the body pane renders nothing. The dialog's
// own comment calls a blank body "actively misleading" because it is
// near-identical to the `unavailable` state -- the user cannot tell "we ship no
// texts" from "this text is empty". `ui.dialog.licenses.text_missing` is the
// PERMANENT POSTURE for that case in the UI; this is the assertion that keeps it
// from ever being a SHIPPED state.
//
// ⛔ IT IS NOT A CHECK THAT THE STRING EXISTS. It is a check that the condition
//   the string describes does not occur in what we stage. Those are different
//   claims and only the second one can fail here.
void TstLicenseTexts::noStagedLicenceTextIsZEROBYTES()
{
    const LicenseTexts lt(m_staged);
    QVERIFY2(lt.available(), "the staged folder did not resolve -- nothing below is measured");

    int checked = 0;
    QStringList empties;
    for (const QVariant &e : lt.entries()) {
        const QString rel = e.toMap().value(QStringLiteral("rel")).toString();
        if (rel.isEmpty())
            continue;
        ++checked;
        if (lt.readText(rel).isEmpty())
            empties << rel;
    }

    // POPULATION GUARD, the 516b538 shape: a sweep that examined nothing reports
    // no empties and looks exactly like a clean stage.
    QVERIFY2(checked == 3 + m_nTexts,
             qPrintable(QStringLiteral("swept %1 of %2 staged entries -- the sweep is not a sweep")
                            .arg(checked).arg(3 + m_nTexts)));

    QVERIFY2(empties.isEmpty(),
             qPrintable(QStringLiteral("ZERO-BYTE licence text(s) staged: %1 -- "
                                       "ui.dialog.licenses.text_missing is a permanent posture, "
                                       "never a shipped state (WORD 8 (2))")
                            .arg(empties.join(QStringLiteral(", ")))));
}

void TstLicenseTexts::readTextRefusesAnythingItDidNotEnumerate()
{
    const LicenseTexts lt(m_staged);
    // Not a path-traversal special case — simply not an entry.
    QVERIFY(lt.readText(QStringLiteral("../../../etc/passwd")).isEmpty());
    QVERIFY(lt.readText(QStringLiteral("LICENSES/../../LICENSE")).isEmpty());
    QVERIFY(lt.readText(QStringLiteral("nope.txt")).isEmpty());
    // CONTROL: a real entry DOES read, so the three empties above are
    // refusals and not a reader that returns nothing for everything.
    QVERIFY(!lt.readText(QStringLiteral("LICENSE")).isEmpty());
}

void TstLicenseTexts::anOversizeTextIsCappedAndSaysSo()
{
    const LicenseTexts lt(m_staged);
    const QVariantMap big = lt.read(QStringLiteral("THIRD-PARTY-NOTICES.md"));
    QVERIFY2(big.value(QStringLiteral("truncated")).toBool(),
             "the 560 KB notices aggregate was NOT reported truncated");
    QVERIFY(big.value(QStringLiteral("text")).toString().size() > 0);
    QVERIFY(big.value(QStringLiteral("text")).toString().toUtf8().size()
            <= LicenseTexts::kMaxRenderBytes);
    // CONTROL: a small text is NOT reported truncated, so the flag tracks the
    // cap rather than being stuck on.
    const QVariantMap small = lt.read(QStringLiteral("LICENSES/MIT.txt"));
    QVERIFY2(!small.value(QStringLiteral("truncated")).toBool(),
             "a small licence text was reported truncated");
}

void TstLicenseTexts::anAbsentFolderIsUnavailableNotEmptyLying()
{
    const LicenseTexts none(m_dir.filePath(QStringLiteral("does-not-exist")));
    QVERIFY(!none.available());
    QCOMPARE(none.entries().size(), 0);
    QVERIFY(none.readText(QStringLiteral("LICENSE")).isEmpty());
    // An empty string is also what a resolved-but-empty folder gives, so the
    // caller must branch on `available`, not on entries().isEmpty() alone.
    const LicenseTexts empty(QString{});
    QVERIFY(!empty.available());
}

void TstLicenseTexts::aCappedReadIsAnEXACTBYTEPREFIXAndEndsAtALineBoundary()
{
    const LicenseTexts lt(m_staged);
    const QString shown = lt.readText(QStringLiteral("THIRD-PARTY-NOTICES.md"));
    QVERIFY(!shown.isEmpty());

    // THE PROPERTY THAT MATTERS, and it is ONE assertion rather than a list
    // of symptoms: what the dialog renders, re-encoded, must be an EXACT
    // BYTE PREFIX of the file that ships. That fails if the cut mangled a
    // character, if the decoder substituted anything, or if any byte moved.
    QFile src(QDir(m_staged).filePath(QStringLiteral("THIRD-PARTY-NOTICES.md")));
    QVERIFY(src.open(QIODevice::ReadOnly));
    const QByteArray all = src.readAll();
    const QByteArray rendered = shown.toUtf8();
    QVERIFY2(all.startsWith(rendered),
             "what the viewer renders is NOT a byte prefix of the shipped file");

    // A LICENCE TEXT MUST NOT END MID-CLAUSE. Before this, the cut landed at
    // "...or additions\n   to that Work or " — mid-sentence of an Apache
    // paragraph, indistinguishable to a reader from where the clause ends.
    QVERIFY2(shown.endsWith(QLatin1Char('\n')),
             "the capped text does not end at a line boundary");

    // AND IT MUST STILL BE THE CAP, not an arbitrary early stop: the
    // boundary walk-back cost 19 bytes when measured, so a result far below
    // the cap would mean something else truncated it.
    QVERIFY(rendered.size() <= LicenseTexts::kMaxRenderBytes);
    QVERIFY2(rendered.size() > LicenseTexts::kMaxRenderBytes - 4096,
             "the capped read stopped well short of the cap");
}

void TstLicenseTexts::aCutThatWOULDSplitAMultiByteCharacterDoesNot()
{
    // THE HAZARD IS LATENT IN THE SHIPPED FILE, NOT ABSENT: measured, the
    // real cut lands on ASCII today, but the aggregate carries 2528
    // non-ASCII bytes and grows with every dependency drop, so whether byte
    // 262144 splits a character is a property of content this desk does not
    // control. So it is pinned on a fixture that FORCES the split, with no
    // newline anywhere — which also exercises the no-boundary fallback that
    // the real file never reaches.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString dir = tmp.filePath(QStringLiteral("lic"));
    QVERIFY(QDir().mkpath(dir));

    QByteArray blob(LicenseTexts::kMaxRenderBytes - 1, 'a');
    blob += QByteArray::fromStdString("\xe2\x82\xac");   // EURO SIGN, 3 bytes
    blob += QByteArray(1024, 'b');                       // ensure it is over the cap
    QFile f(QDir(dir).filePath(QStringLiteral("wide.txt")));
    QVERIFY(f.open(QIODevice::WriteOnly));
    QCOMPARE(f.write(blob), qint64(blob.size()));
    f.close();

    const LicenseTexts lt(dir);
    QCOMPARE(lt.entries().size(), 1);
    bool cut = false;
    const QString shown = lt.readText(QStringLiteral("wide.txt"), &cut);
    QVERIFY2(cut, "the oversize fixture was not reported truncated");

    // The same one property: an exact byte prefix. A split sequence decodes
    // to U+FFFD, which re-encodes to three DIFFERENT bytes, so this fails.
    QVERIFY2(blob.startsWith(shown.toUtf8()),
             "the cut split a character: the render is not a byte prefix");
    QVERIFY2(!shown.contains(QChar(0xFFFD)),
             "the rendered text contains a replacement character");

    // CONTROL: the cut really did land where it would have split — i.e. the
    // fixture exercises the hazard rather than dodging it. The euro begins
    // at kMaxRenderBytes-1, so a raw read(cap) takes exactly its first byte.
    QCOMPARE(blob.at(LicenseTexts::kMaxRenderBytes - 1), '\xe2');
    QVERIFY2(shown.toUtf8().size() == LicenseTexts::kMaxRenderBytes - 1,
             "the fallback dropped more than the dangling lead byte");
}

QTEST_MAIN(TstLicenseTexts)
#include "tst_license_texts.moc"
