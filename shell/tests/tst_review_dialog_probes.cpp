// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// §5 ADVERSARIAL REVIEW PROBES (v1.0.1 items 2+3) — UNCOMMITTED reviewer
// harness. Drives the REAL NewDocumentDialog.qml / ResizeDialog.qml
// (byte-copied at runtime from shell/qml into a synthetic AppnameShell
// module dir so the checked-in sources stay the single truth) with REAL
// synthesized key/mouse events in an offscreen window, against a REAL
// engine document (cabi link). Probes:
//   P2  disabled-delegate keyboard skip (closed-combo arrows, popup
//       arrows+Return, type-ahead) while ppi is invalid;
//   P3  ppi-invalidation revert invariants (unit/value/truth coherence);
//   P5  SpinBox stale-text corners (dpi editor uncommitted text vs
//       computed px; preset pick over stale dpi text; Create submit);
//   P6  Create gate on COMPUTED px (physical value valid-by-validator
//       but out of px range; int-overflow wraparound);
//   P1  Resize preview outW/outH vs the ENGINE's accepted result
//       (doc.resize applied for real, engine truth read back);
//   P4  units-combo displayed text vs the checked-in en catalog rows;
//   PIN preset A4 end-to-end: real events -> Create -> engine doc dims.
//   SMD D3 follow-on (items 2+3 §5 review flag): SelectionModifyDialog's
//       shared amount SpinBox — the feather arm (the one non-integer op,
//       10x fixed-point display) must accept the '.' keystroke ("7.3"
//       typed must commit feather 7.3, never 73.0 — the ResizeDialog 10x
//       trap); integer arms pinned at contract range 1..=500. Engine
//       truth via the §31.3 selection-mask bounds readback against
//       direct-applyCommand references.

#include "document_controller.h"
#include "i18n_catalog.h"
#include "selection_scan.h"
#include "units.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QDir>
#include <QDateTime>
#include <QTextStream>
#include <QFile>
#include <QCoreApplication>
#include <QtTest>

using namespace appname::shell;

namespace {

QString copyFail; // set by setUpModuleDir on error

// Byte-copy the REAL dialog sources + Theme into <tmp>/AppnameShell with
// a qmldir, so the dialogs load exactly as checked in (Theme resolves as
// the module singleton, ResizeDialog's `import AppnameShell` resolves to
// this dir).
bool setUpModuleDir(const QString &root)
{
    const QString mod = root + QStringLiteral("/AppnameShell");
    if (!QDir().mkpath(mod)) {
        copyFail = QStringLiteral("mkpath failed: ") + mod;
        return false;
    }
    const struct { const char *src; const char *dst; } files[] = {
        {"/qml/Theme.qml", "/Theme.qml"},
        {"/qml/NewDocumentDialog.qml", "/NewDocumentDialog.qml"},
        {"/qml/dialogs/ResizeDialog.qml", "/ResizeDialog.qml"},
        {"/qml/dialogs/SelectionModifyDialog.qml", "/SelectionModifyDialog.qml"},
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
                 "NewDocumentDialog 1.0 NewDocumentDialog.qml\n"
                 "ResizeDialog 1.0 ResizeDialog.qml\n"
                 "SelectionModifyDialog 1.0 SelectionModifyDialog.qml\n");
    return true;
}

const char kScene[] =
    "import QtQuick\n"
    "import QtQuick.Controls\n"
    "import AppnameShell\n"
    "ApplicationWindow {\n"
    "    id: win\n"
    "    width: 900; height: 700; visible: true\n"
    "    property alias nd: nd\n"
    "    property alias rd: rd\n"
    "    property alias smd: smd\n"
    "    NewDocumentDialog { id: nd }\n"
    "    ResizeDialog { id: rd }\n"
    "    SelectionModifyDialog { id: smd }\n"
    "}\n";

} // namespace

class TstReviewDialogProbes : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanupTestCase();
    void cleanup();

    // P4 + pins
    void unitComboTextsMatchCatalog();
    void presetA4EndToEndEngineTruth();
    void presetOneWayCustomFlip();
    // P6
    void createGateOnComputedPx();
    void createGateIntOverflow();
    // P5
    void dpiStaleTextVsComputedAndSubmit();
    void presetPickOverStaleDpiText();
    // P2
    void resizeMouseCannotReachPhysicalWhileInvalid();
    void resizeClosedComboArrowBypass();
    void resizeTypeAheadBypass();
    void resizePopupKeysBypass();
    // P3
    void ppiInvalidationRevertInvariants();
    void ppiRevertAftermathOnNextValidEdit();
    // P1 extra + P5b (SpinBox decimal typing)
    void resizePhysicalDecimalTypingDropped();
    void resizePhysicalDrivenSubmit();
    // P1
    void resizePreviewMatchesEngine();
    void resizePreviewDegenerateDerivedZero();
    // SMD D3 follow-on (SelectionModifyDialog shared-SpinBox validator)
    void selModFeatherDecimalTypingTenXTrap();
    void selModIntegerArmDecimalRejected();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    CatalogQml *m_i18n = nullptr;
    UnitsQml *m_units = nullptr;
    DocumentController *m_controller = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QObject *m_nd = nullptr; // NewDocumentDialog (QQuickPopup, not an item)
    QObject *m_rd = nullptr; // ResizeDialog
    QObject *m_smd = nullptr; // SelectionModifyDialog
    QTemporaryDir m_dir;

    // Resolve a QML `id:` from the dialog's DOCUMENT context. The dialog
    // instance itself carries the outer (scene) context; the inner ids
    // live in the component-internal context, reachable via any object
    // created inside the dialog's own document — walk the children until
    // one resolves the name.
    QObject *idOf(QObject *dialog, const char *name)
    {
        QList<QObject *> candidates = dialog->findChildren<QObject *>();
        candidates.prepend(dialog);
        for (QObject *c : candidates) {
            QQmlContext *ctx = qmlContext(c);
            if (!ctx)
                continue;
            QQmlExpression e(ctx, c, QLatin1String(name));
            bool undefined = false;
            const QVariant v = e.evaluate(&undefined);
            if (e.hasError())
                continue;
            QObject *o = v.value<QObject *>();
            if (o)
                return o;
        }
        return nullptr;
    }
    QQuickItem *itemOf(QObject *dialog, const char *name)
    {
        return qobject_cast<QQuickItem *>(idOf(dialog, name));
    }
    QPoint centerInWindow(QQuickItem *item)
    {
        const QPointF c = item->mapToScene(
            QPointF(item->width() / 2.0, item->height() / 2.0));
        return c.toPoint();
    }
    void clickItem(QQuickItem *item)
    {
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          centerInWindow(item));
    }
    void openDialog(QObject *dialog)
    {
        QMetaObject::invokeMethod(dialog, "open");
        QTRY_VERIFY(dialog->property("opened").toBool());
    }
    void closeDialog(QObject *dialog)
    {
        QMetaObject::invokeMethod(dialog, "close");
        QTRY_VERIFY(!dialog->property("visible").toBool());
    }
    // Focus a TextField/TextInput-ish item and replace its text by REAL
    // key events (select-all + type).
    void retype(QQuickItem *editor, const QString &text)
    {
        editor->forceActiveFocus();
        QTRY_VERIFY(editor->hasActiveFocus());
        QTest::keyClick(m_window, Qt::Key_A, Qt::ControlModifier);
        if (text.isEmpty()) {
            QTest::keyClick(m_window, Qt::Key_Backspace);
            return;
        }
        for (const QChar &ch : text)
            QTest::keyClick(m_window, ch.toLatin1());
    }
    // The editable text item inside a SpinBox.
    QQuickItem *spinEditor(QQuickItem *spin)
    {
        return spin->property("contentItem").value<QQuickItem *>();
    }
    // Pick a combo entry index via REAL mouse events (open the popup,
    // click the delegate). Requires a valid selectable delegate.
    bool comboPickByMouse(QQuickItem *combo, int index);
    void freshDoc(int w, int h)
    {
        QVERIFY2(m_controller->newDocument(w, h, QStringLiteral("white")),
                 "engine newDocument failed");
        QTRY_COMPARE(m_controller->docWidth(), w);
        QTRY_COMPARE(m_controller->docHeight(), h);
    }
    void makeRectSelection(int x, int y, int w, int h)
    {
        QVariantMap rect;
        rect.insert(QStringLiteral("x"), x);
        rect.insert(QStringLiteral("y"), y);
        rect.insert(QStringLiteral("w"), w);
        rect.insert(QStringLiteral("h"), h);
        QVariantMap params;
        params.insert(QStringLiteral("source"), QStringLiteral("rect"));
        params.insert(QStringLiteral("rect"), rect);
        const QVariantMap r = m_controller->applyCommand(
            QStringLiteral("selection.set"), {}, params);
        QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
                 qPrintable(r.value(QStringLiteral("message")).toString()));
        QTRY_VERIFY(m_controller->selectionActive());
    }
    // Engine-truth selection mask bounds via the public 1.2 readback door
    // (the announceSelectionState strip walk, §31.3 — same chunking).
    QRect selectionMaskBounds()
    {
        const int w = m_controller->docWidth();
        const int h = m_controller->docHeight();
        if (w <= 0 || h <= 0)
            return QRect();
        const int maxRows = qMax(1, (1 << 26) / qMax(1, w));
        QRect bounds;
        for (int y = 0; y < h; y += maxRows) {
            const int rows = qMin(maxRows, h - y);
            QImage strip;
            if (!m_controller->readSelectionImage(QRect(0, y, w, rows),
                                                  &strip))
                return QRect();
            bounds = accumulateMaskBounds(strip, y, bounds);
        }
        return bounds;
    }
    // Fresh 600x600 doc + centered 40x40 rect selection, ONE modify op
    // applied directly (the scripting door) — the engine-truth reference
    // the dialog run must reproduce.
    QRect refBoundsAfterModify(const char *param, const QVariant &v)
    {
        freshDoc(600, 600);
        makeRectSelection(280, 280, 40, 40);
        QVariantMap params;
        params.insert(QLatin1String(param), v);
        const QVariantMap r = m_controller->applyCommand(
            QStringLiteral("selection.modify"), {}, params);
        if (!r.value(QStringLiteral("ok")).toBool()) {
            qWarning() << "reference selection.modify" << param << v
                       << "failed:" << r;
            return QRect();
        }
        return selectionMaskBounds();
    }
    QQuickItem *buttonOf(QObject *dialog, const char *catalogText)
    {
        for (QQuickItem *b : dialog->findChildren<QQuickItem *>()) {
            if (b->property("text").toString() == QLatin1String(catalogText))
                return b;
        }
        return nullptr;
    }

    friend int main(int, char **);
};

bool TstReviewDialogProbes::comboPickByMouse(QQuickItem *combo, int index)
{
    clickItem(combo); // open popup
    QObject *popup = combo->property("popup").value<QObject *>();
    if (!popup)
        return false;
    if (!QTest::qWaitFor([popup] { return popup->property("opened").toBool(); },
                         3000))
        return false;
    QQuickItem *list =
        popup->property("contentItem").value<QQuickItem *>();
    if (!list)
        return false;
    QQuickItem *delegate = nullptr;
    // The popup contentItem is a ListView; itemAtIndex is invokable.
    QMetaObject::invokeMethod(list, "itemAtIndex", Qt::DirectConnection,
                              Q_RETURN_ARG(QQuickItem *, delegate),
                              Q_ARG(int, index));
    if (!delegate)
        return false;
    clickItem(delegate);
    // C-395: VERIFY THE PICK, DO NOT ASSUME IT. This returned `true`
    // unconditionally after the click, so a silent no-op reported success --
    // and the caller's QCOMPARE(engineW, previewW) then compares two values
    // that are co-stale by construction (preview and engine both derive from
    // this combo), so they AGREE and the case passes having never exercised
    // the driver it names. Every other failure path in this function returns
    // false; the one step that actually selects verified nothing.
    //
    // The fixed 50 ms sleep is replaced rather than kept: waiting FOR THE
    // CONDITION is both the post-condition and the settle, and it cannot be
    // outrun by a slow close transition the way a constant can.
    return QTest::qWaitFor(
        [combo, index] {
            return combo->property("currentIndex").toInt() == index;
        },
        2000);
}

void TstReviewDialogProbes::initTestCase()
{
    QVERIFY2(m_dir.isValid(), "temp dir");
    QVERIFY2(setUpModuleDir(m_dir.path()), qPrintable(copyFail));

    // REAL checked-in en catalog (rule 8: catalog truth, not stand-ins).
    QFile en(QStringLiteral(APPNAME_REPO_I18N_DIR "/en.json"));
    QVERIFY(en.open(QIODevice::ReadOnly));
    m_catalog = Catalog::fromTexts(
        {{QStringLiteral("en"), QString::fromUtf8(en.readAll())}});
    m_i18n = new CatalogQml(m_catalog, this);
    m_units = new UnitsQml(this);
    m_controller = new DocumentController(&m_i18n->catalog());
    m_controller->setParent(this);

    m_engine = new QQmlApplicationEngine(this);
    m_engine->addImportPath(m_dir.path());
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                m_units);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                m_controller);
    m_engine->loadData(QByteArray(kScene),
                       QUrl(QStringLiteral("qrc:/review/scene.qml")));
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "scene failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
    m_nd = m_window->property("nd").value<QObject *>();
    m_rd = m_window->property("rd").value<QObject *>();
    m_smd = m_window->property("smd").value<QObject *>();
    QVERIFY(m_nd);
    QVERIFY(m_rd);
    QVERIFY(m_smd);
}

void TstReviewDialogProbes::cleanupTestCase()
{
    // The engine must die BEFORE the context-property objects (the same
    // ordering main.cpp encodes) — reversed, every binding TypeErrors at
    // teardown (observed first-hand on the first harness run).
    delete m_engine;
    m_engine = nullptr;
}

void TstReviewDialogProbes::cleanup()
{
    if (m_nd && m_nd->property("visible").toBool())
        closeDialog(m_nd);
    if (m_rd && m_rd->property("visible").toBool())
        closeDialog(m_rd);
    if (m_smd && m_smd->property("visible").toBool())
        closeDialog(m_smd);
    QTest::qWait(30);
}

// ---- P4: units combo displayed text vs the cataloged rows -----------------

void TstReviewDialogProbes::unitComboTextsMatchCatalog()
{
    openDialog(m_nd);
    QQuickItem *unitBox = itemOf(m_nd, "unitBox");
    QVERIFY(unitBox);
    const QVariantList model = unitBox->property("model").toList();
    QCOMPARE(model.size(), 4);
    // The en catalog rows read first-hand from i18n/en.json.
    const char *expected[] = {"px", "in", "cm", "mm"};
    for (int i = 0; i < 4; ++i)
        QCOMPARE(model.at(i).toString(), QLatin1String(expected[i]));
    QCOMPARE(unitBox->property("currentText").toString(),
             QLatin1String("px"));

    // Preset combo rows against the catalog (Custom + the six names).
    QQuickItem *presetBox = itemOf(m_nd, "presetBox");
    QVERIFY(presetBox);
    const QVariantList pModel = presetBox->property("model").toList();
    QCOMPARE(pModel.size(), 7);
    const char *pExpected[] = {"Custom", "A4", "A5", "Letter",
                               "HD 1080p", "4K UHD", "Social Square"};
    for (int i = 0; i < 7; ++i)
        QCOMPARE(pModel.at(i).toString(), QLatin1String(pExpected[i]));

    // Resize dialog's unit combo rides the same rows.
    closeDialog(m_nd);
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *rUnitBox = itemOf(m_rd, "unitBox");
    QVERIFY(rUnitBox);
    const QVariantList rModel = rUnitBox->property("model").toList();
    QCOMPARE(rModel.size(), 4);
    for (int i = 0; i < 4; ++i)
        QCOMPARE(rModel.at(i).toString(), QLatin1String(expected[i]));
}

// ---- PIN: preset A4 end-to-end, engine-accepted truth ----------------------

void TstReviewDialogProbes::presetA4EndToEndEngineTruth()
{
    openDialog(m_nd);
    QQuickItem *presetBox = itemOf(m_nd, "presetBox");
    QVERIFY(presetBox);
    QVERIFY2(comboPickByMouse(presetBox, 1), "could not pick preset A4");
    QCOMPARE(presetBox->property("currentIndex").toInt(), 1);
    QCOMPARE(m_nd->property("unitKey").toString(), QLatin1String("mm"));
    QQuickItem *widthField = itemOf(m_nd, "widthField");
    QQuickItem *heightField = itemOf(m_nd, "heightField");
    QQuickItem *dpiBox = itemOf(m_nd, "dpiBox");
    QVERIFY(widthField && heightField && dpiBox);
    QCOMPARE(widthField->property("text").toString(), QLatin1String("210"));
    QCOMPARE(heightField->property("text").toString(), QLatin1String("297"));
    QCOMPARE(dpiBox->property("value").toInt(), 300);
    // The one displayed truth (mechanically re-derived: 210/25.4*300 =
    // 2480.31 -> 2480; 297/25.4*300 = 3507.87 -> 3508).
    QCOMPARE(m_nd->property("widthPx").toInt(), 2480);
    QCOMPARE(m_nd->property("heightPx").toInt(), 3508);
    QVERIFY(m_nd->property("dimsOk").toBool());

    // Create by REAL mouse click; the ENGINE-ACCEPTED envelope is the
    // cross-check (doc dims read back from the engine).
    QQuickItem *footer = m_nd->property("footer").value<QQuickItem *>();
    QVERIFY(footer);
    QQuickItem *createBtn = nullptr;
    const auto buttons = footer->findChildren<QQuickItem *>();
    for (QQuickItem *b : buttons) {
        if (b->property("text").toString() == QLatin1String("Create")) {
            createBtn = b;
            break;
        }
    }
    QVERIFY2(createBtn, "Create button not found by catalog text");
    QVERIFY(createBtn->property("enabled").toBool());
    clickItem(createBtn);
    QTRY_COMPARE(m_controller->docWidth(), 2480);
    QTRY_COMPARE(m_controller->docHeight(), 3508);
    QTRY_VERIFY(!m_nd->property("visible").toBool());
}

// ---- one-way Custom flip ----------------------------------------------------

void TstReviewDialogProbes::presetOneWayCustomFlip()
{
    openDialog(m_nd);
    QQuickItem *presetBox = itemOf(m_nd, "presetBox");
    QQuickItem *widthField = itemOf(m_nd, "widthField");
    QQuickItem *unitBox = itemOf(m_nd, "unitBox");
    QVERIFY(presetBox && widthField && unitBox);
    QVERIFY2(comboPickByMouse(presetBox, 2), "could not pick preset A5");
    QCOMPARE(presetBox->property("currentIndex").toInt(), 2);
    // Programmatic-set side: picking the preset itself must NOT flip
    // back to Custom (textEdited never fires programmatically).
    QCOMPARE(presetBox->property("currentIndex").toInt(), 2);
    // A unit switch converts dims programmatically — still no flip.
    QVERIFY2(comboPickByMouse(unitBox, 1), "could not pick inches");
    QCOMPARE(m_nd->property("unitKey").toString(), QLatin1String("in"));
    QCOMPARE(presetBox->property("currentIndex").toInt(), 2);
    const int pxBefore = m_nd->property("widthPx").toInt();
    QCOMPARE(pxBefore, 1748); // A5 px meaning preserved across the switch
    // A REAL user keystroke in a dimension field flips to Custom.
    widthField->forceActiveFocus();
    QTRY_VERIFY(widthField->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Home); // extend the INTEGER part —
    // typing at end would exceed decimals:3 and be validator-rejected
    QTest::keyClick(m_window, Qt::Key_5);
    QCOMPARE(presetBox->property("currentIndex").toInt(), 0);
}

// ---- P6: Create gate rides COMPUTED px, not the raw validator --------------

void TstReviewDialogProbes::createGateOnComputedPx()
{
    openDialog(m_nd);
    QQuickItem *unitBox = itemOf(m_nd, "unitBox");
    QQuickItem *widthField = itemOf(m_nd, "widthField");
    QQuickItem *dpiBox = itemOf(m_nd, "dpiBox");
    QVERIFY(unitBox && widthField && dpiBox);
    QVERIFY2(comboPickByMouse(unitBox, 1), "could not pick inches");
    QCOMPARE(m_nd->property("unitKey").toString(), QLatin1String("in"));
    QCOMPARE(dpiBox->property("value").toInt(), 300);

    // 4000 in @300 = 1_200_000 px: the FIELD text is validator-valid
    // (DoubleValidator, no top) but the computed px is out of range.
    retype(widthField, QStringLiteral("4000"));
    QCOMPARE(widthField->property("text").toString(),
             QLatin1String("4000"));
    QVERIFY(widthField->property("acceptableInput").toBool());
    QCOMPARE(m_nd->property("widthPx").toInt(), 1200000);
    QVERIFY2(!m_nd->property("dimsOk").toBool(),
             "gate accepted a validator-valid physical value whose px "
             "conversion is out of the door range");

    // 3333 in @300 = 999_900 px: back in range, gate opens.
    retype(widthField, QStringLiteral("3333"));
    QCOMPARE(m_nd->property("widthPx").toInt(), 999900);
    QVERIFY(m_nd->property("dimsOk").toBool());
}

void TstReviewDialogProbes::createGateIntOverflow()
{
    openDialog(m_nd);
    QQuickItem *unitBox = itemOf(m_nd, "unitBox");
    QQuickItem *widthField = itemOf(m_nd, "widthField");
    QVERIFY(unitBox && widthField);
    QVERIFY2(comboPickByMouse(unitBox, 1), "could not pick inches");
    // 14316558.09 in @300dpi = 4294967427 px = 2^32 + 131 — if the int
    // property wraps (ToInt32), widthPx reads 131 and the gate OPENS for
    // an absurd request. Record what actually happens.
    retype(widthField, QStringLiteral("14316558.09"));
    QVERIFY(widthField->property("acceptableInput").toBool());
    const int wrapped = m_nd->property("widthPx").toInt();
    const bool gateOpen = m_nd->property("dimsOk").toBool();
    qWarning() << "OVERFLOW PROBE: widthPx =" << wrapped
               << "dimsOk =" << gateOpen;
    QVERIFY2(!gateOpen, "px-range gate OPEN on a 2^32-wrapped conversion");
}

// ---- P5: SpinBox stale-text corners ----------------------------------------

void TstReviewDialogProbes::dpiStaleTextVsComputedAndSubmit()
{
    openDialog(m_nd);
    QQuickItem *presetBox = itemOf(m_nd, "presetBox");
    QQuickItem *dpiBox = itemOf(m_nd, "dpiBox");
    QVERIFY(presetBox && dpiBox);
    QVERIFY2(comboPickByMouse(presetBox, 1), "could not pick preset A4");
    QCOMPARE(m_nd->property("widthPx").toInt(), 2480);

    // Type 600 into the dpi editor, do NOT commit.
    QQuickItem *editor = spinEditor(dpiBox);
    QVERIFY(editor);
    retype(editor, QStringLiteral("600"));
    QCOMPARE(editor->property("text").toString(), QLatin1String("600"));
    const int valueUncommitted = dpiBox->property("value").toInt();
    const int shownPxUncommitted = m_nd->property("widthPx").toInt();
    qWarning() << "STALE DPI PROBE: editor=600 value=" << valueUncommitted
               << "computed widthPx=" << shownPxUncommitted;

    // Now click Create (REAL mouse) and read the ENGINE result.
    QQuickItem *footer = m_nd->property("footer").value<QQuickItem *>();
    QVERIFY(footer);
    QQuickItem *createBtn = nullptr;
    for (QQuickItem *b : footer->findChildren<QQuickItem *>()) {
        if (b->property("text").toString() == QLatin1String("Create")) {
            createBtn = b;
            break;
        }
    }
    QVERIFY(createBtn);
    clickItem(createBtn);
    QTRY_VERIFY(!m_nd->property("visible").toBool());
    const int engineW = m_controller->docWidth();
    const int engineH = m_controller->docHeight();
    qWarning() << "STALE DPI PROBE: engine doc =" << engineW << "x" << engineH
               << "(300dpi A4 = 2480x3508; 600dpi A4 = 4961x7016)";
    // Coherence bar: whatever committed, the submit must equal the
    // dialog's own px truth at accept time — i.e. one of the two honest
    // conversions, never a torn pair.
    const bool coherent = (engineW == 2480 && engineH == 3508)
        || (engineW == 4961 && engineH == 7016);
    QVERIFY2(coherent, "torn submit: neither the 300dpi nor the 600dpi pair");
}

void TstReviewDialogProbes::presetPickOverStaleDpiText()
{
    openDialog(m_nd);
    QQuickItem *presetBox = itemOf(m_nd, "presetBox");
    QQuickItem *dpiBox = itemOf(m_nd, "dpiBox");
    QVERIFY(presetBox && dpiBox);
    // dpi value is already 300 (default). Type 600 uncommitted, then
    // pick A4 (which sets dpiBox.value = 300 — a no-op assignment).
    QQuickItem *editor = spinEditor(dpiBox);
    QVERIFY(editor);
    retype(editor, QStringLiteral("600"));
    QVERIFY2(comboPickByMouse(presetBox, 1), "could not pick preset A4");
    const QString editorText = editor->property("text").toString();
    const int value = dpiBox->property("value").toInt();
    const int shownPx = m_nd->property("widthPx").toInt();
    qWarning() << "PRESET-OVER-STALE-DPI: editor=" << editorText
               << "value=" << value << "widthPx=" << shownPx;
    // The dialog's own coherence claim: after picking A4 the computed
    // row must show the A4 pin, and the dpi the row rides must be the
    // dpi the field DISPLAYS. A display/value tear is the finding.
    QCOMPARE(shownPx, 2480);
    QCOMPARE(editorText, QStringLiteral("300"));
}

// ---- P2: reachability of "unselectable" physical units while ppi invalid ---

void TstReviewDialogProbes::resizeMouseCannotReachPhysicalWhileInvalid()
{
    // Positive control: the shipped guard (disabled delegates) does stop
    // the MOUSE path.
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QVERIFY(unitBox && ppi);
    QCOMPARE(ppi->property("text").toString(), QString());
    QVERIFY(!m_rd->property("ppiOk").toBool());
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    comboPickByMouse(unitBox, 2); // click the DISABLED cm delegate
    if (unitBox->property("popup").value<QObject *>()->property("visible").toBool())
        QTest::keyClick(m_window, Qt::Key_Escape);
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0);
}

void TstReviewDialogProbes::resizeClosedComboArrowBypass()
{
    // The disposition under review: "physical units are UNSELECTABLE
    // (delegates disabled) while ppi is empty/invalid — the broken state
    // is unreachable". Closed-popup arrow navigation must therefore NOT
    // reach a physical unit.
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QVERIFY(unitBox && pixels);
    QVERIFY(!m_rd->property("ppiOk").toBool());
    QCOMPARE(pixels->property("value").toInt(), 640);
    unitBox->forceActiveFocus();
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Down);
    QTest::qWait(20);
    const QString unit = m_rd->property("unitKey").toString();
    qWarning() << "P2(a) closed-combo Key_Down: unitKey=" << unit
               << "currentIndex=" << unitBox->property("currentIndex").toInt()
               << "pixels.value=" << pixels->property("value").toInt()
               << "displayText=" << pixels->property("displayText").toString()
               << "drivenPx=" << m_rd->property("drivenPx").toInt()
               << "outW=" << m_rd->property("outW").toInt()
               << "outH=" << m_rd->property("outH").toInt();
    // Belt check: Apply must be dead in this state (it is gated on
    // unitKey==px || ppiOk).
    for (QQuickItem *b : m_rd->findChildren<QQuickItem *>()) {
        if (b->property("text").toString() == QLatin1String("Apply")) {
            qWarning() << "P2(a) Apply enabled in bypass state:"
                       << b->property("enabled").toBool();
            break;
        }
    }
    QCOMPARE(unit, QLatin1String("px"));
}

void TstReviewDialogProbes::resizeTypeAheadBypass()
{
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QVERIFY(unitBox);
    QVERIFY(!m_rd->property("ppiOk").toBool());
    unitBox->forceActiveFocus();
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_M); // type-ahead: "mm"
    QTest::qWait(20);
    const QString unit = m_rd->property("unitKey").toString();
    qWarning() << "P2(b) type-ahead 'm': unitKey=" << unit;
    QCOMPARE(unit, QLatin1String("px"));
}

void TstReviewDialogProbes::resizePopupKeysBypass()
{
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QVERIFY(unitBox);
    QVERIFY(!m_rd->property("ppiOk").toBool());
    unitBox->forceActiveFocus();
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Space); // open popup
    QObject *popup = unitBox->property("popup").value<QObject *>();
    QVERIFY(popup);
    QVERIFY(QTest::qWaitFor(
        [popup] { return popup->property("opened").toBool(); }, 2000));
    QTest::keyClick(m_window, Qt::Key_Down);
    QTest::keyClick(m_window, Qt::Key_Down);
    QTest::keyClick(m_window, Qt::Key_Return);
    QTest::qWait(30);
    const QString unit = m_rd->property("unitKey").toString();
    qWarning() << "P2(c) popup Down,Down,Return: unitKey=" << unit;
    if (popup->property("visible").toBool())
        QTest::keyClick(m_window, Qt::Key_Escape);
    QCOMPARE(unit, QLatin1String("px"));
}

// ---- P3: ppi-invalidation revert invariants ---------------------------------

void TstReviewDialogProbes::ppiInvalidationRevertInvariants()
{
    freshDoc(1000, 800);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QVERIFY(unitBox && ppi && pixels);
    QCOMPARE(pixels->property("value").toInt(), 1000); // docWidth seed

    // Valid ppi 300 by real typing.
    retype(ppi, QStringLiteral("300"));
    QVERIFY(m_rd->property("ppiOk").toBool());
    QCOMPARE(m_rd->property("lastGoodPpi").toDouble(), 300.0);

    // Select cm (index 2) by real mouse — now selectable.
    QVERIFY2(comboPickByMouse(unitBox, 2), "cm not selectable at valid ppi");
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("cm"));
    // 1000 px @300 = 8.4667 cm -> thousandths 8467; drivenPx back = 1000.
    QCOMPARE(pixels->property("value").toInt(), 8467);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 1000);
    QCOMPARE(pixels->property("displayText").toString(),
             QLatin1String("8.467"));

    // Invalidate ppi by real keystrokes: select-all + Backspace (one
    // text change 300 -> "").
    retype(ppi, QString());
    QCOMPARE(ppi->property("text").toString(), QString());
    QVERIFY(!m_rd->property("ppiOk").toBool());
    qWarning() << "P3 after 300->'': unitKey="
               << m_rd->property("unitKey").toString()
               << "currentIndex=" << unitBox->property("currentIndex").toInt()
               << "pixels.value=" << pixels->property("value").toInt()
               << "displayText=" << pixels->property("displayText").toString()
               << "lastGoodPpi=" << m_rd->property("lastGoodPpi").toDouble()
               << "drivenPx=" << m_rd->property("drivenPx").toInt();
    // The commit's disposition: "if ppi is invalidated WHILE a physical
    // unit is live, the combo reverts to px carrying the value across at
    // the last-good ppi".
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0);
    QCOMPARE(pixels->property("value").toInt(), 1000);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 1000);
}

void TstReviewDialogProbes::ppiRevertAftermathOnNextValidEdit()
{
    // What ACTUALLY happens one edit later: after 300 -> "" (no revert,
    // lastGoodPpi poisoned by Number("") == 0), the user types a fresh
    // VALID ppi digit — record the state the dialog lands in.
    freshDoc(1000, 800);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QVERIFY(unitBox && ppi && pixels);
    retype(ppi, QStringLiteral("300"));
    QVERIFY(m_rd->property("ppiOk").toBool());
    QVERIFY2(comboPickByMouse(unitBox, 2), "cm not selectable at valid ppi");
    QCOMPARE(pixels->property("value").toInt(), 8467);
    retype(ppi, QString()); // 300 -> ""
    const double poisoned = m_rd->property("lastGoodPpi").toDouble();
    // User fixes the field: type '5' (a VALID ppi).
    ppi->forceActiveFocus();
    QTRY_VERIFY(ppi->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_5);
    QVERIFY(m_rd->property("ppiOk").toBool()); // "5" is valid
    qWarning() << "P3-aftermath: lastGoodPpi-after-empty=" << poisoned
               << "| after typing valid '5': unitKey="
               << m_rd->property("unitKey").toString()
               << "pixels.value=" << pixels->property("value").toInt()
               << "displayText=" << pixels->property("displayText").toString()
               << "drivenPx=" << m_rd->property("drivenPx").toInt()
               << "lastGoodPpi=" << m_rd->property("lastGoodPpi").toDouble();
    // The disposition's own bar: "the shown quantity never changes
    // meaning". The quantity was 8.467 cm (= 1000 px @300).
    // Coherence check: whatever unit is now shown, the driven quantity
    // must still be 8.467 cm-equivalent at SOME honestly-applied ppi —
    // a collapse to the 1px floor is meaning destruction.
    QVERIFY2(pixels->property("value").toInt() != 1,
             "driving value collapsed to the 1px floor after the user "
             "typed a fresh VALID ppi — the 8.467 cm quantity was "
             "destroyed by the poisoned lastGoodPpi revert");
}

void TstReviewDialogProbes::resizePhysicalDecimalTypingDropped()
{
    // The commit claims the driving SpinBox rides "the
    // SelectionModifyDialog pattern at DoubleValidator decimals:3
    // precision". The SpinBox never sets `validator:` — QQuickSpinBox's
    // DEFAULT IntValidator stays live, so the '.' keystroke should be
    // REJECTED even in physical mode. Pin the claim: typing "7.3" into
    // the cm-mode editor must yield editor text "7.3".
    freshDoc(1000, 800);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QQuickItem *driver = itemOf(m_rd, "driver");
    QVERIFY(unitBox && ppi && pixels && driver);
    QVERIFY2(comboPickByMouse(driver, 0), "driver width pick failed");
    retype(ppi, QStringLiteral("300"));
    QVERIFY(m_rd->property("ppiOk").toBool());
    QVERIFY2(comboPickByMouse(unitBox, 2), "cm not selectable");
    QQuickItem *editor = spinEditor(pixels);
    QVERIFY(editor);
    QObject *validator =
        pixels->property("validator").value<QObject *>();
    qWarning() << "P5b: SpinBox validator class ="
               << (validator ? validator->metaObject()->className()
                             : "null");
    retype(editor, QStringLiteral("7.3"));
    const QString typed = editor->property("text").toString();
    qWarning() << "P5b: after typing '7.3' the editor holds" << typed;
    QTest::keyClick(m_window, Qt::Key_Return);
    qWarning() << "P5b: committed value (thousandths) ="
               << pixels->property("value").toInt() << "drivenPx ="
               << m_rd->property("drivenPx").toInt();
    QCOMPARE(typed, QStringLiteral("7.3"));
}

void TstReviewDialogProbes::resizePhysicalDrivenSubmit()
{
    // Item 1 full loop on the PHYSICAL arm with an integer-typed value
    // (decimals are untypeable — see resizePhysicalDecimalTypingDropped):
    // 9 cm @300 = 1062.99 -> 1063 px; derived h = round(1063*800/1000)
    // = 850.4 -> 850. Engine must agree exactly with the preview.
    freshDoc(1000, 800);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QQuickItem *driver = itemOf(m_rd, "driver");
    QVERIFY(unitBox && ppi && pixels && driver);
    QVERIFY2(comboPickByMouse(driver, 0), "driver width pick failed");
    retype(ppi, QStringLiteral("300"));
    QVERIFY(m_rd->property("ppiOk").toBool());
    QVERIFY2(comboPickByMouse(unitBox, 2), "cm not selectable");
    retype(spinEditor(pixels), QStringLiteral("9"));
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(pixels->property("value").toInt(), 9000);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 1063);
    const int previewW = m_rd->property("outW").toInt();
    const int previewH = m_rd->property("outH").toInt();
    QQuickItem *applyBtn = nullptr;
    for (QQuickItem *b : m_rd->findChildren<QQuickItem *>()) {
        if (b->property("text").toString() == QLatin1String("Apply")) {
            applyBtn = b;
            break;
        }
    }
    QVERIFY(applyBtn);
    QVERIFY(applyBtn->property("enabled").toBool());
    clickItem(applyBtn);
    QTRY_VERIFY2(!m_rd->property("visible").toBool(),
                 qPrintable(m_rd->property("errorText").toString()));
    qWarning() << "P1-physical: preview" << previewW << "x" << previewH
               << "engine" << m_controller->docWidth() << "x"
               << m_controller->docHeight();
    QCOMPARE(m_controller->docWidth(), previewW);
    QCOMPARE(m_controller->docHeight(), previewH);
    QCOMPARE(m_controller->docWidth(), 1063);
    QCOMPARE(m_controller->docHeight(), 850);
}

// ---- P1: preview vs engine-accepted result ---------------------------------

void TstReviewDialogProbes::resizePreviewMatchesEngine()
{
    struct Case {
        int docW, docH;
        int driverIdx; // 0 width, 1 height, 2 longestEdge
        const char *value; // typed into the spin editor (px arm)
    };
    // Aspect ratios chosen to land derived sides on awkward fractions
    // (incl. an exact .5 half-case: 1234x777 driving width 3:
    // 3*777/1234 = 1.888...; 800x600 width 3 -> 2.25; 999x1000 longest
    // edge 501 -> width 500.499 -> round 500; half-case 640x400 width
    // 5 -> 3.125; true half: 800x600 driving width 1 -> 0.75; use
    // 1000x500 width 3 -> 1.5 EXACT half).
    const Case cases[] = {
        {1234, 777, 0, "1187"},
        {1000, 500, 0, "3"},     // derived 1.5 — exact half-case
        {999, 1000, 2, "501"},   // longest = height; width derives
        {800, 800, 2, "333"},    // square tie: engine picks width
        {640, 400, 1, "273"},    // height drives; width derives 436.8
    };
    for (const Case &c : cases) {
        freshDoc(c.docW, c.docH);
        openDialog(m_rd);
        QQuickItem *driver = itemOf(m_rd, "driver");
        QQuickItem *pixels = itemOf(m_rd, "pixels");
        QVERIFY(driver && pixels);
        if (c.driverIdx != 0)
            QVERIFY2(comboPickByMouse(driver, c.driverIdx),
                     "driver pick failed");
        retype(spinEditor(pixels), QLatin1String(c.value));
        QTest::keyClick(m_window, Qt::Key_Return); // commit the editor
        QCOMPARE(m_rd->property("drivenPx").toInt(), QString(c.value).toInt());
        const int previewW = m_rd->property("outW").toInt();
        const int previewH = m_rd->property("outH").toInt();

        // Apply by REAL mouse click; engine truth read back.
        QQuickItem *applyBtn = nullptr;
        for (QQuickItem *b : m_rd->findChildren<QQuickItem *>()) {
            if (b->property("text").toString() == QLatin1String("Apply")) {
                applyBtn = b;
                break;
            }
        }
        QVERIFY2(applyBtn, "Apply button not found by catalog text");
        QVERIFY(applyBtn->property("enabled").toBool());
        clickItem(applyBtn);
        QTRY_VERIFY2(!m_rd->property("visible").toBool(),
                     qPrintable(QStringLiteral("Apply did not close: ")
                                + m_rd->property("errorText").toString()));
        const int engineW = m_controller->docWidth();
        const int engineH = m_controller->docHeight();
        qWarning() << "P1:" << c.docW << "x" << c.docH << "driver"
                   << c.driverIdx << "value" << c.value << "preview"
                   << previewW << "x" << previewH << "engine" << engineW
                   << "x" << engineH;

        // C-412: QUARANTINE EVIDENCE, CAPTURED ONLY ON DISAGREEMENT.
        // The classification this test is quarantined for is HARNESS RACE vs
        // PRODUCT NONDETERMINISM, and the discriminator is narrow:
        //
        //   previewW was read BEFORE Apply. Re-read outW HERE, after the commit.
        //     changed  -> the preview had not settled when we sampled it: HARNESS.
        //     same     -> the preview was stable and the engine disagreed: PRODUCT.
        //
        // comboPickByMouse now verifies currentIndex reached the requested index
        // (C-395), so a silent no-op can no longer masquerade as a pass; the
        // driver index is recorded anyway because it is the other way the harness
        // can be wrong while looking right.
        //
        // Dumped to a PER-TASK path -- never a shared filename. Two runs writing
        // one name is a superposition, and this desk has produced that twice.
        if (engineW != previewW || engineH != previewH) {
            const int outNow = m_rd->property("outW").toInt();
            const int outHNow = m_rd->property("outH").toInt();
            const int driverNow = driver ? driver->property("currentIndex").toInt() : -1;
            QString dir = qEnvironmentVariable("APPNAME_QUARANTINE_DIR");
            if (dir.isEmpty())
                dir = QDir::tempPath();
            const QString path = QStringLiteral("%1/quarantine-resizePreview-%2-%3-%4.txt")
                                     .arg(dir)
                                     .arg(QCoreApplication::applicationPid())
                                     .arg(QDateTime::currentMSecsSinceEpoch())
                                     .arg(&c - cases);
            QFile f(path);
            if (f.open(QIODevice::WriteOnly | QIODevice::Text)) {
                QTextStream o(&f);
                o << "QUARANTINE resizePreviewMatchesEngine\n"
                  << "case            doc=" << c.docW << "x" << c.docH
                  << " driverIdx=" << c.driverIdx << " typed=" << c.value << "\n"
                  << "drivenPx        " << m_rd->property("drivenPx").toInt()
                  << "  (matched the typed value, or we would not be here)\n"
                  << "preview BEFORE  " << previewW << "x" << previewH << "\n"
                  << "preview NOW     " << outNow << "x" << outHNow << "\n"
                  << "engine          " << engineW << "x" << engineH << "\n"
                  << "driver index    " << driverNow
                  << "  (expected " << c.driverIdx << ")\n"
                  << "DISCRIMINATOR   "
                  << ((outNow == previewW && outHNow == previewH)
                          ? "preview STABLE -> engine disagreed -> PRODUCT"
                          : "preview MOVED after sampling -> HARNESS race")
                  << "\n";
                f.close();
                qWarning() << "QUARANTINE-EVIDENCE" << path;
            } else {
                qWarning() << "QUARANTINE-EVIDENCE-FAILED could not write" << path;
            }
        }

        QCOMPARE(engineW, previewW);
        QCOMPARE(engineH, previewH);
    }
}

void TstReviewDialogProbes::resizePreviewDegenerateDerivedZero()
{
    // 1000x1 doc, drive width to 100: preview lacks the engine's
    // .max(1.0) clamp — record the divergence first-hand.
    freshDoc(1000, 1);
    openDialog(m_rd);
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QQuickItem *driver = itemOf(m_rd, "driver");
    QVERIFY(pixels && driver);
    // The driver combo persists across opens (pre-existing semantics);
    // reset to width explicitly.
    QVERIFY2(comboPickByMouse(driver, 0), "driver width pick failed");
    retype(spinEditor(pixels), QStringLiteral("100"));
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 100);
    const int previewH = m_rd->property("outH").toInt();
    QQuickItem *applyBtn = nullptr;
    for (QQuickItem *b : m_rd->findChildren<QQuickItem *>()) {
        if (b->property("text").toString() == QLatin1String("Apply")) {
            applyBtn = b;
            break;
        }
    }
    QVERIFY(applyBtn);
    clickItem(applyBtn);
    QTRY_VERIFY(!m_rd->property("visible").toBool());
    qWarning() << "P1-degenerate: preview outH =" << previewH
               << "engine height =" << m_controller->docHeight();
    QCOMPARE(m_controller->docWidth(), 100);
    QCOMPARE(m_controller->docHeight(), 1); // engine clamps to 1
    // Record (not assert) the preview divergence: previewH is 0 here if
    // the dialog formula lacks the clamp.
}

// ---- SMD: SelectionModifyDialog shared-SpinBox validator (D3 class) --------

void TstReviewDialogProbes::selModFeatherDecimalTypingTenXTrap()
{
    // The items-2+3 §5 review flag: the shared amount SpinBox sets no
    // explicit `validator:`, so QQuickSpinBox's DEFAULT IntValidator
    // stays live on the FEATHER arm (the one non-integer op, 10x
    // fixed-point display). The '.' keystroke is then silently eaten —
    // "7.3" types as "73" and commits feather 73.0, the ResizeDialog D3
    // 10x trap. Engine-truth discriminator: post-commit selection-mask
    // bounds vs two direct-applyCommand references.
    const QRect honest = refBoundsAfterModify("feather", 7.3);
    QVERIFY2(!honest.isNull(), "feather 7.3 reference unreadable");
    const QRect trap = refBoundsAfterModify("feather", 73.0);
    QVERIFY2(!trap.isNull(), "feather 73.0 reference unreadable");
    // Positive control (the D1 lesson: prove the discriminator can
    // discriminate, or the equality below is vacuous).
    QVERIFY2(honest != trap,
             "feather 7.3 and 73.0 landed identical mask bounds — the "
             "bounds discriminator is vacuous");

    freshDoc(600, 600);
    makeRectSelection(280, 280, 40, 40);
    openDialog(m_smd);
    QQuickItem *opBox = itemOf(m_smd, "opBox");
    QQuickItem *amount = itemOf(m_smd, "amount");
    QVERIFY(opBox && amount);
    QVERIFY2(comboPickByMouse(opBox, 0), "feather pick failed");
    QVERIFY(!amount->property("integerArm").toBool());
    QQuickItem *editor = spinEditor(amount);
    QVERIFY(editor);
    QObject *validator = amount->property("validator").value<QObject *>();
    qWarning() << "SMD-feather: validator class ="
               << (validator ? validator->metaObject()->className()
                             : "null");
    retype(editor, QStringLiteral("7.3"));
    const QString typed = editor->property("text").toString();
    QTest::keyClick(m_window, Qt::Key_Return);
    const int committedRaw = amount->property("value").toInt();
    const QString shown = amount->property("displayText").toString();
    qWarning() << "SMD-feather: typed-text =" << typed
               << "committed raw(10x) =" << committedRaw
               << "displayText =" << shown;
    // Apply by REAL mouse click; the engine's mask is the truth.
    QQuickItem *applyBtn = buttonOf(m_smd, "Apply");
    QVERIFY2(applyBtn, "Apply button not found by catalog text");
    QVERIFY(applyBtn->property("enabled").toBool());
    clickItem(applyBtn);
    QTRY_VERIFY2(!m_smd->property("visible").toBool(),
                 qPrintable(m_smd->property("errorText").toString()));
    const QRect committed = selectionMaskBounds();
    qWarning() << "SMD-feather: engine bounds =" << committed
               << "honest(7.3) =" << honest << "trap(73.0) =" << trap;
    QCOMPARE(typed, QStringLiteral("7.3"));
    QCOMPARE(committedRaw, 73); // 10x fixed point: 7.3 px rides as 73
    QCOMPARE(shown, QStringLiteral("7.3"));
    QVERIFY2(committed != trap,
             "10x signature: typed 7.3, engine got feather 73.0");
    QCOMPARE(committed, honest);
}

void TstReviewDialogProbes::selModIntegerArmDecimalRejected()
{
    // The four integer arms (expand/contract/smooth/border) are integer
    // 1..=500 BY CONTRACT — '.' is not part of any valid value and is
    // refused at the keystroke (IntValidator, contract range). Observed
    // first-hand (Qt 6.11): digits that overshoot top sit in the editor
    // as INTERMEDIATE text ("501" is typeable) — QIntValidator only
    // hard-refuses non-integer characters — so the range pin lives at
    // the COMMIT boundary: Return clamps to the contract top. The full
    // loop runs expand 12 against direct engine truth.
    const QRect ref = refBoundsAfterModify("expand", 12);
    QVERIFY2(!ref.isNull(), "expand 12 reference unreadable");

    freshDoc(600, 600);
    makeRectSelection(280, 280, 40, 40);
    openDialog(m_smd);
    QQuickItem *opBox = itemOf(m_smd, "opBox");
    QQuickItem *amount = itemOf(m_smd, "amount");
    QVERIFY(opBox && amount);
    QVERIFY2(comboPickByMouse(opBox, 1), "expand pick failed");
    QVERIFY(amount->property("integerArm").toBool());
    QQuickItem *editor = spinEditor(amount);
    QVERIFY(editor);
    QObject *validator = amount->property("validator").value<QObject *>();
    qWarning() << "SMD-integer: validator class ="
               << (validator ? validator->metaObject()->className()
                             : "null")
               << "bottom=" << (validator ? validator->property("bottom")
                                          : QVariant())
               << "top=" << (validator ? validator->property("top")
                                       : QVariant());
    QVERIFY(validator);
    QCOMPARE(validator->property("bottom").toInt(), 1);   // contract min
    QCOMPARE(validator->property("top").toInt(), 500);    // contract max
    // '.' rejected at the keystroke: the digits keep integer meaning.
    retype(editor, QStringLiteral("7.3"));
    qWarning() << "SMD-integer: typed '7.3' ->"
               << editor->property("text").toString();
    QCOMPARE(editor->property("text").toString(), QStringLiteral("73"));
    // Overshooting top is Intermediate while typing; the COMMIT clamps
    // to the contract range.
    retype(editor, QStringLiteral("501"));
    QVERIFY(!editor->property("acceptableInput").toBool());
    QTest::keyClick(m_window, Qt::Key_Return);
    qWarning() << "SMD-integer: typed '501', Return -> value ="
               << amount->property("value").toInt() << "displayText ="
               << amount->property("displayText").toString();
    QCOMPARE(amount->property("value").toInt(), 500);
    // Full loop at 12 — engine agrees with the scripting-door reference.
    retype(editor, QStringLiteral("12"));
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(amount->property("value").toInt(), 12);
    QQuickItem *applyBtn = buttonOf(m_smd, "Apply");
    QVERIFY2(applyBtn, "Apply button not found by catalog text");
    QVERIFY(applyBtn->property("enabled").toBool());
    clickItem(applyBtn);
    QTRY_VERIFY2(!m_smd->property("visible").toBool(),
                 qPrintable(m_smd->property("errorText").toString()));
    const QRect committed = selectionMaskBounds();
    qWarning() << "SMD-integer: engine bounds =" << committed
               << "reference(expand 12) =" << ref;
    QCOMPARE(committed, ref);
}

// Offscreen + the app's own Basic style (the tst_focus_probes pattern).
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstReviewDialogProbes tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_review_dialog_probes.moc"
