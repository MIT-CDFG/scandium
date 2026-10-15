// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Q-D30 RENDERING PIN (operator queue, 2026-09-04 — "a rendering test
// for the Q-D30 dialog; none exists today"). Q-D30's defect was
// invisible to every existing instrument: AdjustmentParamDialog took its
// footer from Qt's `standardButtons`, so "OK"/"Cancel" rendered
// hard-English in every locale — and audit.py could not red on it
// because no emitted catalog id existed to check (a string that never
// ASKS the catalog is outside the audit's population; Q-D30's own
// table). The qd30-qd31 dialog-catalog lane rebuilt the footer on
// AppI18n.tr("ui.dialog.cancel") / tr("ui.dialog.apply").
//
// This pins the fix AT THE RENDERED SURFACE, in the PSEUDO locale
// deliberately: en's value for ui.dialog.cancel IS "Cancel", so an
// en-locale assertion cannot tell the catalog's word from Qt's
// hard-English one — pseudo diverges in every glyph, so a
// `standardButtons` regression reds on sight. The REAL checked-in
// dialog source is byte-copied into a synthetic AppnameShell module
// (the tst_review_dialog_probes arrangement — the checked-in file stays
// the single truth) and booted WITH ITS INPUT SET (openFor a served
// kind: Q-D30 was only findable after setting the dialog's input,
// because it renders empty cold).
//
// AppDoc is deliberately NOT wired: the dialog's only AppDoc reference
// sits inside the Apply click handler, which this test never invokes —
// rendering needs the catalog and the Theme singleton alone, and a load
// error would fail the boot loudly, not silently.

#include "i18n_catalog.h"

#include <QDirIterator>
#include <QFile>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

namespace {

QString copyFail; // set by setUpModuleDir on error

// Byte-copy the REAL dialog + its CurveEditor dependency + Theme into
// <tmp>/AppnameShell with a qmldir (the probes-harness arrangement).
// CurveEditor rides along because the dialog's curves branch names the
// type — the component must COMPILE whole even when a non-curves kind
// is opened.
bool setUpModuleDir(const QString &root)
{
    const QString mod = root + QStringLiteral("/AppnameShell");
    if (!QDir().mkpath(mod)) {
        copyFail = QStringLiteral("mkpath failed: ") + mod;
        return false;
    }
    const struct { const char *src; const char *dst; } files[] = {
        {"/qml/Theme.qml", "/Theme.qml"},
        {"/qml/dialogs/AdjustmentParamDialog.qml", "/AdjustmentParamDialog.qml"},
        {"/qml/dialogs/CurveEditor.qml", "/CurveEditor.qml"},
    };
    for (const auto &f : files) {
        const QString src = QStringLiteral(APPNAME_SHELL_DIR) + QLatin1String(f.src);
        const QString dst = mod + QLatin1String(f.dst);
        QFile::remove(dst);
        if (!QFile::copy(src, dst)) {
            copyFail = QStringLiteral("copy failed: ") + src;
            return false;
        }
    }
    QFile qmldir(mod + QStringLiteral("/qmldir"));
    if (!qmldir.open(QIODevice::WriteOnly)) {
        copyFail = QStringLiteral("qmldir open failed");
        return false;
    }
    qmldir.write("module AppnameShell\n"
                 "singleton Theme 1.0 Theme.qml\n"
                 "AdjustmentParamDialog 1.0 AdjustmentParamDialog.qml\n"
                 "CurveEditor 1.0 CurveEditor.qml\n");
    return true;
}

const char kScene[] =
    "import QtQuick\n"
    "import QtQuick.Controls\n"
    "import AppnameShell\n"
    "ApplicationWindow {\n"
    "    id: win\n"
    "    width: 900; height: 700; visible: true\n"
    "    property alias apd: apd\n"
    "    AdjustmentParamDialog { id: apd }\n"
    "}\n";

} // namespace

class TstQd30DialogRender : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void footerButtonsRenderTheCatalogInPseudo();
    void noShellQmlUsesStandardButtons();
    void theCurveEditorIsDRIVABLEByKeyboard();
    void theKeyboardCursorSTAYSInsideThePlot();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    CatalogQml *m_i18n = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QObject *m_apd = nullptr;
    QTemporaryDir m_dir;
};

void TstQd30DialogRender::initTestCase()
{
    QVERIFY(m_dir.isValid());
    QVERIFY2(setUpModuleDir(m_dir.path()), qPrintable(copyFail));

    // The REAL checked-in PSEUDO catalog (rule 8: catalog truth, not a
    // stand-in) — the locale where "Cancel" cannot pass for a
    // translation.
    QFile pseudo(QStringLiteral(APPNAME_REPO_I18N_DIR "/pseudo.json"));
    QVERIFY(pseudo.open(QIODevice::ReadOnly));
    m_catalog = Catalog::fromTexts(
        {{QStringLiteral("pseudo"), QString::fromUtf8(pseudo.readAll())}});
    m_i18n = new CatalogQml(m_catalog, this);

    m_engine = new QQmlApplicationEngine(this);
    m_engine->addImportPath(m_dir.path());
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                m_i18n);
    m_engine->loadData(QByteArray(kScene),
                       QUrl(QStringLiteral("qrc:/qd30/scene.qml")));
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "scene failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    m_apd = m_window->property("apd").value<QObject *>();
    QVERIFY2(m_apd, "AdjustmentParamDialog did not resolve from the scene");
}

void TstQd30DialogRender::footerButtonsRenderTheCatalogInPseudo()
{
    // Echo-proofing first (the tst_options_bar catalogGround shape): the
    // two ids must RESOLVE in pseudo — a key-echo here would make the
    // rendered-text asserts below self-confirming.
    const QString wantCancel = m_catalog.tr(QStringLiteral("ui.dialog.cancel"));
    const QString wantApply = m_catalog.tr(QStringLiteral("ui.dialog.apply"));
    QVERIFY2(wantCancel != QStringLiteral("ui.dialog.cancel")
                 && wantApply != QStringLiteral("ui.dialog.apply"),
             "pseudo catalog key-echoed the dialog button ids");
    // And the discriminator must actually discriminate: if pseudo's
    // values EQUALLED Qt's hard-English pair, this whole test would be
    // unable to tell the fix from the defect.
    QVERIFY2(wantCancel != QStringLiteral("Cancel")
                 && wantApply != QStringLiteral("OK"),
             "pseudo catalog values equal Qt's hard-English pair — the "
             "locale no longer discriminates and this pin is blind");

    // Open WITH INPUT SET — the served kind with the simplest rows.
    QVERIFY(QMetaObject::invokeMethod(
        m_apd, "openFor",
        Q_ARG(QVariant, QStringLiteral("adjustment.brightness_contrast"))));
    QTRY_VERIFY2(m_apd->property("visible").toBool(),
                 "dialog never became visible after openFor");

    // Collect every visible rendered button text inside the dialog.
    QStringList texts;
    const auto buttons = m_apd->findChildren<QQuickItem *>();
    for (QQuickItem *b : buttons) {
        if (!b->isVisible() || !b->inherits("QQuickAbstractButton"))
            continue;
        const QString t = b->property("text").toString();
        if (!t.isEmpty())
            texts << t;
    }
    QVERIFY2(texts.contains(wantCancel),
             qPrintable(QStringLiteral("no rendered button carries the pseudo "
                                       "ui.dialog.cancel value %1 — rendered: [%2]")
                            .arg(wantCancel, texts.join(QStringLiteral(" | ")))));
    QVERIFY2(texts.contains(wantApply),
             qPrintable(QStringLiteral("no rendered button carries the pseudo "
                                       "ui.dialog.apply value %1 — rendered: [%2]")
                            .arg(wantApply, texts.join(QStringLiteral(" | ")))));
    // The defect's own signature, asserted absent: Qt's hard-English
    // pair on a rendered surface in a non-en locale.
    QVERIFY2(!texts.contains(QStringLiteral("OK"))
                 && !texts.contains(QStringLiteral("Cancel")),
             qPrintable(QStringLiteral("Qt hard-English button text rendered "
                                       "in the pseudo locale: [%1]")
                            .arg(texts.join(QStringLiteral(" | ")))));
    QMetaObject::invokeMethod(m_apd, "close");
}

void TstQd30DialogRender::noShellQmlUsesStandardButtons()
{
    // The CLASS pin — Q-D30's own control column ("grep -l returns this
    // file alone") promoted from a hand-run to a standing check: ZERO
    // `standardButtons` users across shell/qml, so the next dialog
    // cannot re-import the defect. With a positive control, because a
    // detector that would match nothing on a real hit is the C-550
    // class: a bare zero is vacuity-capable.
    // The ASSIGNMENT form on a non-comment line — the fix's own comment
    // block names the word `standardButtons` three times (first run of
    // this pin went red on exactly that), and prose about the defect
    // must never count as the defect.
    const auto usesStandardButtons = [](const QString &src) -> bool {
        static const QRegularExpression use(
            QStringLiteral("\\bstandardButtons\\s*:"));
        for (const QString &line : src.split(QLatin1Char('\n')))
            if (!line.trimmed().startsWith(QLatin1String("//"))
                && line.contains(use))
                return true;
        return false;
    };
    QVERIFY2(usesStandardButtons(
                 QStringLiteral("Dialog {\n    standardButtons: Dialog.Ok\n}")),
             "positive control: the detector cannot see a real assignment");
    QVERIFY2(!usesStandardButtons(
                 QStringLiteral("// under `standardButtons` that required\n")),
             "negative control: the detector counts prose as the defect");

    QStringList offenders;
    int scanned = 0;
    QDirIterator it(QStringLiteral(APPNAME_SHELL_DIR "/qml"),
                    {QStringLiteral("*.qml")}, QDir::Files,
                    QDirIterator::Subdirectories);
    while (it.hasNext()) {
        const QString path = it.next();
        QFile f(path);
        QVERIFY2(f.open(QIODevice::ReadOnly), qPrintable(path));
        ++scanned;
        if (usesStandardButtons(QString::fromUtf8(f.readAll())))
            offenders << QDir(QStringLiteral(APPNAME_SHELL_DIR))
                             .relativeFilePath(path);
    }
    // Population guard: an empty scan is a moved directory, not a green.
    QVERIFY2(scanned >= 18,
             qPrintable(QStringLiteral("only %1 QML files scanned — the "
                                       "population moved and this pin went "
                                       "blind")
                            .arg(scanned)));
    QVERIFY2(offenders.isEmpty(),
             qPrintable(QStringLiteral("standardButtons users in shell/qml "
                                       "(the Q-D30 class): %1")
                            .arg(offenders.join(QStringLiteral(", ")))));
}

// ADDED C-753 — THE ONE BESPOKE CONTROL OF B-03's SIX, DRIVEN BY KEYBOARD.
// Every operation in CurveEditor was mouse-only: add on press, move on drag,
// remove on double-click. The a11y lint passes the file because the lint
// checks LABELS and this control carries a correct Accessible.role/name — so
// a screen reader announced it and then nothing responded. B-03's acceptance
// clause reads "keyboard-accessible controls"; this is that clause made
// executable. NOTHING drove `curveArea` before this, by mouse or key.
void TstQd30DialogRender::theCurveEditorIsDRIVABLEByKeyboard()
{
    // WRAPPED IN A KEY-COUNTING PARENT, so "an unhandled key is not
    // swallowed" can be a MEASUREMENT rather than a guess. Key events
    // propagate from the focused item UP the parent chain, so a key the
    // editor declines must arrive here — and one it handles must not.
    // Without this the only available assertion was "the curve did not
    // change", which a swallowed key satisfies just as well.
    QQmlComponent comp(m_engine);
    comp.setData("import QtQuick\n"
                 "import AppnameShell\n"
                 "Item {\n"
                 "    property int unhandled: 0\n"
                 "    Keys.onPressed: function (e) { unhandled++; }\n"
                 "    Item { objectName: \"sib\"; activeFocusOnTab: true; width: 10; height: 10 }\n"
                 "    CurveEditor { objectName: \"ed\"; width: 200; height: 200 }\n"
                 "}\n",
                 QUrl(QStringLiteral("qrc:/qd30/curve.qml")));
    QVERIFY2(!comp.isError(), qPrintable(comp.errorString()));
    QObject *wrap = comp.create(m_engine->rootContext());
    QVERIFY2(wrap, "wrapper failed to construct");
    auto *wrapItem = qobject_cast<QQuickItem *>(wrap);
    QVERIFY(wrapItem);
    wrapItem->setParentItem(m_window->contentItem());
    QObject *ed = wrap->findChild<QObject *>(QStringLiteral("ed"));
    QVERIFY2(ed, "CurveEditor not found in the wrapper");
    auto *item = qobject_cast<QQuickItem *>(ed);
    QVERIFY(item);
    // TAB MUST REACH IT, and this is the assertion `activeFocusOnTab` is
    // FOR. My first version called forceActiveFocus and asserted focus --
    // which works with or without that property, so the control for it came
    // back MUTE (C-751's shape, caught the same way). Focus the sibling and
    // Tab: declaration order puts the editor next, and a user who cannot
    // Tab to a control cannot use it however well it handles keys.
    auto *sib = wrap->findChild<QQuickItem *>(QStringLiteral("sib"));
    QVERIFY(sib);
    sib->forceActiveFocus();
    QCOMPARE(m_window->activeFocusItem(), sib);
    QTest::keyClick(m_window, Qt::Key_Tab);
    // A29: activeFocusItem names the ONE item a key reaches; hasActiveFocus
    // is true for ancestors and could not tell me this.
    QVERIFY2(m_window->activeFocusItem() == item,
             "Tab does not reach the curve editor, so a keyboard user cannot "
             "get to it at all");

    const QVariantList start = ed->property("points").toList();
    QCOMPARE(start.size(), 2);                       // identity: [0,0]-[1,1]

    // MOVE: Down lowers the selected point's output. Point 0 is at y=0 and
    // would CLAMP there -- the C-752 lesson, where I read a correct clamp as
    // a dead key -- so select point 1 (at y=1) first and lower THAT.
    QTest::keyClick(m_window, Qt::Key_Right);
    QTest::keyClick(m_window, Qt::Key_Down);
    QVariantList after = ed->property("points").toList();
    QCOMPARE(after.size(), 2);
    const double y1 = after.at(1).toList().at(1).toDouble();
    QVERIFY2(y1 < 1.0 && y1 > 0.9,
             qPrintable(QStringLiteral("Down did not lower the point by one step: y=")
                        + QString::number(y1)));

    // ADD: midway between the selected point and the next. Selection is on
    // the LAST point, so there is nothing to be midway to and it must REFUSE
    // rather than invent a placement -- the control agreeing with itself.
    QTest::keyClick(m_window, Qt::Key_Plus);
    QCOMPARE(ed->property("points").toList().size(), 2);
    QTest::keyClick(m_window, Qt::Key_Left);         // back to point 0
    QTest::keyClick(m_window, Qt::Key_Plus);
    QCOMPARE(ed->property("points").toList().size(), 3);

    // REMOVE, and then the FLOOR: minPoints is the engine's own limit
    // (curves.v1.json), so the third removal must refuse rather than send a
    // 1-point array to be rejected.
    QTest::keyClick(m_window, Qt::Key_Delete);
    QCOMPARE(ed->property("points").toList().size(), 2);
    QTest::keyClick(m_window, Qt::Key_Delete);
    QCOMPARE(ed->property("points").toList().size(), 2);

    // AND AN UNHANDLED KEY IS NOT SWALLOWED. The editor sits inside a
    // dialog whose Escape must still close it, so _onKey accepts ONLY the
    // keys it acts on. Measured at the PARENT: a declined key reaches it,
    // and a handled one does not.
    // DELTAS, not absolutes: the Tab above may or may not reach this
    // handler, and an assertion that depends on which is measuring the
    // focus system rather than the editor.
    const int base = wrap->property("unhandled").toInt();
    QTest::keyClick(m_window, Qt::Key_A);
    QCOMPARE(wrap->property("unhandled").toInt(), base + 1);   // declined -> parent
    QTest::keyClick(m_window, Qt::Key_Down);
    QCOMPARE(wrap->property("unhandled").toInt(), base + 1);   // handled -> stopped

    delete wrap;
}

// ADDED C-758 — GEOMETRY, WHICH IS THE RUNG EVERY OTHER TEST HERE MISSES.
// The keyboard cursor is a ring centred on the selected point, and
// _toView([0,0]) is the plot's bottom-left CORNER while _selIndex defaults
// to 0: centred, the cursor starts three quarters clipped. Found by LOOKING
// at the committed tour frame, not by any assertion — so the fix is made
// TESTABLE by pulling the placement into a pure function rather than left
// inside Canvas paint where only pixels could judge it.
void TstQd30DialogRender::theKeyboardCursorSTAYSInsideThePlot()
{
    QQmlComponent comp(m_engine);
    comp.setData("import AppnameShell\nCurveEditor { width: 200; height: 200 }\n",
                 QUrl(QStringLiteral("qrc:/qd30/ring.qml")));
    QVERIFY2(!comp.isError(), qPrintable(comp.errorString()));
    QObject *ed = comp.create(m_engine->rootContext());
    QVERIFY2(ed, "CurveEditor failed to construct");
    auto *item = qobject_cast<QQuickItem *>(ed);
    QVERIFY(item);
    item->setParentItem(m_window->contentItem());
    QTRY_VERIFY(item->width() > 0 && item->height() > 0);

    const double r = ed->property("_ringR").toDouble();
    QVERIFY2(r > 0, "_ringR is not exposed");

    auto centre = [&](double x, double y) {
        QVariant out;
        QMetaObject::invokeMethod(ed, "_ringCentre", Q_RETURN_ARG(QVariant, out),
                                  Q_ARG(QVariant, x), Q_ARG(QVariant, y));
        return out.toList();
    };

    // The plot fills the editor less a 1px border margin, so read its real
    // size rather than assuming 200 — assuming is how a geometry test starts
    // measuring its own arithmetic instead of the code.
    QQuickItem *plot = ed->findChild<QQuickItem *>(QStringLiteral("curvePlot"));
    QVERIFY2(plot, "curvePlot not found");
    const double w = plot->width(), h = plot->height();
    QVERIFY(w > 2 * r && h > 2 * r);

    // BOTTOM-LEFT: the identity curve's point 0, and the DEFAULT selection.
    const QVariantList bl = centre(0.0, h);
    QCOMPARE(bl.size(), 2);
    QVERIFY2(bl.at(0).toDouble() >= r && bl.at(1).toDouble() <= h - r,
             "the ring at the default selection is drawn outside the plot");

    // TOP-RIGHT: the other endpoint, clipped the other way.
    const QVariantList tr = centre(w, 0.0);
    QVERIFY2(tr.at(0).toDouble() <= w - r && tr.at(1).toDouble() >= r,
             "the ring at the far endpoint is drawn outside the plot");

    // THE HANDLE DOTS TAKE THE SAME TREATMENT AT THEIR OWN RADIUS (C-770).
    // A dot centred on the plot corner loses three quarters of itself to the
    // clip exactly as the ring did; insetting each shape by ITS OWN radius
    // is what keeps the ring and the dot it encircles concentric.
    QVariant hv;
    QMetaObject::invokeMethod(ed, "_insetCentre", Q_RETURN_ARG(QVariant, hv),
                              Q_ARG(QVariant, 0.0), Q_ARG(QVariant, h),
                              Q_ARG(QVariant, 3.0));
    const QVariantList hc = hv.toList();
    QCOMPARE(hc.size(), 2);
    QCOMPARE(hc.at(0).toDouble(), 3.0);
    QCOMPARE(hc.at(1).toDouble(), h - 3.0);
    // and the ring, at ITS radius, insets further — so they share a centre
    // only away from the edges, which is the intended geometry.
    QVERIFY2(hc.at(0).toDouble() < bl.at(0).toDouble(),
             "the handle inset is not smaller than the ring's");

    // ★ AND THE PAINT LOOP MUST ACTUALLY CALL IT. Pinning a helper's
    // arithmetic is NOT pinning its use: with only the arithmetic asserted,
    // deleting the call from the handle loop left this suite GREEN while the
    // dots went back to being clipped — measured, the control came back MUTE
    // on the first attempt. Canvas paint runs in JS and no runtime assertion
    // here can reach it, so the USE is pinned at the source, which is the
    // tst_about_credits technique applied one level in.
    {
        QFile f(QStringLiteral(APPNAME_SHELL_DIR "/qml/dialogs/CurveEditor.qml"));
        QVERIFY2(f.open(QIODevice::ReadOnly), "CurveEditor.qml unreadable");
        const QString src = QString::fromUtf8(f.readAll());
        const int loop = src.indexOf(QLatin1String("// handles"));
        QVERIFY2(loop > 0, "the handle-drawing loop is gone -- re-derive this pin");
        const int end = src.indexOf(QLatin1String("ctx.fill();"), loop);
        QVERIFY2(end > loop, "the handle loop no longer fills");
        QVERIFY2(src.mid(loop, end - loop).contains(QLatin1String("_insetCentre")),
                 "the handle loop no longer insets: endpoint dots will be "
                 "drawn centred on the plot corner and clipped");
    }

    // AND AN INTERIOR POINT IS NOT MOVED — the clamp must not become a
    // shrink that pulls every cursor toward the middle.
    //
    // ★ OFF-CENTRE ON PURPOSE, and this is the whole lesson of the arm.
    // I first probed at (w/2, h/2) and the control came back MUTE: a
    // mutation returning "always the plot centre" is INDISTINGUISHABLE from
    // "unchanged" at exactly that point. The control point had been chosen
    // to coincide with the degenerate case, so the assertion could not fail
    // in the direction it existed to guard.
    const QVariantList inner = centre(w * 0.25, h * 0.75);
    QCOMPARE(inner.at(0).toDouble(), w * 0.25);
    QCOMPARE(inner.at(1).toDouble(), h * 0.75);

    delete ed;
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstQd30DialogRender tc;
    return QTest::qExec(&tc, argc, argv);
}

#include "tst_qd30_dialog_render.moc"
