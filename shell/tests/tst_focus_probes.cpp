// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// A11Y-5 hook C: the A11Y-2 review's focus/modality probe shapes as
// durable, re-runnable evidence (they lived in a session-scoped
// scratchpad; the KS harness adopts these scenes as scripted evidence,
// per the junction brief item 3). Each scene runs offscreen on the
// Basic style — the app's own configuration — and reports through
// result properties; the runner asserts the recorded conclusions:
//   Q5: overlay children expose no `modal` (modality detection via
//       overlay items is categorically dead);
//   P5/Q7: a QQC2 Control is a focus scope (activeFocus propagates);
//   QD: an all-disabled pane's focus chain walks OUT (containment
//       checks are load-bearing in the pane cycle).

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QtTest>

namespace {

QObject *loadProbe(QQmlApplicationEngine &engine, const char *file)
{
    const QString path =
        QStringLiteral(APPNAME_PROBES_DIR "/") + QLatin1String(file);
    engine.load(QUrl::fromLocalFile(path));
    if (engine.rootObjects().isEmpty())
        return nullptr;
    return engine.rootObjects().last();
}

} // namespace

class TstFocusProbes : public QObject {
    Q_OBJECT
private slots:
    void overlayChildrenExposeNoModal();
    void controlIsAFocusScope();
    void allDisabledPaneChainWalksOut();
};

void TstFocusProbes::overlayChildrenExposeNoModal()
{
    QQmlApplicationEngine engine;
    QObject *win = loadProbe(engine, "OverlayModality.qml");
    QVERIFY2(win, "OverlayModality.qml failed to load");
    QTRY_VERIFY_WITH_TIMEOUT(win->property("done").toBool(), 5000);
    QVERIFY2(win->property("itemsSeen").toInt() > 0,
             "no overlay children while a modal dialog is open — the "
             "probe scene lost its subject");
    QVERIFY2(!win->property("modalPropertySeen").toBool(),
             "an overlay child exposed `modal` — the Q5 conclusion no "
             "longer holds; re-evaluate the pane-chord guard design");
}

void TstFocusProbes::controlIsAFocusScope()
{
    QQmlApplicationEngine engine;
    QObject *win = loadProbe(engine, "FocusScopePropagation.qml");
    QVERIFY2(win, "FocusScopePropagation.qml failed to load");
    QTRY_VERIFY_WITH_TIMEOUT(win->property("done").toBool(), 5000);
    QVERIFY2(win->property("barHasActiveFocus").toBool(),
             "focused child did NOT propagate activeFocus to the "
             "ToolBar — the P5/Q7 conclusion (and the pane cycle's "
             "detection) no longer holds");
}

void TstFocusProbes::allDisabledPaneChainWalksOut()
{
    QQmlApplicationEngine engine;
    QObject *win = loadProbe(engine, "DisabledChainWalkout.qml");
    QVERIFY2(win, "DisabledChainWalkout.qml failed to load");
    QTRY_VERIFY_WITH_TIMEOUT(win->property("done").toBool(), 5000);
    QVERIFY2(win->property("walkedOutside").toBool(),
             "the focus chain did NOT exit the all-disabled pane — the "
             "QD conclusion changed; the containment check may be "
             "removable (re-review, do not just delete)");
}

// Offscreen + the app's own Basic style, forced before the app exists.
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstFocusProbes tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_focus_probes.moc"
