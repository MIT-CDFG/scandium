// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// DELTA RE-REVIEW PROBES (v1.0.1 items 2+3 fix round @ 2355e94) —
// reviewer harness, lane-adopted; INDEPENDENT of the committed
// tst_review_dialog_probes.cpp. Each slot probes a fixed finding
// through a path the committed harness does NOT exercise:
//   D1v  popup OPEN + type-ahead mid-popup while ppi invalid; closed
//        Key_End jump (last row "mm"); asserts unitKey AND currentIndex
//        (no display tear), popup escapable.
//   D2v1 invalidate ppi via select-all+CUT (clipboard path, not
//        Backspace) — full revert invariants + lastGoodPpi NOT poisoned.
//   D2v2 invalidate ppi via replacing with "0" (the round-1 report's
//        "'0' poisons identically" case) — same invariants.
//   D3v  PASTE "7.3" (Ctrl+V, whole-string validator path, not
//        per-keystroke) into the cm-mode spin editor — text survives,
//        commit = 7300 thousandths, drivenPx 862 (7.3*300/2.54).
//   D4v  the exact 2^32+131 conversion on RESIZE's drivenPx side
//        (2.54 cm @ ppi 4294967427 -> 4294967427 px): drivenPx must
//        read 2000000 (capped), never 131 (wrapped); Apply must reject
//        LOUDLY (errorText set, dialog stays open, doc untouched).
//   D5v  height-driven degenerate (1x1000 doc, height -> 100): preview
//        outW must ASSERT 1 (the committed slot only records) and the
//        engine must agree end-to-end.

#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"

#include <QClipboard>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
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
    if (!QDir().mkpath(mod)) {
        copyFail = QStringLiteral("mkpath failed: ") + mod;
        return false;
    }
    const struct { const char *src; const char *dst; } files[] = {
        {"/qml/Theme.qml", "/Theme.qml"},
        {"/qml/NewDocumentDialog.qml", "/NewDocumentDialog.qml"},
        {"/qml/dialogs/ResizeDialog.qml", "/ResizeDialog.qml"},
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
                 "ResizeDialog 1.0 ResizeDialog.qml\n");
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
    "    NewDocumentDialog { id: nd }\n"
    "    ResizeDialog { id: rd }\n"
    "}\n";

} // namespace

class TstDeltaReviewProbes : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanupTestCase();
    void cleanup();

    void d1PositiveControlPathsLive();
    void d1PopupTypeAheadMidPopupAndEndKey();
    void d2InvalidateViaCutNotBackspace();
    void d2InvalidateViaZeroText();
    void d3PasteDecimalString();
    void d4ResizeExactWrapValueOnDrivenPx();
    void d5HeightDrivenDegenerateAsserted();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    CatalogQml *m_i18n = nullptr;
    UnitsQml *m_units = nullptr;
    DocumentController *m_controller = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QObject *m_rd = nullptr;
    QTemporaryDir m_dir;

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
    QQuickItem *spinEditor(QQuickItem *spin)
    {
        return spin->property("contentItem").value<QQuickItem *>();
    }
    bool comboPickByMouse(QQuickItem *combo, int index);
    void freshDoc(int w, int h)
    {
        QVERIFY2(m_controller->newDocument(w, h, QStringLiteral("white")),
                 "engine newDocument failed");
        QTRY_COMPARE(m_controller->docWidth(), w);
        QTRY_COMPARE(m_controller->docHeight(), h);
    }
    QQuickItem *buttonByText(QObject *dialog, const char *text)
    {
        for (QQuickItem *b :
             qobject_cast<QObject *>(dialog)->findChildren<QQuickItem *>()) {
            if (b->property("text").toString() == QLatin1String(text))
                return b;
        }
        return nullptr;
    }
    // Establish the standard D2/D3 pre-state: 1000x800 doc, ppi 300
    // typed for real, cm picked by real mouse (value 8467 thousandths,
    // drivenPx 1000).
    void armCmAt300(QQuickItem **unitBoxOut, QQuickItem **ppiOut,
                    QQuickItem **pixelsOut)
    {
        freshDoc(1000, 800);
        openDialog(m_rd);
        QQuickItem *unitBox = itemOf(m_rd, "unitBox");
        QQuickItem *ppi = itemOf(m_rd, "ppi");
        QQuickItem *pixels = itemOf(m_rd, "pixels");
        QVERIFY(unitBox && ppi && pixels);
        retype(ppi, QStringLiteral("300"));
        QVERIFY(m_rd->property("ppiOk").toBool());
        QCOMPARE(m_rd->property("lastGoodPpi").toDouble(), 300.0);
        QVERIFY2(comboPickByMouse(unitBox, 2), "cm not selectable at 300");
        QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("cm"));
        QCOMPARE(pixels->property("value").toInt(), 8467);
        QCOMPARE(m_rd->property("drivenPx").toInt(), 1000);
        *unitBoxOut = unitBox;
        *ppiOut = ppi;
        *pixelsOut = pixels;
    }

    friend int main(int, char **);
};

bool TstDeltaReviewProbes::comboPickByMouse(QQuickItem *combo, int index)
{
    clickItem(combo);
    QObject *popup = combo->property("popup").value<QObject *>();
    if (!popup)
        return false;
    if (!QTest::qWaitFor([popup] { return popup->property("opened").toBool(); },
                         3000))
        return false;
    QQuickItem *list = popup->property("contentItem").value<QQuickItem *>();
    if (!list)
        return false;
    QQuickItem *delegate = nullptr;
    QMetaObject::invokeMethod(list, "itemAtIndex", Qt::DirectConnection,
                              Q_RETURN_ARG(QQuickItem *, delegate),
                              Q_ARG(int, index));
    if (!delegate)
        return false;
    clickItem(delegate);
    // C-397: the identical defect to tst_review_dialog_probes' copy, and its caller
    // at :212 says "cm not selectable at 300" -- a failure message for a condition
    // this helper never checked. Every other path here returns false; the one step
    // that selects returned true unconditionally after a fixed sleep.
    return QTest::qWaitFor(
        [combo, index] {
            return combo->property("currentIndex").toInt() == index;
        },
        2000);
}

void TstDeltaReviewProbes::initTestCase()
{
    QVERIFY2(m_dir.isValid(), "temp dir");
    QVERIFY2(setUpModuleDir(m_dir.path()), qPrintable(copyFail));

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
                       QUrl(QStringLiteral("qrc:/delta/scene.qml")));
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "scene failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
    m_rd = m_window->property("rd").value<QObject *>();
    QVERIFY(m_rd);
}

void TstDeltaReviewProbes::cleanupTestCase()
{
    delete m_engine;
    m_engine = nullptr;
}

void TstDeltaReviewProbes::cleanup()
{
    if (m_rd && m_rd->property("visible").toBool())
        closeDialog(m_rd);
    QTest::qWait(30);
}

// ---- D1v positive control: the probed key paths are LIVE at valid ppi ------

void TstDeltaReviewProbes::d1PositiveControlPathsLive()
{
    // Vacuity guard for the negative D1 probe below: with ppi VALID the
    // exact same key paths (popup-open type-ahead + Return; closed
    // Key_End) MUST reach the physical units — proving the events land
    // and type-ahead/End are real input paths on this platform.
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QVERIFY(unitBox && ppi);
    retype(ppi, QStringLiteral("300"));
    QVERIFY(m_rd->property("ppiOk").toBool());

    unitBox->forceActiveFocus();
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Space); // open popup
    QObject *popup = unitBox->property("popup").value<QObject *>();
    QVERIFY(popup);
    QVERIFY(QTest::qWaitFor(
        [popup] { return popup->property("opened").toBool(); }, 2000));
    QTest::keyClick(m_window, Qt::Key_C); // type-ahead "cm" mid-popup
    QTest::qWait(20);
    QTest::keyClick(m_window, Qt::Key_Return);
    QTest::qWait(30);
    qWarning() << "D1v-pos popup 'c'+Return at valid ppi: unitKey="
               << m_rd->property("unitKey").toString();
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("cm"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 2);

    // Closed-combo Key_End at valid ppi reaches the last row "mm".
    closeDialog(m_rd);
    openDialog(m_rd); // onOpened resets unitKey to px; ppi text persists
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QVERIFY(m_rd->property("ppiOk").toBool());
    unitBox->forceActiveFocus();
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_End);
    QTest::qWait(20);
    qWarning() << "D1v-pos closed Key_End at valid ppi: unitKey="
               << m_rd->property("unitKey").toString();
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("mm"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 3);
}

// ---- D1v: popup-open type-ahead mid-popup + closed Key_End ------------------

void TstDeltaReviewProbes::d1PopupTypeAheadMidPopupAndEndKey()
{
    freshDoc(640, 400);
    openDialog(m_rd);
    QQuickItem *unitBox = itemOf(m_rd, "unitBox");
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QQuickItem *ppi = itemOf(m_rd, "ppi");
    QVERIFY(unitBox && pixels && ppi);
    // ppi text persists across open/close (only unitKey resets in
    // onOpened) — clear it by real keystrokes so this slot is
    // order-independent of the positive control above.
    if (!ppi->property("text").toString().isEmpty())
        retype(ppi, QString());
    QVERIFY(!m_rd->property("ppiOk").toBool());
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(pixels->property("value").toInt(), 640);

    // (a) Popup OPEN, then type-ahead 'c' ("cm") and 'i' ("in") WHILE
    // the popup is up, then Return — a path the committed harness never
    // drives (it type-aheads only on the CLOSED combo).
    unitBox->forceActiveFocus();
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Space); // open popup
    QObject *popup = unitBox->property("popup").value<QObject *>();
    QVERIFY(popup);
    QVERIFY(QTest::qWaitFor(
        [popup] { return popup->property("opened").toBool(); }, 2000));
    QTest::keyClick(m_window, Qt::Key_C); // type-ahead "cm" mid-popup
    QTest::qWait(20);
    QTest::keyClick(m_window, Qt::Key_Return);
    QTest::qWait(30);
    qWarning() << "D1v(a) popup type-ahead 'c'+Return: unitKey="
               << m_rd->property("unitKey").toString() << "currentIndex="
               << unitBox->property("currentIndex").toInt();
    if (popup->property("visible").toBool())
        QTest::keyClick(m_window, Qt::Key_Escape);
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0); // no display tear

    QTest::keyClick(m_window, Qt::Key_Space);
    QVERIFY(QTest::qWaitFor(
        [popup] { return popup->property("opened").toBool(); }, 2000));
    QTest::keyClick(m_window, Qt::Key_I); // "in" mid-popup
    QTest::qWait(20);
    QTest::keyClick(m_window, Qt::Key_Return);
    QTest::qWait(30);
    if (popup->property("visible").toBool())
        QTest::keyClick(m_window, Qt::Key_Escape);
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0);

    // (b) Closed-combo Key_End (jumps to the LAST row "mm" in one step —
    // not an arrow, not type-ahead).
    QTRY_VERIFY(unitBox->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_End);
    QTest::qWait(20);
    qWarning() << "D1v(b) closed Key_End: unitKey="
               << m_rd->property("unitKey").toString() << "currentIndex="
               << unitBox->property("currentIndex").toInt();
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0);

    // Driving value never disturbed by any rejected path.
    QCOMPARE(pixels->property("value").toInt(), 640);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 640);
}

// ---- D2v1: invalidate via select-all + CUT ----------------------------------

void TstDeltaReviewProbes::d2InvalidateViaCutNotBackspace()
{
    QQuickItem *unitBox = nullptr, *ppi = nullptr, *pixels = nullptr;
    armCmAt300(&unitBox, &ppi, &pixels);
    if (QTest::currentTestFailed())
        return;

    // Select-all + Ctrl+X (QKeySequence::Cut) — the clipboard removal
    // path, distinct from the Backspace path the committed harness pins.
    ppi->forceActiveFocus();
    QTRY_VERIFY(ppi->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_A, Qt::ControlModifier);
    QTest::keyClick(m_window, Qt::Key_X, Qt::ControlModifier);
    if (ppi->property("text").toString() != QString()) {
        // Platform fallback: drive the same handler via the API cut.
        QMetaObject::invokeMethod(ppi, "cut");
    }
    QCOMPARE(ppi->property("text").toString(), QString());
    QVERIFY(!m_rd->property("ppiOk").toBool());
    qWarning() << "D2v1 after 300->cut: unitKey="
               << m_rd->property("unitKey").toString() << "currentIndex="
               << unitBox->property("currentIndex").toInt() << "pixels.value="
               << pixels->property("value").toInt() << "lastGoodPpi="
               << m_rd->property("lastGoodPpi").toDouble() << "drivenPx="
               << m_rd->property("drivenPx").toInt();
    // Full revert invariants ON the invalidating edit:
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0);
    QCOMPARE(pixels->property("value").toInt(), 1000);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 1000);
    // lastGoodPpi must NOT be poisoned (Number("") == 0 assignment).
    QCOMPARE(m_rd->property("lastGoodPpi").toDouble(), 300.0);
}

// ---- D2v2: invalidate via replacing with "0" --------------------------------

void TstDeltaReviewProbes::d2InvalidateViaZeroText()
{
    QQuickItem *unitBox = nullptr, *ppi = nullptr, *pixels = nullptr;
    armCmAt300(&unitBox, &ppi, &pixels);
    if (QTest::currentTestFailed())
        return;

    // Round-1 report: "'0' poisons identically" — 0 < DoubleValidator
    // bottom 0.001, so "0" is non-acceptable (Intermediate) and must
    // revert WITHOUT lastGoodPpi taking Number("0") == 0.
    retype(ppi, QStringLiteral("0"));
    QCOMPARE(ppi->property("text").toString(), QStringLiteral("0"));
    QVERIFY(!m_rd->property("ppiOk").toBool());
    qWarning() << "D2v2 after 300->'0': unitKey="
               << m_rd->property("unitKey").toString() << "currentIndex="
               << unitBox->property("currentIndex").toInt() << "pixels.value="
               << pixels->property("value").toInt() << "lastGoodPpi="
               << m_rd->property("lastGoodPpi").toDouble() << "drivenPx="
               << m_rd->property("drivenPx").toInt();
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("px"));
    QCOMPARE(unitBox->property("currentIndex").toInt(), 0);
    QCOMPARE(pixels->property("value").toInt(), 1000);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 1000);
    QCOMPARE(m_rd->property("lastGoodPpi").toDouble(), 300.0);
}

// ---- D3v: PASTE "7.3" (whole-string validation path) ------------------------

void TstDeltaReviewProbes::d3PasteDecimalString()
{
    QQuickItem *unitBox = nullptr, *ppi = nullptr, *pixels = nullptr;
    armCmAt300(&unitBox, &ppi, &pixels);
    if (QTest::currentTestFailed())
        return;

    QQuickItem *editor = spinEditor(pixels);
    QVERIFY(editor);
    QGuiApplication::clipboard()->setText(QStringLiteral("7.3"));
    editor->forceActiveFocus();
    QTRY_VERIFY(editor->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_A, Qt::ControlModifier);
    QTest::keyClick(m_window, Qt::Key_V, Qt::ControlModifier); // paste
    const QString pasted = editor->property("text").toString();
    qWarning() << "D3v after paste '7.3': editor holds" << pasted;
    QCOMPARE(pasted, QStringLiteral("7.3"));
    QTest::keyClick(m_window, Qt::Key_Return);
    qWarning() << "D3v committed thousandths ="
               << pixels->property("value").toInt() << "drivenPx ="
               << m_rd->property("drivenPx").toInt();
    // Mechanical derivation (units.cpp cm arm: value*dpi/2.54):
    // 7.3*300/2.54 = 862.2047... -> round 862; thousandths 7300.
    QCOMPARE(pixels->property("value").toInt(), 7300);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 862);
}

// ---- D4v: exact 2^32+131 on Resize drivenPx + loud engine rejection ---------

void TstDeltaReviewProbes::d4ResizeExactWrapValueOnDrivenPx()
{
    QQuickItem *unitBox = nullptr, *ppi = nullptr, *pixels = nullptr;
    armCmAt300(&unitBox, &ppi, &pixels);
    if (QTest::currentTestFailed())
        return;

    // Drive 2.54 cm, then raise ppi to 4294967427 (validator-valid, no
    // top): px = 2.54 * 4294967427 / 2.54 = 4294967427 = 2^32 + 131
    // EXACTLY. Uncapped, ToInt32 wraps this to 131 (the round-1 D4
    // wrap); capped it must read 2000000.
    QQuickItem *driver = itemOf(m_rd, "driver");
    QVERIFY(driver);
    QVERIFY2(comboPickByMouse(driver, 0), "driver width pick failed");
    retype(spinEditor(pixels), QStringLiteral("2.54"));
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(pixels->property("value").toInt(), 2540);
    retype(ppi, QStringLiteral("4294967427"));
    QVERIFY(m_rd->property("ppiOk").toBool());
    QCOMPARE(m_rd->property("unitKey").toString(), QLatin1String("cm"));
    const int driven = m_rd->property("drivenPx").toInt();
    qWarning() << "D4v drivenPx at exact 2^32+131 conversion =" << driven
               << "outW=" << m_rd->property("outW").toInt()
               << "outH=" << m_rd->property("outH").toInt();
    QVERIFY2(driven != 131, "ToInt32 wrap reached drivenPx");
    QCOMPARE(driven, 2000000);

    // The engine must reject LOUDLY (out-of-range), never accept a
    // wrapped small resize: Apply -> errorText set, dialog stays open,
    // doc untouched.
    QQuickItem *applyBtn = buttonByText(m_rd, "Apply");
    QVERIFY(applyBtn);
    QVERIFY(applyBtn->property("enabled").toBool());
    clickItem(applyBtn);
    QTest::qWait(100);
    const QString err = m_rd->property("errorText").toString();
    qWarning() << "D4v Apply rejection errorText =" << err;
    QVERIFY(m_rd->property("visible").toBool()); // did NOT close
    QVERIFY2(!err.isEmpty(), "engine rejection was SILENT");
    QCOMPARE(m_controller->docWidth(), 1000);  // doc untouched
    QCOMPARE(m_controller->docHeight(), 800);
}

// ---- D5v: height-driven degenerate, ASSERTED preview == engine --------------

void TstDeltaReviewProbes::d5HeightDrivenDegenerateAsserted()
{
    // 1 x 1000 doc, drive HEIGHT to 100: derived W = round(1*100/1000)
    // = round(0.1) = 0 without the clamp; engine derive().max(1.0) = 1.
    // The committed harness only RECORDS the width-driven twin; this
    // slot ASSERTS the height-driven preview.
    freshDoc(1, 1000);
    openDialog(m_rd);
    QQuickItem *pixels = itemOf(m_rd, "pixels");
    QQuickItem *driver = itemOf(m_rd, "driver");
    QVERIFY(pixels && driver);
    QVERIFY2(comboPickByMouse(driver, 1), "driver height pick failed");
    retype(spinEditor(pixels), QStringLiteral("100"));
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(m_rd->property("drivenPx").toInt(), 100);
    const int previewW = m_rd->property("outW").toInt();
    const int previewH = m_rd->property("outH").toInt();
    qWarning() << "D5v preview =" << previewW << "x" << previewH;
    QCOMPARE(previewW, 1); // the engine's max(1.0) floor, ASSERTED
    QCOMPARE(previewH, 100);
    QQuickItem *applyBtn = buttonByText(m_rd, "Apply");
    QVERIFY(applyBtn);
    clickItem(applyBtn);
    QTRY_VERIFY2(!m_rd->property("visible").toBool(),
                 qPrintable(m_rd->property("errorText").toString()));
    QCOMPARE(m_controller->docWidth(), previewW);   // engine == preview
    QCOMPARE(m_controller->docHeight(), previewH);
    QCOMPARE(m_controller->docWidth(), 1);
    QCOMPARE(m_controller->docHeight(), 100);
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstDeltaReviewProbes tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_delta_review_probes.moc"
