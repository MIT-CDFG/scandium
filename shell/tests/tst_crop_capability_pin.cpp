// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// C-130 SHELL-SIDE PIN — PROVE THE CROP DIALOG BEHAVES CORRECTLY WHEN THE
// CAPABILITY GOES LIVE, BEFORE IT GOES LIVE.
//
// STATE OF THE WORLD THIS PIN IS WRITTEN AGAINST:
//   * the engine SERVES non-destructive crop (deletePixels:false sets
//     crop_rect; every export door frames on image_rect());
//   * `appname_version()` still returns abi_minor 6, `abiHasBlock17()`
//     requires >= 7, so `DocumentController::cropNonDestructiveLive()` is
//     FALSE, and CropDialog.qml therefore forces `deletePixels` ON and
//     DISABLED — every crop from the app destroys pixels;
//   * the engine fix (abi_minor 6 -> 7) is another desk's and is coming,
//     WITH NO CHANGE TO THE QML BINDINGS.
//
// The claim under test is exactly the one nobody has measured: "the
// property flips and the QML just works." This pin measures it. It does
// NOT change the bindings — they are correct; the property is what is
// wrong, and it is being fixed elsewhere.
//
// THE SEAM (no engine edit, no production edit): `AppDoc` is a QML
// CONTEXT PROPERTY (shell/src/main.cpp: setContextProperty("AppDoc",
// &controller)), never a QML singleton — so a test may bind any QObject
// under that name. `CropAppDocStub` below DELEGATES every member
// CropDialog reads to a REAL DocumentController over a REAL engine
// document, and overrides exactly one bit: `cropNonDestructiveLive`.
// It also SPIES the doc.crop envelope on its way through, which is the
// load-bearing part: a dialog that merely ENABLES the toggle while still
// sending deletePixels:true would look fixed and not be.
//
// The property is declared CONSTANT on the real controller, so this pin
// builds a FRESH engine + dialog per capability state rather than
// toggling one live — that is the production evaluation order.
//
// DISCRIMINATION: every assertion is made in BOTH states, with opposite
// expectations. A pin that passes in both states pins nothing.

#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"

#include <QDir>
#include <QFile>
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

// ---- the seam -------------------------------------------------------------
//
// Everything CropDialog.qml reads off AppDoc, delegated to the real
// controller, EXCEPT cropNonDestructiveLive which is forced to the value
// under test. Declared CONSTANT to match the real controller's shape.
class CropAppDocStub : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool cropNonDestructiveLive READ cropNonDestructiveLive CONSTANT)
    Q_PROPERTY(bool hasDocument READ hasDocument NOTIFY infoChanged)
    Q_PROPERTY(int docWidth READ docWidth NOTIFY infoChanged)
    Q_PROPERTY(int docHeight READ docHeight NOTIFY infoChanged)

public:
    CropAppDocStub(DocumentController *real, bool live, QObject *parent = nullptr)
        : QObject(parent), m_real(real), m_live(live)
    {
        connect(real, &DocumentController::infoChanged,
                this, &CropAppDocStub::infoChanged);
    }

    // MUTATION POINT M5 (discrimination control): return `false` here
    // unconditionally and every LIVE-state assertion below must red. That
    // is what proves this pin discriminates rather than passing in both
    // states.
    bool cropNonDestructiveLive() const { return m_live; }

    bool hasDocument() const { return m_real->hasDocument(); }
    int docWidth() const { return m_real->docWidth(); }
    int docHeight() const { return m_real->docHeight(); }

    // The wire spy. Records the envelope, then forwards it to the REAL
    // controller so the engine's own outcome still governs the dialog's
    // close/error arm.
    Q_INVOKABLE QVariantMap applyCommand(const QString &kind,
                                         const QVariantMap &target,
                                         const QVariantMap &params,
                                         const QString &mergeKey = QString())
    {
        ++m_calls;
        m_lastKind = kind;
        m_lastParams = params;
        return m_real->applyCommand(kind, target, params, mergeKey);
    }

    int calls() const { return m_calls; }
    QString lastKind() const { return m_lastKind; }
    QVariantMap lastParams() const { return m_lastParams; }

signals:
    void infoChanged();

private:
    DocumentController *m_real = nullptr;
    bool m_live = false;
    int m_calls = 0;
    QString m_lastKind;
    QVariantMap m_lastParams;
};

namespace {

QString copyFail; // set by setUpModuleDir on error

// Byte-copy the REAL CropDialog + Theme into <tmp>/AppnameShell with a
// qmldir, so the dialog loads exactly as checked in and shell/qml stays
// the single truth (the tst_review_dialog_probes pattern).
bool setUpModuleDir(const QString &root)
{
    const QString mod = root + QStringLiteral("/AppnameShell");
    if (!QDir().mkpath(mod)) {
        copyFail = QStringLiteral("mkpath failed: ") + mod;
        return false;
    }
    const struct { const char *src; const char *dst; } files[] = {
        {"/qml/Theme.qml", "/Theme.qml"},
        {"/qml/dialogs/CropDialog.qml", "/CropDialog.qml"},
    };
    for (const auto &f : files) {
        const QString src =
            QStringLiteral(APPNAME_SHELL_DIR) + QLatin1String(f.src);
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
                 "CropDialog 1.0 CropDialog.qml\n");
    return true;
}

const char kScene[] =
    "import QtQuick\n"
    "import QtQuick.Controls\n"
    "import AppnameShell\n"
    "ApplicationWindow {\n"
    "    id: win\n"
    "    width: 800; height: 700; visible: true\n"
    "    property alias cd: cd\n"
    "    CropDialog { id: cd }\n"
    "}\n";

} // namespace

// One complete engine + scene + stub, built for a single capability state.
struct Harness {
    QQmlApplicationEngine *engine = nullptr;
    CropAppDocStub *doc = nullptr;
    QQuickWindow *window = nullptr;
    QObject *dialog = nullptr;
};

class TstCropCapabilityPin : public QObject {
    Q_OBJECT

private slots:
    void initTestCase();
    void cleanupTestCase();

    // Positive controls — an instrument that cannot tell the two states
    // apart makes every assertion below vacuous.
    void theSeamDrivesTheCapabilityPropertyBothWays();
    void theVisibilityInstrumentIsLiveInBothStates();

    // CAPABILITY LIVE (what the abi_minor 6->7 flip must produce).
    void liveCapabilityEnablesTheToggle();
    void liveCapabilityDefaultsToTheDestructiveArmAndStaysChoosable();
    void liveCapabilityDropsTheMilestoneNote();
    void liveCapabilityApplySendsTrueByDefaultAndFalseWhenUnticked();

    // C-131 TRIPWIRE — converts "when X lands, do Y" into "has X landed?"
    void theCapabilityFlipIsNotAheadOfTheGeometryConsumption();

    // C-140 THE OUTPUT-FRAME GEOMETRY PIN. Three engine states, and the
    // ORIGIN asserted against a NON-ZERO-ORIGIN case — a size-only fix
    // passes a size-only pin, so the size is the half that does not
    // discriminate.
    void theOutputFrameIsPublishedWithItsOriginInAllThreeStates();
    void theCanvasPaintsTheCroppedRegionNotMerelyTheCroppedSize();

    // CAPABILITY DEAD (today's shipped behaviour — unchanged).
    void deadCapabilityLocksTheToggle();
    void deadCapabilityForcesTheDestructiveArm();
    void deadCapabilityShowsTheMilestoneNote();
    void deadCapabilityApplySendsDeletePixelsTrue();

private:
    // ---- harness ----------------------------------------------------------
    Harness *build(bool live)
    {
        Harness *h = new Harness;
        h->doc = new CropAppDocStub(m_controller, live);
        h->engine = new QQmlApplicationEngine;
        h->engine->addImportPath(m_dir.path());
        h->engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                     m_i18n);
        h->engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                     h->doc);
        h->engine->loadData(QByteArray(kScene),
                            QUrl(QStringLiteral("qrc:/croppin/scene.qml")));
        if (h->engine->rootObjects().isEmpty())
            return h;
        h->window = qobject_cast<QQuickWindow *>(h->engine->rootObjects().first());
        if (h->window)
            h->dialog = h->window->property("cd").value<QObject *>();
        return h;
    }

    // The engine must die BEFORE the context-property objects (the
    // ordering main.cpp encodes; reversed, every binding TypeErrors at
    // teardown).
    void destroy(Harness *h)
    {
        if (!h)
            return;
        delete h->engine;
        delete h->doc;
        delete h;
    }

    QObject *idOf(QObject *root, const char *name)
    {
        QList<QObject *> candidates = root->findChildren<QObject *>();
        candidates.prepend(root);
        for (QObject *c : candidates) {
            QQmlContext *ctx = qmlContext(c);
            if (!ctx)
                continue;
            QQmlExpression e(ctx, c, QLatin1String(name));
            bool undefined = false;
            const QVariant v = e.evaluate(&undefined);
            if (e.hasError())
                continue;
            if (QObject *o = v.value<QObject *>())
                return o;
        }
        return nullptr;
    }
    QQuickItem *itemOf(QObject *root, const char *name)
    {
        return qobject_cast<QQuickItem *>(idOf(root, name));
    }

    // The milestone note carries no id, so it is found by its CATALOG
    // text (rule 8: catalog truth, never a stand-in literal).
    QString milestoneNoteText() const
    {
        QVariantMap args;
        args.insert(QStringLiteral("kind"),
                    QStringLiteral("non-destructive crop"));
        return m_i18n->tr(QStringLiteral("ui.panel.layers.milestone_tooltip"),
                          args);
    }

    QQuickItem *milestoneNote(QObject *dialog)
    {
        const QString want = milestoneNoteText();
        QQuickItem *hit = nullptr;
        int seen = 0;
        for (QQuickItem *i : dialog->findChildren<QQuickItem *>()) {
            if (!i->inherits("QQuickLabel"))
                continue;
            if (i->property("text").toString() == want) {
                ++seen;
                hit = i;
            }
        }
        // Exactly one, or the lookup is not identifying what we think.
        return seen == 1 ? hit : nullptr;
    }

    QQuickItem *buttonOf(QObject *dialog, const QString &catalogText)
    {
        for (QQuickItem *b : dialog->findChildren<QQuickItem *>()) {
            if (b->inherits("QQuickButton")
                && b->property("text").toString() == catalogText)
                return b;
        }
        return nullptr;
    }

    void freshDoc(int w, int h)
    {
        QVERIFY2(m_controller->newDocument(w, h, QStringLiteral("white")),
                 "engine newDocument failed");
        QTRY_COMPARE(m_controller->docWidth(), w);
        QTRY_COMPARE(m_controller->docHeight(), h);
    }

    // Open the dialog on a sub-rect through the REAL production entry
    // point (openWithRect is what the canvas crop drag calls), click the
    // real Apply button, and hand back the spied envelope.
    void applyThroughDialog(Harness *h, QVariantMap *paramsOut)
    {
        QVERIFY(h->dialog);
        QMetaObject::invokeMethod(h->dialog, "openWithRect",
                                  Q_ARG(QVariant, QVariant(10)),
                                  Q_ARG(QVariant, QVariant(20)),
                                  Q_ARG(QVariant, QVariant(100)),
                                  Q_ARG(QVariant, QVariant(80)));
        QTRY_VERIFY(h->dialog->property("opened").toBool());

        QQuickItem *apply =
            buttonOf(h->dialog, m_i18n->tr(QStringLiteral("ui.dialog.apply")));
        QVERIFY2(apply, "Apply button not found by catalog text");
        QVERIFY2(apply->property("enabled").toBool(),
                 "Apply disabled — hasDocument did not reach the dialog");

        const int before = h->doc->calls();
        const QPointF c = apply->mapToScene(
            QPointF(apply->width() / 2.0, apply->height() / 2.0));
        QTest::mouseClick(h->window, Qt::LeftButton, Qt::NoModifier,
                          c.toPoint());
        QTRY_VERIFY2(h->doc->calls() > before,
                     "Apply click did not reach applyCommand");
        QCOMPARE(h->doc->lastKind(), QStringLiteral("doc.crop"));
        *paramsOut = h->doc->lastParams();
    }

    QTemporaryDir m_dir;
    Catalog m_catalog;
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
};

void TstCropCapabilityPin::initTestCase()
{
    QVERIFY2(m_dir.isValid(), "temp dir");
    QVERIFY2(setUpModuleDir(m_dir.path()), qPrintable(copyFail));

    QFile en(QStringLiteral(APPNAME_REPO_I18N_DIR "/en.json"));
    QVERIFY(en.open(QIODevice::ReadOnly));
    m_catalog = Catalog::fromTexts(
        {{QStringLiteral("en"), QString::fromUtf8(en.readAll())}});
    m_i18n = new CatalogQml(m_catalog, this);
    m_controller = new DocumentController(&m_i18n->catalog());
    m_controller->setParent(this);

    // The note text must actually resolve, or every note assertion below
    // is comparing two empty strings.
    QVERIFY2(!milestoneNoteText().isEmpty(), "milestone note text empty");
    QVERIFY2(milestoneNoteText()
                 != QStringLiteral("ui.panel.layers.milestone_tooltip"),
             "milestone note key did not resolve through the catalog");
}

void TstCropCapabilityPin::cleanupTestCase() {}

// ---- positive controls ----------------------------------------------------

// If the seam cannot drive the property both ways, this pin pins nothing
// and every state-specific assertion below is vacuous. This is the guard
// that says so out loud.
void TstCropCapabilityPin::theSeamDrivesTheCapabilityPropertyBothWays()
{
    for (bool live : {true, false}) {
        Harness *h = build(live);
        QVERIFY2(h->dialog, "CropDialog failed to load");
        // Read the property back THROUGH the dialog's own QML context —
        // the same lookup the bindings use, not the C++ getter.
        QQmlContext *ctx = qmlContext(h->dialog);
        QVERIFY(ctx);
        QQmlExpression e(ctx, h->dialog,
                         QStringLiteral("AppDoc.cropNonDestructiveLive"));
        bool undefined = false;
        const QVariant v = e.evaluate(&undefined);
        QVERIFY2(!e.hasError(), qPrintable(e.error().toString()));
        QVERIFY2(!undefined, "AppDoc.cropNonDestructiveLive is undefined "
                             "— the stub does not expose the property");
        QCOMPARE(v.toBool(), live);
        destroy(h);
    }
}

// `visible` on a child of a closed Dialog is false for reasons that have
// nothing to do with the binding. Prove the instrument reports TRUE for a
// control that is always shown, in BOTH states, so the note's
// visible==false in the live state is a measurement and not an artifact.
void TstCropCapabilityPin::theVisibilityInstrumentIsLiveInBothStates()
{
    for (bool live : {true, false}) {
        Harness *h = build(live);
        QVERIFY(h->dialog);
        QMetaObject::invokeMethod(h->dialog, "open");
        QTRY_VERIFY(h->dialog->property("opened").toBool());
        QQuickItem *sw = itemOf(h->dialog, "deletePixels");
        QVERIFY2(sw, "deletePixels Switch not found");
        QVERIFY2(sw->isVisible(),
                 "positive control failed: the always-shown Switch reports "
                 "invisible, so visible==false proves nothing");
        // And the note item EXISTS in both states (visibility, not
        // existence, is what the binding controls).
        QVERIFY2(milestoneNote(h->dialog),
                 "milestone note not found by catalog text (or not unique)");
        destroy(h);
    }
}

// ---- capability LIVE ------------------------------------------------------

void TstCropCapabilityPin::liveCapabilityEnablesTheToggle()
{
    Harness *h = build(true);
    QVERIFY(h->dialog);
    QMetaObject::invokeMethod(h->dialog, "open");
    QTRY_VERIFY(h->dialog->property("opened").toBool());
    QQuickItem *sw = itemOf(h->dialog, "deletePixels");
    QVERIFY(sw);
    QVERIFY2(sw->property("enabled").toBool(),
             "capability live but the deletePixels toggle is still locked");
    destroy(h);
}

// The one that matters most: reachable is not the same as DEFAULT. The
// contract default is deletePixels:false, and the flip must land there.
void TstCropCapabilityPin::liveCapabilityDefaultsToTheDestructiveArmAndStaysChoosable()
{
    // RC3 D-5 (operator WORD 35, sitting S-17): the dialog opens with
    // "Delete cropped pixels" ON, so Apply crops the DOCUMENT -- and, with the
    // capability live, the toggle is ENABLED, so the non-destructive crop is
    // one untick away (the case below drives it).
    Harness *h = build(true);
    QVERIFY(h->dialog);
    QMetaObject::invokeMethod(h->dialog, "open");
    QTRY_VERIFY(h->dialog->property("opened").toBool());
    QQuickItem *sw = itemOf(h->dialog, "deletePixels");
    QVERIFY(sw);
    QVERIFY2(sw->property("checked").toBool(),
             "capability live and deletePixels defaults OFF -- Apply would set "
             "only the output frame and the document keeps its old size (S-17)");
    QVERIFY2(sw->property("enabled").toBool(),
             "capability live but the toggle is locked -- the non-destructive "
             "crop is unreachable");
    destroy(h);
}

void TstCropCapabilityPin::liveCapabilityDropsTheMilestoneNote()
{
    Harness *h = build(true);
    QVERIFY(h->dialog);
    QMetaObject::invokeMethod(h->dialog, "open");
    QTRY_VERIFY(h->dialog->property("opened").toBool());
    QQuickItem *note = milestoneNote(h->dialog);
    QVERIFY2(note, "milestone note not found by catalog text (or not unique)");
    QVERIFY2(!note->isVisible(),
             "capability live but the dialog still shows the "
             "not-implemented milestone note");
    destroy(h);
}

// THE LOAD-BEARING ONE. A dialog that enables the control while still
// sending the destructive value would look fixed and not be. This asserts
// the WIRE, on the DEFAULT path, with no user interaction with the toggle.
void TstCropCapabilityPin::liveCapabilityApplySendsTrueByDefaultAndFalseWhenUnticked()
{
    // RC3 D-5: untouched, Apply sends deletePixels:true (the document takes
    // the cropped size); unticked, it sends false (the non-destructive arm,
    // still reachable). Both read off the REAL envelope.
    freshDoc(200, 150);
    Harness *h = build(true);
    QVERIFY(h->dialog);
    QVERIFY(QTest::qWaitForWindowExposed(h->window));
    QVariantMap params;
    applyThroughDialog(h, &params);
    QVERIFY2(params.contains(QStringLiteral("deletePixels")),
             "doc.crop envelope carries no deletePixels key");
    QVERIFY2(params.value(QStringLiteral("deletePixels")).toBool(),
             "capability live, toggle untouched, and Apply sent deletePixels:"
             "false -- the document would keep its old size (S-17)");
    // The rect rode the same envelope (the spy is reading the real
    // payload, not a default-constructed map).
    const QVariantMap rect = params.value(QStringLiteral("rect")).toMap();
    QCOMPARE(rect.value(QStringLiteral("x")).toInt(), 10);
    QCOMPARE(rect.value(QStringLiteral("y")).toInt(), 20);
    QCOMPARE(rect.value(QStringLiteral("w")).toInt(), 100);
    QCOMPARE(rect.value(QStringLiteral("h")).toInt(), 80);
    destroy(h);

    // Unticked: the non-destructive arm.
    freshDoc(200, 150);
    Harness *h2 = build(true);
    QVERIFY(h2->dialog);
    QVERIFY(QTest::qWaitForWindowExposed(h2->window));
    QMetaObject::invokeMethod(h2->dialog, "open");
    QTRY_VERIFY(h2->dialog->property("opened").toBool());
    QQuickItem *sw = itemOf(h2->dialog, "deletePixels");
    QVERIFY(sw);
    QMetaObject::invokeMethod(sw, "toggle");
    QVERIFY2(!sw->property("checked").toBool(), "the toggle did not untick");
    QVariantMap params2;
    applyThroughDialog(h2, &params2);
    QVERIFY2(!params2.value(QStringLiteral("deletePixels")).toBool(),
             "unticked, and Apply still sent deletePixels:true -- the "
             "non-destructive crop is unreachable");
    destroy(h2);
}

// ---- C-131 tripwire -------------------------------------------------------
//
// A RECORD OF A PENDING CONDITION IS NOT A MONITOR OF IT. The four slots
// above pin what the dialog does when the capability goes live; this slot
// asks the question nobody asked for days — HAS IT GONE LIVE? — and reds
// if it has while the shell is still painting from the CANVAS rect.
//
// The coupling, measured by O-INT on the running binary: cabi `info` now
// publishes `imageRect: [x,y,w,h]` from DocumentGeometry::image_rect(),
// with the ORIGIN CARRIED, while `width`/`height` keep meaning CANVAS:
//     no crop          w/h 200 100   imageRect [ 0,  0, 200, 100]
//     non-destructive  w/h 200 100   imageRect [40, 20, 120,  60]
//     destructive      w/h 120  60   imageRect [ 0,  0, 120,  60]
// The shell's paint extent and viewToDoc still derive from docWidth()/
// docHeight() with no document-origin term, and composite_region_rgba8
// builds its ROI from ABSOLUTE document coordinates — so the moment the
// capability is live in a tree whose shell has not consumed the origin,
// THE CANVAS SHOWS THE WRONG REGION AT THE RIGHT SIZE.
//
// This is a DISPLAY transient, not data loss: the pixels survive and the
// export door is already correct (it frames on image_rect()).
void TstCropCapabilityPin::theCapabilityFlipIsNotAheadOfTheGeometryConsumption()
{
    const bool live = m_controller->cropNonDestructiveLive();
    qInfo() << "REAL DocumentController::cropNonDestructiveLive() in this "
               "tree ="
            << live;
    if (!live) {
        QSKIP("capability not yet live in THIS tree (abi_minor < 7) — "
              "tripwire ARMED, not fired. It fires the moment this lane "
              "picks up the engine bump.");
    }

    // Live. The shell must now expose the output frame's ORIGIN, not just
    // the canvas size. Asked of the metaobject so this slot needs no
    // knowledge of the eventual property's exact name.
    const QMetaObject *mo = m_controller->metaObject();
    QStringList names;
    for (int i = 0; i < mo->propertyCount(); ++i)
        names << QString::fromLatin1(mo->property(i).name());
    const bool consumesOutputFrame =
        names.filter(QStringLiteral("imageRect"), Qt::CaseInsensitive).size() > 0;

    // Positive control: the metaobject sweep can see properties at all.
    QVERIFY2(names.contains(QStringLiteral("docWidth")),
             "metaobject sweep found no docWidth — the instrument is broken "
             "and its negative result means nothing");

    QVERIFY2(consumesOutputFrame,
             "THE CAPABILITY IS LIVE AND THE SHELL DOES NOT CONSUME "
             "imageRect. The crop dialog now offers the non-destructive "
             "arm, a user can take it, crop_rect gets a non-zero origin — "
             "and the canvas still paints read_region(0,0,docWidth,"
             "docHeight) with a viewToDoc that has no origin term, so it "
             "SHOWS THE WRONG REGION AT THE RIGHT SIZE. Consume "
             "info.imageRect in the paint extent and the doc<->view "
             "transform. Note width/height still legitimately mean CANVAS "
             "for the canvas-size surfaces; this is not a global rename.");
}

// ---- C-140: THE OUTPUT-FRAME GEOMETRY PIN ---------------------------------
//
// ★★★★ THE ORIGIN IS THE ASSERTION. THE SIZE IS NOT.
//
// The defect class this pin exists against is a test whose fixture never
// varies the dimension its name claims. "The shell tracks the crop" is
// satisfied by a shell that merely ECHOES THE CANVAS as long as every
// fixture crop starts at (0,0) — and it is satisfied by a SIZE-ONLY fix
// (cache resized to 120x60, still read from (0,0)) as long as nothing
// ever asks WHICH pixels came back. So the fixture below uses a crop
// whose origin is (40,20), NOT (0,0), and the destructive arm rides
// along as the CONTROL: the two arms must DISAGREE, or the pin is
// consistent with a shell that reports the canvas under both.
//
//   state             docWidth/docHeight     imageRect
//   no crop               200 100            [ 0,  0, 200, 100]
//   non-destructive       200 100            [40, 20, 120,  60]   <- origin
//   destructive           120  60            [ 0,  0, 120,  60]   <- control
//
// The numbers are NOT copied in as expectations of the engine's internals:
// state 1 is asserted against the document this test created, and states 2
// and 3 are asserted against the rect THIS TEST SUBMITTED. If the engine
// disagreed with the published table, these assertions would say so.

namespace {
constexpr int kCanvasW = 200;
constexpr int kCanvasH = 100;
// ★ OFF-ORIGIN ON BOTH AXES, and deliberately NOT centred in the canvas:
// a centred rect makes several plausible origin bugs invisible because
// the wrong answer and the right answer share a midpoint.
constexpr int kCropX = 40;
constexpr int kCropY = 20;
constexpr int kCropW = 120;
constexpr int kCropH = 60;

QVariantMap cropParams(bool deletePixels)
{
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), kCropX);
    rect.insert(QStringLiteral("y"), kCropY);
    rect.insert(QStringLiteral("w"), kCropW);
    rect.insert(QStringLiteral("h"), kCropH);
    QVariantMap params;
    params.insert(QStringLiteral("rect"), rect);
    params.insert(QStringLiteral("deletePixels"), deletePixels);
    return params;
}

// A short stroke centred on an ABSOLUTE document point.
QVariantList strokeAt(double x, double y)
{
    QVariantList pts;
    for (int i = 0; i < 3; ++i) {
        QVariantMap p;
        p.insert(QStringLiteral("x"), x + i);
        p.insert(QStringLiteral("y"), y);
        p.insert(QStringLiteral("pressure"), 1.0);
        p.insert(QStringLiteral("t"), double(i) * 8.0);
        pts.append(p);
    }
    return pts;
}

// Channel-dominance counters. The workspace background is 0x404245 and
// the checker is grey/white, so neither can register as either colour.
int countRed(const QImage &img)
{
    int n = 0;
    for (int y = 0; y < img.height(); ++y)
        for (int x = 0; x < img.width(); ++x) {
            const QColor c = img.pixelColor(x, y);
            if (c.red() > 140 && c.green() < 90 && c.blue() < 90)
                ++n;
        }
    return n;
}
int countBlue(const QImage &img)
{
    int n = 0;
    for (int y = 0; y < img.height(); ++y)
        for (int x = 0; x < img.width(); ++x) {
            const QColor c = img.pixelColor(x, y);
            if (c.blue() > 140 && c.red() < 90 && c.green() < 90)
                ++n;
        }
    return n;
}
} // namespace

void TstCropCapabilityPin::theOutputFrameIsPublishedWithItsOriginInAllThreeStates()
{
    // Liveness floor. Without the bump the non-destructive arm is not
    // servable and every assertion below would be measuring the wrong
    // world — a SKIP would be a lie, because this pin's whole subject is
    // the state after the flip.
    QVERIFY2(m_controller->cropNonDestructiveLive(),
             "abi_minor < 7 in this tree: the non-destructive arm cannot be "
             "reached, so this pin would pass vacuously. Merge the engine "
             "bump before believing any result here");

    // ---- STATE 1: NO CROP. The frame IS the canvas, at the origin. -------
    freshDoc(kCanvasW, kCanvasH);
    QCOMPARE(m_controller->docWidth(), kCanvasW);
    QCOMPARE(m_controller->docHeight(), kCanvasH);
    QCOMPARE(m_controller->imageRect(), QRect(0, 0, kCanvasW, kCanvasH));

    // ---- STATE 2: NON-DESTRUCTIVE. THE CASE THAT DISCRIMINATES. ---------
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("doc.crop"), {},
                                cropParams(false), {})
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "non-destructive doc.crop was refused — the capability reports "
             "live but the engine did not serve it, and nothing below would "
             "mean what it says");
    QTRY_COMPARE(m_controller->imageRect(),
                 QRect(kCropX, kCropY, kCropW, kCropH));
    const QRect nonDestructive = m_controller->imageRect();

    // ★ width/height are NOT overloaded: the CANVAS did not move.
    QCOMPARE(m_controller->docWidth(), kCanvasW);
    QCOMPARE(m_controller->docHeight(), kCanvasH);

    // ★★★★ THE ORIGIN ASSERTION, stated three ways so no single weak
    // implementation satisfies it by accident.
    QCOMPARE(nonDestructive.topLeft(), QPoint(kCropX, kCropY));
    QVERIFY2(nonDestructive.topLeft() != QPoint(0, 0),
             "the frame origin is (0,0) after an OFF-ORIGIN crop — the shell "
             "is reporting a SIZE and defaulting the origin, which reads the "
             "right number of pixels from the wrong place");
    QVERIFY2(nonDestructive
                 != QRect(0, 0, m_controller->docWidth(),
                          m_controller->docHeight()),
             "imageRect equals the CANVAS rect while a non-destructive crop "
             "is active — the shell is echoing width/height, not tracking "
             "the output frame");

    // ---- STATE 3: DESTRUCTIVE. THE CONTROL. -----------------------------
    // Same rect, other arm. Here the canvas DOES shrink and the origin IS
    // (0,0) — so a shell that echoes the canvas passes THIS state. That is
    // exactly why it cannot be the only state measured.
    freshDoc(kCanvasW, kCanvasH);
    QCOMPARE(m_controller->imageRect(), QRect(0, 0, kCanvasW, kCanvasH));
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("doc.crop"), {},
                                cropParams(true), {})
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "destructive doc.crop was refused");
    QTRY_COMPARE(m_controller->docWidth(), kCropW);
    QCOMPARE(m_controller->docHeight(), kCropH);
    const QRect destructive = m_controller->imageRect();
    QCOMPARE(destructive, QRect(0, 0, kCropW, kCropH));

    // ★★ THE ARMS MUST DISAGREE. If these two rects were equal, every
    // assertion above would also hold for a shell that reported the canvas
    // in both states, and this pin would certify the defect.
    QVERIFY2(destructive != nonDestructive,
             "the destructive and non-destructive arms produced the SAME "
             "imageRect — the two states are indistinguishable here, so this "
             "pin cannot tell a crop-tracking shell from a canvas-echoing "
             "one");
    QCOMPARE(destructive.size(), nonDestructive.size()); // same SIZE...
    QVERIFY2(destructive.topLeft() != nonDestructive.topLeft(),
             "...and the same ORIGIN: the size is the half that does not "
             "discriminate, which is the whole point of this pin");
}

// ★★ THE PAINT-LEVEL HALF. The property being right does not mean the
// canvas READS from it: refreshRegion's blit worked only because
// cache-index WAS absolute-doc-coord, and that identity breaks silently.
//
// Two marks are inked in ABSOLUTE document coordinates, in different
// colours, one INSIDE the future crop and one OUTSIDE it but inside the
// canvas. After the crop the rendered canvas must contain the inside mark
// and NOT the outside one. This separates the three implementations that
// a pixel COUNT alone cannot:
//     correct            RED present, BLUE absent
//     size-only fix      RED absent,  BLUE present  (right size, wrong pixels)
//     no consumption     RED present, BLUE present  (full canvas painted)
void TstCropCapabilityPin::theCanvasPaintsTheCroppedRegionNotMerelyTheCroppedSize()
{
    QVERIFY2(m_controller->cropNonDestructiveLive(),
             "abi_minor < 7 in this tree — see the sibling pin");

    QQuickWindow win;
    win.resize(420, 320);
    CanvasView *canvas = new CanvasView(win.contentItem());
    canvas->setSize(QSizeF(420, 320));
    canvas->setController(m_controller);
    win.show();
    QVERIFY(QTest::qWaitForWindowExposed(&win));

    freshDoc(kCanvasW, kCanvasH);
    QTest::qWait(60);

    // ★★★ MARK PLACEMENT IS THE WHOLE DISCRIMINATION, AND GETTING IT
    // WRONG IS THIS PIN'S OWN VERSION OF THE DEFECT IT HUNTS. Measured:
    // an earlier fixture put the RED mark at (70,45), which is inside the
    // CORRECT region (40,20,120,60) AND inside the SIZE-ONLY region
    // (0,0,120,60) — the two overlap heavily — so the size-only mutation
    // ran GREEN against it. A mark must be EXCLUSIVE to its region.
    //
    //   correct region  x in [40,160)  y in [20, 80)
    //   size-only read  x in [ 0,120)  y in [ 0, 60)
    //
    // RED  (140,68): x >= 120 AND y >= 60 -> in CORRECT, never in size-only.
    // BLUE ( 14,10): x <  40  AND y <  20 -> in size-only, never in CORRECT.
    // Both are exclusive on BOTH axes, so neither a horizontal nor a
    // vertical origin bug can leave a mark where it does not belong.
    canvas->setBrushSize(14.0);
    canvas->setBrushColor(QColor(Qt::red));
    QVERIFY2(canvas->tourStroke(QStringLiteral("brush.stroke"),
                                strokeAt(140, 68)),
             "the INSIDE mark did not ink");
    canvas->setBrushColor(QColor(Qt::blue));
    QVERIFY2(canvas->tourStroke(QStringLiteral("brush.stroke"),
                                strokeAt(14, 10)),
             "the OUTSIDE mark did not ink");
    QTest::qWait(80);
    // ★★ ZOOM 1:1, NOT FIT. Measured: at fit zoom (~3.3x) a wrongly-placed
    // cache is displaced by origin*zoom ~ (132,66) px and the offending
    // mark lands OFF-SCREEN, so the grab shows its absence and the pin
    // reads a masked defect as a pass. At 1:1 the worst-case displacement
    // is (40,20) px and every mark stays inside a 420x320 viewport, so
    // absence in the grab means absence in the cache.
    canvas->zoomFit();
    canvas->zoomOneToOne();
    QTest::qWait(80);

    // ★ POSITIVE CONTROL, load-bearing: the assertions below are partly
    // ABSENCE assertions, and "no blue pixels" is ALSO what a canvas that
    // paints nothing at all produces. This half proves the instrument can
    // see both colours before their absence is allowed to mean anything.
    const QImage before = win.grabWindow();
    const int redBefore = countRed(before);
    const int blueBefore = countBlue(before);
    QVERIFY2(redBefore > 0 && blueBefore > 0,
             qPrintable(QStringLiteral("POSITIVE CONTROL FAILED: with NO crop "
                                       "active the grab shows red=%1 blue=%2. "
                                       "The probe cannot see the marks, so "
                                       "every absence assertion below would "
                                       "pass vacuously")
                            .arg(redBefore)
                            .arg(blueBefore)));

    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("doc.crop"), {},
                                cropParams(false), {})
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "non-destructive doc.crop was refused");
    QTRY_COMPARE(m_controller->imageRect(),
                 QRect(kCropX, kCropY, kCropW, kCropH));
    QTest::qWait(120);
    canvas->zoomFit();
    canvas->zoomOneToOne();
    QTest::qWait(120);

    const QImage after = win.grabWindow();
    const int redAfter = countRed(after);
    const int blueAfter = countBlue(after);

    // ★★★★ THE ASSERTION THAT CATCHES A SIZE-ONLY FIX. RED lives only in
    // the correct region, so a cache sized to the crop but still READ from
    // (0,0) never contains it. Mutation-proven: this line reds under that
    // mutation while the size of the painted frame stays exactly right.
    QVERIFY2(redAfter > 0,
             qPrintable(QStringLiteral("the canvas LOST the mark that is "
                                       "INSIDE the crop (red=%1). It is not "
                                       "reading from the frame's ORIGIN — a "
                                       "cache sized to the crop but still read "
                                       "from (0,0) looks exactly like this")
                            .arg(redAfter)));
    QVERIFY2(blueAfter == 0,
             qPrintable(QStringLiteral("the canvas still shows the mark that "
                                       "is OUTSIDE the crop (blue=%1). The "
                                       "active crop is invisible: the shell is "
                                       "painting the FULL CANVAS")
                            .arg(blueAfter)));

    canvas->setController(nullptr);
}

// ---- capability DEAD (today) ----------------------------------------------

void TstCropCapabilityPin::deadCapabilityLocksTheToggle()
{
    Harness *h = build(false);
    QVERIFY(h->dialog);
    QMetaObject::invokeMethod(h->dialog, "open");
    QTRY_VERIFY(h->dialog->property("opened").toBool());
    QQuickItem *sw = itemOf(h->dialog, "deletePixels");
    QVERIFY(sw);
    QVERIFY2(!sw->property("enabled").toBool(),
             "capability dead but the toggle is reachable — the NI arm "
             "would be offered and refused");
    destroy(h);
}

void TstCropCapabilityPin::deadCapabilityForcesTheDestructiveArm()
{
    Harness *h = build(false);
    QVERIFY(h->dialog);
    QMetaObject::invokeMethod(h->dialog, "open");
    QTRY_VERIFY(h->dialog->property("opened").toBool());
    QQuickItem *sw = itemOf(h->dialog, "deletePixels");
    QVERIFY(sw);
    QVERIFY2(sw->property("checked").toBool(),
             "capability dead but deletePixels is OFF — the only servable "
             "arm is not selected");
    destroy(h);
}

void TstCropCapabilityPin::deadCapabilityShowsTheMilestoneNote()
{
    Harness *h = build(false);
    QVERIFY(h->dialog);
    QMetaObject::invokeMethod(h->dialog, "open");
    QTRY_VERIFY(h->dialog->property("opened").toBool());
    QQuickItem *note = milestoneNote(h->dialog);
    QVERIFY2(note, "milestone note not found by catalog text (or not unique)");
    QVERIFY2(note->isVisible(),
             "capability dead but the milestone note is hidden");
    destroy(h);
}

void TstCropCapabilityPin::deadCapabilityApplySendsDeletePixelsTrue()
{
    freshDoc(200, 150);
    Harness *h = build(false);
    QVERIFY(h->dialog);
    QVERIFY(QTest::qWaitForWindowExposed(h->window));
    QVariantMap params;
    applyThroughDialog(h, &params);

    QVERIFY2(params.contains(QStringLiteral("deletePixels")),
             "doc.crop envelope carries no deletePixels key");
    QVERIFY2(params.value(QStringLiteral("deletePixels")).toBool(),
             "capability dead but Apply sent deletePixels:false — the "
             "shell would be asking for an arm it believes is NI");
    destroy(h);
}

// Offscreen + the app's own Basic style (the tst_focus_probes pattern).
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstCropCapabilityPin tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_crop_capability_pin.moc"
