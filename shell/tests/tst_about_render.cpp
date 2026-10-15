// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09's acceptance is "About SHOWS all". `tst_about_credits` proves the
// SET by reading AboutDialog.qml as TEXT, and nineteen Main.qml harnesses
// prove it CONSTRUCTS — a construction error reds there, as one did when an
// unguarded AppBuildCommit threw ReferenceError in three suites. NOTHING
// proved it RENDERS: no test had ever opened it, so the Repeater actually
// producing labels, and their text actually resolving, were unverified.
//
// THE CATALOG IS THE MERGED ONE, NOT CANONICAL, because that is what SHIPS.
// Three of this dialog's ids live in the shell SEED (shell/i18n/ui.en.json)
// and reach the product only through merge_catalogs.py. Booting canonical
// alone would key-echo them and this test would be asserting against a
// catalog no user ever sees.
//
// ECHO-PROOFING RUNS FIRST (the Q-D30 shape). Catalog::tr echoes the key
// back on a miss, and an echo is NON-EMPTY AND TRUTHY (trap C10) — so a
// "the label has text" assertion is self-confirming unless the ids are
// shown to RESOLVE first. That check is the reason this test can mean
// anything at all.

#include "i18n_catalog.h"

#include <QFile>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QDir>
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
        // EVERY type AboutDialog.qml references must be listed here: this
        // module dir is a hand-kept mirror, and an unlisted type fails the
        // whole suite at initTestCase with "X is not a type" rather than at
        // the assertion that cares. That is the mirror working, not noise --
        // it is the only thing that notices a new dependency.
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
    "    width: 800; height: 700; visible: true\n"
    "}\n";

// THE DIALOG IS CREATED AFTER THE ENGINE IS FULLY SET UP, NOT INLINE IN THE
// SCENE. Measured: instantiated inline, every AppI18n binding evaluates
// during loadData and throws "Cannot call method 'tr' of null" — the plain
// Labels recover when the property resolves, but REPEATER DELEGATES created
// during that failed pass keep their broken binding and render EMPTY. The
// three credit rows silently vanished and the dialog still looked populated.
//
// tst_qd30_dialog_render survives the same nulls only because its rows are
// built on `openFor`, AFTER load. Copying its harness was right; copying its
// SCENE would have inherited a hazard it happens not to trip.
//
// The PRODUCT is unaffected — main.cpp sets AppI18n before engine.load() —
// so this is a test-construction fault, and pinning it here is what keeps
// the pin honest rather than accidentally green.

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

class TstAboutRender : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void everyRecordedCreditRendersAndResolves();
    void anUnsetVersionRendersNOTHING();
    void aSetVersionAndCommitDORender();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    CatalogQml *m_i18n = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QObject *m_dlg = nullptr;
    QTemporaryDir m_dir;
};

void TstAboutRender::initTestCase()
{
    QVERIFY(m_dir.isValid());
    QVERIFY2(setUpModuleDir(m_dir.path()), qPrintable(copyFail));

    // canonical + seed, merged — the catalog the product actually stages.
    QFile can(QStringLiteral(APPNAME_REPO_I18N_DIR "/en.json"));
    QVERIFY2(can.open(QIODevice::ReadOnly), "canonical en.json unreadable");
    QJsonObject merged = QJsonDocument::fromJson(can.readAll()).object();
    QFile seed(QStringLiteral(APPNAME_SHELL_DIR "/i18n/ui.en.json"));
    QVERIFY2(seed.open(QIODevice::ReadOnly), "shell seed unreadable");
    const QJsonObject s = QJsonDocument::fromJson(seed.readAll()).object();
    // Canonical WINS on collision, exactly as merge_catalogs.py does — the
    // seed can never shadow an O-I18N-owned id.
    for (auto it = s.begin(); it != s.end(); ++it)
        if (!merged.contains(it.key()))
            merged.insert(it.key(), it.value());
    m_catalog = Catalog::fromTexts(
        {{QStringLiteral("en"),
          QString::fromUtf8(QJsonDocument(merged).toJson())}});
    m_i18n = new CatalogQml(m_catalog, this);

    m_engine = new QQmlApplicationEngine(this);
    m_engine->addImportPath(m_dir.path());
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"), m_i18n);
    m_engine->loadData(QByteArray(kScene), QUrl(QStringLiteral("qrc:/about/scene.qml")));
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "scene failed to load");
    auto *win = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(win);
    QQmlComponent comp(m_engine);
    comp.setData("import AppnameShell\nAboutDialog {}\n",
                 QUrl(QStringLiteral("qrc:/about/dlg.qml")));
    QVERIFY2(!comp.isError(), qPrintable(comp.errorString()));
    m_dlg = comp.create(m_engine->rootContext());
    QVERIFY2(m_dlg, "AboutDialog failed to construct");
    m_dlg->setProperty("parent", QVariant::fromValue(win->contentItem()));
}

void TstAboutRender::everyRecordedCreditRendersAndResolves()
{
    const QStringList ids = {
        QStringLiteral("ui.dialog.about.credit.libmypaint"),
        QStringLiteral("ui.dialog.about.credit.lucide_icons"),
        QStringLiteral("ui.dialog.about.credit.lensfun_database"),
        QStringLiteral("ui.dialog.about.qt_lgpl"),
    };
    // ECHO-PROOF FIRST. Without this the render asserts below prove nothing:
    // a key-echo is non-empty and truthy, so "the label has text" passes on
    // a catalog that resolved NOTHING.
    for (const QString &id : ids)
        QVERIFY2(m_catalog.tr(id) != id,
                 qPrintable(QStringLiteral("merged catalog key-echoed ") + id));

    QVERIFY(QMetaObject::invokeMethod(m_dlg, "open"));
    QTRY_VERIFY2(m_dlg->property("visible").toBool(), "About never became visible");

    // VISUAL children, not findChildren (C-731). Repeater-created delegates
    // are given a VISUAL parent by the Repeater; their QObject parent is not
    // the ColumnLayout, so `findChildren` — which walks QObject parentage —
    // CANNOT SEE THEM. The nine inline Labels are QObject children by
    // construction, so the wrong instrument found exactly the rows that were
    // never in doubt and missed exactly the ones under test. Measured on the
    // real path: findChildren=0, childItems=3.
    QStringList rendered;
    collectVisibleText(m_dlg->property("contentItem").value<QQuickItem *>(), rendered);
    QVERIFY2(!rendered.isEmpty(), "no visible text rendered in the About dialog");

    for (const QString &id : ids) {
        const QString want = m_catalog.tr(id);
        QVERIFY2(rendered.contains(want),
                 qPrintable(QStringLiteral("obligation not rendered: ") + id));
    }

    // The PENDING record must not appear. Its id is minted and its record
    // says "vendor drop not landed yet"; rendering it would attest to a
    // component that is not in the product.
    const QString cg = m_catalog.tr(QStringLiteral("ui.dialog.about.credit.cgats21_profiles"));
    QVERIFY2(!rendered.contains(cg),
             "cgats21 (Pending record) is RENDERED in the About dialog");
}

void TstAboutRender::anUnsetVersionRendersNOTHING()
{
    // appVersion is empty until F-01/F-05 lands. The row must render NOTHING
    // rather than a plausible placeholder — three declared versions exist and
    // none is the release (C-708), so any default would be a confident
    // falsehood. This pins the honest-empty posture so a later "helpful"
    // default cannot land silently.
    QCOMPARE(m_dlg->property("appVersion").toString(), QString());
    const QString versionText =
        m_catalog.tr(QStringLiteral("ui.dialog.about.version"));
    QStringList shown;
    collectVisibleText(m_dlg->property("contentItem").value<QQuickItem *>(), shown);
    QVERIFY2(!shown.contains(versionText),
             "the version row rendered while appVersion is unset");
}

// CUSTOM main, copied from tst_qd30_dialog_render — the ONLY harness in
// this tree that boots a real dialog. QTEST_MAIN constructs the application
// itself, which is too late for BOTH of these: the platform must be chosen
// and the Quick style set BEFORE QGuiApplication exists. Copying a working
// harness beats re-deriving one; that is C-650's lesson pointed at test
// scaffolding rather than at a sweep.
void TstAboutRender::aSetVersionAndCommitDORender()
{
    // THE OTHER DIRECTION. `anUnsetVersionRendersNOTHING` alone is a
    // ONE-DIRECTIONAL GUARD: it passes when the row is honestly empty AND
    // when the row has been DELETED ENTIRELY — a dead row and a correct one
    // are indistinguishable to it. O-WATCH's §5c ("ask which direction the
    // guard runs", five one-directional guards found on main this week) is
    // the method; this is it applied to a test THIS DESK shipped one tick
    // earlier, which is where it was most likely to be believed unchecked.
    //
    // Without this arm, deleting both rows from AboutDialog.qml leaves the
    // suite GREEN while the About box silently loses its provenance —
    // exactly the class of silent loss C-706 and C-731 were each about.
    m_dlg->setProperty("appVersion", QStringLiteral("1.0.2"));
    m_dlg->setProperty("buildCommit", QStringLiteral("deadbee-dirty"));

    QStringList shown;
    collectVisibleText(m_dlg->property("contentItem").value<QQuickItem *>(), shown);

    const QString wantV = m_catalog.tr(QStringLiteral("ui.dialog.about.version"),
                                       {{QStringLiteral("version"), QStringLiteral("1.0.2")}});
    const QString wantC = m_catalog.tr(QStringLiteral("ui.dialog.about.commit"),
                                       {{QStringLiteral("commit"), QStringLiteral("deadbee-dirty")}});
    QVERIFY2(shown.contains(wantV),
             qPrintable(QStringLiteral("version row did not render when SET; wanted: ") + wantV));
    QVERIFY2(shown.contains(wantC),
             qPrintable(QStringLiteral("commit row did not render when SET; wanted: ") + wantC));

    // Leave the dialog as found — the version pin above must not depend on
    // slot order, and a mutated fixture is how a suite starts lying to the
    // next test in it (C-730: my own probe left a modal open for three runs).
    m_dlg->setProperty("appVersion", QString());
    m_dlg->setProperty("buildCommit", QString());
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstAboutRender tc;
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_about_render.moc"
