// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// PRIORITY 3 — THE DISCARD GATE, DRIVEN THROUGH THE REAL DOOR.
//
// WHY THIS FILE EXISTS, AND WHAT WAS ACTUALLY MISSING. The data-loss
// protections SHIPPED: documentDirty/documentHasUnsavedWork are both
// Q_PROPERTY, confirmDiscard() gates three doors (File > New, File >
// Open, window close), and DiscardConfirmDialog.qml exists. The
// BEHAVIOUR was demonstrated correct by execution. What did not exist
// was an ARM, so a regression would ship silently:
//
//   tour.cpp:1034   "★ OPENED DIRECTLY rather than through
//                   confirmDiscard()" — proves the dialog RENDERS, and
//                   under Q-D6 that is the command-door equivalent.
//   tst_dirty_flag  asserts documentDirty()/windowTitle() — the MODEL.
//   census          DiscardConfirmDialog had ***ZERO*** references
//                   anywhere in shell/tests, of any kind (O-UI 9,
//                   measured with controls: NewDocumentDialog 15 files,
//                   ResizeDialog 3, CurveEditor 2 — the search fires).
//
// Frame 31 proves the dialog renders. It does not prove the GATE FIRES.
// Operator ruling 2026-09-04: THE TEST, not a tour frame — "a standing
// test beats a photograph".
//
// WHAT IS DRIVEN, AND IT IS THE REAL PATH END TO END:
//   real File > New MenuItem (triggered() on the actual item, the
//   tst_layers_fill_route/tst_auto_adjust_menu idiom)
//     -> NewDocumentDialog opens
//     -> accept() -> its onAccepted calls confirmGate(create)
//     -> Main.qml confirmDiscard(action)
//     -> DiscardConfirmDialog opens (ONLY when there is work to lose)
//     -> the dialog's REAL Cancel / Discard Buttons are clicked
//
// THE THREE CASES ARE EACH OTHER'S CONTROL, which is what keeps the
// gate assertion from being satisfiable by "nothing ever happened":
//   pristine  -> gate does NOT fire, document replaced immediately
//   cancel    -> gate fires, document SURVIVES
//   discard   -> gate fires, document REPLACED
// The same predicate (dialog visible) is required FALSE in the first
// and TRUE in the other two, against one harness and one instrument.
//
// HONEST LIMITS, RECORDED RATHER THAN PAPERED OVER:
// - Offscreen. This proves the gate fires and the buttons act; it does
//   not prove a compositor draws the modal over the canvas.
// - The document-survived assertion is an ENGINE readback (pixel +
//   rootLayerId), not a screenshot: the property under test is "the work
//   still exists", which is engine truth, not pixels on a screen.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QtTest>
#include <QTemporaryDir>

using namespace appname::shell;

class TstDiscardGate : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void init();

    void pristineDocumentBypassesTheGate();
    void cancelPreservesTheDocument();
    void discardReplacesTheDocument();
    void windowTitleNamesTheDocumentAndMarksUnsaved();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;

    // ---- engine truth ----------------------------------------------
    QColor pixel00()
    {
        QImage px;
        if (!m_controller->readRegionImage(QRect(0, 0, 1, 1), &px))
            return QColor();
        return px.pixelColor(0, 0);
    }
    bool pixelIsRed()
    {
        const QColor c = pixel00();
        return c.red() > 200 && c.blue() < 60 && c.green() < 60;
    }
    bool hasUnsavedWork() const
    {
        return m_controller->property("documentHasUnsavedWork").toBool();
    }
    // DOCUMENT IDENTITY. `canUndo()` was the first discriminator here and
    // it is WRONG — measured, as a red, on this suite's first run: a fresh
    // newDocument already reports canUndo TRUE, because `doc.new` is itself
    // a journal entry (the panel's row 0 reads "New Document"). So "the
    // document was replaced" cannot be read off the undo stack. The root
    // layer id is new for every document and identifies one directly.
    QString docIdentity() const
    {
        return m_controller->property("rootLayerId").toString();
    }
    void fillCanvasRed()
    {
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"), QVariantList{1.0, 0.0, 0.0, 0.0});
        QVariantMap params;
        params.insert(QStringLiteral("content"), QStringLiteral("color"));
        params.insert(QStringLiteral("color"), color);
        QVariantMap target;
        target.insert(QStringLiteral("layerId"),
                      m_controller->layers().first().toMap().value(
                          QStringLiteral("layerId")));
        QVERIFY2(m_controller
                     ->applyCommand(QStringLiteral("fill"), target, params,
                                    QString())
                     .value(QStringLiteral("ok"))
                     .toBool(),
                 "setup fill was refused");
    }

    // ---- finding the real controls ---------------------------------
    // The dialogs carry an `id` but no objectName, so they are reached
    // by CLASS NAME — the same traversal tour.cpp:1044-1048 already
    // uses to reach this very dialog. Recorded because it is the reason
    // findChild<>(objectName) is not used here.
    QObject *byClassName(const char *needle) const
    {
        const QList<QObject *> all = m_window->findChildren<QObject *>();
        for (QObject *o : all) {
            if (QString::fromLatin1(o->metaObject()->className())
                    .contains(QLatin1String(needle)))
                return o;
        }
        return nullptr;
    }
    QObject *discardDialog() const { return byClassName("DiscardConfirmDialog"); }
    QObject *newDocDialog() const { return byClassName("NewDocumentDialog"); }

    static bool isVisible(QObject *o)
    {
        return o && o->property("visible").toBool();
    }
    // Evaluate a JS expression against a real QML object — the
    // tst_auto_adjust_menu triggerMenuItem idiom, which drives the
    // object's OWN handler rather than a shell-side reimplementation.
    bool evalOn(QObject *target, const QString &js) const
    {
        QQmlExpression expr(qmlContext(target), target, js);
        expr.evaluate();
        if (expr.hasError()) {
            qWarning() << "eval failed on" << target << js
                       << expr.error().toString();
            return false;
        }
        return true;
    }
    // The File > New MenuItem, located by the catalog key its `text`
    // binding renders — NOT by menu index, which reorders.
    QObject *fileNewMenuItem() const
    {
        const QString want =
            m_i18n->catalog().tr(QStringLiteral("menu.file.new"));
        const QList<QObject *> all = m_window->findChildren<QObject *>();
        for (QObject *o : all) {
            if (!QString::fromLatin1(o->metaObject()->className())
                     .contains(QLatin1String("MenuItem")))
                continue;
            if (o->property("text").toString() == want)
                return o;
        }
        return nullptr;
    }
    // A Button inside the discard dialog, by its rendered catalog text.
    QQuickItem *discardDialogButton(const char *catalogKey) const
    {
        QObject *dlg = discardDialog();
        if (!dlg)
            return nullptr;
        const QString want = m_i18n->catalog().tr(QLatin1String(catalogKey));
        const QList<QQuickItem *> all = dlg->findChildren<QQuickItem *>();
        for (QQuickItem *it : all) {
            if (!QString::fromLatin1(it->metaObject()->className())
                     .contains(QLatin1String("Button")))
                continue;
            if (it->property("text").toString() == want)
                return it;
        }
        return nullptr;
    }
    // Drive the real Button. A synthesized mouse click is attempted
    // first; the modal lives in the window overlay, so if it is not
    // hit-testable offscreen the REAL clicked() signal is emitted on
    // the REAL button instead — which still runs that button's own
    // onClicked handler, never a copy of it.
    bool clickRealButton(QQuickItem *btn, const char *what)
    {
        if (!btn) {
            qWarning() << "button not found:" << what;
            return false;
        }
        if (btn->width() > 0 && btn->height() > 0) {
            const QPointF c = btn->mapToScene(
                QPointF(btn->width() / 2.0, btn->height() / 2.0));
            if (c.x() >= 0 && c.y() >= 0 && c.x() < m_window->width()
                && c.y() < m_window->height()) {
                QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                                  c.toPoint());
                if (!isVisible(discardDialog()))
                    return true; // the real event closed it: done
            }
        }
        return evalOn(btn, QStringLiteral("clicked()"));
    }
    // Dirty the document and assert the precondition the gate keys on.
    void makeDirtyDocument()
    {
        QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
        QVERIFY2(!hasUnsavedWork(),
                 "a pristine newDocument already reports unsaved work — the "
                 "gate's own discriminator is broken and every case below "
                 "would pass for the wrong reason");
        fillCanvasRed();
        QVERIFY2(pixelIsRed(), "setup readback is not the red state");
        QVERIFY2(hasUnsavedWork(), "one applied command left no unsaved work");
    }
    // File > New, accepted. Returns false if either real control is
    // missing, so a vanished menu row fails loudly instead of silently
    // skipping the drive.
    bool driveFileNewAndAccept()
    {
        QObject *item = fileNewMenuItem();
        if (!item) {
            qWarning() << "File > New MenuItem not found";
            return false;
        }
        if (!evalOn(item, QStringLiteral("triggered()")))
            return false;
        QObject *nd = newDocDialog();
        if (!nd) {
            qWarning() << "NewDocumentDialog not found";
            return false;
        }
        return evalOn(nd, QStringLiteral("accept()"));
    }
};

void TstDiscardGate::initTestCase()
{
    // Mirrors shell/src/main.cpp's wiring (the tst_palette_pins /
    // tst_history_panel harness), so the tree under test is the tree
    // the application actually has.
    m_i18n = new CatalogQml(Catalog::load(QStringLiteral("en")));
    m_controller = new DocumentController(&m_i18n->catalog());
    m_announcer = new Announcer(&m_i18n->catalog());
    m_controller->setAnnouncer(m_announcer);

    m_engine = new QQmlApplicationEngine;
    m_engine->addImageProvider(QStringLiteral("lucide"), new LucideIconProvider);
    m_engine->addImageProvider(QStringLiteral("layerthumb"),
                               new LayerThumbProvider(m_controller));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"), m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                m_controller);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppA11y"),
                                                m_announcer);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                new UnitsQml(m_engine));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppKeys"),
                                                new QObject(m_engine));
    m_engine->loadFromModule("AppnameShell", "Main");
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "Main.qml failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    m_announcer->setSubject(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));

    // The three real controls this suite drives must EXIST before any
    // case runs, or a later failure would read as a gate defect when it
    // is really a missing control (G-231: check the claim at the level
    // it is made at).
    QVERIFY2(fileNewMenuItem(), "File > New MenuItem not in the menu bar");
    QVERIFY2(newDocDialog(), "NewDocumentDialog not instantiated in Main.qml");
    QVERIFY2(discardDialog(), "DiscardConfirmDialog not instantiated in Main.qml");
}

void TstDiscardGate::init()
{
    // Every case starts from a closed dialog, asserted rather than
    // assumed: "the dialog became visible" means nothing if a previous
    // case left it open.
    QObject *dlg = discardDialog();
    QVERIFY(dlg);
    if (isVisible(dlg))
        evalOn(dlg, QStringLiteral("close()"));
    QTRY_VERIFY2(!isVisible(discardDialog()),
                 "a previous case left the discard dialog open");
}

void TstDiscardGate::pristineDocumentBypassesTheGate()
{
    // THE NEGATIVE ARM, and it pins a prediction that was WRONG when
    // this slice was planned: a pristine newDocument is documentDirty
    // but has NOTHING TO LOSE, so documentHasUnsavedWork is false and
    // the gate must NOT fire. Without this arm, "the dialog appeared"
    // could be true of every path and prove nothing about the gate.
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    QVERIFY2(!hasUnsavedWork(),
             "a pristine document reports unsaved work — the discriminator "
             "this whole gate keys on is broken");

    const QString before = docIdentity();
    QVERIFY2(!before.isEmpty(), "no root layer id to compare against");

    QVERIFY(driveFileNewAndAccept());
    QTest::qWait(80); // give an errant dialog time to appear
    QVERIFY2(!isVisible(discardDialog()),
             "the gate fired on a document with nothing to lose");
    // ...and the action still RAN: bypassing the gate must not mean
    // bypassing File > New. Without this the case would pass if the menu
    // item did nothing at all.
    QTRY_VERIFY2(docIdentity() != before,
                 "the bypass path did not actually create a new document");
}

void TstDiscardGate::cancelPreservesTheDocument()
{
    // THE ARM THAT MATTERS: File > New on a dirty document must STOP AND
    // ASK, and Cancel must leave the work intact.
    makeDirtyDocument();
    QVERIFY2(!isVisible(discardDialog()), "dialog visible before the drive");
    const QString before = docIdentity();

    QVERIFY(driveFileNewAndAccept());
    QTRY_VERIFY2(isVisible(discardDialog()),
                 "File > New on a dirty document did NOT stop and ask — this "
                 "is the property Q-D6 says frame 31 cannot prove");
    // The document must still be intact WHILE the modal is up.
    QVERIFY2(pixelIsRed(), "the document changed before the user answered");

    QVERIFY(clickRealButton(discardDialogButton("ui.dialog.cancel"), "Cancel"));
    QTRY_VERIFY2(!isVisible(discardDialog()), "Cancel did not close the dialog");

    // SURVIVED — engine truth, not shell state.
    QVERIFY2(pixelIsRed(), "Cancel discarded the document anyway");
    QVERIFY2(docIdentity() == before, "Cancel replaced the document anyway");
    QVERIFY2(m_controller->canUndo(), "Cancel threw away the undo stack");
    QVERIFY2(hasUnsavedWork(), "Cancel cleared the unsaved-work flag");
}

void TstDiscardGate::discardReplacesTheDocument()
{
    // THE OTHER HALF: the same gate, the same predicate, the opposite
    // answer — which is what proves the Cancel arm above is not simply
    // a path where nothing ever happens.
    makeDirtyDocument();
    const QString before = docIdentity();
    QVERIFY(driveFileNewAndAccept());
    QTRY_VERIFY2(isVisible(discardDialog()),
                 "the gate did not fire on a dirty document");

    QVERIFY(clickRealButton(
        discardDialogButton("ui.dialog.discard_confirm.discard"), "Discard"));
    QTRY_VERIFY2(!isVisible(discardDialog()), "Discard did not close the dialog");

    // REPLACED — by IDENTITY, the same predicate the Cancel arm requires
    // to be UNCHANGED. The two arms therefore disagree on one instrument.
    QTRY_VERIFY2(docIdentity() != before,
                 "Discard did not replace the document");
    QVERIFY2(!pixelIsRed(), "Discard left the old document's pixels");
}

// RC3 (operator WORD 36 (b), sitting S-20 note): the window title names the
// document ("<name> — Scandium") and carries the dirty dot while there is
// unsaved work. Read off the REAL window (Main.qml binds title to
// AppDoc.windowTitle), through edit -> save -> edit.
void TstDiscardGate::windowTitleNamesTheDocumentAndMarksUnsaved()
{
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    fillCanvasRed();
    QTRY_COMPARE(m_window->title(), QStringLiteral("\u25CF Untitled \u2014 Scandium"));
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    QVERIFY(m_controller->saveDocument(tmp.filePath(QStringLiteral("Harbour Sketch.appname"))));
    QTRY_COMPARE(m_window->title(), QStringLiteral("Harbour Sketch \u2014 Scandium"));
    fillCanvasRed();
    QTRY_COMPARE(m_window->title(), QStringLiteral("\u25CF Harbour Sketch \u2014 Scandium"));
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstDiscardGate tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}

#include "tst_discard_gate.moc"
