// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// STANDING PRIORITY (4)-3 — THE DIRTY FLAG, PINNED.
//
// WHY THIS FILE EXISTS AT ALL, stated plainly: the dirty computation has
// been correct and SHIPPED since 7e8bcfa9 (2026-08-13, "dirty-title
// indicator") and NOTHING TESTED IT for eighteen days. It renders as a
// bullet on the title bar, so it does not throw, does not exit non-zero,
// and no assertion named it -- the defect class that DISPLAYS instead of
// FAILING, invisible to every instrument and visible to every person.
// A regression that stopped the marker appearing would have been caught
// by nothing.
//
// THE DISCRIMINATOR IS undoBackToTheSavedBaselineClearsIt. Every other
// case here would also pass against a one-way "has been modified"
// boolean, which is the plausible wrong implementation. Only the undo
// case can tell a CURSOR COMPARISON from a latch: it requires the flag
// to go back to false when the journal returns to the baseline, which a
// latch can never do. Without it this file would pin the feature's name
// and not its meaning.

#include "document_controller.h"
#include "i18n_catalog.h"

#include <QtTest/QtTest>
#include <QTemporaryDir>

using namespace appname::shell;

class TstDirtyFlag : public QObject
{
    Q_OBJECT

private slots:
    void init();
    void cleanup();

    // The case the expression could NOT reach while it was a local inside
    // windowTitle(): exposed as a property it is now callable with no
    // document, and m_documentPath is empty then.
    void noDocumentIsNotDirty();
    // A never-saved document is dirty by the second disjunct alone.
    void neverSavedDocumentIsDirtyAndMarked();
    void saveClearsTheFlagAndTheMarker();
    void editAfterSaveSetsItAgain();
    void undoBackToTheSavedBaselineClearsIt();

    // THE DOOR PREDICATE, which is NOT the title predicate. These three
    // exist because collapsing the two shipped a cry-wolf confirm that
    // tst_gesture_chrome caught and this file did not.
    void pristineUntitledHasNothingToLose();
    void editedUntitledHasWorkToLose();
    void doorAndTitleDisagreeOnAPristineUntitledDocument();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    DocumentController *m_controller = nullptr;
    QTemporaryDir *m_tmp = nullptr;

    static constexpr int kDocW = 64;
    static constexpr int kDocH = 48;
    static constexpr quint64 kSeed = 0x0D147Fu;

    static bool marked(const QString &title)
    {
        return title.startsWith(QStringLiteral("● "));
    }
    QString savePath() const
    {
        return m_tmp->path() + QStringLiteral("/d.appname");
    }
    // One journalled edit. selection.all is the cheapest command that
    // moves the cursor without needing a canvas, a window, or pixels --
    // and the canUndo precondition below is a VACUITY GUARD: if this
    // command ever stops being journalled, the cases that depend on a
    // moved cursor must fail HERE, loudly, instead of quietly testing
    // nothing.
    void oneJournalledEdit()
    {
        const QVariantMap r = m_controller->applyCommand(
            QStringLiteral("selection.all"), {}, {}, {});
        QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
                 "precondition: selection.all must apply");
        QVERIFY2(m_controller->canUndo(),
                 "vacuity: selection.all did not move the journal cursor, so "
                 "every dirty assertion below would be testing nothing");
    }
};

void TstDirtyFlag::init()
{
    // NO document here on purpose -- noDocumentIsNotDirty needs a
    // controller that has never had one, and a case that must construct
    // its own precondition is worse than one that starts from nothing.
    m_controller = new DocumentController(&m_catalog);
    m_controller->setDeterministicIds(kSeed);
    m_tmp = new QTemporaryDir;
    QVERIFY(m_tmp->isValid());
}

void TstDirtyFlag::cleanup()
{
    delete m_controller;
    m_controller = nullptr;
    delete m_tmp;
    m_tmp = nullptr;
}

void TstDirtyFlag::noDocumentIsNotDirty()
{
    QVERIFY(!m_controller->hasDocument());
    QVERIFY2(!m_controller->documentDirty(),
             "no document must not read as dirty: m_documentPath is empty "
             "with no document, so the never-saved disjunct alone would "
             "make every door confirm-on-nothing");
    QVERIFY(!marked(m_controller->windowTitle()));
}

void TstDirtyFlag::neverSavedDocumentIsDirtyAndMarked()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QVERIFY2(m_controller->documentDirty(),
             "a doc.new has no path: it exists nowhere on disk");
    QVERIFY2(marked(m_controller->windowTitle()),
             "the title marker and the property must agree -- they are one "
             "computation, and windowTitle() reading its own copy is the "
             "second-path disagreement this change removed");
}

void TstDirtyFlag::saveClearsTheFlagAndTheMarker()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QVERIFY(m_controller->documentDirty());
    QVERIFY(m_controller->saveDocument(savePath()));
    QVERIFY2(!m_controller->documentDirty(),
             "save captures the baseline AND supplies the path: both "
             "disjuncts must fall at once");
    QVERIFY(!marked(m_controller->windowTitle()));
}

void TstDirtyFlag::editAfterSaveSetsItAgain()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QVERIFY(m_controller->saveDocument(savePath()));
    QVERIFY(!m_controller->documentDirty());
    oneJournalledEdit();
    QVERIFY2(m_controller->documentDirty(),
             "the cursor moved off the saved baseline");
    QVERIFY(marked(m_controller->windowTitle()));
}

void TstDirtyFlag::undoBackToTheSavedBaselineClearsIt()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QVERIFY(m_controller->saveDocument(savePath()));
    oneJournalledEdit();
    QVERIFY(m_controller->documentDirty());

    m_controller->undo();

    // THE WHOLE POINT OF THIS FILE. A latch cannot pass this line.
    QVERIFY2(!m_controller->documentDirty(),
             "undo returned the journal cursor to the saved baseline, so the "
             "document is byte-identical to what is on disk and must NOT be "
             "reported as having unsaved changes -- a one-way modified-flag "
             "would still say dirty here and would pass every other case in "
             "this file");
    QVERIFY(!marked(m_controller->windowTitle()));
}

void TstDirtyFlag::pristineUntitledHasNothingToLose()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QVERIFY2(!m_controller->documentHasUnsavedWork(),
             "a freshly created, untouched document has produced nothing a "
             "user could lose; a confirm here is the cry-wolf that trains "
             "people to click through the dialog that mattered");
}

void TstDirtyFlag::editedUntitledHasWorkToLose()
{
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    oneJournalledEdit();
    QVERIFY2(m_controller->documentHasUnsavedWork(),
             "never saved and edited: the work exists nowhere else at all, "
             "which is the strongest case for confirming");
}

void TstDirtyFlag::doorAndTitleDisagreeOnAPristineUntitledDocument()
{
    // THE POINT OF THE SPLIT, pinned as a disagreement rather than
    // described in a comment. If some later change collapses the two
    // predicates back together -- in either direction -- this fails.
    QVERIFY(m_controller->newDocument(kDocW, kDocH, QStringLiteral("white")));
    QVERIFY2(m_controller->documentDirty(),
             "TITLE: not on disk anywhere, so the marker is correct");
    QVERIFY2(!m_controller->documentHasUnsavedWork(),
             "DOOR: nothing has been produced, so nothing can be destroyed");
    QVERIFY(marked(m_controller->windowTitle()));
}

QTEST_MAIN(TstDirtyFlag)
#include "tst_dirty_flag.moc"
