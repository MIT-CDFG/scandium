// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09's display half, QML layer: the REAL LicensesDialog booted against a
// REAL LicenseTexts over a fixture that mirrors what packaging stages
// (3 top-level + LICENSES/8 = 11). tst_license_texts pins the C++ walk;
// nothing pinned that the walk's result ever REACHES a user, which is the
// half an About box is actually judged on.
//
// THE UNAVAILABLE ARM IS NOT DEFENSIVE PADDING. It is the state every dev
// build is in, and it is reached through a SECOND QQmlContext holding an
// unavailable model rather than by deleting the fixture — so both arms run
// in one process against one engine, and neither can pass by the other's
// setup leaking into it.

#include "i18n_catalog.h"
#include "license_texts.h"

#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

namespace {

QString copyFail;

bool setUpModuleDir(const QString &root)
{
    const QString mod = root + QStringLiteral("/AppnameShell");
    if (!QDir().mkpath(mod)) { copyFail = QStringLiteral("mkpath failed"); return false; }
    const struct { const char *src; const char *dst; } files[] = {
        {"/qml/Theme.qml", "/Theme.qml"},
        {"/qml/dialogs/AboutDialog.qml", "/AboutDialog.qml"},
        {"/qml/dialogs/LicensesDialog.qml", "/LicensesDialog.qml"},
    };
    for (const auto &f : files) {
        const QString src = QStringLiteral(APPNAME_SHELL_DIR) + QLatin1String(f.src);
        const QString dst = mod + QLatin1String(f.dst);
        QFile::remove(dst);
        if (!QFile::copy(src, dst)) { copyFail = QStringLiteral("copy failed: ") + src; return false; }
    }
    QFile qmldir(mod + QStringLiteral("/qmldir"));
    if (!qmldir.open(QIODevice::WriteOnly)) { copyFail = QStringLiteral("qmldir open failed"); return false; }
    qmldir.write("module AppnameShell\n"
                 "singleton Theme 1.0 Theme.qml\n"
                 "AboutDialog 1.0 AboutDialog.qml\n"
                 "LicensesDialog 1.0 LicensesDialog.qml\n");
    return true;
}

const char kScene[] =
    "import QtQuick\n"
    "import QtQuick.Controls\n"
    "import AppnameShell\n"
    "ApplicationWindow {\n"
    "    id: win\n"
    // 1280x800 — the APPLICATION's own window size, and the tour's, so a
    // layout assertion here measures what a user gets. ★ IT WAS CHANGED FOR
    // A WRONG REASON AND KEPT FOR A RIGHT ONE: I first blamed the 900x700
    // window for a body width that would not move, and it was not the cause
    // — the number I was reading came from an internal Flickable child.
    // Recorded rather than quietly corrected, because the wrong reason is
    // the reusable part.
    "    width: 1280; height: 800; visible: true\n"
    "}\n";

// VISUAL traversal, and it finds INVISIBLE items too. Both matter: A28 —
// a view's delegates are given a VISUAL parent, so findChildren (which
// walks QObject parentage) cannot see them; and the unavailable arm has to
// assert that a control is PRESENT AND HIDDEN, which a visible-only walk
// reports identically to a control that was never built.
QQuickItem *findItem(QQuickItem *root, const QString &objectName)
{
    if (!root) return nullptr;
    for (QQuickItem *k : root->childItems()) {
        if (k->objectName() == objectName) return k;
        if (QQuickItem *hit = findItem(k, objectName)) return hit;
    }
    return nullptr;
}

void collectVisibleText(QQuickItem *root, QStringList &out)
{
    if (!root) return;
    for (QQuickItem *k : root->childItems()) {
        if (k->isVisible()) {
            const QString t = k->property("text").toString();
            if (!t.isEmpty()) out << t;
        }
        collectVisibleText(k, out);
    }
}

} // namespace

class TstLicensesDialog : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void theStagedTextsReachQmlAndTheListNamesThem();
    void openingATextRendersItAndCarriesItsNameForAReader();
    void anOversizeTextSaysItIsPartial();
    void withNothingStagedTheDialogEXPLAINSRatherThanListingNothing();
    void theAboutDoorOpensInBOTHStates();
    void theDialogOpensSHOWINGSomethingNotABlankPane();
    void aKeyboardUserCanMOVEThroughTheTextsAndTheBodyFOLLOWS();
    void escapeCLOSESIt();
    void openingItGIVESTheListKeyboardFocusWithoutAClick();
    void cleanup();
    void theBodyFitsTheTextsAndTheListSAYSThereIsMore();
    void theHighlightFOLLOWSWhateverTheEntryPoint();

private:
    QQuickItem *contentOf(QObject *dlg) const
    { return dlg->property("contentItem").value<QQuickItem *>(); }

    Catalog m_catalog = Catalog::fromTexts({});
    CatalogQml *m_i18n = nullptr;
    LicenseTexts *m_texts = nullptr;        // over the staged fixture
    LicenseTexts *m_absent = nullptr;       // over a folder that is not there
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_win = nullptr;
    QQmlContext *m_absentCtx = nullptr;
    QObject *m_dlg = nullptr;
    QTemporaryDir m_dir;
    QString m_staged;
};

void TstLicensesDialog::initTestCase()
{
    QVERIFY(m_dir.isValid());
    QVERIFY2(setUpModuleDir(m_dir.path()), qPrintable(copyFail));

    // MIRROR PACKAGING, NOT THE REPO ROOT: build_dmg.sh stages NOTICE_FILES
    // at the top level and LICENSES/ whole. Pointing at the repo root would
    // sweep in README and every other file and the count would mean nothing.
    m_staged = m_dir.filePath(QStringLiteral("licenses"));
    QVERIFY(QDir().mkpath(m_staged + QStringLiteral("/LICENSES")));
    const QString repo = QStringLiteral(APPNAME_REPO_ROOT);
    for (const char *f : {"LICENSE", "LICENSE-SDK", "THIRD-PARTY-NOTICES.md"}) {
        const QString src = QDir(repo).filePath(QLatin1String(f));
        QVERIFY2(QFile::exists(src), qPrintable(QStringLiteral("missing notice source: ") + src));
        QVERIFY(QFile::copy(src, QDir(m_staged).filePath(QLatin1String(f))));
    }
    const QDir spdx(QDir(repo).filePath(QStringLiteral("LICENSES")));
    const auto texts = spdx.entryList(QDir::Files, QDir::Name);
    QCOMPARE(texts.size(), 8);          // the staging contract, pinned
    for (const QString &t : texts)
        QVERIFY(QFile::copy(spdx.filePath(t),
                            QDir(m_staged + QStringLiteral("/LICENSES")).filePath(t)));

    m_texts = new LicenseTexts(m_staged, this);
    QCOMPARE(m_texts->entries().size(), 11);
    m_absent = new LicenseTexts(m_dir.filePath(QStringLiteral("no-such-folder")), this);
    QVERIFY(!m_absent->available());

    // canonical + seed, merged — the catalog the product actually stages.
    // Every id this dialog uses lives ONLY in the shell seed, so a
    // canonical-only boot would key-echo all four and each assert below
    // would pass against a catalog no user will ever see (C10).
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
    m_i18n = new CatalogQml(m_catalog, this);

    m_engine = new QQmlApplicationEngine(this);
    m_engine->addImportPath(m_dir.path());
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"), m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppLicenseTexts"), m_texts);
    m_engine->loadData(QByteArray(kScene), QUrl(QStringLiteral("qrc:/lic/scene.qml")));
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "scene failed to load");
    m_win = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_win);

    // The unavailable arm's context, a CHILD of the root context so it
    // inherits AppI18n and overrides only the model.
    m_absentCtx = new QQmlContext(m_engine->rootContext(), this);
    m_absentCtx->setContextProperty(QStringLiteral("AppLicenseTexts"), m_absent);

    // CONSTRUCTED AFTER THE ENGINE IS SET UP, never inline in the scene:
    // instantiated inline, every AppI18n binding evaluates during loadData
    // and throws "Cannot call method 'tr' of null" — plain Labels recover
    // when the property resolves, but DELEGATES created during that failed
    // pass keep the broken binding and render EMPTY, which is a populated-
    // looking dialog with empty rows (measured on tst_about_render).
    QQmlComponent comp(m_engine);
    comp.setData("import AppnameShell\nLicensesDialog {}\n",
                 QUrl(QStringLiteral("qrc:/lic/dlg.qml")));
    QVERIFY2(!comp.isError(), qPrintable(comp.errorString()));
    m_dlg = comp.create(m_engine->rootContext());
    QVERIFY2(m_dlg, "LicensesDialog failed to construct");
    m_dlg->setProperty("parent", QVariant::fromValue(m_win->contentItem()));
    QVERIFY(m_dlg->property("haveTexts").toBool());
}

// A MUTATED FIXTURE IS HOW A SUITE STARTS LYING TO THE NEXT TEST IN IT:
// C-730 was my own probe leaving a modal open and breaking a sibling, found
// only after three runs that grepped my own output.
void TstLicensesDialog::cleanup()
{
    if (m_dlg) {
        QMetaObject::invokeMethod(m_dlg, "close");
        m_dlg->setProperty("currentRel", QString());
        m_dlg->setProperty("currentName", QString());
        m_dlg->setProperty("currentTruncated", false);
    }
}

void TstLicensesDialog::theStagedTextsReachQmlAndTheListNamesThem()
{
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY2(m_dlg->property("visible").toBool(), "dialog never became visible");

    QQuickItem *list = findItem(contentOf(m_dlg), QStringLiteral("licensesList"));
    QVERIFY2(list, "licensesList not in the visual tree");

    // `count` IS THE MODEL'S SIZE, NOT A RENDER COUNT — the same fact
    // Repeater::count carries. It proves the C++ entries crossed into QML;
    // it does NOT prove a row was built, which is why the walk follows.
    QCOMPARE(list->property("count").toInt(), 11);

    QStringList names;
    for (const QVariant &e : m_texts->entries())
        names << e.toMap().value(QStringLiteral("name")).toString();

    QStringList rendered;
    collectVisibleText(list, rendered);
    QVERIFY2(!rendered.isEmpty(), "the list rendered no rows at all");

    // A ListView VIRTUALISES: it builds the rows that fit plus a cache
    // buffer, so asserting all eleven were built would pin the viewport
    // height, not the feature. What must hold is that every row it DID
    // build names a real staged text, and that the first one is there.
    for (const QString &r : rendered)
        QVERIFY2(names.contains(r),
                 qPrintable(QStringLiteral("row names something not staged: ") + r));
    QVERIFY2(rendered.contains(names.first()),
             qPrintable(QStringLiteral("first staged text is not listed: ") + names.first()));
}

void TstLicensesDialog::openingATextRendersItAndCarriesItsNameForAReader()
{
    const QString id = QStringLiteral("ui.dialog.licenses.body");
    QVERIFY2(m_catalog.tr(id) != id, "merged catalog key-echoed the body label");

    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "show",
                                      Q_ARG(QVariant, QStringLiteral("LICENSE")),
                                      Q_ARG(QVariant, QStringLiteral("LICENSE"))));

    QQuickItem *body = findItem(contentOf(m_dlg), QStringLiteral("licensesBody"));
    QVERIFY2(body, "licensesBody not in the visual tree");
    const QString shown = body->property("text").toString();
    QVERIFY2(!shown.isEmpty(), "the body pane is empty after a read");
    QCOMPARE(shown, m_texts->readText(QStringLiteral("LICENSE")));
    QCOMPARE(m_dlg->property("currentName").toString(), QStringLiteral("LICENSE"));

    // LIMIT STATED: the attached Accessible.name VALUE is not read here —
    // doing so needs QtQuick private headers. Its PRESENCE is pinned
    // mechanically by tst_a11y_tree_lint (ES 31.5 item 1, and I watched it
    // fire), and what this asserts is the half the lint cannot see: that
    // the id and its {name} argument actually compose to a resolved string
    // carrying the document's name, rather than to a key echo.
    const QString label = m_catalog.tr(id, {{QStringLiteral("name"), QStringLiteral("LICENSE")}});
    QVERIFY2(label != id, "body label key-echoed with its argument");
    QVERIFY2(label.contains(QStringLiteral("LICENSE")),
             qPrintable(QStringLiteral("body label does not name the document: ") + label));
}

void TstLicensesDialog::anOversizeTextSaysItIsPartial()
{
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());

    QQuickItem *notice = findItem(contentOf(m_dlg), QStringLiteral("licensesTruncated"));
    QVERIFY2(notice, "licensesTruncated not in the visual tree");

    // CONTROL FIRST, so "the notice appeared" below is a response to the
    // oversize text and not the notice's resting state.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "show",
                                      Q_ARG(QVariant, QStringLiteral("LICENSE")),
                                      Q_ARG(QVariant, QStringLiteral("LICENSE"))));
    QVERIFY2(!m_dlg->property("currentTruncated").toBool(),
             "a 35 KB text reported itself truncated");
    QTRY_VERIFY2(!notice->isVisible(), "the truncation notice shows for a small text");

    bool wasCapped = false;
    const QString full = m_texts->readText(QStringLiteral("THIRD-PARTY-NOTICES.md"), &wasCapped);
    QVERIFY2(wasCapped, "the 573 KB notices file was not capped — fixture changed?");

    QVERIFY(QMetaObject::invokeMethod(m_dlg, "show",
                                      Q_ARG(QVariant, QStringLiteral("THIRD-PARTY-NOTICES.md")),
                                      Q_ARG(QVariant, QStringLiteral("THIRD-PARTY-NOTICES.md"))));
    QVERIFY2(m_dlg->property("currentTruncated").toBool(),
             "the capped read did not set currentTruncated");
    QTRY_VERIFY2(notice->isVisible(), "the text was cut and the dialog did not say so");

    QQuickItem *body = findItem(contentOf(m_dlg), QStringLiteral("licensesBody"));
    QVERIFY(body);
    QCOMPARE(body->property("text").toString().size(), full.size());
}

void TstLicensesDialog::withNothingStagedTheDialogEXPLAINSRatherThanListingNothing()
{
    QQmlComponent comp(m_engine);
    comp.setData("import AppnameShell\nLicensesDialog {}\n",
                 QUrl(QStringLiteral("qrc:/lic/dlg-absent.qml")));
    QVERIFY2(!comp.isError(), qPrintable(comp.errorString()));
    QObject *dlg = comp.create(m_absentCtx);
    QVERIFY2(dlg, "LicensesDialog failed to construct on the absent model");
    dlg->setProperty("parent", QVariant::fromValue(m_win->contentItem()));

    QVERIFY2(!dlg->property("haveTexts").toBool(),
             "the dialog claims texts over a folder that does not exist");
    QVERIFY(QMetaObject::invokeMethod(dlg, "open"));
    QTRY_VERIFY(dlg->property("visible").toBool());

    QQuickItem *said = findItem(contentOf(dlg), QStringLiteral("licensesUnavailable"));
    QVERIFY2(said, "licensesUnavailable not in the visual tree");
    QVERIFY2(said->isVisible(), "nothing is staged and the dialog says nothing");

    const QString want = m_catalog.tr(QStringLiteral("ui.dialog.licenses.unavailable"));
    QVERIFY2(want != QStringLiteral("ui.dialog.licenses.unavailable"),
             "merged catalog key-echoed the unavailable notice");
    QCOMPARE(said->property("text").toString(), want);

    // THE OTHER DIRECTION: the list must be GONE, not merely empty. An
    // empty list beside an explanation reads as a load failure, which is
    // the confusion the explanation exists to prevent.
    QQuickItem *list = findItem(contentOf(dlg), QStringLiteral("licensesList"));
    QVERIFY(list);
    QVERIFY2(!list->isVisible(), "an empty list is shown next to the explanation");

    QMetaObject::invokeMethod(dlg, "close");
    delete dlg;
}

void TstLicensesDialog::theAboutDoorOpensInBOTHStates()
{
    // A DOOR THAT IS DISABLED WHEN THERE IS NOTHING TO SHOW EXPLAINS
    // NOTHING. The dialog behind it carries the explanation and the pointer
    // to Help > Open Licenses Folder, so the button must open in both
    // states — and pinning BOTH is the point: `enabled: haveTexts` passes
    // any one-directional check written against the available arm alone.
    for (const bool staged : {true, false}) {
        QQmlComponent comp(m_engine);
        comp.setData("import AppnameShell\nAboutDialog {}\n",
                     QUrl(QStringLiteral("qrc:/lic/about.qml")));
        QVERIFY2(!comp.isError(), qPrintable(comp.errorString()));
        QObject *about = comp.create(staged ? m_engine->rootContext() : m_absentCtx);
        QVERIFY2(about, "AboutDialog failed to construct");
        about->setProperty("parent", QVariant::fromValue(m_win->contentItem()));
        QVERIFY(QMetaObject::invokeMethod(about, "open"));
        QTRY_VERIFY(about->property("visible").toBool());

        QQuickItem *door = findItem(contentOf(about), QStringLiteral("aboutOpenLicenses"));
        QVERIFY2(door, "the About dialog has no third-party-notices door");
        QVERIFY2(door->isVisible(), "the door is not shown");
        QVERIFY2(door->property("enabled").toBool(),
                 staged ? "the door is disabled with texts staged"
                        : "the door is disabled with nothing staged, so the "
                          "dialog's explanation is unreachable");

        QObject *behind = about->findChild<QObject *>(QStringLiteral("aboutLicensesDialog"));
        QVERIFY2(behind, "no LicensesDialog behind the door");
        QCOMPARE(behind->property("haveTexts").toBool(), staged);
        QVERIFY(QMetaObject::invokeMethod(behind, "open"));
        QTRY_VERIFY2(behind->property("visible").toBool(),
                     "the licenses dialog did not open");

        QMetaObject::invokeMethod(behind, "close");
        QMetaObject::invokeMethod(about, "close");
        delete about;
    }
}

void TstLicensesDialog::theDialogOpensSHOWINGSomethingNotABlankPane()
{
    // A master-detail that opens with an empty right-hand pane looks like a
    // load failure, and this one has an "unavailable" state that looks
    // almost the same -- so an empty body on open is actively confusing
    // rather than merely unhelpful.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());

    QTRY_VERIFY2(!m_dlg->property("currentRel").toString().isEmpty(),
                 "the dialog opened with nothing selected");
    QQuickItem *body = findItem(contentOf(m_dlg), QStringLiteral("licensesBody"));
    QVERIFY(body);
    QVERIFY2(!body->property("text").toString().isEmpty(),
             "the dialog opened with a blank body pane");
}

void TstLicensesDialog::aKeyboardUserCanMOVEThroughTheTextsAndTheBodyFOLLOWS()
{
    // ES 31.5 and G-06: every control reachable by keyboard. A list whose
    // rows only respond to onClicked is MOUSE-ONLY in effect -- arrows may
    // move a highlight while the pane behind it never changes, which is
    // worse than no navigation because it looks like it worked.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());

    QQuickItem *list = findItem(contentOf(m_dlg), QStringLiteral("licensesList"));
    QQuickItem *body = findItem(contentOf(m_dlg), QStringLiteral("licensesBody"));
    QVERIFY(list && body);

    QVERIFY(QMetaObject::invokeMethod(list, "forceActiveFocus"));
    QTRY_VERIFY2(list->hasActiveFocus(), "the list cannot take keyboard focus");

    const QString relBefore = m_dlg->property("currentRel").toString();
    const QString textBefore = body->property("text").toString();

    QTest::keyClick(m_win, Qt::Key_Down);

    QTRY_VERIFY2(m_dlg->property("currentRel").toString() != relBefore,
                 "arrowing the list did not change the selected text");
    const QString relAfter = m_dlg->property("currentRel").toString();
    QTRY_VERIFY2(body->property("text").toString() != textBefore,
                 "the selection moved but the body pane did not follow");
    QCOMPARE(body->property("text").toString(), m_texts->readText(relAfter));
}

void TstLicensesDialog::escapeCLOSESIt()
{
    // A MODAL a keyboard user cannot dismiss is a trap, not a dialog: it
    // covers the window and takes the input.
    //
    // WHAT THIS ACTUALLY PINS IS `closePolicy`, AND I HAD IT WRONG. The
    // comment here first claimed the test passed only because the Dialog
    // sets `focus` -- then the control came back MUTE: removing `focus`
    // changed nothing, while narrowing closePolicy to NoAutoClose turned
    // this RED. Escape reaches a Popup through the window overlay, not
    // through active focus. The wrong attribution is recorded rather than
    // quietly deleted, because a passing test with a false reason is how a
    // later desk "simplifies" the property that was really holding it up.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());
    QTest::keyClick(m_win, Qt::Key_Escape);
    QTRY_VERIFY2(!m_dlg->property("visible").toBool(),
                 "Escape did not close the modal");
}

void TstLicensesDialog::openingItGIVESTheListKeyboardFocusWithoutAClick()
{
    // THE PROPERTY A KEYBOARD USER ACTUALLY EXPERIENCES: open the dialog and
    // the arrows work, with no click first. The arrow test above cannot say
    // this -- it calls forceActiveFocus itself, which is exactly the step a
    // keyboard user has no way to perform.
    //
    // ★ LIMIT STATED, AND IT COST THREE CONTROLS TO FIND. This pins the
    // PROPERTY, not the MECHANISM. LicensesDialog carries three things that
    // could deliver it -- the Dialog's `focus`, the ListView's `focus`, and
    // the onOpened forceActiveFocus -- and this slot passes with ALL THREE
    // STRIPPED, in every combination (measured, 2^3 minus the trivial case).
    // The offscreen harness grants content focus on its own, so it cannot
    // discriminate them. They are kept on Qt's documented Popup contract,
    // NOT on evidence from here, and a desk deleting them as dead must
    // re-check on a REAL window -- which this suite cannot be.
    //
    // What this WOULD catch is the change that actually threatens the
    // property: another focusable control landing ahead of the list, or the
    // list ceasing to be focusable at all.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());

    QQuickItem *list = findItem(contentOf(m_dlg), QStringLiteral("licensesList"));
    QVERIFY(list);
    // `hasActiveFocus()` IS THE WRONG INSTRUMENT HERE and I measured that:
    // it stayed true with all three focus mechanisms stripped out, i.e. it
    // passed under every mutation. The window's OWN activeFocusItem is the
    // discriminating one -- it names the single item that keys will reach.
    // `hasActiveFocus()` IS THE WRONG INSTRUMENT and I measured that: it
    // stayed true with every mechanism stripped AND is true for ancestors,
    // so it cannot say WHERE a key would land. The window's own
    // activeFocusItem names the single item that receives it.
    QQuickItem *focused = m_win->activeFocusItem();
    QVERIFY2(focused, "nothing holds keyboard focus after open");
    bool inList = false;
    for (QQuickItem *p = focused; p; p = p->parentItem())
        if (p == list) { inList = true; break; }
    QVERIFY2(inList,
             "keyboard focus is not in the list after open, so the arrow "
             "keys do nothing until the user clicks a row");
}

void TstLicensesDialog::theBodyFitsTheTextsAndTheListSAYSThereIsMore()
{
    // BOTH NUMBERS HERE CAME FROM LOOKING AT THE RENDERED DIALOG (C-761),
    // and both are measured rather than chosen.
    //
    // WIDTH: the eleven staged texts are a MIXED corpus — eight are
    // hard-wrapped at 74-78 columns, three are unwrapped paragraphs (ISC p90
    // 471, MIT 105, LGPL-3.0-only max 937). So WordWrap must stay, or those
    // three demand 900-column horizontal scrolling; and the pane must fit the
    // wrapped majority, or every upstream line re-wraps and orphans its last
    // word. At 520px it did exactly that, defeating the file's own stated
    // intent. A floor, not an equality: a WIDER pane is never the defect.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());

    QQuickItem *body = findItem(contentOf(m_dlg), QStringLiteral("licensesBody"));
    QVERIFY(body);
    // ★ MEASURE THE ITEM, NOT AN ANCESTOR — and this cost three wrong
    // hypotheses before I dumped the chain instead of reasoning about it.
    // Walking up from the TextArea lands on the Flickable's internal
    // contentItem, which reports 571.844px and is NOT the viewport: it did
    // not move when the width went 660 -> 690, nor when the harness window
    // went 900 -> 1280, and I read that stability as evidence about the
    // dialog TWICE. The TextArea's own width is 690 — exactly what the QML
    // asks for, and what the text lays out in.
    // A FLOOR AT ~78 COLUMNS, the corpus MAX, at the measured ~8.2px glyph
    // width. A floor and not an equality: a wider pane is never the defect.
    QVERIFY2(body->width() >= 640,
             qPrintable(QStringLiteral("the text is laid out in %1px — too narrow "
                                       "for the corpus max of 78 columns")
                            .arg(body->width())));

    // DISCOVERABILITY: eleven entries, eight rows visible. Without a
    // permanent scrollbar there is no affordance that the other three
    // exist — and the count grows with every dependency drop, so sizing the
    // list to today's eleven would be brittle where this is not.
    QQuickItem *list = findItem(contentOf(m_dlg), QStringLiteral("licensesList"));
    QVERIFY(list);
    QCOMPARE(list->property("count").toInt(), 11);
    QVERIFY2(list->property("contentHeight").toReal() > list->height(),
             "the list no longer overflows — if it now fits, this pin needs "
             "re-deriving rather than deleting");
    bool bar = false;
    for (QQuickItem *k : list->childItems())
        if (QLatin1String(k->metaObject()->className()).contains(QLatin1String("ScrollBar")))
            bar = true;
    QVERIFY2(bar, "the licences list has no scrollbar: three of eleven texts "
                  "are off-screen with nothing to say so");
}

void TstLicensesDialog::theHighlightFOLLOWSWhateverTheEntryPoint()
{
    // The list highlight and the shown text must agree no matter which door
    // was used. Driving show() directly -- which is what these tests do --
    // used to move the body while currentIndex stayed at 0, so the list
    // pointed at one text while the pane rendered another.
    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY(m_dlg->property("visible").toBool());
    QQuickItem *list = findItem(contentOf(m_dlg), QStringLiteral("licensesList"));
    QVERIFY(list);

    const QVariantList entries = m_texts->entries();
    int want = -1;
    for (int i = 0; i < entries.size(); ++i)
        if (entries.at(i).toMap().value(QStringLiteral("rel")).toString()
            == QLatin1String("THIRD-PARTY-NOTICES.md"))
            want = i;
    QVERIFY2(want > 0, "the notices aggregate is not a staged entry, or is first");

    QVERIFY(QMetaObject::invokeMethod(m_dlg, "show",
                                      Q_ARG(QVariant, QStringLiteral("THIRD-PARTY-NOTICES.md")),
                                      Q_ARG(QVariant, QStringLiteral("THIRD-PARTY-NOTICES.md"))));
    QTRY_COMPARE(list->property("currentIndex").toInt(), want);
    QCOMPARE(m_dlg->property("currentRel").toString(),
             QStringLiteral("THIRD-PARTY-NOTICES.md"));
}

// CUSTOM main: the platform must be chosen and the Quick style set BEFORE
// QGuiApplication exists, which QTEST_MAIN is too late for. Copied from
// tst_about_render rather than re-derived (C-650).
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstLicensesDialog tc;
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_licenses_dialog.moc"
