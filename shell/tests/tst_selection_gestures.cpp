// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 6 (operator repeat report): the GESTURE-LEVEL regression
// net for mouse-driven selections — this defect class is invisible to
// the command-driven tour (the tour applies selection.set directly and
// never exercises press/move/release), so each of the five selection
// tools gets a real synthesized-mouse case against a REAL engine
// document in an offscreen window: rect marquee and ellipse commit on
// drag-release; lasso auto-closes on release; polygon closes on
// double-click; wand fires at the clicked point. Assertions read
// ENGINE truth (selectionActive + the mask bbox via the readback door),
// never shell echo state.

#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "selection_scan.h"

#include <cmath>

#include <QElapsedTimer>
#include <QGuiApplication>
#include <QQuickWindow>
#include <QtTest>

using namespace appname::shell;

namespace {

// Engine-truth selection bounds through the same chunked scan the
// announcer uses (strip reads ≤ 2^26 px).
QRect engineSelectionBounds(DocumentController *c)
{
    const int w = c->docWidth();
    const int h = c->docHeight();
    if (w <= 0 || h <= 0)
        return QRect();
    const int maxRows = qMax(1, (1 << 26) / qMax(1, w));
    QRect bounds;
    for (int y = 0; y < h; y += maxRows) {
        QImage strip;
        if (!c->readSelectionImage(QRect(0, y, w, qMin(maxRows, h - y)),
                                   &strip))
            return QRect();
        bounds = accumulateMaskBounds(strip, y, bounds);
    }
    return bounds;
}

// Review D6: the discriminating acceptance — a PARTIAL selection is
// cleared, then a fill must reach a corner OUTSIDE the old selection
// (a full-canvas seed gates nothing and cannot fail).
#define VERIFY_DESELECTED_AND_FILL_REACHES_CORNER(ctl)                        \
    do {                                                                      \
        QVERIFY(!(ctl)->selectionActive());                                   \
        QVariantMap color_;                                                   \
        color_.insert(QStringLiteral("model"), QStringLiteral("rgb"));        \
        color_.insert(QStringLiteral("comps"),                                \
                      QVariantList{0.2, 0.4, 0.6, 0.0});                      \
        QVariantMap fill_;                                                    \
        fill_.insert(QStringLiteral("content"), QStringLiteral("color"));     \
        fill_.insert(QStringLiteral("color"), color_);                        \
        QVariantMap target_;                                                  \
        target_.insert(QStringLiteral("layerId"),                             \
                       (ctl)->layers().first().toMap().value(                 \
                           QStringLiteral("layerId")));                       \
        QVERIFY((ctl)                                                         \
                    ->applyCommand(QStringLiteral("fill"), target_, fill_,    \
                                   {})                                        \
                    .value(QStringLiteral("ok"))                              \
                    .toBool());                                               \
        QImage corner_;                                                       \
        QVERIFY((ctl)->readRegionImage(QRect(0, 0, 1, 1), &corner_));         \
        const QColor c_ = corner_.pixelColor(0, 0);                           \
        QVERIFY2(c_ != QColor(Qt::white) && c_.blue() > c_.red(),             \
                 "post-deselect fill did not reach the corner OUTSIDE the "   \
                 "old partial selection — the mask survived the deselect");   \
    } while (false)

// Engine-truth ALPHA bounds of one layer through the 1.4 per-layer
// readback door (layerThumbImage: full-doc rect, content-not-
// contribution, straight-alpha RGBA8888). Deliberately test-local —
// independent of the shell's own entry-time derivation, so the G-3
// cases do not assert the fix against itself. Null rect = no opaque
// pixels OR a typed readback refusal (callers distinguish via
// composite probes where it matters).
QRect layerAlphaBounds(DocumentController *c, const QString &layerId)
{
    const QImage img = c->layerThumbImage(layerId);
    if (img.isNull())
        return QRect();
    Q_ASSERT(img.format() == QImage::Format_RGBA8888);
    QRect bounds;
    for (int y = 0; y < img.height(); ++y) {
        const uchar *line = img.constScanLine(y);
        int x0 = -1;
        int x1 = -1;
        for (int x = 0; x < img.width(); ++x) {
            if (line[x * 4 + 3]) {
                if (x0 < 0)
                    x0 = x;
                x1 = x;
            }
        }
        if (x0 >= 0) {
            const QRect row(x0, y, x1 - x0 + 1, 1);
            bounds = bounds.isNull() ? row : bounds.united(row);
        }
    }
    return bounds;
}

} // namespace

class TstSelectionGestures : public QObject {
    Q_OBJECT
private slots:
    void init();    // fresh doc per case
    void cleanup(); // deselect, drop state

    void rectMarqueeCommitsOnRelease();
    void ellipseCommitsOnRelease();
    void lassoAutoClosesOnRelease();
    void polygonClosesOnDoubleClick();
    void wandFiresAtClickPoint();
    // Tier-1 row 5: the engine forks on the flag — typed-pending vs
    // serve on a multi-raster document (the wire proof AND the shipped
    // capability in one case).
    void wandSampleAllLayersServesOnMultiRasterDocument();
    // G-1 (MAJOR): Esc mid-stroke must abandon the stroke.
    void brushEscapeMidStrokeDoesNotCommit();
    void brushFocusLossMidStrokeDoesNotCommit();
    void brushGrabLossMidStrokeDoesNotCommit();
    // G-1 round 5 (MINOR-1): the document VANISHING must disarm the
    // controller, not just clear the canvas flag.
    void documentVanishDisarmsControllerSession();
    // item 8: pen gesture commits a shape layer by mouse.
    void penClosesAndCommitsShapeLayer();
    void penFirstAnchorClickCloses();
    // item 9: the deselect paths (canvas-level; Cmd+D is Main.qml's
    // Shortcut — the command it fires is covered by deselectCommand).
    // item 10: drag layer B — layer A's pixels never move, in preview
    // frames or in the committed result.
    void movePreviewShiftsOnlyActiveLayer();
    // item 12: fit-and-center fires only on viewport overflow.
    void fitIfOverflowFitsOnlyWhenOverflowing();
    // item 13: commitTransform DECOMPOSES — translate-only commits via
    // the EXACT integer-translate arm (layer.transform, the move door);
    // scale/rotate/shear rides transform.free (SERVED — the §18.3 arms
    // landed in appname-tools, re-established at this build) with typed
    // refusals surfaced as a NAMED WALL (frame live, document unchanged,
    // wall queryable); aspect-lock default on corner scaling.
    void transformTranslateCommitsViaIntegerArm();
    void transformTranslateSnapsAndRoundsFractionalDrag();
    void transformScaleCommitsWhenArmServes();
    void transformScaleCommitWallsTyped();
    void transformAspectLockDefaultsAndShiftUnlocks();
    // §5 REVIEW (item 13, independent case — uncommitted): negative
    // fractional drags round HALF AWAY FROM ZERO (-60.5 → -61), and a
    // multi-segment, multi-GESTURE pure-translate chain holds the linear
    // cells at exactly 1/0 (exact ==, not fuzzy) with the integer deltas
    // summing exactly — preview == committed, engine-readback-discriminated
    // against ±1 rounding alternatives on both axes.
    void transformNegativeFractionalChainRoundsAwayFromZero();
    // G-3 CRITICAL (q-gesture QA finding, desk-routed): the perspective
    // session framed its quad at DOCUMENT bounds while the engine —
    // contract-correct (transform.perspective.yaml: "the homography is
    // solved from source bounds -> quad") — maps the LAYER'S SOURCE
    // BOUNDS onto the submitted quad, so any non-full-canvas layer
    // FLOODED across the document on commit (reported applied; silent-
    // wrong). The fix derives the active layer's source bounds at
    // session entry (one full-doc 1.4 readLayer + alpha scan) and
    // frames the quad THERE; a typed readback refusal (masked layer —
    // the cabi node-refusal matrix) WALLS the session instead of
    // opening blind (the item-13 wall pattern; the silent doc-bounds
    // fallback WAS the bug); an empty layer opens no session (queryable
    // wall, honest silence — no cataloged text exists, mint queued).
    void perspectiveCornerDragCommitsOnEnter();
    void perspectiveMildKeystoneStaysBounded();
    void perspectiveIdentityAndEscStayInert();
    void perspectiveMaskedLayerWallsTyped();
    void perspectiveEmptyLayerOpensNoSession();
    void perspectiveFullCanvasLayerDegenerate();
    // ---- v1.0.1 ITEM 19: the free-transform frame is the ACTIVE
    // LAYER'S CONTENT, not the canvas. Every case here discriminates
    // through a REAL GESTURE (does the handle grab? does the press land
    // inside the box?) rather than through the frame property, because
    // the property is shell echo and the gesture is what the user does.
    void transformFrameStartsAtLayerContentBounds();
    void transformFrameHandlesAreNotAtCanvasCorners();
    void transformFullCanvasLayerDegenerate();
    void transformCommitTargetsTheEntryLayer();
    void transformEmptyLayerOpensNoSessionAndSaysSo();
    void transformMaskedLayerFallsBackLabelled();
    // ---- v1.0.1 ITEM 21: the live preview, stacked on 19.
    void transformPreviewShowsOnlyTheActiveLayer();
    void transformPreviewMatchesTheCommit();
    void transformPreviewMaskedLayerShowsFrameOnly();
    void transformPreviewIssuesNoCommandsAndEscLeavesBitsAlone();
    void transformAboveTheReadbackCapFramesButSaysNoPreview();
    void deselectCommand();
    void escapeNeverDeselects();
    void escapeMidGestureCancelsWithoutDeselect();
    void marqueeClickOutsideDeselects();
    void modifierClickOutsideIsNoOp();
    // item 16: click-outside deselect beyond marquee-only. Ellipse was
    // already live but UNPINNED (triage); lasso is the built extension.
    void ellipseClickOutsideDeselects();
    void lassoClickOutsideDeselects();
    void lassoModifierClickOutsideIsNoOp();
    // item 15: the polygon's one real gap — first-vertex close.
    void polygonFirstVertexClickCloses();

private:
    // One window/controller for the suite; a fresh document per case.
    //
    // items 19+21: the suite catalog was EMPTY (pure key-echo), and
    // statusPending REFUSES an uncataloged id by design — so with an
    // empty catalog the honest-state notices could not be observed at
    // all, and "it said nothing" would have been indistinguishable from
    // "the arm never fired". Only the three ids those arms emit are
    // seeded, with DISTINCTIVE sentinel text: an assertion on the
    // sentinel proves the id resolved THROUGH the catalog (not
    // key-echoed) and proves WHICH id fired. `ui.status.rejected` is
    // deliberately left out so the existing perspective cases keep
    // asserting against its key-echo exactly as before. Drift between
    // these ids and the shipped catalogs is the i18n audit's job
    // (RED-1/RED-2), not this fixture's.
    Catalog m_catalog = Catalog::fromTexts(
        {{QStringLiteral("en"),
          QStringLiteral(
              "{\"ui.status.transform_layer_empty\":\"SENTINEL-EMPTY\","
              "\"ui.pending.transform_masked_layer\":\"SENTINEL-MASKED "
              "{code}\","
              "\"ui.pending.transform_preview_too_large\":\"SENTINEL-BIG\"}")}});
    DocumentController *m_controller = nullptr;
    QQuickWindow *m_window = nullptr;
    CanvasView *m_canvas = nullptr;

    void setTool(const char *tool)
    {
        m_canvas->setProperty("tool", QLatin1String(tool));
    }
    // The view transform after resetView: zoom 1 with the 640x400 doc
    // centered in the 800x600 view — offset ((800-640)/2, (600-400)/2).
    // setZoomCentered keeps the view-center doc point invariant and
    // zoomFit centers the doc, so this derivation is exact (guarded by
    // the zoomPercent==100 assert in init()).
    QPoint docToView(int x, int y) const { return QPoint(x + 80, y + 100); }
    void resetView();
    // item 13 cases: selection-scoped color fill on one layer (the same
    // steps the item-10 case composes inline; selection dropped after).
    void fillRectOnLayer(int layerIdx, const QRect &r, double red,
                         double green, double blue);
    // items 19/21: -1 on a refused add (callers QVERIFY the index).
    int addRasterLayer();
    void addRevealAllMask(int layerIdx);
    QString layerIdAt(int layerIdx) const;

    friend int main(int, char **);
    void bootOnce();
};

void TstSelectionGestures::bootOnce()
{
    m_controller = new DocumentController(&m_catalog);
    m_window = new QQuickWindow;
    m_window->resize(800, 600);
    m_canvas = new CanvasView(m_window->contentItem());
    m_canvas->setSize(QSizeF(800, 600));
    m_canvas->setController(m_controller);
    m_window->show();
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
}

void TstSelectionGestures::resetView()
{
    m_canvas->zoomOneToOne();
}

void TstSelectionGestures::init()
{
    if (!m_controller)
        bootOnce();
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    resetView();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);
    QVERIFY(!m_controller->selectionActive());
}

void TstSelectionGestures::cleanup()
{
    if (m_controller && m_controller->hasDocument())
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                                   {});
}

void TstSelectionGestures::rectMarqueeCommitsOnRelease()
{
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(200, 180));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 250));
    QVERIFY2(m_controller->selectionActive(),
             "drag-release on the rect marquee did not commit a selection");
    QCOMPARE(engineSelectionBounds(m_controller), QRect(100, 100, 200, 150));
}

void TstSelectionGestures::ellipseCommitsOnRelease()
{
    setTool("select.ellipse");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 250));
    QVERIFY2(m_controller->selectionActive(),
             "drag-release on the ellipse marquee did not commit");
    const QRect b = engineSelectionBounds(m_controller);
    // Ellipse rasterizes inside its bounding box; allow the engine an
    // antialias pixel each side but the box must be substantially there.
    QVERIFY2(QRect(98, 98, 205, 155).contains(b) && b.width() >= 195
                 && b.height() >= 145,
             qPrintable(QStringLiteral("ellipse bounds off: %1,%2 %3x%4")
                            .arg(b.x())
                            .arg(b.y())
                            .arg(b.width())
                            .arg(b.height())));
    // Review D5: a rect-rasterization regression (e.g. the source
    // ternary mislabeling) fits inside the bbox window — discriminate
    // by SHAPE: the bbox corner is OUTSIDE an ellipse (mask 0) while
    // the center is fully INSIDE (mask 255); a rect mask reads 255 at
    // both (reviewer-probed values, zero flake margin).
    QImage px;
    QVERIFY(m_controller->readSelectionImage(QRect(101, 101, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 0);
    QVERIFY(m_controller->readSelectionImage(QRect(200, 175, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 255);
}

void TstSelectionGestures::lassoAutoClosesOnRelease()
{
    setTool("select.lasso");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseMove(m_window, docToView(150, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 220));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral("lasso release did not auto-close and "
                                       "commit; status: %1")
                            .arg(m_controller->statusText())));
    const QRect b = engineSelectionBounds(m_controller);
    QVERIFY2(b.width() >= 150 && b.height() >= 100,
             "lasso committed but with degenerate bounds");
}

void TstSelectionGestures::polygonClosesOnDoubleClick()
{
    setTool("select.polygon");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 250));
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier,
                       docToView(200, 250));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral("polygon double-click did not close "
                                       "and commit; status: %1")
                            .arg(m_controller->statusText())));
    const QRect b = engineSelectionBounds(m_controller);
    QVERIFY2(b.width() >= 190 && b.height() >= 140,
             "polygon committed but with degenerate bounds");
}

void TstSelectionGestures::wandFiresAtClickPoint()
{
    setTool("select.wand");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(320, 200));
    QVERIFY2(m_controller->selectionActive(),
             "wand click did not fire a selection at the seed point");
    // Uniform white document: the flood covers the whole canvas.
    QCOMPARE(engineSelectionBounds(m_controller), QRect(0, 0, 640, 400));
}

// Tier-1 row 5 (C-666): the sample-all-layers toggle, discriminated by
// the engine's OWN fork — kind_impls.rs sampling_source: the false arm
// samples the active layer, and headless multi-raster documents pend
// TYPED on the tool-state row; the true arm resolves the flattened
// composite and needs NO active-layer state. So on a 2-raster document
// the same click refuses without the flag and serves with it — the flag
// must have RIDDEN the envelope for the engine to fork on it (the
// options-bar wire-spy idiom), and the behavioral fork is exactly the
// capability the row ships (a wand that answers on layered documents).
void TstSelectionGestures::wandSampleAllLayersServesOnMultiRasterDocument()
{
    QVERIFY(addRasterLayer() >= 0); // 2 raster layers: the old typed-pending shape
    setTool("select.wand");
    // 1.0 final D-8: there is no choice any more. With the shared canvas
    // property OFF (the state the removed checkbox left by default), one
    // click still serves from the composite: the wand always sends the
    // flag.
    m_canvas->setProperty("sampleAllLayers", false);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(320, 200));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral(
                            "multi-raster wand did not serve with no "
                            "Sample All choice; status: %1")
                            .arg(m_controller->statusText())));
    QCOMPARE(engineSelectionBounds(m_controller), QRect(0, 0, 640, 400));
}

void TstSelectionGestures::penClosesAndCommitsShapeLayer()
{
    m_canvas->setPenCommitsShape(true);
    setTool("vector.pen");
    const int layersBefore = m_controller->layers().size();
    // Three corner anchors (click = corner), double-click closes (§20.1).
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 120));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 260));
    QTest::mouseDClick(m_window, Qt::LeftButton, Qt::NoModifier,
                       docToView(200, 260));
    const QVariantList layers = m_controller->layers();
    QVERIFY2(layers.size() == layersBefore + 1,
             qPrintable(QStringLiteral("pen close did not add a shape layer "
                                       "(%1 -> %2); status: %3")
                            .arg(layersBefore)
                            .arg(layers.size())
                            .arg(m_controller->statusText())));
}

void TstSelectionGestures::penFirstAnchorClickCloses()
{
    m_canvas->setPenCommitsShape(true);
    setTool("vector.pen");
    const int layersBefore = m_controller->layers().size();
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 120));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 260));
    // Close by clicking the FIRST anchor (within the 8px/zoom tolerance).
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(101, 101));
    QVERIFY2(m_controller->layers().size() == layersBefore + 1,
             qPrintable(QStringLiteral("first-anchor click did not close "
                                       "and commit; status: %1")
                            .arg(m_controller->statusText())));
}

void TstSelectionGestures::movePreviewShiftsOnlyActiveLayer()
{
    // Layer A (background): a dark region around the probe point PA.
    auto fillRectOn = [&](int layerIdx, const QRect &r, double red,
                          double green, double blue) {
        m_controller->setActiveLayerIndex(layerIdx);
        QVariantMap rect;
        rect.insert(QStringLiteral("x"), r.x());
        rect.insert(QStringLiteral("y"), r.y());
        rect.insert(QStringLiteral("w"), r.width());
        rect.insert(QStringLiteral("h"), r.height());
        QVariantMap sel;
        sel.insert(QStringLiteral("source"), QStringLiteral("rect"));
        sel.insert(QStringLiteral("rect"), rect);
        m_controller->applyCommand(QStringLiteral("selection.set"), {}, sel,
                                   {});
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"),
                     QVariantList{red, green, blue, 0.0});
        QVariantMap fill;
        fill.insert(QStringLiteral("content"), QStringLiteral("color"));
        fill.insert(QStringLiteral("color"), color);
        QVariantMap target;
        target.insert(QStringLiteral("layerId"),
                      m_controller->layers().at(layerIdx).toMap().value(
                          QStringLiteral("layerId")));
        QVERIFY(m_controller
                    ->applyCommand(QStringLiteral("fill"), target, fill, {})
                    .value(QStringLiteral("ok"))
                    .toBool());
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                                   {});
    };
    fillRectOn(0, QRect(20, 260, 120, 120), 0.1, 0.1, 0.5); // A: dark blue
    // Layer B above, with a red block the drag will move.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    const int bIdx = m_controller->layers().size() - 1;
    fillRectOn(bIdx, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1); // B: red
    m_controller->setActiveLayerIndex(bIdx);

    // PA = inside A's dark region AND covered by a (80,60)-shifted
    // composite whose source pixel is WHITE (outside A): under the OLD
    // whole-composite echo, white blends over dark at PA (discriminating
    // — mutation-verified); under the layer-only echo, B's image is
    // transparent at PA's offset source. Geometry: A = (20,260,120x120);
    // PA=(90,330): inside A; source (10,270) outside A = white.
    const QPoint paView = docToView(90, 330);
    const QImage before = m_window->grabWindow();
    const QRgb paBefore = before.pixel(paView);

    // Drag B by (+150, +120) with the move tool; grab MID-DRAG.
    setTool("move");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(350, 100));
    QTest::mouseMove(m_window, docToView(430, 160));
    const QImage midDrag = m_window->grabWindow();
    // The OLD whole-composite echo blended shifted WHITE over PA at 0.6
    // opacity (discriminating by construction); the layer-only echo
    // leaves PA untouched.
    QVERIFY2(midDrag.pixel(paView) == paBefore,
             "layer A's pixels moved/changed in a preview frame — the "
             "whole-composite echo is back (item 10)");
    QTest::mouseMove(m_window, docToView(500, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(500, 220));

    // Committed result, engine truth: A's region unchanged; B moved.
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(90, 330, 1, 1), &px));
    const QColor a = px.pixelColor(0, 0);
    QVERIFY2(a.blue() > a.red(),
             "layer A's pixels changed in the COMMITTED result");
    QVERIFY(m_controller->readRegionImage(QRect(500, 220, 1, 1), &px));
    const QColor bNew = px.pixelColor(0, 0);
    QVERIFY2(bNew.red() > bNew.blue(),
             "layer B did not land at the drag destination");
    QVERIFY(m_controller->readRegionImage(QRect(350, 100, 1, 1), &px));
    const QColor bOld = px.pixelColor(0, 0);
    QVERIFY2(bOld != QColor(bNew),
             "layer B still renders at its origin post-commit");
}

void TstSelectionGestures::fitIfOverflowFitsOnlyWhenOverflowing()
{
    // RECORDED OMISSION (§5 review D3, the item-9 precedent): these
    // arms exercise CanvasView::fitIfOverflow() directly; the four
    // Main.qml call sites (drop, import dialog, paste menu, paste
    // shortcut — each gated on the operation's success return) are QML
    // glue no harness drives (the tour calls controller methods
    // directly). A glue-level regression ships green here; the gap
    // closes when a Main.qml-driving harness lands (the 11e/KS work is
    // the standing candidate).
    //
    // PARTIALLY CLOSED 2026-08-19 (recorded here 2026-09-03, 15 days
    // late). The harness landed the same day this omission was written:
    // 75087091 17:11 recorded it, 470ef31f 22:50 closed it, and that
    // commit's own subject says so. tst_gesture_chrome's
    // pasteMenuGlueFitsOnOverflow now drives pasteAndFit() for real.
    //
    // ⛔ AND THE SENTENCE THAT SAT HERE CARRIED A COUNT, SO IT WENT STALE IN
    // TWO DAYS -- BY THE HAND THAT WROTE IT. It read "ONE of the four sites,
    // not four: pasteIntoSelection(), the drop handler and the import dialog
    // are still undriven." On 2026-09-03 pasteIntoSelection() GOT coverage
    // (tst_gesture_chrome::pasteIntoMenuGlueFitsOnOverflow, mutation-verified),
    // which made this comment wrong about its own subject one commit later.
    // ★ So it is replaced by a DERIVATION rather than a corrected number,
    //   because the number will decay again the moment site 3 or 4 lands:
    //
    //     /usr/bin/grep -n fitIfOverflow shell/qml/Main.qml     # the sites
    //     /usr/bin/grep -rln fitIfOverflow shell/tests/         # who drives them
    //
    //   Read the second list's cases and see which SITE each one reaches --
    //   a harness that calls CanvasView::fitIfOverflow() directly (as the
    //   arms below do) is not driving any Main.qml site at all, and that
    //   distinction is the whole subject of this omission.
    // ★★ The general rule is §7's: carry the predicate, never the value. A
    //   count in a comment is a liability with a fuse, and this one's fuse
    //   was two days long.
    // Note the four DOORS named above are not the four CALL SITES:
    // pasteAndFit() serves both the Edit menu and the canvas context
    // menu, and no Shortcut calls fitIfOverflow at all.
    //
    // Overflow arm: at 400% the 640x400 doc renders 2560x1600 in the
    // 800x600 view — fitIfOverflow must zoomFit. zoomFit fits with its
    // 24px margin: (800-24)/640 = 1.2125 → 121% (the first cut of this
    // pin assumed marginless 125% and shipped red — corrected).
    m_canvas->setZoomCentered(4.0);
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 400);
    m_canvas->fitIfOverflow();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 121);
    // No-overflow arm at a NON-fit-point zoom (§5 review D4: at 121%
    // this arm was vacuous — a spurious zoomFit re-lands on the same
    // 121, so the always-fit mutation passed it): at 110% the doc
    // renders 704x440 in 800x600 — no overflow, and a spurious fit
    // would move the zoom to 121. Must hold 110.
    m_canvas->setZoomCentered(1.1);
    m_canvas->fitIfOverflow();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 110);
    // And a zoomed-OUT view (content smaller than the viewport) must
    // not be yanked to fit either.
    m_canvas->setZoomCentered(0.5);
    m_canvas->fitIfOverflow();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 50);
    // §5 review arms (item-12): the two gates the landed net left
    // undiscriminated — the `>=` and `&&` mutations both survived the
    // full suite (review D1/D2).
    // Borderline-EQUAL: at exactly 125% the doc width renders
    // 640*1.25 = 800 == the 800px view (exact in binary FP) — equal is
    // NOT "exceeds", so the zoom must hold. This is the arm 742e989's
    // message advertised; d988de1's pin correction (121%, w=776<800)
    // silently stopped exercising the boundary.
    m_canvas->setZoomCentered(1.25);
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 125);
    m_canvas->fitIfOverflow();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 125);
    // One-axis overflow: at 140% only the width overflows (896 > 800;
    // height 560 < 600) — must still fit (the || gate; an && mutation
    // holds it at 140).
    m_canvas->setZoomCentered(1.4);
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 140);
    m_canvas->fitIfOverflow();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 121);
}

int TstSelectionGestures::addRasterLayer()
{
    if (!m_controller
             ->applyCommand(QStringLiteral("layer.add"), {},
                            QVariantMap{{QStringLiteral("kind"),
                                         QStringLiteral("raster")},
                                        {QStringLiteral("newLayerId"),
                                         m_controller->newUuid()}},
                            {})
             .value(QStringLiteral("ok"))
             .toBool())
        return -1;
    return m_controller->layers().size() - 1;
}

void TstSelectionGestures::addRevealAllMask(int layerIdx)
{
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerIdAt(layerIdx));
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("mask.add"), target,
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("from"),
                                            QStringLiteral("reveal_all")},
                                           {QStringLiteral("newMaskId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
}

QString TstSelectionGestures::layerIdAt(int layerIdx) const
{
    return m_controller->layers()
        .at(layerIdx)
        .toMap()
        .value(QStringLiteral("layerId"))
        .toString();
}

void TstSelectionGestures::fillRectOnLayer(int layerIdx, const QRect &r,
                                           double red, double green,
                                           double blue)
{
    m_controller->setActiveLayerIndex(layerIdx);
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), r.x());
    rect.insert(QStringLiteral("y"), r.y());
    rect.insert(QStringLiteral("w"), r.width());
    rect.insert(QStringLiteral("h"), r.height());
    QVariantMap sel;
    sel.insert(QStringLiteral("source"), QStringLiteral("rect"));
    sel.insert(QStringLiteral("rect"), rect);
    m_controller->applyCommand(QStringLiteral("selection.set"), {}, sel, {});
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{red, green, blue, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().at(layerIdx).toMap().value(
                      QStringLiteral("layerId")));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok"))
                .toBool());
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}

void TstSelectionGestures::transformTranslateCommitsViaIntegerArm()
{
    // Two-layer discriminating geometry (the item-10 audit discipline):
    // A (background, dark-blue block) must NEVER move; B (active, red
    // block) must land at the destination with its origin vacated. The
    // OLD behavior — the raw accumulated matrix submitted via
    // transform.free — refuses NI and moves NOTHING, so BOTH content
    // probes red under it (mutation-verified: neutering the decompose→
    // integer-arm leg reds this case).
    fillRectOnLayer(0, QRect(20, 260, 120, 120), 0.1, 0.1, 0.5); // A block
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    const int bIdx = m_controller->layers().size() - 1;
    fillRectOnLayer(bIdx, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1); // B block
    m_controller->setActiveLayerIndex(bIdx);

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    // Drag INSIDE the frame (no handle within 8px, no rotate ring) by a
    // known integer offset (+150, +120).
    //
    // ITEM 19 MOVED THIS PRESS POINT, deliberately, and the reason is
    // the item itself. The press was doc (320,200), which sat inside the
    // old CANVAS-shaped frame but 60 px BELOW layer B's actual content
    // (300,60,100x80). Under a layer-bounded frame that press lands on
    // nothing, the drag produces no gesture, and Enter drops an identity
    // matrix — so this case stopped committing. The delta is unchanged
    // at (+150,+120) and every probe below is unchanged; only the grab
    // point moved, to B's own centre. Sibling cases needed no change
    // because they fill layer 0, whose content IS the full canvas.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(350, 100));
    QTest::mouseMove(m_window, docToView(500, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(500, 220));
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "the frame must persist after a translate drag (commit is "
             "Enter/double-click)");
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    // Frame closed, no wall: the translate-only matrix rode the SERVED
    // integer-translate arm (layer.transform), engine-applied.
    QVERIFY2(!m_canvas->property("transformActive").toBool(),
             qPrintable(QStringLiteral(
                            "frame did not close on a translate commit — the "
                            "decompose→integer-arm leg is broken; status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY(!m_canvas->property("transformWallActive").toBool());
    QVERIFY2(m_controller->statusText().contains(
                 QStringLiteral("ui.status.applied")),
             qPrintable(m_controller->statusText()));
    // DOOR PIN (engine journal truth): the translate-only class must
    // ride layer.transform — the §18.3 EXACT integer-translate arm —
    // never the resampling transform.free door. With the §18.3 arms
    // serving, both doors move pixels; the journal kind is what
    // discriminates the decomposition (the neuter mutation reds here).
    const QVariantList hist = m_controller->historyEntries();
    QVERIFY(!hist.isEmpty());
    QCOMPARE(hist.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("layer.transform"));

    // Engine readback truth. B's block (300,60,100x80)+(150,120) →
    // (450,180,100x80): destination red, origin vacated to the white
    // background, A's region untouched.
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(500, 220, 1, 1), &px));
    const QColor dest = px.pixelColor(0, 0);
    QVERIFY2(dest.red() > dest.blue(),
             "B's content did not land at the drag destination");
    QVERIFY(m_controller->readRegionImage(QRect(350, 100, 1, 1), &px));
    const QColor origin = px.pixelColor(0, 0);
    QVERIFY2(origin.red() == origin.green() && origin.green() == origin.blue(),
             "B's origin was not vacated — the transform did not move the "
             "layer's pixels");
    QVERIFY(m_controller->readRegionImage(QRect(90, 330, 1, 1), &px));
    const QColor a = px.pixelColor(0, 0);
    QVERIFY2(a.blue() > a.red(),
             "layer A moved — the commit must target the ACTIVE layer only");
}

void TstSelectionGestures::transformTranslateSnapsAndRoundsFractionalDrag()
{
    // The recorded rounding policy: fractional drag translation rounds
    // to integers BEFORE submission (qRound — half away from zero), and
    // the PREVIEW must not promise sub-pixel placement (the drag delta
    // snaps live). At 200% zoom an odd view-pixel drag is a fractional
    // doc delta: 301 view px = 150.5 doc px → previews AND commits 151.
    fillRectOnLayer(0, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    setTool("transform.free");
    m_canvas->setZoomCentered(2.0);
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 200);
    // View mapping at 200%: offset = (400,300) - (320,200)*2 = (-240,-100)
    // → view = (2x-240, 2y-100). Doc (320,200) = view (400,300).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      QPoint(400, 300));
    QTest::mouseMove(m_window, QPoint(701, 300)); // doc (470.5, 200)
    const QVariantList m = m_canvas->transformPreviewMatrix();
    QCOMPARE(m.at(6).toDouble(), 151.0); // snapped, not 150.5 (qRound up)
    QCOMPARE(m.at(7).toDouble(), 0.0);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        QPoint(701, 300));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QVERIFY2(!m_canvas->property("transformActive").toBool(),
             qPrintable(QStringLiteral("fractional-drag commit failed — a "
                                       "non-integer matrix reached the "
                                       "engine? status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY(!m_canvas->property("transformWallActive").toBool());
    // Engine truth discriminates 151 from 150: block lands at x 451..550.
    // (450,100): source pixel 450-151=299 is white — under a truncation
    // (150) the source would be 300 = red.
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(450, 100, 1, 1), &px));
    const QColor at450 = px.pixelColor(0, 0);
    QVERIFY2(at450.red() == at450.green() && at450.green() == at450.blue(),
             "content at x=450 — the 150.5 drag rounded DOWN (policy is "
             "qRound: half away from zero → 151)");
    QVERIFY(m_controller->readRegionImage(QRect(460, 100, 1, 1), &px));
    const QColor at460 = px.pixelColor(0, 0);
    QVERIFY2(at460.red() > at460.blue(),
             "block did not land at the rounded destination (x 451..550)");
}

void TstSelectionGestures::transformScaleCommitsWhenArmServes()
{
    // The wall's flip side, pinned live: the §18.3 resample arms are
    // SERVED on this engine (appname-tools TransformFreeKind — ground
    // re-established at this build; the stale arms-are-NI note in the
    // item brief is corrected of record), so an UNLOCKED scale commit
    // rides transform.free, the engine BAKES, and the ok branch closes
    // the frame with no wall. Geometry: block (300,60,100x80) scaled
    // 1.1 about the (0,0) anchor lands at (330,66,110x88) — probe
    // (430,100) flips white→red (source ≈ (390.9,90.9), deep inside the
    // block: no kernel edge effects), probe (310,100) flips red→white
    // (source ≈ (281.8,90.9), deep outside).
    fillRectOnLayer(0, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    setTool("transform.free");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(704, 410)); // locked scale 1.1
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(704, 410));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    QVERIFY2(!m_canvas->property("transformActive").toBool(),
             qPrintable(QStringLiteral("served scale commit did not close "
                                       "the frame; status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY(!m_canvas->property("transformWallActive").toBool());
    QVERIFY2(m_controller->statusText().contains(
                 QStringLiteral("ui.status.applied")),
             qPrintable(m_controller->statusText()));
    // DOOR PIN: the scale class rides transform.free (engine journal).
    const QVariantList hist = m_controller->historyEntries();
    QVERIFY(!hist.isEmpty());
    QCOMPARE(hist.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("transform.free"));
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(430, 100, 1, 1), &px));
    const QColor grew = px.pixelColor(0, 0);
    QVERIFY2(grew.red() > grew.blue(),
             "the baked 1.1 scale did not reach (430,100)");
    QVERIFY(m_controller->readRegionImage(QRect(310, 100, 1, 1), &px));
    const QColor vacated = px.pixelColor(0, 0);
    QVERIFY2(vacated.red() == vacated.green()
                 && vacated.green() == vacated.blue(),
             "(310,100) still holds block content — the scale did not bake");
}

void TstSelectionGestures::transformScaleCommitWallsTyped()
{
    // The NAMED WALL (the availability-truth REFUSED half): a commit
    // the engine refuses TYPED must (1) leave the document UNCHANGED in
    // engine readback, (2) keep the frame LIVE (the user's work is not
    // thrown away), (3) fire the queryable wall state with the engine's
    // code, (4) reach the status line through the existing refusal
    // path. With the §18.3 arms serving, the reachable refusal class
    // driven here is the §11.1 POSITION lock (write-only pre-info-v2 —
    // exactly the class the availability design says MUST surface as
    // the engine's refusal, never a pre-computed disable).
    // Mutation-verified: making the refusal branch close the frame
    // (silently dropping the transform) reds this case.
    fillRectOnLayer(0, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().at(0).toMap().value(
                      QStringLiteral("layerId")));
    QVariantMap locks;
    locks.insert(QStringLiteral("position"), true);
    QVariantMap lockParams;
    lockParams.insert(QStringLiteral("locks"), locks);
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.set_props"), target,
                               lockParams, {})
                .value(QStringLiteral("ok"))
                .toBool());

    setTool("transform.free");
    // Corner handle 4 (bottom-right, doc 640,400), anchor = opposite
    // corner (0,0): drag to (704,410) → aspect-locked scale 1.1.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(704, 410));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(704, 410));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "scale commit CLOSED the frame — the wall must keep the "
             "user's work live");
    QVERIFY2(m_canvas->property("transformWallActive").toBool(),
             "wall state did not fire on the engine's typed refusal");
    QCOMPARE(m_canvas->property("transformWallCode").toString(),
             QStringLiteral("error.command.locked"));
    QVERIFY2(m_controller->statusText().contains(
                 QStringLiteral("error.command.locked")),
             qPrintable(QStringLiteral("the refusal did not reach the status "
                                       "surface; status: %1")
                            .arg(m_controller->statusText())));

    // Document unchanged — engine truth: the block is still at its
    // origin and the point the 1.1 scale WOULD have covered is still
    // white ((430,100): red only if a bake happened — the serves-case
    // proves that probe flips when one does).
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(310, 100, 1, 1), &px));
    const QColor origin = px.pixelColor(0, 0);
    QVERIFY2(origin.red() > origin.blue(),
             "document CHANGED under a walled scale commit");
    QVERIFY(m_controller->readRegionImage(QRect(430, 100, 1, 1), &px));
    const QColor scaledReach = px.pixelColor(0, 0);
    QVERIFY2(scaledReach.red() == scaledReach.green()
                 && scaledReach.green() == scaledReach.blue(),
             "content appeared where the refused scale would have put it");

    // The wall names the LATEST attempt on the live frame: adjusting the
    // frame clears it; Esc then cancels frame AND wall.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(320, 200));
    QTest::mouseMove(m_window, docToView(321, 200));
    QVERIFY2(!m_canvas->property("transformWallActive").toBool(),
             "adjusting the frame must clear the stale wall");
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(321, 200));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QVERIFY(!m_canvas->property("transformActive").toBool());
    QVERIFY(m_controller->readRegionImage(QRect(350, 100, 1, 1), &px));
    const QColor after = px.pixelColor(0, 0);
    QVERIFY2(after.red() > after.blue(),
             "cancel after a wall submitted something");
}

void TstSelectionGestures::transformAspectLockDefaultsAndShiftUnlocks()
{
    // Preview-state assertions with mechanically-derived numbers (the
    // matrix readback is the same wire-order convention the commit
    // submits). Corner 4 drag, anchor (0,0): doc (640,400) → (704,410)
    // gives sx=704/640=1.1, sy=410/400=1.025.
    setTool("transform.free");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    // LOCKED default: dominant magnitude 1.1 on BOTH axes.
    QTest::mouseMove(m_window, docToView(704, 410));
    QVariantList m = m_canvas->transformPreviewMatrix();
    QCOMPARE(m.at(0).toDouble(), 704.0 / 640.0);
    QCOMPARE(m.at(4).toDouble(), 704.0 / 640.0);
    QCOMPARE(m.at(1).toDouble(), 0.0);
    QCOMPARE(m.at(3).toDouble(), 0.0);
    QCOMPARE(m.at(6).toDouble(), 0.0); // anchor at origin: no translation
    QCOMPARE(m.at(7).toDouble(), 0.0);
    // LOCKED flip: (704,-40) → sy raw -0.1 — uniform magnitude, per-axis
    // sign PRESERVED (the old collapsed-sign rule yields +1.1 here).
    QTest::mouseMove(m_window, docToView(704, -40));
    m = m_canvas->transformPreviewMatrix();
    QCOMPARE(m.at(0).toDouble(), 704.0 / 640.0);
    QCOMPARE(m.at(4).toDouble(), -(704.0 / 640.0));
    // Shift UNLOCKS free scaling: back to (704,410) with Shift held —
    // axes independent (1.1 × 1.025).
    QTest::mouseEvent(QTest::MouseMove, m_window, Qt::NoButton,
                      Qt::ShiftModifier, docToView(704, 410));
    m = m_canvas->transformPreviewMatrix();
    QCOMPARE(m.at(0).toDouble(), 704.0 / 640.0);
    QCOMPARE(m.at(4).toDouble(), 410.0 / 400.0);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(704, 410));

    // EDGE handle: one axis by geometry, modifier IGNORED (drag WITH
    // Shift and the off-axis must still hold). Right edge (base 640,200)
    // maps to (704,205) under the accumulated scale(1.1, 1.025); drag to
    // doc x=563 → incremental sx = 563/704 on top of 1.1; sy unchanged.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(704, 205));
    QTest::mouseEvent(QTest::MouseMove, m_window, Qt::NoButton,
                      Qt::ShiftModifier, docToView(563, 205));
    m = m_canvas->transformPreviewMatrix();
    QCOMPARE(m.at(0).toDouble(), (704.0 / 640.0) * (563.0 / 704.0));
    QCOMPARE(m.at(4).toDouble(), 410.0 / 400.0);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(563, 205));
    // Nothing committed anywhere above: cancel drops the frame.
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QVERIFY(!m_canvas->property("transformActive").toBool());
}

void TstSelectionGestures::transformNegativeFractionalChainRoundsAwayFromZero()
{
    // §5 REVIEW independent case (item 13). Derivation (mechanical):
    // at 200% (setZoomCentered(2.0) from the 100% ground: offset
    // (400,300) - 2*(320,200) = (-240,-100), so doc = ((vx+240)/2,
    // (vy+100)/2)):
    //   gesture 1: press view (400,300) = doc (320,200); end view
    //   (279,279) = doc (259.5,189.5) → d = (-60.5,-10.5) → qRound
    //   half AWAY from zero → (-61,-11) (truncation gives (-60,-10),
    //   round-half-even gives -60 for -60.5).
    //   gesture 2: press view (400,300) = doc (320,200) (inside the
    //   moved frame: local (381,211) ∈ base, far from every mapped
    //   handle); end view (359,300) = doc (299.5,200) → d = (-20.5,0)
    //   → (-21,0).
    // Chain: T(-61,-11) * T(-21,0) — linear cells must stay EXACTLY
    // 1/0 (products/sums of exact FP values), deltas sum exactly:
    // (-82,-11). Block (300,60,100x80) → x 218..317, y 49..128.
    fillRectOnLayer(0, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    setTool("transform.free");
    m_canvas->setZoomCentered(2.0);
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 200);

    // Gesture 1 — multi-segment drag (intermediate moves recompute the
    // preview; only the last one stands).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      QPoint(400, 300));
    QTest::mouseMove(m_window, QPoint(450, 320));
    QTest::mouseMove(m_window, QPoint(371, 288)); // fractional midpoints
    QTest::mouseMove(m_window, QPoint(305, 341));
    QTest::mouseMove(m_window, QPoint(279, 279)); // doc (259.5, 189.5)
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        QPoint(279, 279));
    QVariantList m = m_canvas->transformPreviewMatrix();
    QVERIFY2(m.at(6).toDouble() == -61.0,
             qPrintable(QStringLiteral("gesture-1 dx: %1 (want exactly -61 — "
                                       "-60.5 must round AWAY from zero)")
                            .arg(m.at(6).toDouble(), 0, 'g', 17)));
    QVERIFY(m.at(7).toDouble() == -11.0);

    // Gesture 2 — second chain link, negative half again.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      QPoint(400, 300));
    QTest::mouseMove(m_window, QPoint(383, 306));
    QTest::mouseMove(m_window, QPoint(359, 300)); // doc (299.5, 200)
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        QPoint(359, 300));
    m = m_canvas->transformPreviewMatrix();
    // Linear cells EXACTLY 1/0 across the chain (exact ==, no fuzz):
    // the translate class must hold with zero epsilon consumed.
    QVERIFY2(m.at(0).toDouble() == 1.0 && m.at(4).toDouble() == 1.0
                 && m.at(1).toDouble() == 0.0 && m.at(3).toDouble() == 0.0,
             "pure-translate chain drifted the linear cells off exact 1/0");
    QVERIFY2(m.at(6).toDouble() == -82.0 && m.at(7).toDouble() == -11.0,
             qPrintable(QStringLiteral("chained deltas not exact: (%1, %2), "
                                       "want (-82, -11)")
                            .arg(m.at(6).toDouble(), 0, 'g', 17)
                            .arg(m.at(7).toDouble(), 0, 'g', 17)));

    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QVERIFY2(!m_canvas->property("transformActive").toBool(),
             qPrintable(m_controller->statusText()));
    QVERIFY(!m_canvas->property("transformWallActive").toBool());
    const QVariantList hist = m_controller->historyEntries();
    QVERIFY(!hist.isEmpty());
    QCOMPARE(hist.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("layer.transform"));

    // Engine truth, discriminated against the ±1 alternatives:
    //   dx=-82 → x 218..317 ((218) red / (318) white; under -81 both flip,
    //   under truncation -80 both flip);
    //   dy=-11 → y 49..128 ((230,49) red / (230,129) white; under -10
    //   both flip).
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(218, 89, 1, 1), &px));
    const QColor left = px.pixelColor(0, 0);
    QVERIFY2(left.red() > left.blue(),
             "(218,89) white — dx rounded toward zero (-81/-80), not -82");
    QVERIFY(m_controller->readRegionImage(QRect(318, 89, 1, 1), &px));
    const QColor right = px.pixelColor(0, 0);
    QVERIFY2(right.red() == right.green() && right.green() == right.blue(),
             "(318,89) has content — block landed right of the -82 promise");
    QVERIFY(m_controller->readRegionImage(QRect(230, 49, 1, 1), &px));
    const QColor top = px.pixelColor(0, 0);
    QVERIFY2(top.red() > top.blue(),
             "(230,49) white — dy rounded toward zero (-10), not -11");
    QVERIFY(m_controller->readRegionImage(QRect(230, 129, 1, 1), &px));
    const QColor below = px.pixelColor(0, 0);
    QVERIFY2(below.red() == below.green() && below.green() == below.blue(),
             "(230,129) has content — block landed below the -11 promise");
}

void TstSelectionGestures::perspectiveCornerDragCommitsOnEnter()
{
    // THE G-3 RED CASE (the finding's exact repro): a 240x120 block on
    // its own layer, TL corner dragged (0,0)->(120,90), Enter. Under
    // the doc-bounds frame the engine mapped the LAYER's source bounds
    // onto the dragged DOC quad and the block flooded to the whole
    // 640x400 canvas (reported applied — silent-wrong). Engine-truth
    // assertion through the per-layer readback door: the block must
    // NOT flood.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    const int idx = m_controller->layers().size() - 1;
    fillRectOnLayer(idx, QRect(200, 140, 240, 120), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString layerId = m_controller->layers().at(idx).toMap()
                                .value(QStringLiteral("layerId")).toString();
    QCOMPARE(layerAlphaBounds(m_controller, layerId),
             QRect(200, 140, 240, 120));

    const int histBefore = m_controller->historyEntries().size();
    setTool("transform.perspective");
    // The finding's drag: press at doc (0,0) — under the FIXED frame no
    // handle lives there (they sit on the LAYER's corners), so this
    // exact gesture is inert and Enter drops an identity session; under
    // the doc-bounds frame it grabs the TL handle and commits the
    // flood.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(120, 90));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(120, 90));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    const QRect after = layerAlphaBounds(m_controller, layerId);
    QVERIFY2(after.width() <= 240 && after.height() <= 120,
             qPrintable(QStringLiteral(
                            "G-3 FLOOD: the 240x120 block grew to %1x%2 — the "
                            "doc-space quad mapped the layer's source bounds "
                            "across the document")
                            .arg(after.width())
                            .arg(after.height())));
    QCOMPARE(after, QRect(200, 140, 240, 120));
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
}

void TstSelectionGestures::perspectiveMildKeystoneStaysBounded()
{
    // The finding's second signature: a 40x40 marker under a mild
    // keystone flooded to ~629x393. Part 1 replays that gesture (10%
    // of the DOC dragged at the doc TL — inert under the fixed frame);
    // part 2 drives a REAL 10%-of-the-marker keystone on the fixed
    // frame's WYSIWYG handles and pins the bounded, applied result.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    const int idx = m_controller->layers().size() - 1;
    fillRectOnLayer(idx, QRect(300, 180, 40, 40), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString layerId = m_controller->layers().at(idx).toMap()
                                .value(QStringLiteral("layerId")).toString();

    // Part 1 — the doc-corner keystone (finding repro).
    const int histBefore = m_controller->historyEntries().size();
    setTool("transform.perspective");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(64, 40)); // 10% of 640x400
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(64, 40));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect part1 = layerAlphaBounds(m_controller, layerId);
    QVERIFY2(part1.width() <= 40 && part1.height() <= 40,
             qPrintable(QStringLiteral("G-3 FLOOD: 40x40 marker grew to "
                                       "%1x%2 under a mild keystone")
                            .arg(part1.width())
                            .arg(part1.height())));
    QCOMPARE(part1, QRect(300, 180, 40, 40));
    QCOMPARE(m_controller->historyEntries().size(), histBefore);

    // Part 2 — the WYSIWYG keystone: re-arm (same-tool re-selection),
    // grab the LAYER's TL handle, pull it inward by 4px (10% of 40).
    // Mechanical expectation (engine operators, read first-hand):
    // the homography maps source bounds (300,180)-(340,220) onto
    // ((304,184),(340,180),(340,220),(300,220)) — quad bbox = the same
    // 40x40 rect; the engine's transformed_bounds then pads the
    // destination rect by floor-1/ceil+1 (xform.rs, its own operator)
    // and the DLT corner round-trip carries sub-ulp slop that can move
    // the floor one more pixel, so kernel-tap trace alpha may reach 2px
    // past the bbox on each side — the DERIVED containment bound is
    // bbox.adjusted(-2,-2,2,2), which still discriminates the flood
    // (640x400) by an order of magnitude. transform.perspective in the
    // journal proves the commit door.
    setTool("transform.perspective");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 180));
    QTest::mouseMove(m_window, docToView(304, 184));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(304, 184));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    const QVariantList hist = m_controller->historyEntries();
    QVERIFY(!hist.isEmpty());
    QCOMPARE(hist.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("transform.perspective"));
    const QRect part2 = layerAlphaBounds(m_controller, layerId);
    QVERIFY2(!part2.isNull(), "keystone commit erased the marker");
    QVERIFY2(QRect(300, 180, 40, 40).adjusted(-2, -2, 2, 2).contains(part2),
             qPrintable(QStringLiteral("keystone result escaped the source "
                                       "bounds: %1,%2 %3x%4")
                            .arg(part2.x())
                            .arg(part2.y())
                            .arg(part2.width())
                            .arg(part2.height())));
    const QImage layer = m_controller->layerThumbImage(layerId);
    QVERIFY(!layer.isNull());
    // Old TL corner pixel: outside the pulled-in quad -> transparent
    // (was 255); marker center: deep inside -> stays opaque.
    QVERIFY2(qAlpha(layer.pixelColor(300, 180).rgba()) < 128,
             "the TL keystone pull did not vacate the old corner — the "
             "commit did not map source bounds onto the dragged quad");
    QVERIFY2(qAlpha(layer.pixelColor(320, 200).rgba()) > 200,
             "marker interior vanished under a mild keystone");
}

void TstSelectionGestures::perspectiveIdentityAndEscStayInert()
{
    // STAY-GREEN pins: identity (no drag) commits NOTHING on Enter
    // (the existing identity drop, now bounds->bounds by construction)
    // and Esc cancels the session without touching the document.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    const int idx = m_controller->layers().size() - 1;
    fillRectOnLayer(idx, QRect(300, 180, 40, 40), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString layerId = m_controller->layers().at(idx).toMap()
                                .value(QStringLiteral("layerId")).toString();
    const int histBefore = m_controller->historyEntries().size();

    setTool("transform.perspective");
    QVERIFY2(m_canvas->property("perspectiveActive").toBool(),
             "session did not open on a plain filled layer");
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return); // identity -> no commit
    QVERIFY(!m_canvas->property("perspectiveActive").toBool());
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
    QCOMPARE(layerAlphaBounds(m_controller, layerId), QRect(300, 180, 40, 40));

    setTool("transform.perspective"); // re-arm
    QVERIFY(m_canvas->property("perspectiveActive").toBool());
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 180));
    QTest::mouseMove(m_window, docToView(310, 190));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(310, 190));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape); // cancel, not commit
    QVERIFY(!m_canvas->property("perspectiveActive").toBool());
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
    QCOMPARE(layerAlphaBounds(m_controller, layerId), QRect(300, 180, 40, 40));
}

void TstSelectionGestures::perspectiveMaskedLayerWallsTyped()
{
    // The F3b class: the 1.4 readback door REFUSES TYPED for masked
    // layers (cabi node-refusal matrix), so the entry-time derivation
    // cannot see the layer's bounds. The session must NOT open blind at
    // doc bounds (that silent fallback WAS G-3) — it WALLS: no session,
    // the engine's typed code queryable and on the status line through
    // the EXISTING ui.status.rejected path, document untouched.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    const int idx = m_controller->layers().size() - 1;
    fillRectOnLayer(idx, QRect(300, 180, 40, 40), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().at(idx).toMap().value(
                      QStringLiteral("layerId")));
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("mask.add"), target,
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("from"),
                                            QStringLiteral("reveal_all")},
                                           {QStringLiteral("newMaskId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());

    const int histBefore = m_controller->historyEntries().size();
    setTool("transform.perspective");
    QVERIFY2(!m_canvas->property("perspectiveActive").toBool(),
             "session opened BLIND on a masked layer — the doc-bounds "
             "fallback is the G-3 defect");
    QVERIFY(m_canvas->property("perspectiveWallActive").toBool());
    QCOMPARE(m_canvas->property("perspectiveWallCode").toString(),
             QStringLiteral("error.command.not_implemented"));
    QVERIFY2(m_controller->statusText().contains(
                 QStringLiteral("error.command.not_implemented")),
             qPrintable(QStringLiteral("the typed refusal did not reach the "
                                       "status surface; status: %1")
                            .arg(m_controller->statusText())));

    // A walled session accepts no input: the finding's gesture and
    // Enter are inert, the document unchanged (composite probes — the
    // per-layer door refuses on masked layers by design).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(120, 90));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(120, 90));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(320, 200, 1, 1), &px));
    const QColor center = px.pixelColor(0, 0);
    QVERIFY2(center.red() > center.blue(),
             "the marker moved under a WALLED perspective session");
    QVERIFY(m_controller->readRegionImage(QRect(10, 10, 1, 1), &px));
    const QColor corner = px.pixelColor(0, 0);
    QVERIFY2(corner == QColor(Qt::white),
             "content appeared outside the marker under a walled session");
}

void TstSelectionGestures::perspectiveEmptyLayerOpensNoSession()
{
    // The empty-layer ruling: a layer with no opaque pixels has no
    // bounds to frame — no session, no zero-rect frame, and no
    // fabricated status text (no cataloged empty-layer string exists
    // and minting any id — even queryable-only — is outside this
    // lane's fence; recorded in beginPerspectiveFrame). The wall is
    // queryable with an EMPTY code (the no-wire-refusal arm; the
    // masked case's typed code is the discriminator), the engine is
    // never asked, the document never changes.
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {},
                               QVariantMap{{QStringLiteral("kind"),
                                            QStringLiteral("raster")},
                                           {QStringLiteral("newLayerId"),
                                            m_controller->newUuid()}},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);
    const int histBefore = m_controller->historyEntries().size();

    setTool("transform.perspective");
    QVERIFY2(!m_canvas->property("perspectiveActive").toBool(),
             "session opened on an EMPTY layer (a zero-rect or doc-rect "
             "frame is a lie either way)");
    QVERIFY(m_canvas->property("perspectiveWallActive").toBool());
    QVERIFY2(m_canvas->property("perspectiveWallCode").toString().isEmpty(),
             qPrintable(QStringLiteral("the empty-layer arm carried a wire "
                                       "code no door produced: %1")
                            .arg(m_canvas->property("perspectiveWallCode")
                                     .toString())));
    QVERIFY2(!m_controller->statusText().contains(
                 QStringLiteral("ui.status.rejected")),
             "the empty-layer arm minted a status rejection with no "
             "cataloged text behind it");

    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return); // inert — no session
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
}

void TstSelectionGestures::perspectiveFullCanvasLayerDegenerate()
{
    // Degenerate agreement: on a full-canvas layer the derived source
    // bounds ARE the document bounds — the frame opens exactly where it
    // always did, the doc-corner drag grabs the TL handle, and the
    // commit applies through the same door. Behavior unchanged by the
    // fix for this class.
    const QString layerId = m_controller->layers().at(0).toMap()
                                .value(QStringLiteral("layerId")).toString();
    QCOMPARE(layerAlphaBounds(m_controller, layerId), QRect(0, 0, 640, 400));

    setTool("transform.perspective");
    QVERIFY(m_canvas->property("perspectiveActive").toBool());
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(120, 90));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(120, 90));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QVERIFY(!m_canvas->property("perspectiveActive").toBool());
    const QVariantList hist = m_controller->historyEntries();
    QVERIFY(!hist.isEmpty());
    QCOMPARE(hist.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("transform.perspective"));
    QVERIFY2(m_controller->statusText().contains(
                 QStringLiteral("ui.status.applied")),
             qPrintable(m_controller->statusText()));

    // Session-entry derivation cost, measured (the desk's number): the
    // one-shot full-doc readLayer + alpha scan at 640x400, then at
    // 2560x1600 (a 4M-px readback in a Debug build — the recorded
    // worst-case of this interim until the bounds ABI arm lands).
    QElapsedTimer t;
    t.start();
    setTool("transform.perspective"); // re-arm on this 640x400 doc
    qInfo("G-3 derivation cost: 640x400 session entry %lld ms",
          static_cast<long long>(t.elapsed()));
    QVERIFY(m_canvas->property("perspectiveActive").toBool());
    QVERIFY(m_controller->newDocument(2560, 1600, QStringLiteral("white")));
    t.restart();
    setTool("transform.perspective"); // documentReset cancelled the session
    qInfo("G-3 derivation cost: 2560x1600 session entry %lld ms",
          static_cast<long long>(t.elapsed()));
    QVERIFY(m_canvas->property("perspectiveActive").toBool());
}

// ===================================================================
// v1.0.1 ITEM 19 — the free-transform frame is the ACTIVE LAYER'S
// CONTENT, not the canvas.
//
// The fixture rect below is (300,60,40,40) and NOT the design entry's
// (300,180,40,40) on purpose: the latter's centre is (320,200), which
// is EXACTLY the 640x400 canvas centre, so the layer-bounds and
// canvas-bounds pivots would coincide and a fixture named for the
// difference between them would not vary that difference at all.
// ===================================================================

void TstSelectionGestures::transformFrameStartsAtLayerContentBounds()
{
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(300, 60, 40, 40), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString id = layerIdAt(idx);
    QCOMPARE(layerAlphaBounds(m_controller, id), QRect(300, 60, 40, 40));

    setTool("transform.free");
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "Cmd+T opened no session on a layer that has content");
    QVERIFY2(!m_canvas->property("transformFrameIsCanvasFallback").toBool(),
             "an unmasked layer took the canvas fallback");

    // THE DISCRIMINATOR IS THE GRAB, not a property read. (300,60) is
    // the CONTENT top-left. On a layer-bounded frame it is handle 0 and
    // this drag SCALES about the opposite corner (340,100). On the old
    // canvas-bounded frame the same point is 63 px from the nearest
    // canvas handle and merely INSIDE the box, so the identical drag
    // TRANSLATES — and a translate cannot change the content's SIZE,
    // which is the quantity asserted below.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 60));
    QTest::mouseMove(m_window, docToView(290, 50));
    QTest::mouseMove(m_window, docToView(280, 40));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(280, 40));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    // ENGINE TRUTH. Corner handles are aspect-locked by default, so the
    // 40x40 content scales 1.5x about (340,100): 60x60 at (280,40). The
    // translate-shaped regression lands 40x40 at the SAME ORIGIN — the
    // size is what separates them, so the size is what is pinned.
    // Tolerance +-4: MEASURED, not guessed — the §18.2 resample spreads
    // the alpha footprint of a 1.5x upscale to 63x63 at (278,38). It is
    // still five times narrower than the 20 px the translate-shaped
    // regression misses this by.
    const QRect b = layerAlphaBounds(m_controller, id);
    QVERIFY2(std::abs(b.width() - 60) <= 4 && std::abs(b.height() - 60) <= 4,
             qPrintable(QStringLiteral("frame was not the layer's content: "
                                       "committed %1x%2 at %3,%4, expected "
                                       "60x60 (a 40x40 result is the "
                                       "canvas-bounded frame translating "
                                       "instead of scaling)")
                            .arg(b.width())
                            .arg(b.height())
                            .arg(b.x())
                            .arg(b.y())));
    QVERIFY2(std::abs(b.x() - 280) <= 4 && std::abs(b.y() - 40) <= 4,
             qPrintable(QStringLiteral("scaled content landed at %1,%2, "
                                       "expected 280,40")
                            .arg(b.x())
                            .arg(b.y())));
    // Second, independent discriminator: the DOOR. A scale decomposes to
    // transform.free; the translate the mutation produces would ride
    // layer.transform's exact integer arm instead.
    const QVariantList hist = m_controller->historyEntries();
    QVERIFY(!hist.isEmpty());
    QCOMPARE(hist.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("transform.free"));
}

void TstSelectionGestures::transformFrameHandlesAreNotAtCanvasCorners()
{
    // The negative twin of the case above, and the reason the pair is
    // needed: case 1 proves a handle appeared where the LAYER is; this
    // proves one no longer sits where the CANVAS is. Either alone is
    // satisfiable by a frame that is simply bigger than both.
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(300, 60, 40, 40), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString id = layerIdAt(idx);

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    const int histBefore = m_controller->historyEntries().size();

    // Canvas top-left. On the old frame this is handle 0 and this drag
    // scales the layer to ~90%. On a layer-bounded frame it is outside
    // the box entirely — no handle, no rotate ring, no translate — so
    // the press yields NO gesture and Enter finds an identity matrix.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(30, 30));
    QTest::mouseMove(m_window, docToView(60, 60));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(60, 60));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    QCOMPARE(m_controller->historyEntries().size(), histBefore);
    QVERIFY2(layerAlphaBounds(m_controller, id) == QRect(300, 60, 40, 40),
             "a drag from the CANVAS corner moved the layer — the frame "
             "still has handles where the canvas is");
}

void TstSelectionGestures::transformFullCanvasLayerDegenerate()
{
    // STAY-GREEN PIN, and deliberately also the CONTROL for item 19's
    // own mutation: on a full-canvas layer the derived content bounds
    // ARE the canvas rect, so reverting the frame to doc bounds changes
    // nothing here. This case must stay GREEN under that mutation. If it
    // reds, the mutation broke something other than the thing item 19
    // changed, and the two cases above are measuring the wrong axis.
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(0, 0, 640, 400), 0.1, 0.2, 0.8);
    m_controller->setActiveLayerIndex(idx);
    const QString id = layerIdAt(idx);
    QCOMPARE(layerAlphaBounds(m_controller, id), QRect(0, 0, 640, 400));

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    const int histBefore = m_controller->historyEntries().size();

    // The canvas corner IS this layer's corner: the handle grabs either
    // way, and the committed geometry is identical either way.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(30, 30));
    QTest::mouseMove(m_window, docToView(60, 60));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(60, 60));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    QCOMPARE(m_controller->historyEntries().size(), histBefore + 1);
    // Aspect-locked uniform 0.90625 about (640,400).
    const QRect b = layerAlphaBounds(m_controller, id);
    QVERIFY2(std::abs(b.x() - 60) <= 3 && std::abs(b.y() - 38) <= 3
                 && std::abs(b.width() - 580) <= 3
                 && std::abs(b.height() - 362) <= 3,
             qPrintable(QStringLiteral("full-canvas scale landed %1,%2 %3x%4, "
                                       "expected ~60,38 580x362")
                            .arg(b.x())
                            .arg(b.y())
                            .arg(b.width())
                            .arg(b.height())));
}

void TstSelectionGestures::transformCommitTargetsTheEntryLayer()
{
    // §B.6: commitTransform resolved the active layer AT COMMIT TIME.
    // That was latent while the frame was canvas-shaped for every layer
    // alike; item 19 makes it live, because the frame — and the
    // `reference` pivot derived from it — now belong to ONE layer.
    // THIS CASE REDS ON THE PRE-ITEM-19 CODE. It is not regression
    // cover; it is the proof the defect was real.
    const int a = addRasterLayer();
    QVERIFY(a > 0);
    fillRectOnLayer(a, QRect(300, 60, 40, 40), 0.8, 0.1, 0.1);
    const int b = addRasterLayer();
    QVERIFY(b > a);
    fillRectOnLayer(b, QRect(100, 250, 40, 40), 0.1, 0.8, 0.1);
    const QString idA = layerIdAt(a);
    const QString idB = layerIdAt(b);

    m_controller->setActiveLayerIndex(a);
    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());

    // The mid-session switch. The frame on screen is still A's.
    m_controller->setActiveLayerIndex(b);
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "the layer switch closed the frame — this case can no "
             "longer discriminate anything");

    // Translate from inside A's frame (centre (320,80): 20 px from the
    // nearest handle, 28 px from the nearest rotate ring).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(320, 80));
    QTest::mouseMove(m_window, docToView(345, 100));
    QTest::mouseMove(m_window, docToView(370, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(370, 120));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    // Integer-translate arm: exact, no tolerance earned or needed.
    QVERIFY2(layerAlphaBounds(m_controller, idA) == QRect(350, 100, 40, 40),
             "the ENTRY layer did not move — the commit followed the "
             "active layer instead of the frame");
    QVERIFY2(layerAlphaBounds(m_controller, idB) == QRect(100, 250, 40, 40),
             "the layer that became active MID-SESSION was transformed "
             "with another layer's frame geometry");
}

void TstSelectionGestures::transformEmptyLayerOpensNoSessionAndSaysSo()
{
    // Nothing to frame: a zero-rect box and a doc-rect box are each a
    // lie, so no session opens — G-3's perspective ruling mirrored,
    // code-less wall included (no wire door refused; the empty bounds
    // ARE the door's answer).
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    m_controller->setActiveLayerIndex(idx);
    const int histBefore = m_controller->historyEntries().size();

    setTool("transform.free");
    QVERIFY2(!m_canvas->property("transformActive").toBool(),
             "a session opened on an EMPTY layer");
    QVERIFY(m_canvas->property("transformWallActive").toBool());
    QVERIFY2(m_canvas->property("transformWallCode").toString().isEmpty(),
             qPrintable(QStringLiteral("the empty-layer arm carried a wire "
                                       "code no door produced: %1")
                            .arg(m_canvas->property("transformWallCode")
                                     .toString())));
    // ARMED-HONEST, and this half is where G-3 stopped: a control that
    // acts on every other layer and does NOTHING here, with nothing
    // said, is inert-without-explanation. The sentinel proves the notice
    // resolved through the CATALOG rather than key-echoing an id at the
    // user (statusPending refuses uncataloged ids by design).
    QCOMPARE(m_controller->statusText(), QStringLiteral("SENTINEL-EMPTY"));

    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return); // inert — no session
    QCOMPARE(m_controller->historyEntries().size(), histBefore);

    // WALL WITHOUT SESSION is a state this side did not have before item
    // 19 (item 13's wall always rode a live frame), so the teardown that
    // used to be covered by `if (m_transformActive)` no longer is.
    setTool("move");
    QVERIFY2(!m_canvas->property("transformWallActive").toBool(),
             "the empty-layer wall outlived the tool that raised it");
}

void TstSelectionGestures::transformMaskedLayerFallsBackLabelled()
{
    // F3b: the 1.4 per-layer readback refuses masked layers TYPED, so
    // there are no layer bounds to frame and no pixels to preview.
    //
    // NOT A WALL, and this is the design decision the case exists to
    // pin. Unlike perspective — where the frame is load-bearing on the
    // homography, so a wrong frame is wrong PIXELS — transform.free's
    // matrix applies as-is in document space and `reference` is
    // validated then discarded. A canvas-shaped frame therefore commits
    // the SAME pixels as a layer-shaped one: it is loose, not wrong.
    // Refusing entry would claim "you cannot do this" about a drag the
    // user can make, aim, and cancel, and would throw away the item-13
    // named wall that already tells them the truth at commit.
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(300, 60, 40, 40), 0.8, 0.1, 0.1);
    addRevealAllMask(idx);
    m_controller->setActiveLayerIndex(idx);
    const int histBefore = m_controller->historyEntries().size();

    setTool("transform.free");
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "entry WALLED on a masked layer — a guard that hides a "
             "refusal the engine will state itself is a false surface "
             "with its polarity reversed");
    QVERIFY(m_canvas->property("transformFrameIsCanvasFallback").toBool());
    QVERIFY(m_canvas->property("transformPreviewUnavailable").toBool());
    QVERIFY2(!m_canvas->property("transformWallActive").toBool(),
             "the fallback raised a WALL, which claims the user is "
             "blocked when they are not");
    // LABELLED, not silent — and the label states the CONSEQUENCE, not
    // just the read that failed. The read's own ui.status.rejected
    // notice is true and materially incomplete: it says nothing about
    // the canvas-sized box the user is now looking at.
    QVERIFY2(m_controller->statusText().startsWith(
                 QStringLiteral("SENTINEL-MASKED")),
             qPrintable(QStringLiteral("the canvas fallback was silent (or "
                                       "spoke the wrong id); status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY2(m_controller->statusText().contains(QStringLiteral("error.")),
             qPrintable(QStringLiteral("the consequence notice dropped the "
                                       "engine's wire code; status: %1")
                            .arg(m_controller->statusText())));

    // The frame really IS canvas-shaped: the canvas corner grabs.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(30, 30));
    QTest::mouseMove(m_window, docToView(60, 60));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(60, 60));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    // The engine's own typed masked refusal arrives as the item-13 NAMED
    // WALL: frame still live, the user's work not thrown away, document
    // untouched.
    QVERIFY2(m_canvas->property("transformWallActive").toBool(),
             "the masked commit produced no wall");
    QVERIFY2(!m_canvas->property("transformWallCode").toString().isEmpty(),
             "the wall carried no wire code — an unnamed wall is the "
             "shape item 13 exists to prevent");
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "the frame was thrown away when the engine refused");
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
    // Composite probes: the per-layer door refuses masked layers by
    // design, so engine truth here has to come through the composite.
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(320, 80, 1, 1), &px));
    const QColor marker = px.pixelColor(0, 0);
    QVERIFY2(marker.red() > marker.blue(),
             "the marker moved under a REFUSED masked transform");
}

// ===================================================================
// v1.0.1 ITEM 21 — the live preview, stacked on 19.
// ===================================================================

void TstSelectionGestures::transformPreviewShowsOnlyTheActiveLayer()
{
    // The operator's own item-10 invariant, ported to scale/rotate —
    // where getting it wrong is far worse than the translate case it was
    // first reported for.
    const int a = addRasterLayer();
    QVERIFY(a > 0);
    fillRectOnLayer(a, QRect(20, 260, 120, 120), 0.1, 0.1, 0.5);
    const int b = addRasterLayer();
    QVERIFY(b > a);
    fillRectOnLayer(b, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(b);

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    QVERIFY2(!m_canvas->property("transformPreviewUnavailable").toBool(),
             "a readable layer reported no preview");

    // PB: inside the SCALED B and outside the unscaled B — only a live
    // preview can put colour there.
    // PA: inside layer A, chosen so that under item 10's whole-composite
    // fallback the source pixel landing on it is WHITE. That fallback is
    // the one thing item 21 must not inherit: translating the whole
    // composite is noisy, but SCALING it makes every other layer appear
    // to scale, which is the defect item 19 exists to kill, amplified.
    const QPoint pbView = docToView(260, 30);
    const QPoint paView = docToView(80, 320);
    const QImage before = m_window->grabWindow();
    const QRgb pbBefore = before.pixel(pbView);
    const QRgb paBefore = before.pixel(paView);

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 60));
    QTest::mouseMove(m_window, docToView(275, 35));
    QTest::mouseMove(m_window, docToView(250, 10));
    const QImage mid = m_window->grabWindow();

    QVERIFY2(mid.pixel(pbView) != pbBefore,
             "no live preview was drawn under the transform frame — the "
             "user drags an empty box and finds out what happened only "
             "after committing");
    const QColor pb(mid.pixel(pbView));
    QVERIFY2(pb.red() > pb.blue(),
             qPrintable(QStringLiteral("the preview at PB is not the dragged "
                                       "layer's colour: %1")
                            .arg(pb.name())));
    QVERIFY2(mid.pixel(paView) == paBefore,
             "a NON-active layer's pixels changed in a preview frame — "
             "item 10's whole-composite fallback is back");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(250, 10));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
}

void TstSelectionGestures::transformPreviewMatchesTheCommit()
{
    // ES §12.3 rule 4 in test form: "what was dragged is what commits;
    // only resolution differs". The preview half is a window grab; the
    // committed half is ENGINE readback. Both are read at the SAME two
    // document points — one the preview claims, one it disclaims.
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    const QPoint inView = docToView(260, 30);   // inside the scaled shape
    const QPoint outView = docToView(200, 30);  // outside it
    const QImage before = m_window->grabWindow();

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 60));
    QTest::mouseMove(m_window, docToView(275, 35));
    QTest::mouseMove(m_window, docToView(250, 10));
    const QImage mid = m_window->grabWindow();
    const QColor previewIn(mid.pixel(inView));
    QVERIFY2(previewIn.red() > previewIn.blue(),
             "the preview did not cover the point it must cover");
    QVERIFY2(mid.pixel(outView) == before.pixel(outView),
             "the preview covered a point outside the scaled shape — a "
             "preview that covers everything discriminates nothing");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(250, 10));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(260, 30, 1, 1), &px));
    const QColor committedIn = px.pixelColor(0, 0);
    QVERIFY2(committedIn.red() > committedIn.blue(),
             qPrintable(QStringLiteral("the commit did NOT cover a point the "
                                       "preview covered: %1")
                            .arg(committedIn.name())));
    QVERIFY(m_controller->readRegionImage(QRect(200, 30, 1, 1), &px));
    QVERIFY2(px.pixelColor(0, 0) == QColor(Qt::white),
             "the commit reached a point the preview said it would not");
}

void TstSelectionGestures::transformPreviewMaskedLayerShowsFrameOnly()
{
    // No readable pixels means NO preview. The only image the shell has
    // in hand at that moment is the whole composite, and drawing THAT
    // under a scale is precisely the lie this item exists to remove.
    const int a = addRasterLayer();
    QVERIFY(a > 0);
    fillRectOnLayer(a, QRect(20, 260, 120, 120), 0.1, 0.1, 0.5);
    const int b = addRasterLayer();
    QVERIFY(b > a);
    fillRectOnLayer(b, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    addRevealAllMask(b);
    m_controller->setActiveLayerIndex(b);

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    QVERIFY(m_canvas->property("transformPreviewUnavailable").toBool());

    // PA sits in the WHITE background at a point where a whole-composite
    // echo at this drag's scale would land layer A's dark block. Picking
    // a point INSIDE A would not discriminate: at scales near 1 the
    // fallback's source for such a point is usually A itself, so the
    // colour would not move and the case would pass while the defect
    // shipped.
    const QPoint paView = docToView(250, 340);
    const QImage before = m_window->grabWindow();
    const QRgb paBefore = before.pixel(paView);

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(100, 100));
    QTest::mouseMove(m_window, docToView(200, 200));
    const QImage mid = m_window->grabWindow();
    QVERIFY2(mid.pixel(paView) == paBefore,
             "a masked layer with NO readable pixels still painted an "
             "echo — the only image available is the whole composite");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 200));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
}

void TstSelectionGestures::transformPreviewIssuesNoCommandsAndEscLeavesBitsAlone()
{
    // ES §39 "commits are the only mutations" + §12.3 rule 5 "previews
    // never mutate tiles". Asserted at EVERY drag step, not just at the
    // end: a preview that submits once and undoes itself would pass an
    // end-state check.
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(300, 60, 100, 80), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString id = layerIdAt(idx);
    const QImage bitsBefore = m_controller->layerThumbImage(id);
    QVERIFY(!bitsBefore.isNull());

    setTool("transform.free");
    QVERIFY(m_canvas->property("transformActive").toBool());
    const int histBefore = m_controller->historyEntries().size();

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 60));
    for (int i = 1; i <= 20; ++i) {
        QTest::mouseMove(m_window, docToView(300 - 2 * i, 60 - i));
        QCOMPARE(m_controller->historyEntries().size(), histBefore);
    }
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(260, 40));
    QCOMPARE(m_controller->historyEntries().size(), histBefore);

    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QCOMPARE(m_controller->historyEntries().size(), histBefore);
    QVERIFY2(!m_canvas->property("transformActive").toBool(),
             "Esc left the frame live");
    const QImage bitsAfter = m_controller->layerThumbImage(id);
    QVERIFY2(!bitsAfter.isNull() && bitsAfter == bitsBefore,
             "the document's bits moved during a preview-only drag");
}

void TstSelectionGestures::transformAboveTheReadbackCapFramesButSaysNoPreview()
{
    // §D.1 + item 21's degradation, in one case, because they are two
    // halves of the same fact: above 2^26 px the BOUNDS are still
    // derivable (the strip loop chunks) but the PIXELS are not (no
    // single read delivers them, no QImage holds them as one plane).
    //
    // 8200x8200 = 67.24 Mpx, just over the 2^26 = 67.108 Mpx cap. The
    // document is TRANSPARENT so the engine's tiles stay sparse: the
    // cost this case is willing to pay is the shell's two strip
    // readbacks, which is the thing under test, not a gigabyte of
    // engine buffer.
    //
    // COST, MEASURED, NOT GUESSED: ~53 s wall on this machine, all of it
    // the two 268 MB strip readbacks and their 67 Mpx alpha scans, paid
    // TWICE (once through the door assertion below, once through session
    // entry — two different claims, so neither is dropped). That is a
    // third of what tst_gesture_net already costs, so it stays in the
    // default set rather than moving to the bench-labelled probes; the
    // qInfo below keeps the number visible instead of letting the suite
    // quietly get slower for an unattributed reason.
    QElapsedTimer capTimer;
    capTimer.start();
    QVERIFY(m_controller->newDocument(8200, 8200,
                                      QStringLiteral("transparent")));
    const int idx = addRasterLayer();
    QVERIFY(idx > 0);
    fillRectOnLayer(idx, QRect(300, 60, 40, 40), 0.8, 0.1, 0.1);
    m_controller->setActiveLayerIndex(idx);
    const QString id = layerIdAt(idx);

    // READ-DOOR TRUTH, not shell echo: the derivation SUCCEEDS above the
    // cap and returns the real content rect. Before the strip-loop fix
    // this single call was one unchunked full-doc readLayer, which fails
    // outright at this size — so ok:false, and the frame would take the
    // masked-refusal path for a reason that has nothing to do with the
    // layer.
    const QVariantMap derived = m_controller->layerContentBounds(id);
    QVERIFY2(derived.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("bounds derivation failed above the "
                                       "2^26 readback cap: %1")
                            .arg(derived.value(QStringLiteral("code"))
                                     .toString())));
    QCOMPARE(derived.value(QStringLiteral("x")).toInt(), 300);
    QCOMPARE(derived.value(QStringLiteral("y")).toInt(), 60);
    QCOMPARE(derived.value(QStringLiteral("w")).toInt(), 40);
    QCOMPARE(derived.value(QStringLiteral("h")).toInt(), 40);
    QVERIFY2(!derived.value(QStringLiteral("snapshot")).toBool(),
             "the door claimed to hand back pixels it cannot hold");

    setTool("transform.free");
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "a session refused to open on a big document");
    QVERIFY2(!m_canvas->property("transformFrameIsCanvasFallback").toBool(),
             "the frame took the canvas fallback — the bounds derivation "
             "did NOT chunk");
    QVERIFY2(m_canvas->property("transformPreviewUnavailable").toBool(),
             "the preview claimed to be available above the cap");
    // ARMED-HONEST: frame and handles are real, the preview is not, and
    // the difference is stated rather than left for the user to notice.
    QCOMPARE(m_controller->statusText(), QStringLiteral("SENTINEL-BIG"));

    // No gesture and no commit here on purpose: docToView's mapping is
    // derived for the 640x400 fixture, and a bake is not the axis this
    // case measures. The frame's GEOMETRY above the cap is asserted
    // through the read door above, which is engine truth.
    qInfo("above-cap (8200x8200) bounds derivation + session entry: %lld ms",
          static_cast<long long>(capTimer.elapsed()));
    setTool("move"); // drop the session before the next case's init()
}

void TstSelectionGestures::deselectCommand()
{
    // The command Cmd+D / the menu item fire (selection.none) — the
    // cheapest probe of the shared commit path (item 9). Review D6: a
    // PARTIAL seed makes the fill acceptance discriminating.
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), 200);
    rect.insert(QStringLiteral("y"), 120);
    rect.insert(QStringLiteral("w"), 240);
    rect.insert(QStringLiteral("h"), 140);
    QVariantMap sel;
    sel.insert(QStringLiteral("source"), QStringLiteral("rect"));
    sel.insert(QStringLiteral("rect"), rect);
    m_controller->applyCommand(QStringLiteral("selection.set"), {}, sel, {});
    QVERIFY(m_controller->selectionActive());
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("selection.none"), {}, {}, {});
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("selection.none refused: %1")
                            .arg(m_controller->statusText())));
    VERIFY_DESELECTED_AND_FILL_REACHES_CORNER(m_controller);
}

void TstSelectionGestures::escapeNeverDeselects()
{
    // §39.2 [NORMATIVE], operator-ruled: an idle Esc must NOT touch a
    // committed selection — this pin REDS if anyone re-adds the
    // deselect rung. (Deselection paths: Cmd+D/menu = deselectCommand;
    // click-outside = marqueeClickOutsideDeselects.)
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 120));
    QTest::mouseMove(m_window, docToView(440, 260));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(440, 260));
    QVERIFY(m_controller->selectionActive());
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QVERIFY2(m_controller->selectionActive(),
             "idle Esc DESELECTED — §39.2: Esc never deselects");
    QCOMPARE(engineSelectionBounds(m_controller), QRect(200, 120, 240, 140));
}

void TstSelectionGestures::escapeMidGestureCancelsWithoutDeselect()
{
    // Review D7: the ladder's FIRST rung — Esc mid-drag cancels the
    // gesture, does NOT deselect, and the subsequent release commits
    // nothing.
    setTool("select.rect");
    m_controller->applyCommand(QStringLiteral("selection.all"), {}, {}, {});
    QVERIFY(m_controller->selectionActive());
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(220, 200));
    QTest::keyClick(m_window, Qt::Key_Escape);
    QVERIFY2(m_controller->selectionActive(),
             "mid-gesture Esc must cancel the drag, not deselect");
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(220, 200));
    // The cancelled gesture's release commits nothing: still the FULL
    // canvas selection (not a 120x100 replace).
    QCOMPARE(engineSelectionBounds(m_controller), QRect(0, 0, 640, 400));
}

void TstSelectionGestures::marqueeClickOutsideDeselects()
{
    setTool("select.rect");
    // Select a small rect by drag, then click OUTSIDE it: deselect,
    // and the fill acceptance runs on THIS path too (review D6).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(200, 180));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 180));
    QVERIFY(m_controller->selectionActive());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(500, 350));
    QVERIFY2(!m_controller->selectionActive(),
             "marquee click outside the selection did not deselect (item 9)");
    VERIFY_DESELECTED_AND_FILL_REACHES_CORNER(m_controller);
    // And a click INSIDE a fresh selection stays a no-op.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(200, 180));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 180));
    QVERIFY(m_controller->selectionActive());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(150, 140));
    QVERIFY2(m_controller->selectionActive(),
             "marquee click INSIDE the selection must not deselect");
}

void TstSelectionGestures::modifierClickOutsideIsNoOp()
{
    // Review D2 (§39.4 NORMATIVE): click-deselect fires only when the
    // effective op is NEW — a shift-click (add) outside the selection
    // is a NO-OP, never a deselect (a slip mid-composition must not
    // destroy the composed selection).
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(200, 180));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 180));
    QVERIFY(m_controller->selectionActive());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::ShiftModifier,
                      docToView(500, 350));
    QVERIFY2(m_controller->selectionActive(),
             "shift-click outside DESELECTED — §39.4 requires a no-op "
             "for non-new ops (review D2)");
}

// ── item 16: click-outside deselect beyond marquee-only ──────────────
//
// The triage recorded ellipse as ALREADY LIVE (submitShapeSelection
// serves both marquee tools) but UNPINNED — so this case is a PIN of
// existing behaviour, not new behaviour, and it is written to have real
// teeth rather than to restate the rect case: the deselecting click
// lands on the ellipse's BOUNDING-BOX CORNER, which is INSIDE the bbox
// and OUTSIDE the ellipse. It can only deselect if outside-ness is the
// engine's 1x1 mask truth. An implementation that tested the click
// against the committed bounding RECT would read that point as inside
// and stay selected — which is exactly the false surface item 16 exists
// to prevent, one polarity over.
void TstSelectionGestures::ellipseClickOutsideDeselects()
{
    setTool("select.ellipse");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 250));
    QVERIFY(m_controller->selectionActive());
    // Same reviewer-probed points as ellipseCommitsOnRelease: mask 0 at
    // the bbox corner, 255 at the center. Re-read here so the case
    // states its own premise instead of inheriting it.
    QImage px;
    QVERIFY(m_controller->readSelectionImage(QRect(101, 101, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 0);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(101, 101));
    QVERIFY2(!m_controller->selectionActive(),
             "ellipse click on the bbox CORNER (mask 0 = outside the "
             "ellipse) did not deselect — item 16 requires engine mask "
             "truth, not a bounding-rect test");
    VERIFY_DESELECTED_AND_FILL_REACHES_CORNER(m_controller);
    // And the ellipse's INSIDE stays a no-op (the named §39.4 D4
    // deviation, re-pinned on this tool).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 250));
    QVERIFY(m_controller->selectionActive());
    QVERIFY(m_controller->readSelectionImage(QRect(200, 175, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 255);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 175));
    QVERIFY2(m_controller->selectionActive(),
             "ellipse click INSIDE the selection must not deselect");
}

// The BUILT half of item 16. A lasso click with no drag never reaches
// the >= 2-point commit contract, so before this slice it fell out of
// submitLasso silently — a gesture that looked identical to the
// marquee's and did nothing.
void TstSelectionGestures::lassoClickOutsideDeselects()
{
    setTool("select.lasso");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseMove(m_window, docToView(150, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 220));
    QVERIFY(m_controller->selectionActive());
    // (500,350) is far outside the lasso's own bounds AND outside its
    // bbox, so this leg needs no shape discrimination; the ellipse case
    // above carries that.
    QImage px;
    QVERIFY(m_controller->readSelectionImage(QRect(500, 350, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 0);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(500, 350));
    QVERIFY2(!m_controller->selectionActive(),
             "lasso click outside the selection did not deselect — item "
             "16's built half");
    VERIFY_DESELECTED_AND_FILL_REACHES_CORNER(m_controller);
    // A lasso click INSIDE stays a no-op, same named D4 deviation. This
    // leg is what discriminates "the deselect reads the engine mask"
    // from "any lasso click clears the selection".
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseMove(m_window, docToView(150, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 220));
    QVERIFY(m_controller->selectionActive());
    QVERIFY(m_controller->readSelectionImage(QRect(250, 150, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 255);
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(250, 150));
    QVERIFY2(m_controller->selectionActive(),
             "lasso click INSIDE the selection must not deselect");
}

// Review D2's §39.4 [NORMATIVE] gate, re-pinned on the NEW call site.
// The gate lives in the shared helper, but a call site can reach the
// helper past its own guard just as easily as through it — the marquee's
// pin does not cover the lasso's.
void TstSelectionGestures::lassoModifierClickOutsideIsNoOp()
{
    setTool("select.lasso");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseMove(m_window, docToView(150, 220));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(150, 220));
    QVERIFY(m_controller->selectionActive());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::ShiftModifier,
                      docToView(500, 350));
    QVERIFY2(m_controller->selectionActive(),
             "shift-click outside DESELECTED on the lasso — §39.4 "
             "requires a no-op for non-new ops (review D2)");
}

// ── item 15: the polygon's first-vertex close ────────────────────────
//
// The operator's premise was "polygon is registry-only / zero gesture".
// PROBE-FIRST found that stale: select.polygon has had live gesture code
// since item 6, closing on double-click and on Enter. The one real gap
// was the pen's first-vertex close, which polygon did not share.
void TstSelectionGestures::polygonFirstVertexClickCloses()
{
    setTool("select.polygon");
    // Leg order is deliberate: the POSITION control runs before the
    // COUNT control, because the two share a vertex at ~250px from the
    // first and a tolerance mutation trips whichever runs first. Running
    // position first makes each mutation's red attributable to the leg
    // that actually claims it.

    // ── LEG A: the POSITION control, with the count bound SATISFIED ──
    // Three vertices must already be down before distance can be the
    // only thing holding the close off — with fewer, the count bound
    // blocks the close regardless of where the click lands, and a leg
    // written with only two vertices down proves NOTHING about position.
    // (That was this case's first draft; the tolerance mutation redded a
    // different leg entirely, which is how the vacuity surfaced.)
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 250));
    // Now 3 vertices down. This click is ~149px from the first vertex —
    // far outside the 8px/zoom tolerance, so it must APPEND, not close.
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(150, 240));
    QVERIFY2(!m_controller->selectionActive(),
             "polygon closed on a click ~149px from the first vertex "
             "with the count bound already met — the close is not "
             "discriminating position");

    // ── LEG B: the close itself ──
    // 4px from the first vertex at zoom 1, inside the 8px/zoom tolerance
    // ported from the pen.
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(104, 100));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral("polygon first-vertex click did not "
                                       "close and commit (item 15); "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    // The committed quad is (100,100)-(300,100)-(300,250)-(150,240): the
    // closing click is NOT appended as a vertex. Non-degenerate bounds
    // prove the commit carried real geometry rather than an empty mask.
    const QRect b = engineSelectionBounds(m_controller);
    QVERIFY2(b.width() >= 190 && b.height() >= 140,
             qPrintable(QStringLiteral("polygon closed but with degenerate "
                                       "bounds: %1,%2 %3x%4")
                            .arg(b.x())
                            .arg(b.y())
                            .arg(b.width())
                            .arg(b.height())));
    // Shape discrimination: the committed QUAD, not its bounding box.
    // (110,240) sits left of the (150,240)->(100,100) edge and is
    // outside; (280,150) is well inside. Without this pair a
    // bounds-only assertion would pass for a rect regression.
    QImage px;
    QVERIFY(m_controller->readSelectionImage(QRect(110, 240, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 0);
    QVERIFY(m_controller->readSelectionImage(QRect(280, 150, 1, 1), &px));
    QCOMPARE(int(px.constScanLine(0)[0]), 255);

    // ── LEG C: the vertex-COUNT bound, with the tolerance SATISFIED ──
    // Two vertices down, then a click 4px from the first — well inside
    // the tolerance. It must NOT close: the engine's polygon contract
    // needs 3 points, so below that the first vertex has not yet earned
    // its second meaning and the click just appends.
    //
    // ★ HOW THIS LEG DISCRIMINATES, and why the obvious form does not.
    // Asserting "not selectionActive right after the click" is VACUOUS:
    // a mutant that closes at >= 2 vertices calls submitPolygon() with 2
    // points, which the 3-point contract REFUSES — so nothing commits
    // and the naive assertion passes on the mutant too. (Measured: the
    // >= 3 -> >= 2 mutation ran GREEN against that form.) The real
    // difference is whether the SESSION SURVIVED, so this leg drives the
    // session to a commit only reachable if it did: append a third real
    // vertex, then Enter. Correct code has 4 vertices and commits; the
    // mutant closed and died back at the click, so Enter finds a
    // 1-vertex session and commits nothing.
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    QVERIFY(!m_controller->selectionActive());
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(104, 100));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 250));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QVERIFY2(m_controller->selectionActive(),
             "the polygon session did not survive a first-vertex click "
             "taken with only 2 vertices down — below the engine's "
             "3-point contract that click must APPEND, not close");
}

// G-1 (MAJOR) — Esc mid-stroke did not abandon a brush stroke; the release
// committed it. §39.2 [NORMATIVE]: "Esc resolves the innermost in-progress
// context ONLY". Every other in-progress context honoured that — lasso,
// polygon, pen, transform, perspective, puppet — and a live stroke did not,
// which read as an omission from that ladder rather than a ruled exception.
//
// The mechanism was that cancelGesture() cleared eight pieces of gesture
// state and never m_strokeInProgress, so mouseReleaseEvent still reached
// strokeEnd() and committed the stroke the user had just cancelled.
void TstSelectionGestures::brushEscapeMidStrokeDoesNotCommit()
{
    const QRect corridor(90, 290, 220, 20); // the band the stroke would ink

    // ★ POSITIVE CONTROL FIRST, and it is load-bearing rather than
    // ceremonial: the assertion below is "the corridor holds no ink", and
    // an unpainted corridor is ALSO what a brush that never works at all
    // produces — the failure value and the healthy value coincide. So this
    // half proves the probe can SEE ink before the other half reads its
    // absence as meaning anything.
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
    QImage inked;
    QVERIFY2(m_controller->readRegionImage(corridor, &inked),
             "readRegionImage refused the stroke corridor — the probe itself "
             "is unusable and the absence check below would be vacuous");
    QCOMPARE(inked.size(), corridor.size());
    int inkedDark = 0;
    for (int y = 0; y < inked.height(); ++y)
        for (int x = 0; x < inked.width(); ++x)
            if (qGray(inked.pixel(x, y)) < 128)
                ++inkedDark;
    QVERIFY2(inkedDark > 0,
             qPrintable(QStringLiteral("POSITIVE CONTROL FAILED: an UNCANCELLED "
                                       "brush stroke left no ink in the corridor "
                                       "(%1 dark px). The absence assertion below "
                                       "cannot discriminate, because \"no ink\" is "
                                       "then also what a working cancel looks like")
                            .arg(inkedDark)));

    // Fresh document, so the control's ink cannot be mistaken for the
    // cancelled stroke's.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    resetView();
    // Named here rather than left to `againDark` further down: if the swap
    // did not restore the transform, this fails pointing at the cause.
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);

    // ...now the same gesture, with Esc pressed BEFORE the release.
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));

    // ★★ THE LIVENESS FLOOR (round-4 MAJOR-1). The two
    // !gestureInFlight() pins below are absence assertions with NO positive
    // control — in a case whose own opening comment argues at length that an
    // absence assertion needs one. This line is that control, and it is the
    // FIRST OF TWO positive readers of gestureInFlight() in this file; the
    // other is the documentReset arm's own floor at the bottom of this case.
    // (Round-5 NIT-1: it said "the ONLY", which the same commit falsified by
    // adding the second one. tst_chord_table.cpp also asserts it TRUE, but
    // that is a SEPARATE executable with its own add_test — ctest -N lists
    // this one as #20 and that one as #32 — so it contributes nothing here.)
    //
    // Without this line two independent regressions go green:
    //   (a) the press above stops arming — then both !gestureInFlight()
    //       pins, dark == 0, !strokeEnd() and endDark == 0 all pass, and the
    //       only red is the wedge arm at the bottom, whose message blames
    //       coordinate mapping and would be misdiagnosed as a transform bug;
    //   (b) gestureInFlight() loses its m_strokeInProgress term (or goes
    //       stuck-false) while the press still arms — then NOTHING in this
    //       case reds, not even the wedge arm: the real flag still gates
    //       mouseReleaseEvent's strokeEnd() call, so no ink ever appears and
    //       every remaining assertion holds for the right reason while the
    //       two pins hold for no reason. That variant is a TOTAL silent
    //       green and this line is its only reader.
    //
    // ★ ASYMMETRY, deliberate — do not add a twin of this for the
    // controller. strokeEnd() is exercised POSITIVELY twice in this case
    // (the control stroke above and the wedge arm below both commit through
    // it), so a stuck-false strokeEnd() already reds the positive control.
    // Only the canvas side was exposed; a symmetric controller assertion
    // would measure nothing.
    //
    // ★ SCOPE (round-4 NIT-1): gestureInFlight() is a THREE-WAY OR —
    // m_panning || m_strokeInProgress || m_gesture != Gesture::None. This
    // control is satisfied today through m_strokeInProgress ALONE: the brush
    // branch of mousePressEvent never sets m_gesture, and m_panning needs
    // Space or the middle button, neither of which this case uses. Extend
    // this case to a tool that sets m_gesture and this stops isolating the
    // stroke term.
    QVERIFY2(m_canvas->gestureInFlight(),
             "no stroke in flight BEFORE the Esc — the press did not arm a "
             "stroke, or gestureInFlight() lost its m_strokeInProgress term. "
             "Either way the two !gestureInFlight() pins below would pass "
             "vacuously and this case would pin nothing");
    QTest::keyClick(m_window, Qt::Key_Escape);

    // ★★ THE CANVAS HALF, PINNED SEPARATELY — and it is separate. The two
    // halves of this fix are MUTUALLY MASKING: clearing only the flag, or
    // calling only strokeCancel(), each produces "no ink" and satisfies every
    // pixel assertion below. Delete `m_strokeInProgress = false` and keep the
    // strokeCancel() call, and mouseReleaseEvent still reaches strokeEnd(),
    // which returns early on !m_strokeActive EMITTING NOTHING — so
    // onStrokeCommitted never runs, the flag is never cleared, and the
    // corridor stays clean for the wrong reason.
    //
    // ★ NO PIXEL ASSERTION CAN SEE THAT, because readRegionImage reads the
    // ENGINE COMPOSITE and the damage is in the canvas overlay: a stranded
    // flag keeps gestureInFlight() true, which gates the context menu
    // (Main.qml) and the Delete/Backspace chord, while mouseMoveEvent keeps
    // appending to m_activeEcho — so continuing the drag after Esc repaints a
    // GHOST of the cancelled stroke until the next completed one.
    QVERIFY2(!m_canvas->gestureInFlight(),
             "gestureInFlight() is still true immediately after Esc. It ORs "
             "three terms (m_panning || m_strokeInProgress || m_gesture != "
             "Gesture::None); on this path only m_strokeInProgress is "
             "reachable, so cancelGesture disarmed the controller and left "
             "the flag — the context menu and the Delete/Backspace chord "
             "stay gated and the echo keeps growing");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
    QVERIFY2(!m_canvas->gestureInFlight(),
             "gestureInFlight() is still true after the release that followed "
             "Esc — of its three terms (m_panning, m_strokeInProgress, "
             "m_gesture) only m_strokeInProgress is reachable here, and it "
             "strands until the next COMPLETED stroke");

    QImage after;
    QVERIFY(m_controller->readRegionImage(corridor, &after));
    QCOMPARE(after.size(), corridor.size());
    int dark = 0;
    for (int y = 0; y < after.height(); ++y)
        for (int x = 0; x < after.width(); ++x)
            if (qGray(after.pixel(x, y)) < 128)
                ++dark;
    QVERIFY2(dark == 0,
             qPrintable(QStringLiteral("Esc did not abandon the stroke: the "
                                       "release COMMITTED it and left %1 dark "
                                       "px in the corridor. §39.2 makes Esc "
                                       "resolve the innermost in-progress "
                                       "context, and a live stroke is one")
                            .arg(dark)));

    // ★★ THE CONTROLLER'S SESSION IS ACTUALLY DISARMED — and this is the
    // only assertion in the case that touches `strokeCancel()` at all.
    // Round 1's mutation reverted the WHOLE Esc arm, so it never separated
    // the canvas half from the controller half: with `strokeCancel()`
    // deleted and only the flag cleared, every assertion above still
    // passes, because `strokeBegin` is UNCONDITIONAL — it re-arms and
    // clears the sample buffer on every press, so a stale session is
    // overwritten rather than inherited. The tool cannot "wedge".
    //
    // So pin the controller directly. `strokeEnd()` returns false on an
    // inactive session; with the session still armed and the abandoned
    // samples still buffered it INGESTS AND COMMITS THEM, which is the
    // real bug a canvas-only fix leaves behind — one call away, and
    // reachable the moment any path reaches strokeEnd without an
    // intervening strokeBegin.
    // ★★ ROUND-4 MINOR-1 — the bare `!strokeEnd()` measured the WRONG
    // PROPOSITION. It says "not committable RIGHT NOW"; the comment above
    // and the header note on strokeCancel() both claim "the session was
    // DISARMED". Only the weaker one was measured, so `m_strokeActive =
    // false` in strokeCancel() was individually unpinned: delete it, keep
    // the `m_strokeSamples.clear()` on the next line, and everything passed
    // — the flag stays set but the buffer is empty, so strokeEnd() takes the
    // EMPTY-SAMPLES branch, returns false, and its strokeCommitted(false) is
    // swallowed by onStrokeCommitted's `!m_strokeInProgress` early return.
    // Even the emission was invisible.
    //
    // Feeding the session two samples FIRST measures the real proposition —
    // post-cancel samples are REFUSED. On a correctly disarmed session both
    // appends are no-ops (strokeAppend early-returns on !m_strokeActive) and
    // strokeEnd() still returns false, so this strengthens without
    // weakening: the whole-arm mutation still reds AND deleting the
    // flag-clear alone now reds too. Both points sit inside the corridor, so
    // endDark below is a second, pixel-level reader of the same failure.
    // (Backward tSeconds is safe — but round-5 NIT-3: "ties" was the wrong
    // reason, and the hazard it waved away is real. validateSamples DOES
    // reject non-monotonic t outright ("t not strictly increasing"), so had
    // a backward t reached the appended samples, encodeSamplesBlob would
    // return nullopt, strokeEnd would take the INVALID-BLOB branch, return
    // false, and MUT-A would have gone GREEN — the fix would have destroyed
    // the mutation it was written to join. It does not reach them because
    // appendNormalizedSample's guard is `t <= prev.t`: it repairs ANY
    // non-increasing t to prev.t + 0.0001, not only exact ties. These two t
    // values are genuinely BACKWARD relative to the event-derived timestamps
    // still buffered under the mutation, never tied, so it is the guard's
    // real range and not tie-handling that makes this safe.)
    m_controller->strokeAppend(150, 300, 1.0, 0.10);
    m_controller->strokeAppend(250, 300, 1.0, 0.20);
    QVERIFY2(!m_controller->strokeEnd(),
             "strokeEnd() after Esc reported a commit — strokeCancel() left "
             "the session ARMED, so the two post-cancel samples above were "
             "ACCEPTED and submitted. The session was not disarmed, it was "
             "merely emptied");
    // ★ MIRROR, recorded rather than fixed: the `m_strokeSamples.clear()`
    // beside that flag-clear is UNPINNABLE BY CONSTRUCTION and deliberately
    // carries no assertion. Every reader of that buffer is gated on
    // m_strokeActive (strokeAppend, strokeEnd), and strokeBegin clears it
    // unconditionally on the next press — so with the flag false nothing can
    // observe it. One statement of the pair is observable and now pinned;
    // the other is unobservable and therefore fine. Do NOT "fix" it.
    QImage afterEnd;
    QVERIFY(m_controller->readRegionImage(corridor, &afterEnd));
    QCOMPARE(afterEnd.size(), corridor.size());
    int endDark = 0;
    for (int y = 0; y < afterEnd.height(); ++y)
        for (int x = 0; x < afterEnd.width(); ++x)
            if (qGray(afterEnd.pixel(x, y)) < 128)
                ++endDark;
    QVERIFY2(endDark == 0,
             qPrintable(QStringLiteral("calling strokeEnd() after Esc INKED "
                                       "the corridor (%1 dark px) — the "
                                       "abandoned stroke was still in the "
                                       "controller's sample buffer")
                            .arg(endDark)));

    // ★ ...and the second document's coordinate mapping still lands ink in
    // the corridor. THIS is what the arm below actually discriminates —
    // without it, `dark == 0` above could hold because the strokes missed
    // the corridor entirely after the mid-case newDocument + resetView,
    // not because Esc worked.
    //
    // ★ ROUND-4 MINOR-2, correcting round 3: this arm does NOT also cover
    // wedging, and the claim that it would red on a conditional strokeBegin
    // was wrong. The arm runs AFTER the strokeEnd() above, and strokeEnd()
    // leaves the session disarmed on BOTH paths — it either returns early
    // because m_strokeActive was already false, or clears m_strokeActive
    // before doing anything else. So any wedged state is destroyed two
    // statements before this arm runs, and a conditional strokeBegin would
    // still arm. This arm discriminates the SECOND DOCUMENT'S COORDINATE
    // MAPPING and nothing else; that is the claim worth keeping.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(300, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
    QImage again;
    QVERIFY(m_controller->readRegionImage(corridor, &again));
    QCOMPARE(again.size(), corridor.size());
    int againDark = 0;
    for (int y = 0; y < again.height(); ++y)
        for (int x = 0; x < again.width(); ++x)
            if (qGray(again.pixel(x, y)) < 128)
                ++againDark;
    QVERIFY2(againDark > 0,
             "a normal stroke on the SECOND document left no ink in the "
             "corridor — the coordinate mapping did not survive the mid-case "
             "newDocument + resetView, so the absence checks above were "
             "measuring a missed corridor rather than a working cancel");

    // ★★ ROUND-4 MINOR-3 — THE SECOND CALL SITE. cancelGesture() is reached
    // from THREE places (canvas_view.cpp): the documentReset connection,
    // setTool(), and Escape. Everything above this line exercises only the
    // Escape one, yet the G-1 arm silently changed the other two as well.
    // This pins the documentReset path, which was covered in NEITHER
    // direction: no test drove documentReset with a live stroke, so
    // re-adding an m_strokeInProgress clear anywhere EARLIER in that signal
    // fan-out (onDocumentReset is connected first, and direct connections
    // fire in order) would silently kill the G-1 arm on exactly the path
    // that connection exists to serve, and every assertion above would stay
    // green.
    //
    // ★ ITS OWN LIVENESS FLOOR FIRST — this arm is TWO absence assertions,
    // the canvas pin and the controller pin below; the QVERIFY(newDocument)
    // between them is POSITIVE, which is why the QVERIFY2 message on the
    // next line already says "the two negative pins below". (Round-5 NIT-2:
    // this said "three", which described the fix as supplied rather than the
    // arm as landed.) Two or three, it is the same shape MAJOR-1 was about:
    // without the positive control a press that stops arming makes both of
    // them pass.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QVERIFY2(m_canvas->gestureInFlight(),
             "no stroke in flight before the documentReset — the two "
             "negative pins below would pass vacuously");
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    // ★ ROUND-5 NIT-4 — RESTORE THE MAPPING docToView() PROMISES. That
    // helper is a fixed +80/+100 offset whose comment states its precondition
    // outright: "the view transform AFTER resetView". onDocumentReset ends in
    // zoomFit(), which on a 640x400 document in an 800x600 view gives
    // zoom = min((800-24)/640, (600-24)/400) = 1.2125 and offset (12, 57.5) —
    // so without this line the trailing mouseRelease below, written as
    // docToView(200, 300), actually lands on document point (221, 282).
    // Harmless TODAY (the flag is false and m_gesture is None, so the release
    // is a no-op) and exactly the silent-wrong trap the next appended
    // assertion would fall into. The mid-case newDocument above is followed by
    // resetView() for the same reason; this one was not. No zoomPercent
    // assertion is added beside it: this case already asserts zoomOneToOne
    // lands on 100 after the mid-case swap, so a second one would be a
    // duplicate reader, and resetView() is inert with respect to both pins
    // below (zoomOneToOne -> setZoomCentered touches zoom/offset/ants only).
    resetView();
    QVERIFY2(!m_canvas->gestureInFlight(),
             "documentReset left the stroke in flight");
    // ★ THIS is the load-bearing one. Re-adding a flag clear to
    // onDocumentReset would satisfy the canvas pin above while making
    // cancelGesture's `if (m_strokeInProgress)` arm dead — leaving the
    // CONTROLLER armed with the previous document's samples, one strokeEnd()
    // away from landing stale geometry in the new document (V-D4a).
    QVERIFY2(!m_controller->strokeEnd(),
             "documentReset left the controller session armed — the previous "
             "document's samples are still buffered and committable into the "
             "NEW document");
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 300));

    // ★ MIRRORS, stated rather than patched. The THIRD call site —
    // setTool() mid-stroke — is left unpinned by this round: it is a real
    // scope expansion (a tool switch now ABANDONS a live stroke instead of
    // committing it under the original m_strokeKind) but it is documented at
    // cancelGesture() where the §39.2 rationale lives, and it shares its
    // mechanism with the two arms pinned here. mouseUngrabEvent, eraser,
    // Esc-after-release and zero-move cancel remain uncovered and are NOT
    // claimed by this round.
}

// G-2 (MAJOR, q-gesture T33, REPRODUCED there; regression case authored here).
// Esc is not the only way to stop being the one who owns a stroke. Taking focus
// off the canvas mid-drag left m_strokeInProgress true, and the release then
// called strokeEnd() and COMMITTED the stroke (bounds 93,293 214x14 as
// reproduced). focusOutEvent already cleared m_spaceHeld "for exactly this
// hygiene family" by its own comment, and handled precisely one member of it.
//
// ★ THE PATTERN BELOW IS COPIED FROM brushEscapeMidStrokeDoesNotCommit IN THIS
// FILE, NOT RE-DERIVED. There is a same-named case in tst_gesture_net.cpp with
// ONE assertion and no floor at all; this file's version is the one that argues
// its own absence assertion needs a control, so it is the one to copy.
void TstSelectionGestures::brushFocusLossMidStrokeDoesNotCommit()
{
    const QRect corridor(90, 290, 220, 20); // the band the stroke would ink

    // ★ POSITIVE CONTROL FIRST, load-bearing for the same reason it is in the
    // Esc case: the assertion at the end is "the corridor holds no ink", and an
    // unpainted corridor is ALSO what a brush that never works at all produces.
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
    QImage inked;
    QVERIFY2(m_controller->readRegionImage(corridor, &inked),
             "readRegionImage refused the stroke corridor — the probe itself is "
             "unusable and the absence check below would be vacuous");
    QCOMPARE(inked.size(), corridor.size());
    int inkedDark = 0;
    for (int y = 0; y < inked.height(); ++y)
        for (int x = 0; x < inked.width(); ++x)
            if (qGray(inked.pixel(x, y)) < 128)
                ++inkedDark;
    QVERIFY2(inkedDark > 0,
             qPrintable(QStringLiteral("POSITIVE CONTROL FAILED: an UNINTERRUPTED "
                                       "brush stroke left no ink in the corridor "
                                       "(%1 dark px). The absence assertion below "
                                       "cannot discriminate.")
                            .arg(inkedDark)));

    // Fresh document, so the control's ink cannot be mistaken for the
    // abandoned stroke's.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    resetView();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);

    // ...now the same gesture, with FOCUS TAKEN AWAY before the release.
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));

    // ★★ LIVENESS FLOOR. Without it, a press that stops arming makes every
    // assertion below pass while testing nothing.
    QVERIFY2(m_canvas->gestureInFlight(),
             "the press did not arm a stroke — the focus-loss assertions below "
             "would pass without exercising anything");

    // ★★ THE TRIGGER, AND THE GUARD THAT MAKES IT REAL. q-gesture's first probe
    // used contentItem()->forceActiveFocus(), which did NOT take focus off the
    // canvas; a non-vacuity guard caught it and the case would otherwise have
    // gone GREEN WHILE TESTING NOTHING. So: setFocus(false), then PROVE the
    // canvas actually lost focus before reading anything as meaningful.
    m_canvas->setFocus(false);
    QVERIFY2(!m_canvas->hasFocus(),
             "the canvas KEPT focus — this probe did not exercise the path it "
             "was written for (q-gesture hit exactly this)");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));

    QVERIFY2(!m_canvas->gestureInFlight(),
             "focus loss left the stroke session live");
    QImage after;
    QVERIFY2(m_controller->readRegionImage(corridor, &after),
             "readRegionImage refused the corridor on the second document");
    QCOMPARE(after.size(), corridor.size());
    int afterDark = 0;
    for (int y = 0; y < after.height(); ++y)
        for (int x = 0; x < after.width(); ++x)
            if (qGray(after.pixel(x, y)) < 128)
                ++afterDark;
    QVERIFY2(afterDark == 0,
             qPrintable(QStringLiteral("G-2: focus loss mid-stroke did NOT abandon "
                                       "the stroke — the release COMMITTED it "
                                       "(%1 dark px in the corridor)")
                            .arg(afterDark)));
}

// G-2, THE OTHER HALF -- the one canvas_view.cpp calls "THE HALF NO TEST CAN
// SEE". That comment is precisely right about QTest MOUSE SYNTHESIS, which
// cannot produce a grab steal. The conclusion drawn from it was too strong:
// a test does not need synthesis, because QQuickItem::ungrabMouse() and
// QQuickWindow::mouseGrabberItem() are BOTH PUBLIC (measured in the installed
// headers, qquickitem.h:297 and qquickwindow.h:102). The "future harness" that
// comment hopes for is the Qt public API.
//
// So this case steals the grab directly and asserts the same property the
// focus-loss case asserts. Everything else is the focus case's pattern,
// copied rather than re-derived.
void TstSelectionGestures::brushGrabLossMidStrokeDoesNotCommit()
{
    const QRect corridor(90, 290, 220, 20);

    // ★ POSITIVE CONTROL, same reason as its two siblings: "no ink" is also
    // what a brush that never works produces.
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
    QImage inked;
    QVERIFY2(m_controller->readRegionImage(corridor, &inked),
             "readRegionImage refused the stroke corridor — the probe itself is "
             "unusable and the absence check below would be vacuous");
    QCOMPARE(inked.size(), corridor.size());
    int inkedDark = 0;
    for (int y = 0; y < inked.height(); ++y)
        for (int x = 0; x < inked.width(); ++x)
            if (qGray(inked.pixel(x, y)) < 128)
                ++inkedDark;
    QVERIFY2(inkedDark > 0,
             qPrintable(QStringLiteral("POSITIVE CONTROL FAILED: an UNINTERRUPTED "
                                       "brush stroke left no ink in the corridor "
                                       "(%1 dark px). The absence assertion below "
                                       "cannot discriminate.")
                            .arg(inkedDark)));

    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    resetView();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);

    // ...now the same gesture, with THE GRAB STOLEN before the release.
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));

    // ★★ LIVENESS FLOOR.
    QVERIFY2(m_canvas->gestureInFlight(),
             "the press did not arm a stroke — the grab-loss assertions below "
             "would pass without exercising anything");

    // ★★ NON-VACUITY ON THE TRIGGER, in BOTH directions. Before: the canvas
    // must actually HOLD the grab, or ungrabMouse() is a no-op on an item that
    // was never the grabber and this case proves nothing.
    QVERIFY2(m_window->mouseGrabberItem() == m_canvas,
             qPrintable(QStringLiteral("the canvas does NOT hold the mouse grab "
                                       "mid-drag (grabber: %1) — ungrabbing it "
                                       "would exercise nothing")
                            .arg(QString::fromLatin1(
                                m_window->mouseGrabberItem()
                                    ? m_window->mouseGrabberItem()
                                          ->metaObject()->className()
                                    : "none"))));

    m_canvas->ungrabMouse();

    // After: prove the steal actually took. Without this, a Qt version that
    // made ungrabMouse() conditional would leave the grab in place and every
    // assertion below would pass for the wrong reason.
    QVERIFY2(m_window->mouseGrabberItem() != m_canvas,
             "the canvas STILL holds the grab after ungrabMouse() — "
             "mouseUngrabEvent cannot have fired, so this probe tests nothing");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));

    QVERIFY2(!m_canvas->gestureInFlight(),
             "grab loss left the stroke session live");
    QImage after;
    QVERIFY2(m_controller->readRegionImage(corridor, &after),
             "readRegionImage refused the corridor on the second document");
    QCOMPARE(after.size(), corridor.size());
    int afterDark = 0;
    for (int y = 0; y < after.height(); ++y)
        for (int x = 0; x < after.width(); ++x)
            if (qGray(after.pixel(x, y)) < 128)
                ++afterDark;
    QVERIFY2(afterDark == 0,
             qPrintable(QStringLiteral("G-2: grab loss mid-stroke did NOT abandon "
                                       "the stroke — the release COMMITTED it "
                                       "(%1 dark px in the corridor)")
                            .arg(afterDark)));
}

// ★★ ROUND-5 MINOR-1 — THE THIRD m_strokeInProgress CLEAR, the one that is
// not inside cancelGesture(), and the only one that was shipped code rather
// than a comment.
//
// The document can VANISH without a documentReset: undoing doc.new is a
// legal journal step that leaves a pre-init handle, refreshInfo's failure
// arm emits infoChanged, and setController()'s infoChanged lambda drops the
// cache, the pending echoes and m_strokeInProgress. It did NOT disarm the
// controller — and because the flag is now false, the NEXT document's
// documentReset finds cancelGesture's `if (m_strokeInProgress)` arm dead, so
// nothing downstream disarms it either. The vanished document's samples ride
// into the new document, committable by one strokeEnd().
//
// That is MUT-F's exact shape, present in shipped code, and it survived four
// rounds because it has no presently-reachable consequence: mouseReleaseEvent
// gates its strokeEnd() on the flag, tourStroke calls strokeBegin first, and
// the next press scrubs the buffer via strokeBegin. But strokeEnd() is
// Q_INVOKABLE (document_controller.h:389) and "one call away" is this lane's
// own stated justification for strokeCancel() existing, so it is closed.
//
// This case does NOT go through Esc, and it is NOT the documentReset arm: it
// is the vanish-without-reset path, which no test drove in any direction.
void TstSelectionGestures::documentVanishDisarmsControllerSession()
{
    setTool("brush.stroke");
    m_canvas->setProperty("brushColor", QColor(Qt::black));
    m_canvas->setProperty("brushSize", 12.0);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));

    // ★ LIVENESS FLOOR (the MAJOR-1 rule, applied at authoring time rather
    // than in the round after). Everything below is an absence assertion, so
    // a press that stops arming would pass all of them. It is also the only
    // observable the shell offers that the CONTROLLER session opened at all —
    // DocumentController exposes no isStrokeActive() — and it is sound as
    // that proxy because mousePressEvent sets m_strokeInProgress and calls
    // strokeBegin() in the SAME branch, four lines apart.
    QVERIFY2(m_canvas->gestureInFlight(),
             "no stroke in flight before the undo — the press did not arm a "
             "stroke, so every disarm assertion below would pass vacuously");

    // The document VANISHES. No documentReset is emitted on this path; the
    // only signal the canvas sees is infoChanged with hasDocument() false.
    m_controller->undo();

    // ★ THE PREMISE, MEASURED RATHER THAN ASSUMED. If undo did not actually
    // step to a pre-init handle, the lambda's `!hasDocument()` gate never
    // fires and this case silently degrades into an ordinary-path test that
    // pins nothing. This is the control that makes the rest discriminate.
    QVERIFY2(!m_controller->hasDocument(),
             "undo did not vanish the document — the infoChanged lambda's "
             "gate never fired and this case measured nothing");
    QVERIFY2(!m_canvas->gestureInFlight(),
             "the document vanished and the canvas still reports a gesture "
             "in flight");

    // ...and now the next document arrives. Its documentReset DOES reach
    // cancelGesture, but cancelGesture's stroke arm is already dead — the
    // lambda cleared the flag. Nothing on this path can disarm the
    // controller except the lambda itself, which is the whole point.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    resetView();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);
    QVERIFY2(!m_controller->strokeEnd(),
             "the VANISHED document's session was still armed: its samples "
             "survived the document swap and strokeEnd() has just committed "
             "them into the NEW document (V-D4a)");

    // ★ MIRROR, decided rather than defaulted: NO pixel-level second reader
    // here, unlike the Esc case. "The corridor holds no ink" is an absence
    // assertion and would need its own positive control — a second, real
    // stroke committed into the new document purely to prove the probe can
    // see ink — which is ceremony this arm does not need: the strokeEnd()
    // pin above is not an absence assertion. Under the defect it returns
    // TRUE, a value the healthy path can never produce.
    //
    // ★ MIRROR, the inverse defect: a path that disarms the CONTROLLER
    // while leaving m_strokeInProgress true. That is MUT-B's shape and it is
    // pinned by the Esc arm above. Verified here rather than assumed:
    // strokeCancel() has exactly two callers in canvas_view.cpp — the one in
    // cancelGesture (flag cleared on the line before) and the one in this
    // lambda (same ordering). CLOSED BY CONSTRUCTION at both.
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
}

// Offscreen + a real QQuickWindow; the engine links in (cabi).
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QGuiApplication app(argc, argv);
    TstSelectionGestures tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_selection_gestures.moc"
