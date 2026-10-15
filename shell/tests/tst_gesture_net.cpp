// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// q-gesture (standing functional-QA session): the GROWING half of the
// mouse-and-keyboard truth layer. tst_selection_gestures pins the five
// selection tools + pen + move (v1.0.1 items 6/8/9/10). This file pins
// every OTHER interactive canvas surface on the same pattern — real
// synthesized mouse/keyboard events against a REAL engine document in
// an offscreen window, assertions read ENGINE truth (readRegionImage /
// layers()), never shell echo state.
//
// Surfaces covered here: shape.rect / shape.ellipse drag-commit, the
// brush/eraser stroke session, the transform.free handle frame (scale,
// rotate ring, Enter-commits / release-does-not / Esc-cancels), and the
// text click-vs-drag layout split.

#include "canvas_view.h"
#include <cmath>
#include <QMouseEvent>
#include "document_controller.h"
#include "i18n_catalog.h"

#include <QGuiApplication>
#include <QQuickWindow>
#include <QDir>
#include <QDirIterator>
#include <QFileInfo>
#include <QFile>
#include <QRegularExpression>
#include <QtTest>

using namespace appname::shell;

namespace {

// Engine-truth content bounds: the bbox of every pixel that is not the
// white background, read through the same door the canvas repaints from.
// QTest::mouseMove fixes the modifier state at NoModifier (Qt6 gives it no
// stateKey parameter), but item 13's aspect lock is read on the MOVE, not
// on the press - so a Shift-held drag cannot be expressed with QTest alone.
// This sends the move as a real QMouseEvent through the window, which
// QQuickWindow delivers to the current grabber item exactly as it delivers
// QTest's. Lower-level than the rest of the file, and used ONLY where the
// modifier must survive the drag.
static void moveWithModifiers(QWindow *w, const QPoint &pos,
                              Qt::KeyboardModifiers mods)
{
    QMouseEvent me(QEvent::MouseMove, QPointF(pos),
                   QPointF(w->mapToGlobal(pos)), Qt::NoButton, Qt::LeftButton,
                   mods);
    QCoreApplication::sendEvent(w, &me);
}

QRect engineContentBounds(DocumentController *c)
{
    const int w = c->docWidth();
    const int h = c->docHeight();
    if (w <= 0 || h <= 0)
        return QRect();
    const int maxRows = qMax(1, (1 << 22) / qMax(1, w));
    QRect bounds;
    for (int y = 0; y < h; y += maxRows) {
        QImage strip;
        if (!c->readRegionImage(QRect(0, y, w, qMin(maxRows, h - y)), &strip))
            return QRect();
        for (int sy = 0; sy < strip.height(); ++sy) {
            for (int sx = 0; sx < strip.width(); ++sx) {
                const QColor p = strip.pixelColor(sx, sy);
                if (p.alpha() < 250 || p.red() < 250 || p.green() < 250
                    || p.blue() < 250)
                    bounds = bounds.united(QRect(sx, y + sy, 1, 1));
            }
        }
    }
    return bounds;
}

// Engine-truth selection bounds via the coverage-mask readback door.
QRect engineSelectionBounds(DocumentController *c)
{
    const int w = c->docWidth();
    const int h = c->docHeight();
    if (w <= 0 || h <= 0)
        return QRect();
    const int maxRows = qMax(1, (1 << 22) / qMax(1, w));
    QRect bounds;
    for (int y = 0; y < h; y += maxRows) {
        QImage strip;
        if (!c->readSelectionImage(QRect(0, y, w, qMin(maxRows, h - y)),
                                   &strip))
            return QRect();
        for (int sy = 0; sy < strip.height(); ++sy) {
            const uchar *row = strip.constScanLine(sy);
            for (int sx = 0; sx < strip.width(); ++sx)
                if (row[sx])
                    bounds = bounds.united(QRect(sx, y + sy, 1, 1));
        }
    }
    return bounds;
}

// Bounds of pixels that are CLEARLY the blue test block — immune to the
// resample ringing / near-white haze a warp can leave behind, which a
// plain "not white" scan counts as content.
QRect engineBlueBounds(DocumentController *c)
{
    const int w = c->docWidth();
    const int h = c->docHeight();
    QRect b;
    for (int y = 0; y < h; y += 2) {
        QImage strip;
        if (!c->readRegionImage(QRect(0, y, w, 1), &strip))
            return QRect();
        for (int x = 0; x < w; x += 2) {
            const QColor p = strip.pixelColor(x, 0);
            if (p.blue() > p.red() + 60 && p.blue() > 100)
                b = b.united(QRect(x, y, 1, 1));
        }
    }
    return b;
}


// Centroid of pixels whose dominant channel matches a marker colour.
// Returns false when the marker is ABSENT — the caller must treat that as
// a FAILURE, never a skip (an instrument that can print neither pass nor
// fail is not an instrument).
bool markerCentroid(DocumentController *c, char channel, QPointF *out)
{
    const int w = c->docWidth();
    const int h = c->docHeight();
    double sx = 0, sy = 0;
    qint64 n = 0;
    for (int y = 0; y < h; ++y) {
        QImage row;
        if (!c->readRegionImage(QRect(0, y, w, 1), &row))
            return false;
        for (int x = 0; x < w; ++x) {
            const QColor p = row.pixelColor(x, 0);
            if (p.alpha() < 200)
                continue;
            const bool hit = channel == 'r'
                ? (p.red() > p.green() + 60 && p.red() > p.blue() + 60)
                : (p.green() > p.red() + 60 && p.green() > p.blue() + 60);
            if (hit) { sx += x; sy += y; ++n; }
        }
    }
    if (n == 0)
        return false;
    *out = QPointF(sx / double(n), sy / double(n));
    return true;
}

// §18.4's mapping, derived INDEPENDENTLY of the door: source bounds ->
// unit square (affine) -> destination quad (projective closed form for
// the unit square, corners in the contract's TL/TR/BR/BL order). Never
// read back from the implementation.
QPointF mapThroughQuad(const QRectF &src, const QPointF q[4], const QPointF &p)
{
    const double u = (p.x() - src.left()) / src.width();
    const double v = (p.y() - src.top()) / src.height();
    const double x0 = q[0].x(), y0 = q[0].y();   // TL -> (0,0)
    const double x1 = q[1].x(), y1 = q[1].y();   // TR -> (1,0)
    const double x2 = q[2].x(), y2 = q[2].y();   // BR -> (1,1)
    const double x3 = q[3].x(), y3 = q[3].y();   // BL -> (0,1)
    const double dx1 = x1 - x2, dx2 = x3 - x2, dx3 = x0 - x1 + x2 - x3;
    const double dy1 = y1 - y2, dy2 = y3 - y2, dy3 = y0 - y1 + y2 - y3;
    double a, b, c, d, e, f, g, hh;
    const double den = dx1 * dy2 - dx2 * dy1;
    if (qFuzzyIsNull(dx3) && qFuzzyIsNull(dy3)) {
        a = x1 - x0; b = x2 - x1; c = x0;
        d = y1 - y0; e = y2 - y1; f = y0;
        g = 0.0; hh = 0.0;
    } else {
        g = (dx3 * dy2 - dx2 * dy3) / den;
        hh = (dx1 * dy3 - dx3 * dy1) / den;
        a = x1 - x0 + g * x1;
        b = x3 - x0 + hh * x3;
        c = x0;
        d = y1 - y0 + g * y1;
        e = y3 - y0 + hh * y3;
        f = y0;
    }
    const double wgt = g * u + hh * v + 1.0;
    return QPointF((a * u + b * v + c) / wgt, (d * u + e * v + f) / wgt);
}

QColor enginePixel(DocumentController *c, int x, int y)
{
    QImage px;
    if (!c->readRegionImage(QRect(x, y, 1, 1), &px))
        return QColor();
    return px.pixelColor(0, 0);
}

bool isWhitish(const QColor &c)
{
    return c.red() > 245 && c.green() > 245 && c.blue() > 245;
}

// Strongly RED-dominant. The test is relative, not absolute: the fill
// door takes 0.85 and the readback returns 176 (a transfer-curve step
// this test has no business hard-coding), so an absolute cut at 180
// rejects the true colour. What the erase control actually needs is only
// that red is unmistakably dominant — which WHITE (r==g==b) can never
// satisfy, whatever the curve does.
bool isRedDominant(const QColor &c)
{
    return c.red() > 120 && c.red() > c.green() + 80
        && c.red() > c.blue() + 80;
}

// Strongly dark on every channel — the default brush is (20,20,20).
bool isDarkish(const QColor &c)
{
    return c.red() < 120 && c.green() < 120 && c.blue() < 120;
}

} // namespace

class TstGestureNet : public QObject {
    Q_OBJECT
private slots:
    void init();
    void cleanup();

    // --- shape primitives (drag-commit on release) ---------------------
    void shapeRectDragCommitsShapeLayer();
    void shapeEllipseDragIsElliptical();
    void shapeDegenerateDragCommitsNothing();

    // --- brush / eraser stroke session ---------------------------------
    void brushDragPaintsAlongPathOnly();
    void smudgeDragReachesTheEngineAsASmudgeStroke();
    void paletteModeToolsReachTheEngineAsTheirFamilyKind();
    void healOnAnEmptyLayerSaysSoInsteadOfSucceedingSilently();
    // DISCRIMINATING CONTROL for brushDragPaintsAlongPathOnly (C-120).
    void brushBentDragInksTheBendNotTheChord();
    void brushEscapeMidStrokeDoesNotCommit();
    void focusLostMidStrokeDoesNotCommit();
    void toolSwitchMidStrokeAbandonsTheStroke();
    void eraserDragRemovesPaint();
    // DISCRIMINATING CONTROL for eraserDragRemovesPaint (C-120).
    void eraserOverColouredBaseRemovesRatherThanCovers();

    // --- transform.free handle frame -----------------------------------
    void transformReleaseWithoutEnterCommitsNothing();
    void transformScaleHandleCommitsOnEnter();
    void transformEscapeCancelsFrame();
    void transformRotateRingCommitsOnEnter();
    void transformFrameFramesTheLayerNotTheCanvas();
    void transformOnEmptyLayerOpensNoSessionAndSaysSo();
    void transformCornerHandleAspectLocksAndShiftUnlocks();
    void transformOnMaskedLayerFramesCanvasAndSaysSo();
    void transformPreviewLeavesNoTrailWhenTheMatrixChanges();
    void shapeStrokeIsServedOrRefusedTyped();

    // --- text tool click-vs-drag ---------------------------------------
    void textClickIsRefusedAndAddsNoLayer();
    void textDragIsRefusedAndAddsNoLayer();

    // --- standing coverage rule ----------------------------------------
    void registryCoverageLint();
    void censusCoverageLint();
    void armedCitationsAreStillArmed();
    void optionsModelCoverageLint();
    void brushSizeOptionReachesTheEngine();
    void brushHardnessOptionReachesTheEngine();
    void shapeStrokeWidthOptionReachesTheEngine();

    // --- perspective / puppet handle sessions --------------------------
    void perspectiveEngineMapsSourceBoundsPerContract();
    void perspectiveCornerDragCommitsOnEnter();
    void perspectiveKeystoneOnLayerFrameCommits();
    void perspectiveInteriorPixelsMapPerHomography();
    void perspectiveIdentityEnterCommitsNothing();
    void perspectiveEscapeCancelsFrame();
    void puppetSessionRefusesTypedRatherThanSilently();

    // --- navigation + modifier composition + crop ----------------------
    void zoomedAndPannedMarqueeLandsOnDocPixels();
    void modifierComposedSelectionsAddAndSubtract();
    void cropDragEmitsExactCropRect();

    // --- exploratory end-to-end run (checkpoint frames) ----------------
    void exploratoryRunNewDocToExport();
    void largeCanvasGestureLatency();
    void maskedLargeCanvasGestureLatency();
    void largeCanvasRepaintSensitiveLatency();
    void shapeCommitRepaintsTheWholeCanvas();
    void maskedLayerMoveRefusesTyped();
    void wandOnTransparentIsAlphaAware();
    void toolOptionReachesTheEngineThroughTheGesture();

private:
    // statusPending REFUSES an uncataloged id by design (DocumentController
    // gates on Catalog::has), so against the empty catalog this fixture used
    // to run, item 19/21's honest-state notices could not be observed AT ALL
    // — "it said nothing" and "the arm never fired" were the same reading.
    // T39 cost me two false ARMED-SILENT reds before I checked the gate.
    // Only the three ids those arms emit are seeded, with DISTINCTIVE
    // sentinel text: asserting on the sentinel proves the id resolved
    // THROUGH the catalog rather than being key-echoed, and proves WHICH id
    // fired. Everything else stays uncataloged so the existing cases keep
    // asserting against key-echo exactly as before. (Same fixture shape as
    // tst_selection_gestures; drift between these ids and the shipped
    // catalogs is the i18n audit's job, not this file's.)
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
    // Same derivation as tst_selection_gestures: zoom 1, the 640x400 doc
    // centered in the 800x600 view — offset (80,100). Guarded by the
    // zoomPercent==100 assert in init().
    QPoint docToView(int x, int y) const { return QPoint(x + 80, y + 100); }
    void bootOnce();
    // Paint a solid block on the active layer through the COMMAND door
    // (setup, not the surface under test).
    void fillBlock(const QRect &r, double red, double green, double blue);
    // Same, but on a FRESH raster layer above the white base and left
    // active: transform cases must not drag the background with the
    // subject (a whole-layer transform of the base turns the uncovered
    // canvas transparent and makes any content-bounds read meaningless).
    void fillBlockOnNewLayer(const QRect &r, double red, double green,
                             double blue);
};

void TstGestureNet::bootOnce()
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

void TstGestureNet::init()
{
    if (!m_controller)
        bootOnce();
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);
    QVERIFY(!m_controller->selectionActive());
    m_canvas->forceActiveFocus();
}

void TstGestureNet::cleanup()
{
    if (m_controller && m_controller->hasDocument())
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    // Drop any half-open tool session so it cannot leak into the next case.
    if (m_canvas) {
        m_canvas->forceActiveFocus();
        QTest::keyClick(m_window, Qt::Key_Escape);
        setTool("select.rect");
    }
}

void TstGestureNet::fillBlock(const QRect &r, double red, double green,
                              double blue)
{
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
    const int idx = qMax(0, m_controller->activeLayerIndex());
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().at(idx).toMap().value(
                      QStringLiteral("layerId")));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok"))
                .toBool());
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
}

void TstGestureNet::fillBlockOnNewLayer(const QRect &r, double red,
                                        double green, double blue)
{
    QVariantMap params;
    params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, params, {})
                .value(QStringLiteral("ok"))
                .toBool());
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);
    fillBlock(r, red, green, blue);
}

// ---------------------------------------------------------------- shapes

void TstGestureNet::shapeRectDragCommitsShapeLayer()
{
    setTool("shape.rect");
    const int before = m_controller->layers().size();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 90));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseMove(m_window, docToView(420, 290));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(420, 290));
    QVERIFY2(m_controller->layers().size() == before + 1,
             qPrintable(QStringLiteral("shape.rect drag did not add a shape "
                                       "layer (%1 -> %2); status: %3")
                            .arg(before)
                            .arg(m_controller->layers().size())
                            .arg(m_controller->statusText())));
    // Engine truth: the dragged rect is painted, the outside is not.
    QVERIFY2(!isWhitish(enginePixel(m_controller, 270, 190)),
             "shape.rect committed a layer but painted no pixels inside it");
    QVERIFY2(isWhitish(enginePixel(m_controller, 60, 50)),
             "shape.rect painted OUTSIDE the dragged rect");
    const QRect b = engineContentBounds(m_controller);
    QVERIFY2(QRect(118, 88, 305, 205).contains(b) && b.width() >= 175
                 && b.height() >= 105,
             qPrintable(QStringLiteral("shape.rect bounds off: %1,%2 %3x%4")
                            .arg(b.x()).arg(b.y()).arg(b.width())
                            .arg(b.height())));
}

void TstGestureNet::shapeEllipseDragIsElliptical()
{
    setTool("shape.ellipse");
    const int before = m_controller->layers().size();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(300, 250));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 250));
    QVERIFY2(m_controller->layers().size() == before + 1,
             qPrintable(QStringLiteral("shape.ellipse drag did not commit; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    // The SHAPE discrimination (the tst_selection_gestures D5 pattern): a
    // rect-rasterization regression fits the bbox, so probe the corner
    // (outside an ellipse) against the center (inside it).
    QVERIFY2(isWhitish(enginePixel(m_controller, 103, 103)),
             "shape.ellipse painted its bbox CORNER — the primitive "
             "rasterized as a rect, not an ellipse");
    QVERIFY2(!isWhitish(enginePixel(m_controller, 200, 175)),
             "shape.ellipse painted nothing at its center");
}

void TstGestureNet::shapeDegenerateDragCommitsNothing()
{
    // submitShapePrimitive() drops sub-pixel drags — a stray click with a
    // shape tool must not mint an empty layer.
    setTool("shape.rect");
    const int before = m_controller->layers().size();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 150));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 150));
    QCOMPARE(m_controller->layers().size(), before);
    QVERIFY2(engineContentBounds(m_controller).isEmpty(),
             "a degenerate shape click painted pixels");
}

// ------------------------------------------------------------ brush/eraser

void TstGestureNet::brushDragPaintsAlongPathOnly()
{
    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 200));
    QTest::mouseMove(m_window, docToView(200, 200));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseMove(m_window, docToView(400, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(400, 200));
    QVERIFY2(!isWhitish(enginePixel(m_controller, 250, 200)),
             qPrintable(QStringLiteral("brush drag painted nothing ON the "
                                       "path; status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY2(isWhitish(enginePixel(m_controller, 250, 340)),
             "brush drag painted far OFF the path");
    const QRect b = engineContentBounds(m_controller);
    QVERIFY2(b.left() >= 80 && b.right() <= 420 && b.top() >= 180
                 && b.bottom() <= 220,
             qPrintable(QStringLiteral("brush stroke bounds off: %1,%2 %3x%4")
                            .arg(b.x()).arg(b.y()).arg(b.width())
                            .arg(b.height())));
}

void TstGestureNet::paletteModeToolsReachTheEngineAsTheirFamilyKind()
{
    // THE TABLE WALKED. canvas_view maps a palette spelling to a (wire
    // kind, mode) pair because this is the first place the two diverge —
    // the user picks "dodge_burn.dodge" and the engine registers
    // "dodge_burn.stroke". This case asserts the ENGINE saw the FAMILY
    // kind, not the palette spelling, which is the thing a branch-entered
    // check could not tell apart from a stroke rejected for an unknown
    // kind.
    //
    // ★★★ THE MODE COLUMN WAS ADDED 2026-08-29 AND IT IS NOT COSMETIC.
    // This walk pinned tool -> KIND and never checked the mode, so it
    // could not have caught a row that reached the right family with the
    // WRONG mode -- and for sponge (Q-D3) the two candidate modes are
    // `saturate` and `desaturate`, OPPOSITE operations. A kind-only walk
    // goes green on the tool doing exactly the wrong thing, which is the
    // failure the operator refused to let a desk guess at.
    struct Row { const char *tool; const char *kind; const char *mode; };
    static const Row rows[] = {
        {"dodge_burn.dodge",     "dodge_burn.stroke",   "dodge"},
        {"dodge_burn.burn",      "dodge_burn.stroke",   "burn"},
        {"blur_sharpen.blur",    "blur_sharpen.stroke", "blur"},
        {"blur_sharpen.sharpen", "blur_sharpen.stroke", "sharpen"},
        // Q-D1(a): heal is the row whose SPELLING equals its kind, so it
        // exercises the mode half alone -- the case the other rows cannot
        // distinguish from a pure rename.
        {"heal.stroke",          "heal.stroke",         "spot"},
        // Q-D3(b): sponge -> desaturate, the operator's ruling. It was a
        // deliberate absence until 2026-08-29 and this walk is where the
        // ruling is now enforced.
        {"dodge_burn.sponge",    "dodge_burn.stroke",   "desaturate"},
    };
    // Guard the walk itself: a table that shrinks to nothing would make
    // every assertion below vacuous while the case stayed green. Bumped
    // 4 -> 6 deliberately, with the two rulings named above.
    QCOMPARE(int(sizeof(rows) / sizeof(rows[0])), 6);

    for (const Row &r : rows) {
        const int before = m_controller->historyEntries().size();
        const int attemptsBefore = m_controller->submissionAttempts();
        setTool(r.tool);
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(100, 200));
        QTest::mouseMove(m_window, docToView(200, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(200, 200));
        const QVariantList h = m_controller->historyEntries();
        // DISCRIMINATE THE TWO FAILURES rather than printing one sentence
        // for both. No journal entry has two very different causes: the
        // tool was never DISPATCHED (no attempt), or it was dispatched
        // with a kind the engine REJECTED (an attempt, no history). An
        // earlier draft said "the engine was never asked" for both, which
        // is false for the second and is a symptom named as a mechanism.
        const bool attempted =
            m_controller->submissionAttempts() > attemptsBefore;
        QVERIFY2(h.size() > before,
                 qPrintable(QStringLiteral("%1 produced NO journal entry — %2; "
                                           "status: %3")
                                .arg(QLatin1String(r.tool),
                                     attempted
                                         ? QStringLiteral("the engine WAS asked "
                                                          "and refused it (wrong "
                                                          "wire kind?)")
                                         : QStringLiteral("the engine was never "
                                                          "asked (not dispatched)"),
                                     m_controller->statusText())));
        QCOMPARE(h.last().toMap().value(QStringLiteral("kind")).toString(),
                 QLatin1String(r.kind));
        // THE MODE, read from the controller because the journal does not
        // carry it. Without this the walk cannot tell `desaturate` from
        // `saturate`.
        QCOMPARE(m_controller->lastStrokeMode(), QLatin1String(r.mode));
        QCOMPARE(m_controller->lastStrokeKind(), QLatin1String(r.kind));
    }

    // ★★ THE OLD SPONGE ARM ASSERTED THE OPPOSITE OF THIS, AND IT WAS
    // RIGHT TO. It pinned sponge as DEAD, and said so as a decision rather
    // than an oversight: dodge_burn.stroke's mode enum is
    // [dodge, burn, saturate, desaturate] and has no `sponge`, so mapping
    // it would have been guessing between two OPPOSITE operations. Its
    // own comment said "if someone wires it, this assertion tells them a
    // ruling is owed." THE RULING CAME -- Q-D3(b), 2026-08-29 -- so the
    // arm is not deleted, it is INVERTED: sponge now walks the table
    // above with its ruled mode, and the assertion that used to prove it
    // dead is replaced by one that proves it does the ruled thing.
    //
    // What survives unchanged is the shape of the guarantee: a dead tool
    // is pinned dead, a live tool is pinned to its exact wire meaning,
    // and neither is left to be inferred from the absence of a failure.
    //
    // ★★★★★ CLONE WAS THE ONE STILL DEAD, AND B-02 (2026-09-05) FLIPPED IT.
    // The old assertion here proved it dead: a bare click submitted nothing,
    // because clone "has no mode that removes its source requirement, so it
    // cannot cross the way heal did". That reasoning was right and it is why
    // clone needed an AFFORDANCE rather than a table row — Option/Alt-click
    // arms a source point (§21.1), and the armed offset is the required
    // param no mode could supply.
    //
    // ★★★ THE PIN NOW HAS BOTH HALVES, AND THE DEAD HALF IS THE CONTROL.
    // Unarmed clone must STILL submit nothing — otherwise this case would
    // pass merely because clone became noisy — and armed clone must submit.
    // Same tool, same gesture, one bit of armed state between them.
    setTool("clone.stroke");

    // (a) UNARMED: unchanged from the dead pin, and still true by design.
    const int beforeClone = m_controller->historyEntries().size();
    const int attemptsBeforeClone = m_controller->submissionAttempts();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(100, 200));
    QCOMPARE(m_controller->historyEntries().size(), beforeClone);
    QVERIFY2(m_controller->submissionAttempts() == attemptsBeforeClone,
             "an UNARMED clone submitted a stroke — its contract requires a "
             "source and none was armed");

    // (b) ARM the source with Option/Alt-click. This is a point-set, not a
    // stroke: it must not itself submit, or the affordance would paint from
    // the pixel the user is sampling.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::AltModifier,
                      docToView(40, 60));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::AltModifier,
                        docToView(40, 60));
    QVERIFY2(m_controller->submissionAttempts() == attemptsBeforeClone,
             "arming the clone source submitted a stroke of its own");

    // (c) ARMED: the same gesture that did nothing above now paints.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 200));
    QTest::mouseMove(m_window, docToView(104, 204));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(104, 204));
    QVERIFY2(m_controller->submissionAttempts() > attemptsBeforeClone,
             "an ARMED clone still submitted nothing — the pin has not flipped");
    QTRY_VERIFY2(m_controller->historyEntries().size() > beforeClone,
                 "the armed clone stroke reached no journal entry");
    QCOMPARE(m_controller->historyEntries().last().toMap()
                 .value(QStringLiteral("kind")).toString(),
             QStringLiteral("clone.stroke"));
}

void TstGestureNet::healOnAnEmptyLayerSaysSoInsteadOfSucceedingSilently()
{
    // Q-D7 RIDER 1. heal.stroke onto a fully transparent layer is a
    // DEFINED no-op (heal.stroke.yaml:210 -- heal never modifies alpha),
    // and until 2026-08-29 it was a SILENT one: stroke accepted, history
    // entry present, composite byte-identical, no error and no toast.
    // Every signal a presenter would check said success.
    //
    // ★★★ THE NEGATIVE ARM IS THE POINT OF THIS CASE. A notice that fires
    // on every heal is exactly as useless as one that never fires, and it
    // would pass a test that only checked the empty-layer path. Both arms
    // run here, in one case, so neither can be satisfied alone.
    QSignalSpy spy(m_controller, &DocumentController::actionUnavailable);
    QVERIFY(spy.isValid());

    // --- ARM 1: an EMPTY layer must speak ---
    const QString empty = m_controller->newUuid();
    QVariantMap add;
    add.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    add.insert(QStringLiteral("newLayerId"), empty);
    QVERIFY(m_controller->applyCommand(QStringLiteral("layer.add"), {}, add, {})
                .value(QStringLiteral("ok")).toBool());
    QCoreApplication::processEvents();
    // Ground the precondition rather than assuming layer.add made it
    // empty: a control that rests on an unmeasured setup proves nothing.
    const QVariantMap eb = m_controller->layerContentBounds(empty);
    QVERIFY2(eb.value(QStringLiteral("ok")).toBool()
                 && eb.value(QStringLiteral("w")).toInt() <= 0,
             "setup failed: the new layer is not empty, so arm 1 cannot test what it claims");

    // ★ THE NEW LAYER MUST BE THE ACTIVE ONE, AND THAT IS SET RATHER THAN
    // ASSUMED. strokeEnd targets activeLayerId(); whether layer.add also
    // ACTIVATES its new layer is a separate behaviour this case does not
    // own, and resting arm 1 on it would make a heal-notice test fail for
    // an activation reason. First run of this case did exactly that.
    {
        const QVariantList rr = m_controller->layers();
        int idx = -1;
        for (int i = 0; i < rr.size(); ++i)
            if (rr.at(i).toMap().value(QStringLiteral("layerId")).toString() == empty)
                idx = i;
        QVERIFY2(idx >= 0, "the layer just added is not in layers()");
        m_controller->setActiveLayerIndex(idx);
        QCoreApplication::processEvents();
        // activeLayerId() is private; assert through the public property
        // the setter actually drives, which is also the one strokeEnd
        // resolves its target from.
        QCOMPARE(m_controller->activeLayerIndex(), idx);
        QCOMPARE(m_controller->layers().at(idx).toMap()
                     .value(QStringLiteral("layerId")).toString(), empty);
    }

    spy.clear();
    setTool("heal.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, docToView(100, 200));
    QTest::mouseMove(m_window, docToView(180, 240));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, docToView(180, 240));
    QCoreApplication::processEvents();
    QVERIFY2(spy.count() >= 1,
             "heal on an EMPTY layer produced no notice — the silent no-op is back");
    QCOMPARE(spy.last().at(1).toString(),
             QStringLiteral("ui.notice.heal_no_effect_empty_target"));
    // ★ THE "DOES IT RESOLVE" HALF IS DELIBERATELY NOT ASSERTED HERE.
    // This fixture builds `Catalog::fromTexts({})` -- an EMPTY catalog by
    // design -- so tr() key-echoes for every id in this suite and the
    // check would pass or fail for reasons that have nothing to do with
    // the notice. It lives in tst_palette_pins, which loads the real "en"
    // catalog. Splitting it is the point: an id that is EMITTED and an id
    // that RESOLVES are two claims, and asserting them where each can
    // actually fail is the only way either means anything.

    // --- ARM 2: a layer WITH content must NOT speak ---
    // The demo-ish base layer this fixture paints on has content by the
    // time the suite reaches here; assert that rather than trust it.
    const QVariantList rows = m_controller->layers();
    QVERIFY(!rows.isEmpty());
    // Find a row that actually HAS content, by measurement, rather than
    // assuming an index. Index 0 is a convention, not a guarantee.
    QString base; int baseIdx = -1;
    for (int i = 0; i < rows.size(); ++i) {
        const QString id = rows.at(i).toMap().value(QStringLiteral("layerId")).toString();
        const QVariantMap b = m_controller->layerContentBounds(id);
        if (b.value(QStringLiteral("ok")).toBool()
            && b.value(QStringLiteral("w")).toInt() > 0) {
            base = id; baseIdx = i; break;
        }
    }
    QVERIFY2(baseIdx >= 0,
             "setup failed: no layer in this document has content, so arm 2 cannot discriminate");
    m_controller->setActiveLayerIndex(baseIdx);
    QCoreApplication::processEvents();

    spy.clear();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, docToView(100, 200));
    QTest::mouseMove(m_window, docToView(180, 240));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, docToView(180, 240));
    QCoreApplication::processEvents();
    QCOMPARE(spy.count(), 0);
}

void TstGestureNet::smudgeDragReachesTheEngineAsASmudgeStroke()
{
    // THE ARM THAT MATTERS IS THE KIND, NOT THE BRANCH. Asserting the
    // dispatch condition was entered proves the shell did something;
    // asserting the JOURNAL carries `smudge.stroke` proves the ENGINE was
    // asked, and those are different claims. Before this wiring the
    // palette could select smudge (ToolPalette.qml:384), the cursor
    // changed, and a drag produced nothing at all — a state a
    // branch-entered check could not tell apart from a rejected stroke.
    const int before = m_controller->historyEntries().size();
    setTool("smudge.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 200));
    QTest::mouseMove(m_window, docToView(200, 200));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 200));
    const QVariantList h = m_controller->historyEntries();
    QVERIFY2(h.size() > before,
             qPrintable(QStringLiteral("a smudge drag produced NO journal "
                                       "entry — the engine was never asked; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    QCOMPARE(h.last().toMap().value(QStringLiteral("kind")).toString(),
             QStringLiteral("smudge.stroke"));
}

// DISCRIMINATING CONTROL for brushDragPaintsAlongPathOnly (C-120).
//
// Its sibling drags a STRAIGHT path, which IS its own chord — so path
// fidelity is the one dimension its fixture never varies, and an engine
// that inks the chord instead of the path lands inside its predicates'
// health class BY CONSTRUCTION. Measured, not argued: with strokeEnd()
// decimating a >=4-sample stroke to first+last, the sibling STAYS GREEN.
//
// This case differs from it in EXACTLY ONE dimension: the path BENDS.
// Same tool, same door, same two expressions at the same two pixels —
// only the polarity flips, because the bend puts the path through
// (250,340) and takes it away from (250,200). Chord and path are now
// 140 px apart at x=250, so the two are separable in BOTH directions:
//   * inked-the-chord  -> (250,340) stays white  -> RED here
//   * inked-the-path   -> (250,200) stays white  -> the sibling's tight
//                         bbox still catches over-painting
// Six events => six samples, so any first+last decimation has >=4 to
// bite on.
void TstGestureNet::brushBentDragInksTheBendNotTheChord()
{
    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 200));
    QTest::mouseMove(m_window, docToView(175, 270));
    QTest::mouseMove(m_window, docToView(250, 340));
    QTest::mouseMove(m_window, docToView(325, 270));
    QTest::mouseMove(m_window, docToView(400, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(400, 200));
    // Positive control: the stroke inked SOMETHING at all. Without this a
    // stroke that committed nothing would satisfy the chord-is-white half
    // below and read as a pass.
    const QRect b = engineContentBounds(m_controller);
    QVERIFY2(!b.isEmpty(),
             qPrintable(QStringLiteral("the bent brush drag committed no "
                                       "pixels at all; status: %1")
                            .arg(m_controller->statusText())));
    // THE DISCRIMINATOR, direction 1: the bend vertex is ON the path and
    // 140 px off the chord.
    QVERIFY2(!isWhitish(enginePixel(m_controller, 250, 340)),
             qPrintable(QStringLiteral("brush drag inked nothing at the BEND "
                                       "(250,340) — the engine followed the "
                                       "CHORD of the drag, not its PATH; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    // THE DISCRIMINATOR, direction 2: the chord midpoint is OFF the path.
    // An engine that straightens the stroke inks here instead.
    QVERIFY2(isWhitish(enginePixel(m_controller, 250, 200)),
             "brush drag inked the CHORD midpoint (250,200), which the bent "
             "path passes 140 px away from — the path was straightened");
    // Bounds must now reach the bend: a path-following engine paints down
    // to y~340, a chord-following one stops near y=200.
    QVERIFY2(b.left() >= 80 && b.right() <= 420 && b.top() >= 180
                 && b.bottom() >= 320 && b.bottom() <= 360,
             qPrintable(QStringLiteral("bent brush stroke bounds off: "
                                       "%1,%2 %3x%4 (expected the bbox to "
                                       "reach the bend at y~340)")
                            .arg(b.x()).arg(b.y()).arg(b.width())
                            .arg(b.height())));
}

void TstGestureNet::brushEscapeMidStrokeDoesNotCommit()
{
    // G-1's pin. §39.2 [NORMATIVE]: Esc resolves the INNERMOST in-progress
    // context, and a live brush stroke is one.
    //
    // LIVENESS FLOOR ADDED T35, on a finding routed to me by the desk and
    // which I accept: this case asserted only that the bounds were EMPTY
    // afterwards, with no before-capture and no floor. An empty-bounds
    // assertion cannot distinguish "Esc correctly abandoned a LIVE stroke"
    // from "no stroke ever existed", so its green carried no information
    // about the property it names. (Their framing is the sharper one:
    // tst_selection_gestures reads PIXELS, where "no ink" is visibly
    // ambiguous and forced its author to prove the probe could see ink;
    // this file read BOUNDS, where emptiness FELT self-evident. A failure
    // class gets solved where it blocks you and ignored where it flatters
    // you.)
    //
    // The floor is a PAIRED CONTROL rather than a mid-stroke engine read —
    // mid-stroke there is nothing in the engine to read, because the echo
    // is a shell-side preview until strokeEnd. So: the identical gesture
    // WITHOUT Esc must paint. That makes the negative arm discriminating —
    // green now means "Esc made the difference", which is the named
    // property.
    auto runStroke = [&](bool pressEscape) -> QRect {
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return QRect(-1, -1, -1, -1);
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        // Focus BEFORE the press: the old body called forceActiveFocus()
        // after the press and both moves, which is the same ordering blind
        // spot the desk has now counted three times across two desks.
        m_canvas->forceActiveFocus();
        setTool("brush.stroke");
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(100, 300));
        QTest::mouseMove(m_window, docToView(200, 300));
        QTest::mouseMove(m_window, docToView(300, 300));
        if (pressEscape)
            QTest::keyClick(m_window, Qt::Key_Escape);
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(300, 300));
        return engineContentBounds(m_controller);
    };

    // FLOOR: the same gesture without Esc must actually paint. If this is
    // empty the probe is blind and the arm below would pass on nothing.
    const QRect control = runStroke(false);
    QVERIFY2(!control.isEmpty(),
             qPrintable(QStringLiteral("LIVENESS FLOOR FAILED: the identical "
                                       "stroke WITHOUT Esc painted nothing "
                                       "(bounds %1,%2 %3x%4), so this probe "
                                       "cannot see a stroke at all and its "
                                       "Esc arm would be vacuous; status: %5")
                            .arg(control.x()).arg(control.y())
                            .arg(control.width()).arg(control.height())
                            .arg(m_controller->statusText())));

    // THE PROPERTY: with Esc, the same gesture commits nothing.
    const QRect escaped = runStroke(true);
    QVERIFY2(escaped.isEmpty(),
             qPrintable(QStringLiteral("Esc mid-stroke did not abandon the "
                                       "stroke — the release committed it "
                                       "(bounds %1,%2 %3x%4; the control arm "
                                       "painted %5x%6)")
                            .arg(escaped.x()).arg(escaped.y())
                            .arg(escaped.width()).arg(escaped.height())
                            .arg(control.width()).arg(control.height())));
}


void TstGestureNet::focusLostMidStrokeDoesNotCommit()
{
    // G-2 (MINOR), filed at T1 as a REASONED CODE READING and never
    // reproduced: the stroke session is absent from the stranded-state
    // hygiene paths that already cover panning. `cancelGesture` was fixed
    // for G-1, but `mouseUngrabEvent` and `focusOutEvent` still contain
    // ZERO references to the stroke state (re-verified T31).
    //
    // The ungrab half is not reachable through QTest mouse synthesis, so
    // G-2 has sat unreproduced for the whole session. The FOCUS-OUT half
    // IS reachable — this case reaches it, and turns a reading into a
    // reproduction (or refutes it).
    //
    // §39.2's hygiene family: focusOutEvent already clears m_spaceHeld
    // for exactly this reason ("the same hygiene family"). A live stroke
    // is the same class of stranded state.
    setTool("brush.stroke");
    m_canvas->forceActiveFocus();
    QVERIFY2(m_canvas->hasActiveFocus(), "setup: the canvas never had focus");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 300));
    QTest::mouseMove(m_window, docToView(200, 300));
    QTest::mouseMove(m_window, docToView(300, 300));

    // Take focus AWAY mid-stroke (an app shortcut opening a panel, a
    // modal stealing focus — the documented P4-D3 scenario's sibling).
    // contentItem()->forceActiveFocus() does NOT take focus off the
    // canvas (the content item is not a focus scope that claims it) —
    // measured: the guard below caught it. Relinquish directly instead.
    m_canvas->setFocus(false);
    QCoreApplication::processEvents();
    // NON-VACUITY: if focus never actually left, this case measures
    // nothing and must say so rather than pass.
    QVERIFY2(!m_canvas->hasActiveFocus(),
             "the canvas kept focus — focusOutEvent never fired, so this "
             "probe did not exercise the path it was written for");

    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 300));
    const QRect b = engineContentBounds(m_controller);
    QVERIFY2(b.isEmpty(),
             qPrintable(QStringLiteral("G-2 REPRODUCED: focus was lost "
                                       "mid-stroke and the release still "
                                       "COMMITTED the stroke (engine content "
                                       "bounds %1,%2 %3x%4). focusOutEvent "
                                       "clears m_spaceHeld but not the "
                                       "stroke session")
                            .arg(b.x()).arg(b.y())
                            .arg(b.width()).arg(b.height())));
}

void TstGestureNet::toolSwitchMidStrokeAbandonsTheStroke()
{
    // NEW ROW, found by the desk reading PAST its own window in
    // canvas_view.cpp and routed to me. The source says it plainly:
    //
    //   "TOOL SWITCH MID-STROKE now ABANDONS the stroke instead of leaving
    //    it live to be committed later under the ORIGINAL m_strokeKind (a
    //    brush stroke landing after the user selected the eraser) ... it is
    //    currently UNPINNED. Any test that drives setTool() mid-stroke
    //    should assert the abandon, not the commit."
    //
    // The hazard is specific and user-visible: switch from brush to eraser
    // mid-drag and, unfixed, the release lands a BRUSH stroke — ink the
    // user asked to stop making, attributed to a tool they had already left.
    //
    // Same PAIRED CONTROL as the Esc pin, for the same reason: a bare
    // "nothing was painted" cannot tell abandonment from a probe that never
    // painted.
    auto runStroke = [&](bool switchTool) -> QRect {
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return QRect(-1, -1, -1, -1);
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        m_canvas->forceActiveFocus();
        setTool("brush.stroke");
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(120, 220));
        QTest::mouseMove(m_window, docToView(240, 220));
        QTest::mouseMove(m_window, docToView(360, 220));
        if (switchTool)
            setTool("eraser.stroke"); // the user picks a different tool
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(360, 220));
        return engineContentBounds(m_controller);
    };

    const QRect control = runStroke(false);
    QVERIFY2(!control.isEmpty(),
             qPrintable(QStringLiteral("LIVENESS FLOOR FAILED: the identical "
                                       "stroke without a tool switch painted "
                                       "nothing (%1x%2) — the arm below would "
                                       "be vacuous; status: %3")
                            .arg(control.width()).arg(control.height())
                            .arg(m_controller->statusText())));

    const QRect switched = runStroke(true);
    QVERIFY2(switched.isEmpty(),
             qPrintable(QStringLiteral("switching tools mid-stroke did NOT "
                                       "abandon it — the release committed "
                                       "%1x%2 at %3,%4 under the original "
                                       "stroke kind (control painted %5x%6)")
                            .arg(switched.width()).arg(switched.height())
                            .arg(switched.x()).arg(switched.y())
                            .arg(control.width()).arg(control.height())));
}

void TstGestureNet::eraserDragRemovesPaint()
{
    // Paint on a layer ABOVE the white base: erasing then reveals the base
    // (a single-layer erase punches through to transparency, which reads
    // as ARGB 0,0,0,0 and cannot be told apart from "never painted").
    QVariantMap params;
    params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, params, {})
                .value(QStringLiteral("ok"))
                .toBool());
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);

    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 200));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 200));
    QVERIFY2(!isWhitish(enginePixel(m_controller, 250, 200)),
             "setup: brush laid no paint to erase");

    // Erase only the TAIL of the stroke (x >= 200).
    setTool("eraser.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(210, 200));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 200));
    const QColor erased = enginePixel(m_controller, 260, 200);
    QVERIFY2(isWhitish(erased) && erased.alpha() >= 250,
             qPrintable(QStringLiteral("eraser drag did not clear the paint "
                                       "under it (pixel %1, alpha %2); "
                                       "status: %3")
                            .arg(erased.name())
                            .arg(erased.alpha())
                            .arg(m_controller->statusText())));
    // Discriminating half: a whole-layer wipe would clear the head too.
    QVERIFY2(!isWhitish(enginePixel(m_controller, 140, 200)),
             "eraser drag cleared paint OUTSIDE its path — the erase hit "
             "the whole layer, not the stroke path");
}

// DISCRIMINATING CONTROL for eraserDragRemovesPaint (C-120).
//
// Its sibling erases over the WHITE document base and then asserts the
// result is whitish — so over that base "REMOVED" and "COVERED WITH
// OPAQUE WHITE" are the same pixel, and the case cannot tell an eraser
// from a white brush. Measured, not argued: with eraser.stroke submitted
// to the engine as an opaque white brush.stroke, the sibling STAYS
// GREEN — and its own status line still reads history.eraser.stroke.
//
// This case differs from it in EXACTLY ONE dimension: the base is
// COLOURED, not white. Same layer stack, same two strokes, same two
// probe pixels — a genuine erase reveals RED, a white-brush impostor
// leaves WHITE, and red and white are never the same pixel.
void TstGestureNet::eraserOverColouredBaseRemovesRatherThanCovers()
{
    // THE ONE VARIED DIMENSION: paint the base layer red before anything
    // else, so "revealed the base" and "painted white" are separable.
    fillBlock(QRect(0, 0, 640, 400), 0.85, 0.05, 0.05);
    const QColor base = enginePixel(m_controller, 260, 200);
    QVERIFY2(isRedDominant(base),
             qPrintable(QStringLiteral("setup: the base did not take the "
                                       "coloured fill (pixel %1) — with a "
                                       "white base this case degenerates "
                                       "into its non-discriminating sibling")
                            .arg(base.name())));

    QVariantMap params;
    params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, params, {})
                .value(QStringLiteral("ok"))
                .toBool());
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);

    // Same brush stroke as the sibling: the default brush is (20,20,20),
    // dark against both the red base and any white impostor.
    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 200));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 200));
    const QColor painted = enginePixel(m_controller, 260, 200);
    QVERIFY2(isDarkish(painted),
             qPrintable(QStringLiteral("setup: brush laid no dark paint over "
                                       "the coloured base (pixel %1)")
                            .arg(painted.name())));

    // Erase only the TAIL of the stroke (x >= 200), exactly as the sibling.
    setTool("eraser.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(210, 200));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 200));
    // THE DISCRIMINATOR: the erased pixel must show the RED BASE THROUGH
    // the hole. An opaque-white brush wearing the eraser's name leaves
    // (255,255,255) here and fails on green/blue, not on red.
    const QColor erased = enginePixel(m_controller, 260, 200);
    QVERIFY2(isRedDominant(erased) && erased.alpha() >= 250,
             qPrintable(QStringLiteral("the erased pixel is %1 (alpha %2), "
                                       "not the red base underneath — the "
                                       "paint was COVERED, not REMOVED "
                                       "(whitish here means an opaque light "
                                       "brush is wearing the eraser's name); "
                                       "status: %3")
                            .arg(erased.name())
                            .arg(erased.alpha())
                            .arg(m_controller->statusText())));
    // The sibling's other half, kept: a whole-layer wipe would clear the
    // head of the stroke too, and over a red base that reads as red.
    const QColor head = enginePixel(m_controller, 140, 200);
    QVERIFY2(isDarkish(head),
             qPrintable(QStringLiteral("the stroke head at (140,200) is %1, "
                                       "not the dark paint — the erase hit "
                                       "the whole layer, not the stroke path")
                            .arg(head.name())));
}

// -------------------------------------------------------------- transform

void TstGestureNet::transformReleaseWithoutEnterCommitsNothing()
{
    fillBlockOnNewLayer(QRect(400, 300, 120, 80), 0.8, 0.1, 0.1);
    const QRect before = engineContentBounds(m_controller);
    setTool("transform.free"); // arms the frame at the document bounds
    // Grab the BR handle (doc 640,400) and scale toward the TL anchor.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(480, 300));
    QTest::mouseMove(m_window, docToView(320, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(320, 200));
    QVERIFY2(engineContentBounds(m_controller) == before,
             "the transform handle release COMMITTED — the frame must "
             "persist until Enter (release ends the gesture only)");
}

void TstGestureNet::transformScaleHandleCommitsOnEnter()
{
    // v1.0.1 ITEM 19 (landed in O-UI's lane): the free-transform frame is
    // the ACTIVE LAYER'S CONTENT bounds, not the canvas. This case grabbed
    // the canvas BR at doc (640,400) and passed for 38 ticks; under item 19
    // that point is empty space, the press starts no scale, and Enter
    // commits an identity. THE CASE WAS STALE, NOT THE CODE — recorded at
    // T39 rather than filed, because the new frame is the honest one.
    fillBlockOnNewLayer(QRect(400, 300, 120, 80), 0.8, 0.1, 0.1);
    const QRect before = engineContentBounds(m_controller);
    // FRAME PRECONDITION: the handle positions below are derived from these
    // bounds, so if the block did not land exactly here every coordinate in
    // this case is aimed at nothing and the drag proves nothing.
    QCOMPARE(before, QRect(400, 300, 120, 80));
    setTool("transform.free");
    // The LAYER frame's BR handle is (520,380). Dragging it to (460,340) is
    // a 0.5 scale about the opposite (TL) handle at (400,300).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(520, 380));
    QTest::mouseMove(m_window, docToView(490, 360));
    QTest::mouseMove(m_window, docToView(460, 340));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(460, 340));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect after = engineContentBounds(m_controller);
    qInfo() << "transform scale 0.5: content" << before << "->" << after
            << "status:" << m_controller->statusText();
    QVERIFY2(after != before,
             qPrintable(QStringLiteral("Enter did not commit the scaled "
                                       "transform frame; status: %1")
                            .arg(m_controller->statusText())));
    QVERIFY2(!after.isEmpty(),
             "the committed transform erased the layer content entirely");
    // EXACT engine truth: 0.5 about the TL anchor sends (400,300 120x80) to
    // (400,300 60x40) — the anchor STAYS PUT, which is the whole point of an
    // opposite-handle anchor. Allow one resample pixel each side; anything
    // looser would pass a wrong anchor or a double-applied `reference`.
    const QRect want(400, 300, 60, 40);
    QVERIFY2(want.adjusted(-1, -1, 1, 1).contains(after)
                 && after.width() >= want.width() - 1
                 && after.height() >= want.height() - 1,
             qPrintable(QStringLiteral("0.5 scale about the TL anchor landed "
                                       "at %1,%2 %3x%4 - expected %5,%6 %7x%8")
                            .arg(after.x()).arg(after.y())
                            .arg(after.width()).arg(after.height())
                            .arg(want.x()).arg(want.y())
                            .arg(want.width()).arg(want.height())));
}

void TstGestureNet::transformEscapeCancelsFrame()
{
    fillBlockOnNewLayer(QRect(400, 300, 120, 80), 0.8, 0.1, 0.1);
    const QRect before = engineContentBounds(m_controller);
    setTool("transform.free");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(320, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(320, 200));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTest::keyClick(m_window, Qt::Key_Return); // nothing left to commit
    QVERIFY2(engineContentBounds(m_controller) == before,
             "Esc did not drop the transform frame — a later Enter still "
             "committed the abandoned scale");
}

void TstGestureNet::transformRotateRingCommitsOnEnter()
{
    // ITEM 19, same correction as the scale case: the ring is measured from
    // the LAYER frame's corners, not the canvas's.
    fillBlockOnNewLayer(QRect(260, 160, 120, 80), 0.1, 0.2, 0.8);
    const QRect before = engineContentBounds(m_controller);
    QCOMPARE(before, QRect(260, 160, 120, 80));
    setTool("transform.free");
    // Rotate ring: >8 and <=22 doc px from a mapped corner. The layer frame's
    // TL corner is (260,160), so (274,160) sits 14px out - in the ring, and
    // 46px from the nearest other handle (the top edge midpoint).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(274, 160));
    QTest::mouseMove(m_window, docToView(300, 130));
    QTest::mouseMove(m_window, docToView(340, 130));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(340, 130));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect after = engineContentBounds(m_controller);
    qInfo() << "transform rotate: content" << before << "->" << after
            << "status:" << m_controller->statusText();
    QVERIFY2(after != before,
             qPrintable(QStringLiteral("the rotate-ring drag committed no "
                                       "change on Enter; status: %1")
                            .arg(m_controller->statusText())));
    // NON-VACUITY ON THE GESTURE KIND. "Changed" is also what a translate
    // does, and the ring sits close enough to a corner handle that a
    // hit-test regression would silently downgrade this to a scale case
    // wearing a rotate name. A rotation off the axes GROWS the axis-aligned
    // bounding box of a rectangle; a scale or a move cannot grow it past
    // the diagonal, and an identity leaves it alone.
    const double diag = std::sqrt(double(before.width() * before.width()
                                         + before.height() * before.height()));
    QVERIFY2(after.width() > before.width() && after.height() > before.height()
                 && after.width() <= diag + 2 && after.height() <= diag + 2,
             qPrintable(QStringLiteral("committed content %1x%2 is not the "
                                       "AABB of a rotated %3x%4 block - the "
                                       "ring press may not have started a "
                                       "ROTATE")
                            .arg(after.width()).arg(after.height())
                            .arg(before.width()).arg(before.height())));
}

// ------------------------------------------------------------------- text

// RC3 D-6 (operator WORD 35, sitting S-15): the Text tool REFUSES on use
// for 1.0 -- a click or a drag adds NO layer and emits armedToolPressed
// ("text"), which Main.qml surfaces as the typed tool-pending refusal
// ("Text is not wired to the canvas yet"; pinned end to end in
// tst_options_bar). Before D-6 a click committed text.add with empty text:
// an invisible layer that could not be typed into.
void TstGestureNet::textClickIsRefusedAndAddsNoLayer()
{
    setTool("text");
    const int before = m_controller->layers().size();
    QSignalSpy armed(m_canvas, SIGNAL(armedToolPressed(QString)));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 180));
    QCOMPARE(m_controller->layers().size(), before);
    QCOMPARE(armed.count(), 1);
    QCOMPARE(armed.at(0).at(0).toString(), QStringLiteral("text"));
}

void TstGestureNet::textDragIsRefusedAndAddsNoLayer()
{
    setTool("text");
    const int before = m_controller->layers().size();
    QSignalSpy armed(m_canvas, SIGNAL(armedToolPressed(QString)));
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 120));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(300, 200));
    QCOMPARE(m_controller->layers().size(), before);
    QCOMPARE(armed.count(), 1);
    QCOMPARE(armed.at(0).at(0).toString(), QStringLiteral("text"));
}

// ------------------------------------------------ standing coverage rule

void TstGestureNet::registryCoverageLint()
{
    // STANDING RULE (operator, 2026-08-20): every tool in the palette
    // registry must carry at least one gesture case. This lint enforces
    // it MECHANICALLY every tick instead of by memory: it parses the
    // checked-in ToolPalette.qml registry and checks each wire id against
    // the roster below, which names WHERE that tool is covered.
    //
    // Two-sided (the tst_a11y_tree_lint pattern): a registered tool with
    // no roster row REDS (a new tool shipped gesture-untested), and a
    // roster row that no longer matches any registered tool ALSO REDS
    // (so the roster can only shrink honestly).
    //
    // Coverage is credited across the whole net, not just this file —
    // re-pinning what another suite already pins with more teeth would be
    // waste, not rigour.
    //
    // ============ WHAT THIS LINT DOES AND DOES NOT ESTABLISH ============
    // C-120 measured two of this file's own cases PASSING under a live
    // mutation of the exact behaviour they are named for:
    //   * brushDragPaintsAlongPathOnly stayed GREEN with strokeEnd()
    //     decimating a >=4-sample stroke to first+last, so the engine
    //     inked the CHORD instead of the PATH;
    //   * eraserDragRemovesPaint stayed GREEN with eraser.stroke
    //     submitted as an opaque WHITE brush.stroke.
    // Those two cases were the roster rows for brush.stroke and
    // eraser.stroke — so this lint was mechanically certifying coverage
    // it could not see, and the word "coverage" in its name is doing work
    // its checks do not do.
    //
    // Two changes follow, and the second one is deliberately NOT an
    // assertion:
    //
    // 1. THE `where` COLUMN IS NOW CHECKED. Until C-122 it was decorative
    //    — only `wire` was ever compared against anything, so a row could
    //    name a case that did not exist and this lint would pass. Every
    //    row's named case must now RESOLVE to a declared slot in the
    //    named test file, and so must every named control. That is a real
    //    check and it reds.
    //
    // 2. DISCRIMINATION IS NOT CHECKABLE HERE, AND THIS LINT SAYS SO
    //    INSTEAD OF IMPLYING OTHERWISE. Whether a case would RED if its
    //    tool broke is a property of the case UNDER MUTATION — a runtime
    //    experiment against a deliberately broken build — not a fact any
    //    source parse can read. A self-declared "mutation-tested" marker
    //    would be worse than nothing: the author who writes a
    //    non-discriminating case writes the marker too, which is exactly
    //    the failure being fixed. So the `control` column below is DATA,
    //    not a claim this lint verifies: it names the paired control
    //    where a mutation actually demonstrated one, and is nullptr
    //    everywhere else. The qInfo prints BOTH denominators so a reader
    //    gets the proven count and the unproven count, never a bare
    //    "34/34" that reads as soundness.
    struct Row {
        const char *wire;
        const char *where;
        // The DISCRIMINATING CONTROL: a case demonstrated to RED under a
        // live mutation of this row's own subject. nullptr = no such
        // demonstration exists. This lint checks only that a named
        // control EXISTS; it cannot check that it discriminates.
        const char *control;
    };
    static const Row roster[] = {
        // --- LIVE tools: a real gesture case drives them to engine truth
        {"move", "tst_selection_gestures::movePreviewShiftsOnlyActiveLayer",
         nullptr},
        {"select.rect", "tst_selection_gestures::rectMarqueeCommitsOnRelease",
         nullptr},
        {"select.ellipse", "tst_selection_gestures::ellipseCommitsOnRelease",
         nullptr},
        {"select.lasso", "tst_selection_gestures::lassoAutoClosesOnRelease",
         nullptr},
        {"select.polygon", "tst_selection_gestures::polygonClosesOnDoubleClick",
         nullptr},
        {"select.wand", "tst_selection_gestures::wandFiresAtClickPoint",
         nullptr},
        {"vector.pen", "tst_selection_gestures::penClosesAndCommitsShapeLayer",
         nullptr},
        {"brush.stroke", "tst_gesture_net::brushDragPaintsAlongPathOnly",
         "tst_gesture_net::brushBentDragInksTheBendNotTheChord"},
        {"eraser.stroke", "tst_gesture_net::eraserDragRemovesPaint",
         "tst_gesture_net::eraserOverColouredBaseRemovesRatherThanCovers"},
        {"shape.rect", "tst_gesture_net::shapeRectDragCommitsShapeLayer",
         nullptr},
        {"shape.ellipse", "tst_gesture_net::shapeEllipseDragIsElliptical",
         nullptr},
        {"crop", "tst_gesture_net::cropDragEmitsExactCropRect",
         nullptr},
        {"transform.free", "tst_gesture_net::transformScaleHandleCommitsOnEnter",
         nullptr},
        {"transform.perspective", "tst_gesture_net::perspectiveCornerDragCommitsOnEnter",
         nullptr},
        // --- ARMED-NOT-LIVE (tier-2 / 11c): census-sanctioned rows whose
        // canvas inertness is pinned on four truths per member.
        // ★ T67: heal is LIVE. Q-D1(a), operator 2026-08-29 — `J` heals with
        // the auto-chosen content-aware source, and canvas_view's kModeTools
        // carries {heal.stroke, heal.stroke, spot}. Retargeted at the walk
        // that drives it live. Found by armedCitationsAreStillArmed, which
        // is the T66 lint doing the job it was built for.
        // ★ T67: heal is LIVE. Q-D1(a), operator 2026-08-29 — `J` heals with
        // the auto-chosen content-aware source, and canvas_view's kModeTools
        // carries {heal.stroke, heal.stroke, spot}. Retargeted at the walk
        // that drives it live. Found by armedCitationsAreStillArmed, which
        // is the T66 lint doing the job it was built for.
        {"heal.stroke", "tst_gesture_net::paletteModeToolsReachTheEngineAsTheirFamilyKind",
         nullptr},
        {"clone.stroke", "tst_palette_pins::r2Tier2MembersArmedNotLiveCanvasInert",
         nullptr},
        {"dodge_burn.dodge", "tst_gesture_net::paletteModeToolsReachTheEngineAsTheirFamilyKind",
         nullptr},
        {"dodge_burn.burn", "tst_gesture_net::paletteModeToolsReachTheEngineAsTheirFamilyKind",
         nullptr},
        // ★ T67: sponge is LIVE. Q-D3(b), operator 2026-08-29 — sponge maps
        // to `desaturate`, the ruling that replaced a deliberate absence.
        {"dodge_burn.sponge", "tst_gesture_net::paletteModeToolsReachTheEngineAsTheirFamilyKind",
         nullptr},
        {"blur_sharpen.blur", "tst_gesture_net::paletteModeToolsReachTheEngineAsTheirFamilyKind",
         nullptr},
        {"blur_sharpen.sharpen", "tst_gesture_net::paletteModeToolsReachTheEngineAsTheirFamilyKind",
         nullptr},
        // ★ T64: smudge is LIVE. O-UI wired it into the stroke dispatch
        // ("a dead tool becomes a live one", 192b34b7) and added
        // smudgeDragReachesTheEngineAsASmudgeStroke to THIS file. The old
        // reference claimed it was "ArmedNotLive…CanvasInert", which is now
        // a false statement about a tool that paints. Same class as T56's
        // penCommitsShape: a classification outliving its basis.
        {"smudge.stroke", "tst_gesture_net::smudgeDragReachesTheEngineAsASmudgeStroke",
         nullptr},
    };

    QFile f(QStringLiteral(APPNAME_TOOLPALETTE_QML));
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             "could not read the checked-in ToolPalette.qml registry");
    const QString src = QString::fromUtf8(f.readAll());
    QRegularExpression re(QStringLiteral("tool:\\s*\"([A-Za-z0-9_.]+)\""));
    QStringList registered;
    auto it = re.globalMatch(src);
    while (it.hasNext())
        registered << it.next().captured(1);
    registered.removeDuplicates();
    // Vacuity guard (rule 9): a regex that stopped matching would make
    // this lint pass over an empty registry.
    // 1.0 final (corrigendum (l)): 22 tools after the removal set.
    QVERIFY2(registered.size() >= 22,
             qPrintable(QStringLiteral("the registry parse found only %1 "
                                       "tools — the ToolPalette.qml row "
                                       "shape changed and this lint went "
                                       "blind").arg(registered.size())));

    QStringList rostered;
    for (const Row &r : roster)
        rostered << QLatin1String(r.wire);

    QStringList uncovered;
    for (const QString &t : registered)
        if (!rostered.contains(t))
            uncovered << t;
    QStringList stale;
    for (const QString &r : rostered)
        if (!registered.contains(r))
            stale << r;

    // C-122 TEETH: resolve every named case (and every named control) to a
    // DECLARED SLOT in the named test file. Until this tick the `where`
    // column was never compared against anything, so a row could name a
    // case that did not exist and this lint passed.
    auto resolveSlot = [](const QString &cite, QString *why) -> bool {
        static QRegularExpression cre(
            QStringLiteral("^(tst_[A-Za-z0-9_]+)::([A-Za-z0-9_]+)"));
        const auto m = cre.match(cite);
        if (!m.hasMatch()) {
            *why = QStringLiteral("not in tst_file::caseName form");
            return false;
        }
        const QString path = QStringLiteral(APPNAME_REPO_ROOT)
            + QStringLiteral("/shell/tests/") + m.captured(1)
            + QStringLiteral(".cpp");
        QFile tf(path);
        if (!tf.open(QIODevice::ReadOnly | QIODevice::Text)) {
            *why = QStringLiteral("no such test file %1").arg(path);
            return false;
        }
        const QString body = QString::fromUtf8(tf.readAll());
        const QRegularExpression sre(
            QStringLiteral("\\bvoid\\s+%1\\s*\\(\\s*\\)").arg(m.captured(2)));
        if (!sre.match(body).hasMatch()) {
            *why = QStringLiteral("%1 declares no slot %2")
                       .arg(m.captured(1), m.captured(2));
            return false;
        }
        return true;
    };

    QStringList unresolved;
    int resolvedCases = 0;
    int withControl = 0;
    for (const Row &r : roster) {
        QString why;
        if (resolveSlot(QLatin1String(r.where), &why))
            ++resolvedCases;
        else
            unresolved << QStringLiteral("%1 -> %2 (%3)")
                              .arg(QLatin1String(r.wire),
                                   QLatin1String(r.where), why);
        if (r.control) {
            ++withControl;
            if (!resolveSlot(QLatin1String(r.control), &why))
                unresolved << QStringLiteral("%1 CONTROL -> %2 (%3)")
                                  .arg(QLatin1String(r.wire),
                                       QLatin1String(r.control), why);
        }
    }
    // Vacuity guard (rule 9), and the tightest one available: EVERY row of
    // this roster names a tst_file::caseName, so anything short of all of
    // them means the citation parse went blind rather than that a row is
    // genuinely unresolvable — and a blind parse would find no misses and
    // pass.
    const int rows = int(sizeof(roster) / sizeof(roster[0]));
    QVERIFY2(resolvedCases + unresolved.size() >= rows,
             qPrintable(QStringLiteral("the citation parse resolved neither "
                                       "way for some rows (%1 ok + %2 bad < "
                                       "%3 rows) — this lint went blind")
                            .arg(resolvedCases)
                            .arg(unresolved.size())
                            .arg(rows)));

    // ★ WHAT IS ESTABLISHED, WITH ITS DENOMINATOR — printed instead of a
    // bare "N/N" that reads as soundness. This is a REGISTRATION check.
    qInfo().noquote()
        << QStringLiteral(
               "REGISTRY<->ROSTER PARTITION: %1 registered tools, %2 roster "
               "rows, %3 named cases resolved to real slots.\n"
               "  THIS ESTABLISHES: every registered tool NAMES a case, and "
               "every named case EXISTS.\n"
               "  THIS DOES NOT ESTABLISH: that any named case would RED if "
               "its tool broke. Discrimination is a property of a case UNDER "
               "MUTATION and no source parse can read it.\n"
               "  DISCRIMINATING CONTROLS DEMONSTRATED: %4 of %2 rows. "
               "UNPROVEN: %5 of %2.")
               .arg(registered.size())
               .arg(rows)
               .arg(resolvedCases)
               .arg(withControl)
               .arg(rows - withControl);

    QVERIFY2(uncovered.isEmpty(),
             qPrintable(QStringLiteral("REGISTRY-ONLY TOOLS (registered in "
                                       "the palette, no gesture case): %1")
                            .arg(uncovered.join(QStringLiteral(", ")))));
    QVERIFY2(stale.isEmpty(),
             qPrintable(QStringLiteral("roster rows no longer in the "
                                       "registry (the roster must shrink "
                                       "honestly): %1")
                            .arg(stale.join(QStringLiteral(", ")))));
    QVERIFY2(unresolved.isEmpty(),
             qPrintable(QStringLiteral("roster citations that name no real "
                                       "slot (a row can no longer claim "
                                       "coverage by naming a case that does "
                                       "not exist): %1")
                            .arg(unresolved.join(QStringLiteral("; ")))));
}


void TstGestureNet::censusCoverageLint()
{
    // STANDING RULE, EXTENDED (operator, 2026-08-20): the coverage lint
    // now covers the FULL interactive-surface census (O-UI item 25's
    // table) "once committed", not just the palette registry.
    //
    // The table is NOT committed yet (verified this tick: no "item 25" on
    // main or lane/ui). The hazard that creates is a lint that sits
    // unenforced and passes vacuously forever, so this case is built to
    // make that impossible:
    //
    //   * census ABSENT  -> records the search explicitly (the tick log
    //     carries "census not committed" every tick, so nobody can read
    //     silence as coverage);
    //   * census PRESENT but this parser cannot read it -> **RED**. A
    //     census that lands in a shape I did not anticipate must break
    //     the lint loudly, never be skipped.
    //
    // Discovery is by CONTENT, not filename, so an unguessable filename
    // cannot dodge it.
    // T16: search the WHOLE REPO, not just reports/. O-UI's item-25
    // census is real ("168 WIRED / 20 ARMED-SILENT / 4 DEAD", O-UI-log)
    // but lives at scratchpad/census-item25.md — an UNTRACKED path. A
    // reports/-only scan would have reported "not committed" forever if it
    // ever landed anywhere else, which is precisely the silent regression
    // the standing rule forbids. Scanning the repo root closes that.
    QStringList entries;
    QDirIterator it(QStringLiteral(APPNAME_REPO_ROOT),
                    QStringList{QStringLiteral("*.md")}, QDir::Files,
                    QDirIterator::Subdirectories);
    while (it.hasNext())
        entries << it.next();
    QVERIFY2(!entries.isEmpty(),
             "no markdown found under the repo root — census discovery is "
             "blind");
    QString censusFile;
    QStringList controls;
    for (const QString &path : entries) {
        const QString name = QFileInfo(path).fileName();
        // MY OWN LOG NAMES THE CENSUS (it records this very rule), so a
        // bare phrase match finds it and parses my scoreboard rows as
        // "controls" — 140 of them, observed at T9. Discovery must exclude
        // this file and must require a real census TABLE, not a mention.
        if (name.compare(QStringLiteral("QA-q-gesture-log.md"),
                         Qt::CaseInsensitive) == 0)
            continue;
        QFile f(path);
        if (!f.open(QIODevice::ReadOnly | QIODevice::Text))
            continue;
        const QString body = QString::fromUtf8(f.readAll());
        if (!body.contains(QLatin1String("interactive-surface census"),
                           Qt::CaseInsensitive)
            && !body.contains(QLatin1String("interactive surface census"),
                              Qt::CaseInsensitive)
            && !body.contains(QLatin1String("census-item25"),
                              Qt::CaseInsensitive)
            && !body.contains(QLatin1String("ITEM-25 CENSUS"),
                              Qt::CaseInsensitive))
            continue;
        // DEFINING PROPERTY: the census declares its class tallies in a
        // SUMMARY TABLE ("| **ARMED-SILENT** | **20** |"). Other documents
        // merely MENTION the census and those words — i18n/MINTED.md does
        // both and was selected twice by weaker tests. Requiring the
        // tallies to actually PARSE picks the real artifact.
        auto declaredIn = [](const QString &text, const char *cls) -> int {
            QRegularExpression r(
                QStringLiteral("\\|\\s*\\*\\*%1\\*\\*\\s*\\|\\s*\\*?\\*?(\\d+)")
                    .arg(QLatin1String(cls)));
            const auto m = r.match(text);
            return m.hasMatch() ? m.captured(1).toInt() : 0;
        };
        if (declaredIn(body, "ARMED-SILENT") + declaredIn(body, "DEAD") <= 0)
            continue;
        // Find, in EVERY table, the column whose header names the
        // control ("control" / "surface" / "tool"), and read THAT column —
        // not column 0. The committed census's rows are
        // `| # | Tool (wire id) | Cite | ... |`, so a column-0 parser
        // silently read the row NUMBERS and reported 6 controls out of a
        // 73-row file: a partial parse that looks like success. Guarding
        // only against zero was not enough.
        const auto lines = body.split(QLatin1Char('\n'));
        int col = -1;
        for (const QString &ln : lines) {
            const QString t = ln.trimmed();
            if (!t.startsWith(QLatin1Char('|'))) {
                col = -1; // table ended
                continue;
            }
            if (t.contains(QLatin1String("---")))
                continue;
            const QStringList cells =
                t.split(QLatin1Char('|'), Qt::SkipEmptyParts);
            if (cells.isEmpty())
                continue;
            if (col < 0) {
                for (int c = 0; c < cells.size(); ++c) {
                    const QString h = cells.at(c).trimmed().toLower();
                    if (h.contains(QLatin1String("control"))
                        || h.contains(QLatin1String("surface"))
                        || h.contains(QLatin1String("tool"))) {
                        col = c;
                        break;
                    }
                }
                continue; // header row consumed
            }
            if (col >= cells.size())
                continue;
            QString v = cells.at(col).trimmed();
            v.remove(QLatin1Char('`'));
            v.remove(QLatin1String("**"));
            if (!v.isEmpty())
                controls << v;
        }
        controls.removeDuplicates();
        if (!controls.isEmpty()) {
            censusFile = path;
            break;
        }
    }
    if (censusFile.isEmpty()) {
        qInfo().noquote()
            << "CENSUS: no census TABLE found — searched" << entries.size()
            << "*.md under the repo root for a control/surface header "
               "column (own log excluded). O-UI's item-25 census exists but "
               "lives at the UNTRACKED scratchpad/census-item25.md (168 "
               "WIRED / 20 ARMED-SILENT / 4 DEAD per O-UI-log); it becomes "
               "enforceable here the moment it is committed anywhere in the "
               "repo. Coverage this tick = palette registry + options model.";
        return;
    }
    qInfo().noquote() << "CENSUS: found" << censusFile << "with"
                      << controls.size() << "enumerated controls";
    // UNDER-PARSE GUARD, derived from the census's OWN declared tallies
    // rather than from whatever my parser happens to produce. The census
    // states its ARMED-SILENT and DEAD counts in a summary table; the
    // enumerated rows must at least cover them. A parser that silently
    // reads the wrong column trips this instead of passing on a partial
    // read.
    QFile cf(censusFile);
    QVERIFY(cf.open(QIODevice::ReadOnly | QIODevice::Text));
    const QString cbody = QString::fromUtf8(cf.readAll());
    auto declared = [&](const char *cls) -> int {
        QRegularExpression r(QStringLiteral("\\|\\s*\\*\\*%1\\*\\*\\s*\\|\\s*\\*?\\*?(\\d+)")
                                 .arg(QLatin1String(cls)));
        const auto m = r.match(cbody);
        return m.hasMatch() ? m.captured(1).toInt() : 0;
    };
    const int wantArmed = declared("ARMED-SILENT");
    const int wantDead = declared("DEAD");
    const int floorRows = wantArmed + wantDead;
    qInfo().noquote()
        << QStringLiteral("CENSUS: declares ARMED-SILENT=%1 DEAD=%2 -> at "
                          "least %3 enumerated rows expected")
               .arg(wantArmed).arg(wantDead).arg(floorRows);
    QVERIFY2(controls.size() >= floorRows && floorRows > 0,
             qPrintable(QStringLiteral("the census at %1 is COMMITTED but "
                                       "this lint parsed only %2 controls "
                                       "against its own declared floor of "
                                       "%3 (ARMED-SILENT %4 + DEAD %5) — a "
                                       "PARTIAL parse, which must never read "
                                       "as coverage; update the parser to "
                                       "the committed shape")
                            .arg(censusFile).arg(controls.size())
                            .arg(floorRows).arg(wantArmed).arg(wantDead)));
    // ---- T42: ACTUAL ENFORCEMENT -------------------------------------
    // WHAT THIS REPLACED, recorded because it is the finding: this case
    // ended with "enforcement roster pending the committed table's shape —
    // the enforcement set from next tick." The census was COMMITTED on
    // 2026-08-21 and that line stayed green and inert for ~21 ticks (G-7,
    // MAJOR, self-filed T42). It parsed the table, guarded the parse, and
    // then asserted NOTHING about coverage — a guard occupying the slot of
    // the guard the operator's extended rule actually asked for.
    //
    // THE ENUMERATED SET is the NUMBERED rows of the census's two control
    // tables ("| # | Tool (wire id) |" and "| # | Control |"): 20 + 4 = 24,
    // which is exactly the census's own declared ARMED-SILENT(20) +
    // DEAD(4). The other tables are group SUMMARIES ("Surface group |
    // Count") and the 6 MISSING SURFACE rows the census itself marks "not
    // a control — no widget ships"; enumerating those as controls would
    // demand gesture cases for widgets that do not exist.
    QStringList enumerated;
    {
        int table = 0; // 1 = tools, 2 = controls
        const QStringList lines = cbody.split(QLatin1Char('\n'));
        for (const QString &ln : lines) {
            if (ln.startsWith(QLatin1String("| # | Tool"))) { table = 1; continue; }
            if (ln.startsWith(QLatin1String("| # | Control"))) { table = 2; continue; }
            if (ln.startsWith(QLatin1String("| Surface"))
                || ln.startsWith(QLatin1String("| Class"))) { table = 0; continue; }
            if (table == 0 || !ln.startsWith(QLatin1Char('|'))) continue;
            QStringList cells = ln.split(QLatin1Char('|'));
            for (QString &c : cells) c = c.trimmed();
            cells.removeAll(QString());
            if (cells.size() < 2) continue;
            bool isNum = false;
            cells.at(0).toInt(&isNum);
            if (!isNum) continue;
            QString name = cells.at(1);
            name.remove(QLatin1Char('`'));
            name.remove(QLatin1String("**"));
            if (!name.isEmpty()) enumerated << name.trimmed();
        }
    }
    qInfo().noquote() << "CENSUS: enumerated control rows:" << enumerated.size();
    // The parse is now EXACT against the census's own declared tallies, not
    // merely floored: a row added or dropped moves this and reds here.
    QCOMPARE(enumerated.size(), floorRows);

    // THE ROSTER. Every enumerated control names the case that covers it,
    // or is explicitly classified as not-a-control. An unmapped row REDS —
    // that is the "census can never silently regress" requirement.
    struct Cover { const char *control; const char *by; };
    static const Cover roster[] = {
        // 18 armed tools — the chrome armed-honesty probe drives each one
        // and asserts it announces on press (18 honest of 18 at T42).
        {"select.quick", "armedControlsAnnouncePendingOnPress"},
        // T67: no longer armed-only — Q-D1(a). See the registry note above.
        {"heal.stroke", "paletteModeToolsReachTheEngineAsTheirFamilyKind"},
        {"clone.stroke", "armedControlsAnnouncePendingOnPress"},
        {"content_aware_fill", "armedControlsAnnouncePendingOnPress"},
        {"fill", "armedControlsAnnouncePendingOnPress"},
        {"dodge_burn.dodge", "paletteModeToolsReachTheEngineAsTheirFamilyKind"},
        {"dodge_burn.burn", "paletteModeToolsReachTheEngineAsTheirFamilyKind"},
        // T67: no longer armed-only — Q-D3(b).
        {"dodge_burn.sponge", "paletteModeToolsReachTheEngineAsTheirFamilyKind"},
        {"blur_sharpen.blur", "paletteModeToolsReachTheEngineAsTheirFamilyKind"},
        {"blur_sharpen.sharpen", "paletteModeToolsReachTheEngineAsTheirFamilyKind"},
        // T64: no longer armed-only — see the note above.
        {"smudge.stroke", "smudgeDragReachesTheEngineAsASmudgeStroke"},
        {"shape.line", "armedControlsAnnouncePendingOnPress"},
        {"shape.polygon", "armedControlsAnnouncePendingOnPress"},
        {"shape.rounded_rect", "armedControlsAnnouncePendingOnPress"},
        {"transform.warp", "armedControlsAnnouncePendingOnPress"},
        {"selection.transform", "armedControlsAnnouncePendingOnPress"},
        {"slice.add", "armedControlsAnnouncePendingOnPress"},
        {"slice.select", "armedControlsAnnouncePendingOnPress"},
        // 2 dangling-plumbing rows: no widget ships, so there is no
        // gesture to make. Classified, not silently skipped.
        {"selectionSubmitted signal", "NOT-A-CONTROL: plumbing, no widget"},
        // ★ T56 CORRECTION. This was classified NOT-A-CONTROL because the
        // census says "penCommitsShape Q_PROPERTY — writable, no UI". That
        // is STALE: Main.qml:942 binds it to a SEGMENT control that ships
        // whenever vector.pen is selected, and clicking the inactive half
        // flips it (measured, and mutation-verified against that button's
        // onClicked). My roster inherited the census's wording instead of
        // checking the tree — a classification is only as current as the
        // document it was read from, and "no widget" is an absence claim
        // like any other.
        {"penCommitsShape Q_PROPERTY",
         "toolOptionRowControlsReachTheCanvasThroughAClick"},
        // 4 CONTROLS rows
        {"HistoryPanel rows", "historyPanelRowClickRewindsTheDocument"},
        {"Filter", "honestlyDisabledControlsStayDisabled"},
        {"FillDialog blend ComboBox", "honestlyDisabledControlsStayDisabled"},
    };
    QStringList unmapped, missingCase;
    const QString netSrc = cbody.isEmpty() ? QString() : QString();
    // Read both suite sources so a renamed/deleted case reds here rather
    // than leaving the roster pointing at nothing.
    QString suites;
    for (const char *f : {"/shell/tests/tst_gesture_net.cpp",
                          "/shell/tests/tst_gesture_chrome.cpp"}) {
        QFile sf(QStringLiteral(APPNAME_REPO_ROOT) + QLatin1String(f));
        if (sf.open(QIODevice::ReadOnly | QIODevice::Text))
            suites += QString::fromUtf8(sf.readAll());
    }
    QVERIFY2(!suites.isEmpty(), "could not read my own suite sources — the "
                                "roster cannot be checked against them");
    for (const QString &ctl : enumerated) {
        const char *by = nullptr;
        for (const Cover &c : roster) {
            if (ctl.startsWith(QLatin1String(c.control))) { by = c.by; break; }
        }
        if (!by) { unmapped << ctl; continue; }
        const QString byName = QString::fromLatin1(by);
        if (byName.startsWith(QLatin1String("NOT-A-CONTROL")))
            continue;
        // MATCH THE DEFINITION, NOT THE NAME. `suites.contains(byName)`
        // was SELF-MATCHING: this roster lives in tst_gesture_net.cpp,
        // which is one of the files being searched, so every entry found
        // ITSELF and the check could never fail. Caught by mutation —
        // pointing an entry at a case that does not exist stayed GREEN.
        // A second inert guard, inside the commit that fixes the first
        // inert guard (G-7). Requiring "void TstGestureXxx::<name>("
        // cannot match the roster literal.
        if (!suites.contains(QStringLiteral("void TstGestureNet::%1(").arg(byName))
            && !suites.contains(
                QStringLiteral("void TstGestureChrome::%1(").arg(byName)))
            missingCase << QStringLiteral("%1 -> %2").arg(ctl, byName);
    }
    QVERIFY2(unmapped.isEmpty(),
             qPrintable(QStringLiteral("CENSUS REGRESSION: %1 enumerated "
                                       "control(s) have NO coverage entry — "
                                       "each needs a real gesture case, an "
                                       "armed-honest probe, or an explicit "
                                       "not-a-control classification: %2")
                            .arg(unmapped.size())
                            .arg(unmapped.join(QStringLiteral("; ")))));
    QVERIFY2(missingCase.isEmpty(),
             qPrintable(QStringLiteral("the roster names case(s) that no "
                                       "longer exist in my suites: %1")
                            .arg(missingCase.join(QStringLiteral("; ")))));
    // COUNT the classes rather than restating them. The previous message
    // hardcoded "18 / 4 / 2", and the moment T56 reclassified
    // penCommitsShape the sentence became false while the assertion stayed
    // true — a summary line that cannot go stale is worth more than one
    // that reads well.
    int nProbe = 0, nCase = 0, nNotControl = 0;
    for (const QString &ctl : enumerated) {
        for (const Cover &c : roster) {
            if (!ctl.startsWith(QLatin1String(c.control)))
                continue;
            const QString by = QString::fromLatin1(c.by);
            if (by.startsWith(QLatin1String("NOT-A-CONTROL")))
                ++nNotControl;
            else if (by == QLatin1String("armedControlsAnnouncePendingOnPress"))
                ++nProbe;
            else
                ++nCase;
            break;
        }
    }
    qInfo().noquote()
        << QStringLiteral("CENSUS ENFORCED: %1/%1 enumerated controls mapped "
                          "(%2 armed-honest probe, %3 real gesture/disabled "
                          "cases, %4 classified not-a-control)")
               .arg(enumerated.size()).arg(nProbe).arg(nCase).arg(nNotControl);
}

// RETIRED at T22. This probe drove a BARE CanvasView and claimed to
// detect the honesty slice, but the sweep is wired at Main.qml:211
// (announcePending -> AppA11y.announce + AppDoc.statusPending). A
// bare-canvas harness cannot observe it and reported "0 honest, 18
// silent" FOREVER — which, read literally under the operator's rule,
// is a MAJOR finding against a slice that in fact works (verified:
// tst_gesture_chrome::armedControlsAnnouncePendingOnPress reads 18
// honest, 0 silent). A probe that structurally cannot see its subject
// is worse than no probe, so it is gone rather than caveated. Canvas
// inertness for these tools remains pinned by
// tst_palette_pins::r2Tier2MembersArmedNotLiveCanvasInert.
void TstGestureNet::optionsModelCoverageLint()
{
    // T14 recorded that the options bar's controls are NOT palette tools,
    // so registryCoverageLint is structurally blind to them. This is that
    // gap closed: the same two-sided coverage discipline applied to 11d's
    // DECLARATIVE OPTIONS MODEL, parsed out of the checked-in Main.qml.
    //
    // Every `binding:` in the model must be on the roster below, and every
    // roster row must still exist in the model — so a new option cannot
    // ship uncovered, and a removed one cannot leave a stale row claiming
    // coverage.
    //
    // ========= WHAT THIS LINT DOES AND DOES NOT ESTABLISH =========
    // Three changes (operator, 2026-08-24), and the third is deliberately
    // NOT an assertion.
    //
    // 1. THE CITATION COLUMN IS NOW CHECKED. Until this tick only
    //    `binding` was ever compared against anything: the `status`
    //    column was free prose that nothing read, so a row could cite a
    //    case that had been deleted and this lint would pass. Every
    //    citation must now RESOLVE to a declared slot in the named test
    //    file. That is a real check and it reds. The prose that used to
    //    be glued onto the end of the citation now lives in its own
    //    `note` field, where it cannot corrupt the parse.
    //
    // 2. THE DENOMINATOR IS SPLIT. The old emission printed one figure —
    //    "10 bindings, 10 roster rows" — that folded together rows which
    //    are not the same kind of thing at all: three of the ten CANNOT
    //    reach the engine (two typed-NI on an absent ARAP backend, one
    //    presentation-only by the D2 ruling), and one of those three has
    //    no case pinning it and never can. Summing them into a single
    //    "coverage" number overstates what is pinned by 3 rows in 10.
    //
    // 3. DISCRIMINATION IS NOT CHECKABLE HERE, AND THIS LINT SAYS SO.
    //    Whether a cited case would RED if its binding stopped reaching
    //    the engine is a property of that case UNDER MUTATION, not a fact
    //    any source parse can read. A self-declared "mutation-tested"
    //    marker would be worse than nothing — the author who writes a
    //    non-discriminating case writes the marker too. What this lint
    //    CAN carry is a MEASURED hazard where one is on record, and the
    //    `hazard` column below is exactly that: DATA, not a check. It is
    //    non-null on `brushColor`, whose cited case C-120 measured
    //    PASSING under a live mutation of its own named subject.
    enum Reach { EngineReaching, TypedNI, PresentationOnly };
    struct Row {
        const char *binding;
        Reach reach;
        // tst_file::caseName, or nullptr when nothing pins this row and
        // nothing can (presentation-only).
        const char *where;
        const char *note;
        // A hazard MEASURED against the cited case, or nullptr. This lint
        // checks only that the text is here; it cannot check discrimination.
        const char *hazard;
    };
    static const Row roster[] = {
        // REACHES THE ENGINE — pinned by a gesture case that commits.
        {"textFontSize", EngineReaching,
         "tst_gesture_net::toolOptionReachesTheEngineThroughTheGesture",
         "slider value -> committed text size", nullptr},
        {"brushSize", EngineReaching,
         "tst_gesture_net::brushSizeOptionReachesTheEngine",
         "painted extent measured at 4 px vs 40 px", nullptr},
        {"brushHardness", EngineReaching,
         "tst_gesture_net::brushHardnessOptionReachesTheEngine",
         "edge falloff measured hard vs soft", nullptr},
        // Pro-Wave row 4. NOT credited to row 4's own door-level pixel
        // proof (tst_options_bar::strokeSurvivesCompositeAndExportDoors):
        // this column means REACHES THE ENGINE THROUGH THE GESTURE, and
        // a door test that writes its own envelope cannot establish
        // that. The pin below drives the spinbox's property and commits
        // a real shape.rect drag.
        {"shapeStrokeWidth", EngineReaching,
         "tst_gesture_net::shapeStrokeWidthOptionReachesTheEngine",
         "spinbox value -> committed shape.rect stroke width", nullptr},
        // REACHES THE ENGINE — covered by an existing gesture case that
        // depends on the option's value.
        {"brushColor", EngineReaching,
         "tst_gesture_net::brushDragPaintsAlongPathOnly",
         "the case paints m_brushColor",
         "C-120 measured this case PASSING under a live mutation of its "
         "own named subject (stroke path decimated to its chord); whether "
         "it discriminates brushColor was never measured either way"},
        {"penCommitsShape", EngineReaching,
         "tst_selection_gestures::penClosesAndCommitsShapeLayer",
         "sets it true", nullptr},
        {"wandTolerance", EngineReaching,
         "tst_gesture_net::wandOnTransparentIsAlphaAware",
         "contract default 32", nullptr},
        {"wandContiguous", EngineReaching,
         "tst_gesture_net::wandOnTransparentIsAlphaAware",
         "contract default true", nullptr},
        // Row 5 (C-666): the case is DISCRIMINATING by construction — the
        // engine forks on the flag (typed-pending without it, serves with
        // it, on a 2-raster document), so the flag must have ridden the
        // envelope for the case to go green. heal/clone's emit arms are
        // pinned at the builder (tst_envelope_builder), not here: heal's
        // spot solve is pixel-identical either way on the suite's docs,
        // and clone does not dispatch (Q-D14).
        {"sampleAllLayers", EngineReaching,
         "tst_selection_gestures::wandSampleAllLayersServesOnMultiRasterDocument",
         "the engine FORKS on the flag, so green requires it on the wire",
         nullptr},
        // CANNOT REACH THE ENGINE TODAY — the puppet solve is typed-NI
        // (ARAP backend absent), pinned as an honest refusal instead by
        // puppetSessionRefusesTypedRatherThanSilently. These flip to
        // engine-reaching pins when the backend lands.
        {"puppetExpansion", TypedNI,
         "tst_gesture_net::puppetSessionRefusesTypedRatherThanSilently",
         "ARAP backend absent; the pin is the refusal, not an effect",
         nullptr},
        {"puppetRigidity", TypedNI,
         "tst_gesture_net::puppetSessionRefusesTypedRatherThanSilently",
         "ARAP backend absent; the pin is the refusal, not an effect",
         nullptr},
        // PRESENTATION-ONLY BY DESIGN — the model's own comment calls it
        // "the D2-ruled presentation-only mesh overlay toggle", so there
        // is no engine side to reach. Rostered explicitly so the claim is
        // visible rather than an unexplained omission.
        {"puppetShowMesh", PresentationOnly, nullptr,
         "D2 ruling, model comment — no engine side exists to reach, so "
         "NO case pins this row and none can", nullptr},
    };

    QFile f(QStringLiteral(APPNAME_MAIN_QML));
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             "could not read the checked-in Main.qml options model");
    const QString src = QString::fromUtf8(f.readAll());
    // Scan the WHOLE file, not a window around toolOptionsModel. The
    // first cut windowed from "toolOptionsModel" to "header: ToolBar" and
    // MISSED brushSize, which is declared in the shared `optRowBrushSize`
    // row ABOVE the model — i.e. a binding could hide in a shared row and
    // read as covered. Verified safe to widen: every `binding:` in
    // Main.qml is an options-model binding, there are no unrelated uses.
    // The model-presence guard below still reds if the model is renamed.
    QVERIFY2(src.contains(QLatin1String("toolOptionsModel")),
             "toolOptionsModel not found in Main.qml — the options model "
             "moved or was renamed and this lint went blind");
    const QString block = src;

    QRegularExpression re(QStringLiteral("binding:\\s*\"([A-Za-z0-9_]+)\""));
    QStringList bindings;
    auto it = re.globalMatch(block);
    while (it.hasNext())
        bindings << it.next().captured(1);
    bindings.removeDuplicates();
    QVERIFY2(bindings.size() >= 6,
             qPrintable(QStringLiteral("the options-model parse found only "
                                       "%1 bindings — the row shape changed "
                                       "and this lint went blind")
                            .arg(bindings.size())));

    QStringList rostered;
    for (const Row &r : roster)
        rostered << QLatin1String(r.binding);
    QStringList uncovered, stale;
    for (const QString &b : bindings)
        if (!rostered.contains(b))
            uncovered << b;
    for (const QString &r : rostered)
        if (!bindings.contains(r))
            stale << r;

    // TEETH: resolve every named case to a DECLARED SLOT in the named test
    // file. Until this tick the citation column was never compared against
    // anything, so a row could name a case that did not exist and this
    // lint passed. (Same shape as registryCoverageLint's resolver; kept
    // local rather than shared so the two lints stay independently
    // readable and independently portable.)
    auto resolveSlot = [](const QString &cite, QString *why) -> bool {
        static QRegularExpression cre(
            QStringLiteral("^(tst_[A-Za-z0-9_]+)::([A-Za-z0-9_]+)"));
        const auto m = cre.match(cite);
        if (!m.hasMatch()) {
            *why = QStringLiteral("not in tst_file::caseName form");
            return false;
        }
        const QString path = QStringLiteral(APPNAME_REPO_ROOT)
            + QStringLiteral("/shell/tests/") + m.captured(1)
            + QStringLiteral(".cpp");
        QFile tf(path);
        if (!tf.open(QIODevice::ReadOnly | QIODevice::Text)) {
            *why = QStringLiteral("no such test file %1").arg(path);
            return false;
        }
        const QString body = QString::fromUtf8(tf.readAll());
        const QRegularExpression sre(
            QStringLiteral("\\bvoid\\s+%1\\s*\\(\\s*\\)").arg(m.captured(2)));
        if (!sre.match(body).hasMatch()) {
            *why = QStringLiteral("%1 declares no slot %2")
                       .arg(m.captured(1), m.captured(2));
            return false;
        }
        return true;
    };

    QStringList unresolved, typedNi, presentationOnly, hazards;
    int engineReaching = 0, cited = 0, resolvedCases = 0;
    for (const Row &r : roster) {
        switch (r.reach) {
        case EngineReaching: ++engineReaching; break;
        case TypedNI:
            typedNi << QLatin1String(r.binding);
            break;
        case PresentationOnly:
            presentationOnly << QLatin1String(r.binding);
            break;
        }
        if (r.where) {
            ++cited;
            QString why;
            if (resolveSlot(QLatin1String(r.where), &why))
                ++resolvedCases;
            else
                unresolved << QStringLiteral("%1 -> %2 (%3)")
                                  .arg(QLatin1String(r.binding),
                                       QLatin1String(r.where), why);
        }
        if (r.hazard)
            hazards << QStringLiteral("%1 -> %2: %3")
                           .arg(QLatin1String(r.binding),
                                QLatin1String(r.where),
                                QLatin1String(r.hazard));
    }
    const int rows = int(sizeof(roster) / sizeof(roster[0]));
    // Vacuity guard (rule 9): every row that names a citation must resolve
    // one way or the other. Anything short of that means the citation
    // parse went blind rather than that a row is genuinely unresolvable —
    // and a blind parse finds no misses and passes.
    QVERIFY2(resolvedCases + unresolved.size() >= cited,
             qPrintable(QStringLiteral("the citation parse resolved neither "
                                       "way for some rows (%1 ok + %2 bad < "
                                       "%3 cited) — this lint went blind")
                            .arg(resolvedCases)
                            .arg(unresolved.size())
                            .arg(cited)));
    // A positive control on the same instrument: at least one citation
    // must have resolved. Zero would mean the resolver never found a test
    // file at all, which reads identically to "nothing to check".
    QVERIFY2(resolvedCases > 0,
             "the citation resolver resolved ZERO of the rostered cases — "
             "the instrument is broken, not the roster");

    // ★ WHAT IS ESTABLISHED, WITH ITS DENOMINATORS — printed instead of a
    // bare "10 bindings, 10 roster rows" that reads as coverage.
    qInfo().noquote()
        << QStringLiteral(
               "OPTIONS-MODEL <-> ROSTER PARTITION: %1 bindings in Main.qml, "
               "%2 roster rows, %3 of %4 named cases resolved to real "
               "slots.\n"
               "  THIS ESTABLISHES: every binding in the model has a roster "
               "row, every roster row is still a binding, and every case a "
               "row names EXISTS.\n"
               "  THIS DOES NOT ESTABLISH: that any named case would RED if "
               "its binding stopped reaching the engine. Discrimination is a "
               "property of a case UNDER MUTATION and no source parse can "
               "read it.\n"
               "  SPLIT DENOMINATOR — the single figure this lint used to "
               "print folded rows that cannot reach the engine in with those "
               "that do:\n"
               "    ENGINE-REACHING:   %5 of %2\n"
               "    TYPED-NI:          %6 of %2 [%7] — an absent backend, "
               "pinned as an honest refusal, NOT as an effect on pixels\n"
               "    PRESENTATION-ONLY: %8 of %2 [%9] — no engine side "
               "exists, so no case pins these and none can (%10 of %2 rows "
               "name no case at all)\n"
               "  MEASURED HAZARDS ON CITED CASES: %11 of %2. %12")
               .arg(bindings.size())
               .arg(rows)
               .arg(resolvedCases)
               .arg(cited)
               .arg(engineReaching)
               .arg(typedNi.size())
               .arg(typedNi.join(QStringLiteral(", ")))
               .arg(presentationOnly.size())
               .arg(presentationOnly.join(QStringLiteral(", ")))
               .arg(rows - cited)
               .arg(hazards.size())
               .arg(hazards.isEmpty() ? QStringLiteral("(none on record)")
                                      : hazards.join(QStringLiteral(" | ")));

    QVERIFY2(uncovered.isEmpty(),
             qPrintable(QStringLiteral("OPTIONS-BAR CONTROLS WITH NO "
                                       "COVERAGE: %1 — an option shipped "
                                       "that no gesture case exercises")
                            .arg(uncovered.join(QStringLiteral(", ")))));
    QVERIFY2(stale.isEmpty(),
             qPrintable(QStringLiteral("roster rows no longer in the options "
                                       "model: %1")
                            .arg(stale.join(QStringLiteral(", ")))));
    // THE TEETH ON THE CITATION COLUMN. Counting resolutions and PRINTING
    // the count is not a check: the first cut of this change did exactly
    // that, and a mutation that renamed one cited case printed "8 of 9
    // resolved" and PASSED — the decorative-column failure this whole
    // change exists to remove, reproduced one level up. This is the
    // assertion that stops it.
    QVERIFY2(unresolved.isEmpty(),
             qPrintable(QStringLiteral("options-roster citations that name "
                                       "no real slot (a row can no longer "
                                       "claim coverage by naming a case that "
                                       "does not exist): %1")
                            .arg(unresolved.join(QStringLiteral("; ")))));
}

void TstGestureNet::brushSizeOptionReachesTheEngine()
{
    // The most-used option in the app, and the same class T14 pinned for
    // text: the slider moves, the shell property moves, and the committed
    // stroke must actually be that wide. Engine truth is the painted
    // extent, measured perpendicular to a horizontal stroke.
    auto strokeHeightAtSize = [&](double sizePx) -> int {
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return -1;
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        m_canvas->forceActiveFocus();
        m_canvas->setProperty("brushSize", sizePx);
        setTool("brush.stroke");
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(160, 200));
        QTest::mouseMove(m_window, docToView(300, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(440, 200));
        const QRect b = engineContentBounds(m_controller);
        return b.isEmpty() ? -1 : b.height();
    };
    const int thin = strokeHeightAtSize(4.0);
    const int fat = strokeHeightAtSize(40.0);
    qInfo().noquote()
        << QStringLiteral("BRUSH-SIZE: 4px stroke height=%1, 40px height=%2")
               .arg(thin).arg(fat);
    QVERIFY2(thin > 0 && fat > 0,
             qPrintable(QStringLiteral("a brush stroke painted nothing "
                                       "(thin=%1 fat=%2); status: %3")
                            .arg(thin).arg(fat)
                            .arg(m_controller->statusText())));
    // Discriminating: a stroke committed at 40 px must be MUCH taller than
    // one at 4 px. An option that never reached the engine gives two equal
    // heights.
    QVERIFY2(fat >= thin * 3,
             qPrintable(QStringLiteral("the brush-size option did not reach "
                                       "the engine: a 4 px stroke painted "
                                       "%1 px tall and a 40 px stroke "
                                       "painted %2 px tall")
                            .arg(thin).arg(fat)));
}

// -------------------------------------------- perspective / puppet frames

void TstGestureNet::brushHardnessOptionReachesTheEngine()
{
    // Item 4 merged this wave — the brush-hardness model row, the FI-9
    // payoff finally reaching the shell. optionsModelCoverageLint RED'd on
    // it the moment it landed (an option shipped with no gesture case),
    // which is the lint doing exactly its job; this is the pin that
    // answers it.
    //
    // Engine truth: hardness is edge falloff. A HARD brush lays a crisp
    // edge; a SOFT one feathers, so its ink spreads further from the
    // stroke's centre line. Measure the painted extent perpendicular to a
    // horizontal stroke at the same size, and compare.
    auto strokeSpreadAtHardness = [&](double hardness) -> int {
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return -1;
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        m_canvas->forceActiveFocus();
        m_canvas->setProperty("brushSize", 30.0);
        m_canvas->setProperty("brushHardness", hardness);
        setTool("brush.stroke");
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(160, 200));
        QTest::mouseMove(m_window, docToView(300, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(440, 200));
        const QRect b = engineContentBounds(m_controller);
        return b.isEmpty() ? -1 : b.height();
    };
    const int hard = strokeSpreadAtHardness(1.0);
    const int soft = strokeSpreadAtHardness(0.0);
    qInfo().noquote()
        << QStringLiteral("BRUSH-HARDNESS: hard(1.0) spread=%1, soft(0.0) "
                          "spread=%2").arg(hard).arg(soft);
    QVERIFY2(hard > 0 && soft > 0,
             qPrintable(QStringLiteral("a stroke painted nothing (hard=%1 "
                                       "soft=%2); status: %3")
                            .arg(hard).arg(soft)
                            .arg(m_controller->statusText())));
    // Discriminating: the two hardness extremes must not paint identical
    // footprints. An option that never reached the engine gives the same
    // spread twice.
    QVERIFY2(hard != soft,
             qPrintable(QStringLiteral("hardness 1.0 and 0.0 painted the "
                                       "IDENTICAL footprint (%1 px both) — "
                                       "the options-bar hardness never "
                                       "reached the engine").arg(hard)));
}

void TstGestureNet::shapeStrokeWidthOptionReachesTheEngine()
{
    // Pro-Wave row 4 merged this wave: the options bar's stroke-width
    // spinbox drives CanvasView's shapeStrokeWidth property, which
    // submitShapePrimitive() folds into vector.shape_add's REQUIRED
    // §20.2 `stroke.widthPx`. optionsModelCoverageLint RED'd the moment
    // the row met main (11 bindings, 10 roster rows) — an option shipped
    // that no gesture case exercises. This is the pin that answers it,
    // on the same pattern as the two brush options above.
    //
    // Row 4 already owns a DOOR-level pixel proof
    // (tst_options_bar::strokeSurvivesCompositeAndExportDoors), and that
    // is deliberately NOT what the roster credits. The roster column
    // means REACHES THE ENGINE THROUGH THE GESTURE; a door test hands
    // applyCommand an envelope it wrote itself, so it is structurally
    // blind to the defect this pin exists for — a property the options
    // bar moves that submitShapePrimitive never folds into the envelope.
    // Only a synthesized drag can see that.
    //
    // ENGINE TRUTH, and why it is quantitative rather than merely
    // different: §20.2 `align` defaults to Center (vector_ops.rs
    // ShapeStrokeStyle::from_wire elects it where the payload is
    // silent), so the band is ±width/2 about the path and a stroked
    // rect's painted bounds OUTSET by width/2 on every side. The
    // committed footprint at width w must therefore be the width-0
    // footprint grown by EXACTLY w in each dimension. A wire that
    // emitted a constant fails the first arm; a wire that emitted a
    // SCALED width still moves the pixels and passes a bare
    // "the two differ" check — it fails only this one.
    auto shapeBoundsAtStrokeWidth = [&](double widthPx) -> QRect {
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return QRect();
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        m_canvas->forceActiveFocus();
        m_canvas->setProperty("shapeStrokeWidth", widthPx);
        setTool("shape.rect");
        // Drag well clear of every edge: at width 40 the band reaches 20
        // px outside the dragged rect, and a bounds read clipped by the
        // canvas would silently coarsen the measurement.
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(200, 140));
        QTest::mouseMove(m_window, docToView(300, 200));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(400, 260));
        return engineContentBounds(m_controller);
    };

    // THE FIXTURE VARIES THE DIMENSION THE NAME CLAIMS: three distinct
    // widths, not one. A stroke-width case driven at a single width
    // proves nothing about width — the defect would be invisible by
    // construction.
    const QRect b0 = shapeBoundsAtStrokeWidth(0.0);
    const QRect b12 = shapeBoundsAtStrokeWidth(12.0);
    const QRect b40 = shapeBoundsAtStrokeWidth(40.0);
    qInfo().noquote()
        << QStringLiteral("SHAPE-STROKE-WIDTH: w=0 %1x%2, w=12 %3x%4, "
                          "w=40 %5x%6")
               .arg(b0.width()).arg(b0.height())
               .arg(b12.width()).arg(b12.height())
               .arg(b40.width()).arg(b40.height());

    // POSITIVE CONTROL / denominator: an instrument that measured
    // nothing must FAIL, never quietly agree. Three gestures, three
    // non-empty footprints.
    QVERIFY2(!b0.isEmpty() && !b12.isEmpty() && !b40.isEmpty(),
             qPrintable(QStringLiteral("a shape.rect drag painted nothing "
                                       "(w=0 empty:%1 w=12 empty:%2 "
                                       "w=40 empty:%3); status: %4")
                            .arg(b0.isEmpty()).arg(b12.isEmpty())
                            .arg(b40.isEmpty())
                            .arg(m_controller->statusText())));

    // ARM 1 — the option reaches the engine at all: three widths, three
    // strictly growing footprints. An option wired to the widget and to
    // the shell property but never folded into the envelope gives three
    // IDENTICAL rects.
    QVERIFY2(b40.width() > b12.width() && b12.width() > b0.width()
                 && b40.height() > b12.height() && b12.height() > b0.height(),
             qPrintable(QStringLiteral("the shape stroke-width option did "
                                       "not reach the engine: widths 0/12/40 "
                                       "committed footprints %1x%2, %3x%4, "
                                       "%5x%6 — they must grow strictly with "
                                       "the driven width")
                            .arg(b0.width()).arg(b0.height())
                            .arg(b12.width()).arg(b12.height())
                            .arg(b40.width()).arg(b40.height())));

    // ARM 2 — the engine received THE VALUE, not just A value. Center
    // alignment puts w/2 outside each side, so the footprint grows by
    // exactly w. Tolerance is for edge antialiasing only (the bounds
    // scan counts any non-white pixel, so a feathered edge can add a
    // pixel per side); it is far tighter than the 12-vs-40 gap, so a
    // scaled or offset width cannot hide inside it.
    const int kTol = 4;
    auto grewBy = [&](const QRect &b, double w) -> bool {
        return qAbs((b.width() - b0.width()) - int(w)) <= kTol
            && qAbs((b.height() - b0.height()) - int(w)) <= kTol;
    };
    QVERIFY2(grewBy(b12, 12.0),
             qPrintable(QStringLiteral("stroke width 12 grew the committed "
                                       "footprint by %1x%2 px, not 12x12 "
                                       "(±%3) — the value the gesture drove "
                                       "is not the value the engine banded")
                            .arg(b12.width() - b0.width())
                            .arg(b12.height() - b0.height()).arg(kTol)));
    QVERIFY2(grewBy(b40, 40.0),
             qPrintable(QStringLiteral("stroke width 40 grew the committed "
                                       "footprint by %1x%2 px, not 40x40 "
                                       "(±%3) — the value the gesture drove "
                                       "is not the value the engine banded")
                            .arg(b40.width() - b0.width())
                            .arg(b40.height() - b0.height()).arg(kTol)));
}

void TstGestureNet::perspectiveEngineMapsSourceBoundsPerContract()
{
    // CONTROL for finding G-3 — this PASSES, and that is the point.
    //
    // contracts/commands/transform.perspective.yaml is titled "Map the
    // SOURCE BOUNDS onto a destination quad", and its quad description
    // pins "the homography is solved from source bounds -> quad (§18.3)".
    // So the engine stretching a small layer to fill the submitted quad is
    // CONTRACT-CORRECT, not a bug. Submitted through the command door with
    // no gesture, this case fixes the engine's half as known-good and
    // isolates G-3 to the shell's quad construction.
    fillBlockOnNewLayer(QRect(300, 180, 40, 40), 0.1, 0.2, 0.8);
    QCOMPARE(engineBlueBounds(m_controller).size(), QSize(39, 39));
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers()
                      .at(m_controller->activeLayerIndex()).toMap()
                      .value(QStringLiteral("layerId")));
    QVariantMap params;
    params.insert(QStringLiteral("quad"),
                  QVariantList{QVariantList{60, 40}, QVariantList{580, 90},
                               QVariantList{580, 310}, QVariantList{60, 360}});
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("transform.perspective"),
                               target, params, {})
                .value(QStringLiteral("ok"))
                .toBool());
    // Source bounds (the 40x40 marker) map ONTO the quad, so the result
    // fills it — the quad spans ~520x320.
    const QRect after = engineBlueBounds(m_controller);
    QVERIFY2(after.width() > 400 && after.height() > 250,
             qPrintable(QStringLiteral("the engine did NOT map source bounds "
                                       "onto the quad (result %1x%2) — the "
                                       "contract's stated behaviour changed, "
                                       "which re-opens G-3's attribution")
                            .arg(after.width()).arg(after.height())));
}

void TstGestureNet::perspectiveCornerDragCommitsOnEnter()
{
    // FINDING G-3 — reds until the shell's quad construction is fixed.
    //
    // transform.perspective arms a quad at the DOCUMENT bounds
    // (beginPerspectiveFrame: m_perspBase = 0,0,docW,docH) and
    // commitPerspective submits those document-space corners as the
    // destination quad. But the door maps the LAYER'S SOURCE BOUNDS onto
    // that quad (contract title + §18.3; the control case above fixes
    // that as correct). The two coincide ONLY for a full-bleed layer —
    // which is why artifacts/tour-chrome/19-perspective.png looks right
    // (the tour transforms the full-canvas Background layer).
    //
    // For any smaller layer the gesture says "stretch this layer to fill
    // a document-sized quad", so a mild keystone floods the canvas with
    // the layer's content and reports success.
    fillBlockOnNewLayer(QRect(200, 140, 240, 120), 0.1, 0.2, 0.8);
    const QRect before = engineContentBounds(m_controller);
    QVERIFY(!before.isEmpty());
    setTool("transform.perspective");
    // Pull the TOP-LEFT corner inward and down — a genuine keystone, not
    // a translation (only one corner moves).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(60, 40));
    QTest::mouseMove(m_window, docToView(120, 90));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(120, 90));
    // The release ends the corner drag; the frame lives until Enter.
    QVERIFY2(engineContentBounds(m_controller) == before,
             "the perspective corner release COMMITTED — the frame must "
             "persist until Enter");
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect after = engineContentBounds(m_controller);
    qInfo() << "perspective TL keystone: content" << before << "->" << after
            << "status:" << m_controller->statusText();
    QVERIFY2(!after.isEmpty(),
             "the committed perspective erased the layer content");
    // FINDING G-3, the ANTI-FLOOD arm. Grabbing the DOCUMENT corner is
    // only a handle while the session is mis-framed at document bounds;
    // once the frame is correctly at the layer's source bounds this grabs
    // nothing and commits nothing, which also satisfies "no flood". So
    // this arm is written to pass in BOTH worlds and fail only on the
    // defect itself: content must never grow past the document.
    QVERIFY2(after.width() < 500 && after.height() < 340,
             qPrintable(QStringLiteral("G-3: the keystone smeared the block "
                                       "across the canvas: %1x%2 -> %3x%4")
                            .arg(before.width()).arg(before.height())
                            .arg(after.width()).arg(after.height())));
}

void TstGestureNet::perspectiveKeystoneOnLayerFrameCommits()
{
    // FINDING G-3, the POSITIVE arm — verified pre-merge against O-UI's
    // fix (94d485e, lane/ui): the session must frame its quad at the
    // LAYER'S SOURCE BOUNDS, which is what the door maps (contract title
    // + §18.3). So the handles sit on the LAYER, and a proportionate
    // keystone there commits a real transform.
    //
    // Two probe lessons paid for pre-merge, recorded so they are not
    // re-learned: (1) a grab hardcoded to the DOCUMENT corner hits no
    // handle once the framing is correct and reads as "commits nothing" —
    // a false FAIL; (2) a fixed pixel delta tuned for a 640x400 frame is
    // a huge distortion on a 240x120 one and trips the contract's
    // degenerate-quad gate (§40.6/§18.4) — a CORRECT refusal that also
    // reads as a fix defect. Both are avoided by deriving the grab point
    // and the delta from the layer's own bounds.
    fillBlockOnNewLayer(QRect(200, 140, 240, 120), 0.1, 0.2, 0.8);
    const QRect frame = engineContentBounds(m_controller);
    QVERIFY(!frame.isEmpty());
    setTool("transform.perspective");
    const int dx = qMax(4, frame.width() * 15 / 100);
    const int dy = qMax(4, frame.height() * 15 / 100);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(frame.x(), frame.y()));
    QTest::mouseMove(m_window,
                     docToView(frame.x() + dx / 2, frame.y() + dy / 2));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(frame.x() + dx, frame.y() + dy));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect after = engineContentBounds(m_controller);
    const QString status = m_controller->statusText();
    qInfo() << "perspective layer-frame keystone:" << frame << "->" << after
            << "status:" << status;
    QVERIFY2(status.contains(QLatin1String("history.transform.perspective")),
             qPrintable(QStringLiteral("a keystone on the LAYER frame "
                                       "committed no transform.perspective "
                                       "— the session is not framed at the "
                                       "layer's source bounds; status: %1")
                            .arg(status)));
    QVERIFY2(!after.isEmpty() && after.width() < 500 && after.height() < 340,
             qPrintable(QStringLiteral("the layer-frame keystone did not "
                                       "stay bounded: %1x%2")
                            .arg(after.width()).arg(after.height())));
}

void TstGestureNet::perspectiveInteriorPixelsMapPerHomography()
{
    // O-WATCH-scoped instrument (#19 follow-up): the shell could construct
    // a PLAUSIBLE-BUT-WRONG quad — swapped corner order, half-pixel origin
    // shift — that no engine-side pin can see, because engine pins test
    // the homography given a quad, not the quad the gesture built. This
    // probe closes that end-to-end by checking where INTERIOR pixels land.
    //
    // Spec followed: two interior markers (an affine-approximation bug
    // maps one of the two wrong); a genuinely non-parallelogram quad
    // (single-corner displacement, so the projective term is exercised);
    // expected positions computed from §18.4's mapping INDEPENDENTLY of
    // the door (mapThroughQuad); and "no result" is unrepresentable — a
    // marker that cannot be located FAILS rather than skipping.
    //
    // TOLERANCE, stated openly because it is the one number I had to
    // derive rather than quote: §18.4 pins the homography exactly (DLT,
    // h33 = 1) and §18.2 pins sampling (Lanczos-3 minify / bicubic
    // magnify), but neither pins raster CENTROID recovery. Budget =
    // 0.5 px destination-grid quantisation + ~0.5 px half-pixel corner
    // convention + ~0.5 px kernel asymmetry across a small marker under
    // the projective term = 1.5 px. It is a budget, not a fitted value.
    //
    // O-WATCH RULING (recorded, review #19 follow-up): KEEP 1.5 px as the
    // acceptance tolerance. The observed residual on the fixed build is
    // 0.14–0.28 px — roughly 5x headroom — and that is a CALIBRATION NOTE,
    // NOT a reason to tighten. Any future argument for a smaller number
    // must attack the DERIVATION (which of the three 0.5 px items
    // genuinely applies to this door) and must never appeal to observed
    // residuals: tightening-to-fit is the failure mode in both directions.
    // This note lives here, beside the constant, because this session is
    // tick-stateless and would otherwise be free to "helpfully" tighten it.
    const double kTolPx = 1.5;

    QVariantMap addParams;
    addParams.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    addParams.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, addParams, {})
                .value(QStringLiteral("ok")).toBool());
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);

    const QRect block(160, 120, 320, 160);
    fillBlock(block, 0.1, 0.2, 0.8);
    const QRect mA(236, 176, 9, 9);   // interior, off-centre
    const QRect mB(396, 236, 9, 9);   // interior, far corner-ward
    fillBlock(mA, 0.95, 0.1, 0.1);    // red
    fillBlock(mB, 0.1, 0.85, 0.2);    // green
    // Centre of the PIXEL SPAN [x, x+w-1], not x + w/2 — a 9 px span from
    // 236 centres on 240, not 240.5. That half pixel is a third of the
    // tolerance budget and must not be spent on my own arithmetic.
    const QPointF pA(mA.x() + (mA.width() - 1) / 2.0,
                     mA.y() + (mA.height() - 1) / 2.0);
    const QPointF pB(mB.x() + (mB.width() - 1) / 2.0,
                     mB.y() + (mB.height() - 1) / 2.0);

    QPointF seenA, seenB;
    QVERIFY2(markerCentroid(m_controller, 'r', &seenA)
                 && markerCentroid(m_controller, 'g', &seenB),
             "setup: a marker was not locatable before the transform");
    QVERIFY2(qAbs(seenA.x() - pA.x()) < 0.25 && qAbs(seenA.y() - pA.y()) < 0.25,
             qPrintable(QStringLiteral("setup: the red marker's centroid "
                                       "(%1,%2) is not where it was painted "
                                       "(%3,%4) — the expected-centre "
                                       "arithmetic is wrong before any "
                                       "transform runs")
                            .arg(seenA.x()).arg(seenA.y())
                            .arg(pA.x()).arg(pA.y())));

    // The frame the SESSION should arm = the layer's source bounds; the
    // gesture displaces ONE corner, 15% per side.
    const QRectF src(block);
    const double dx = block.width() * 0.15;
    const double dy = block.height() * 0.15;
    const QPointF quad[4] = {
        QPointF(src.left() + dx, src.top() + dy), // TL, dragged
        QPointF(src.right(), src.top()),          // TR
        QPointF(src.right(), src.bottom()),       // BR
        QPointF(src.left(), src.bottom())};       // BL
    const QPointF wantA = mapThroughQuad(src, quad, pA);
    const QPointF wantB = mapThroughQuad(src, quad, pB);

    setTool("transform.perspective");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(block.x(), block.y()));
    QTest::mouseMove(m_window, docToView(block.x() + int(dx / 2),
                                         block.y() + int(dy / 2)));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(block.x() + int(dx), block.y() + int(dy)));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);

    const bool gotA = markerCentroid(m_controller, 'r', &seenA);
    const bool gotB = markerCentroid(m_controller, 'g', &seenB);
    qInfo().noquote()
        << QStringLiteral("INTERIOR-MAP: A want(%1,%2) got(%3) | "
                          "B want(%4,%5) got(%6) | status=%7")
               .arg(wantA.x(), 0, 'f', 1).arg(wantA.y(), 0, 'f', 1)
               .arg(gotA ? QStringLiteral("%1,%2").arg(seenA.x(), 0, 'f', 1)
                                  .arg(seenA.y(), 0, 'f', 1)
                         : QStringLiteral("ABSENT"))
               .arg(wantB.x(), 0, 'f', 1).arg(wantB.y(), 0, 'f', 1)
               .arg(gotB ? QStringLiteral("%1,%2").arg(seenB.x(), 0, 'f', 1)
                                  .arg(seenB.y(), 0, 'f', 1)
                         : QStringLiteral("ABSENT"))
               .arg(m_controller->statusText());
    // "No result" is unrepresentable: an absent marker is a FAILURE.
    QVERIFY2(gotA, "the red interior marker VANISHED after the transform");
    QVERIFY2(gotB, "the green interior marker VANISHED after the transform");
    const double eA = QLineF(seenA, wantA).length();
    const double eB = QLineF(seenB, wantB).length();
    QVERIFY2(eA <= kTolPx,
             qPrintable(QStringLiteral("interior point A landed %1 px from "
                                       "the §18.4 mapping (tol %2)")
                            .arg(eA, 0, 'f', 2).arg(kTolPx)));
    QVERIFY2(eB <= kTolPx,
             qPrintable(QStringLiteral("interior point B landed %1 px from "
                                       "the §18.4 mapping (tol %2)")
                            .arg(eB, 0, 'f', 2).arg(kTolPx)));
}

void TstGestureNet::perspectiveIdentityEnterCommitsNothing()
{
    // commitPerspective drops an untouched quad ("no intent — mirror the
    // free-transform identity drop"). A grab-and-release that never moved
    // must therefore change nothing, and must not spend a history entry.
    fillBlockOnNewLayer(QRect(200, 140, 240, 120), 0.1, 0.2, 0.8);
    const QRect before = engineContentBounds(m_controller);
    const int historyBefore = m_controller->historyEntries().size();
    setTool("transform.perspective");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(0, 0));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(engineContentBounds(m_controller), before);
    QCOMPARE(m_controller->historyEntries().size(), historyBefore);
}

void TstGestureNet::perspectiveEscapeCancelsFrame()
{
    fillBlockOnNewLayer(QRect(200, 140, 240, 120), 0.1, 0.2, 0.8);
    const QRect before = engineContentBounds(m_controller);
    setTool("transform.perspective");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(0, 0));
    QTest::mouseMove(m_window, docToView(120, 90));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(120, 90));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTest::keyClick(m_window, Qt::Key_Return); // nothing left to commit
    QVERIFY2(engineContentBounds(m_controller) == before,
             "Esc did not drop the perspective frame — a later Enter still "
             "committed the abandoned keystone");
}

void TstGestureNet::puppetSessionRefusesTypedRatherThanSilently()
{
    // transform.puppet's SOLVE is typed-NI today (the elected ARAP
    // backend is not in). That makes the honest gesture pin the REFUSAL
    // path, not a result: pins placed by click must reach the door on
    // Enter and come back as a surfaced typed refusal — never a silent
    // no-op, and never a spent history entry. When the backend lands this
    // case is what tells us the shell half already worked.
    fillBlockOnNewLayer(QRect(200, 140, 240, 120), 0.1, 0.2, 0.8);
    const QRect before = engineContentBounds(m_controller);
    const int historyBefore = m_controller->historyEntries().size();
    setTool("transform.puppet");
    // Place three pins (a click in free space places a selected pin).
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(240, 170));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(400, 170));
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(320, 250));
    // Alt-drag the last pin's rotation dial (§45.5).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::AltModifier,
                      docToView(320, 250));
    QTest::mouseMove(m_window, docToView(360, 290));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::AltModifier,
                        docToView(360, 290));
    // Depth keys (§45.5 bring-forward / send-backward) must be accepted
    // by the live session, not fall through to the app.
    QTest::keyClick(m_window, Qt::Key_Plus);
    QTest::keyClick(m_window, Qt::Key_Minus);
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QString status = m_controller->statusText();
    qInfo() << "puppet commit status:" << status;
    QVERIFY2(!status.isEmpty(),
             "the puppet commit produced NO status at all — a silent "
             "no-op, not a typed refusal");
    QVERIFY2(status.contains(QLatin1String("reject"))
                 || status.contains(QLatin1String("not_implemented"))
                 || status.contains(QLatin1String("error")),
             qPrintable(QStringLiteral("the puppet commit neither applied "
                                       "nor surfaced a typed refusal; "
                                       "status: %1")
                            .arg(status)));
    // A refused solve must not have touched the document or history.
    QCOMPARE(engineContentBounds(m_controller), before);
    QCOMPARE(m_controller->historyEntries().size(), historyBefore);
}

// ------------------------------------------ navigation / modifiers / crop

void TstGestureNet::zoomedAndPannedMarqueeLandsOnDocPixels()
{
    // The classic "the selection landed somewhere else" class: after the
    // user navigates, a marquee must still select the DOCUMENT pixels
    // under the pointer. Nothing else in the net navigates first — every
    // other case runs at zoom 1, dead centre, where a broken viewToDoc
    // still looks right.
    //
    // The view transform is mirrored here from the documented math so the
    // probe points are derived, not guessed: wheelEvent sets
    // zoom *= 1.25^steps and offset = anchor - anchorDoc*zoom; the
    // space-pan adds its drag delta straight to the offset.
    QPointF offset(80, 100); // zoom 1, 640x400 doc centred in 800x600
    double zoom = 1.0;

    const QPointF anchor(300, 260); // wheel anchor, view coords
    const QPointF anchorDoc = (anchor - offset) / zoom;
    QWheelEvent wheel(anchor, m_window->mapToGlobal(anchor.toPoint()),
                      QPoint(), QPoint(0, 120), Qt::NoButton, Qt::NoModifier,
                      Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(m_window, &wheel);
    zoom *= 1.25;
    offset = anchor - anchorDoc * zoom;
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 125);

    // Space-held left-drag = pan.
    m_canvas->forceActiveFocus();
    QTest::keyPress(m_window, Qt::Key_Space);
    const QPointF panFrom(400, 300), panTo(340, 250);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      panFrom.toPoint());
    QTest::mouseMove(m_window, panTo.toPoint());
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        panTo.toPoint());
    QTest::keyRelease(m_window, Qt::Key_Space);
    offset += (panTo - panFrom);

    auto toView = [&](double x, double y) {
        return QPointF(x * zoom + offset.x(), y * zoom + offset.y()).toPoint();
    };
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(120, 80));
    QTest::mouseMove(m_window, toView(260, 180));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(280, 200));
    QVERIFY2(m_controller->selectionActive(),
             qPrintable(QStringLiteral("a marquee after wheel-zoom + "
                                       "space-pan committed nothing; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    const QRect b = engineSelectionBounds(m_controller);
    // One pixel of slack each side: the view points are rounded to
    // integers before synthesis, so at 125% a half-pixel of rounding maps
    // back to under one document pixel.
    const QRect want(120, 80, 160, 120);
    QVERIFY2(qAbs(b.x() - want.x()) <= 1 && qAbs(b.y() - want.y()) <= 1
                 && qAbs(b.width() - want.width()) <= 1
                 && qAbs(b.height() - want.height()) <= 1,
             qPrintable(QStringLiteral("after zoom 125% + pan the marquee "
                                       "selected %1,%2 %3x%4 — the document "
                                       "pixels under the pointer were "
                                       "%5,%6 %7x%8")
                            .arg(b.x()).arg(b.y()).arg(b.width())
                            .arg(b.height())
                            .arg(want.x()).arg(want.y()).arg(want.width())
                            .arg(want.height())));
}

void TstGestureNet::modifierComposedSelectionsAddAndSubtract()
{
    // §39.4: the effective op comes from the modifiers held at submit.
    // Driven with REAL key press/release so the global modifier state the
    // submit path reads is genuinely set (an event-only modifier would
    // not reach QGuiApplication::keyboardModifiers()).
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    QTest::mouseMove(m_window, docToView(200, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 200));
    QCOMPARE(engineSelectionBounds(m_controller), QRect(100, 100, 100, 100));

    // Shift = ADD: a disjoint second rect must UNION, not replace.
    m_canvas->forceActiveFocus();
    QTest::keyPress(m_window, Qt::Key_Shift);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::ShiftModifier,
                      docToView(300, 100));
    QTest::mouseMove(m_window, docToView(400, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::ShiftModifier,
                        docToView(400, 200));
    QTest::keyRelease(m_window, Qt::Key_Shift);
    const QRect added = engineSelectionBounds(m_controller);
    QVERIFY2(added == QRect(100, 100, 300, 100),
             qPrintable(QStringLiteral("shift-drag did not ADD to the "
                                       "selection (bounds %1,%2 %3x%4; a "
                                       "replace would read 300,100 100x100)")
                            .arg(added.x()).arg(added.y())
                            .arg(added.width()).arg(added.height())));

    // Alt = SUBTRACT: carve the second rect back out.
    QTest::keyPress(m_window, Qt::Key_Alt);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::AltModifier,
                      docToView(300, 100));
    QTest::mouseMove(m_window, docToView(400, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::AltModifier,
                        docToView(400, 200));
    QTest::keyRelease(m_window, Qt::Key_Alt);
    const QRect cut = engineSelectionBounds(m_controller);
    QVERIFY2(cut == QRect(100, 100, 100, 100),
             qPrintable(QStringLiteral("alt-drag did not SUBTRACT (bounds "
                                       "%1,%2 %3x%4; expected the first rect "
                                       "back)")
                            .arg(cut.x()).arg(cut.y())
                            .arg(cut.width()).arg(cut.height())));
}

void TstGestureNet::cropDragEmitsExactCropRect()
{
    // The crop tool commits nothing itself — its drag SETTLES into
    // cropRectDragged, which Main.qml turns into the pre-filled crop
    // dialog. Pin the signal's payload (the chrome half rides in
    // tst_gesture_chrome).
    QSignalSpy spy(m_canvas, &CanvasView::cropRectDragged);
    QVERIFY(spy.isValid());
    setTool("crop");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(140, 90));
    QTest::mouseMove(m_window, docToView(300, 200));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(420, 290));
    QCOMPARE(spy.count(), 1);
    const QList<QVariant> a = spy.takeFirst();
    QCOMPARE(a.at(0).toInt(), 140);
    QCOMPARE(a.at(1).toInt(), 90);
    QCOMPARE(a.at(2).toInt(), 280);
    QCOMPARE(a.at(3).toInt(), 200);

    // A degenerate crop click must settle NOTHING (no dialog on a stray
    // click).
    QSignalSpy spy2(m_canvas, &CanvasView::cropRectDragged);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(200, 150));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(200, 150));
    QCOMPARE(spy2.count(), 0);
}

// --------------------------------------------------- exploratory GUI run

void TstGestureNet::exploratoryRunNewDocToExport()
{
    // The tick's one end-to-end run, driven ENTIRELY by synthesized
    // mouse/keyboard events: new doc -> draw -> select -> fill -> text ->
    // shape -> export. Every checkpoint asserts ENGINE truth and grabs a
    // frame; frames land in $APPNAME_QA_FRAMES (a temp dir when unset) and
    // are diffed against the maintained references by the tick harness.
    const QString outDir = qEnvironmentVariable("APPNAME_QA_FRAMES");
    // STANDING RULE (operator, 2026-08-20): time the scripted sequence
    // end-to-end each tick and track the trend. Reported, never asserted
    // here — an absolute threshold in a test would fire on machine noise;
    // the tick compares against the previous tick's recorded number.
    QElapsedTimer latency;
    latency.start();
    // Per-leg breakdown: the v1.0.1 performance-head charter names
    // brush-stroke latency, pan/zoom, dialog open and canvas redraw as
    // the interactive probes. The legs this scripted sequence already
    // drives are timed individually so a regression names its own leg
    // instead of moving one opaque total.
    QElapsedTimer leg;
    QStringList legs;
    auto mark = [&](const char *name) {
        legs << QStringLiteral("%1=%2ms").arg(QLatin1String(name))
                    .arg(leg.restart());
    };
    leg.start();
    int shot = 0;
    auto checkpoint = [&](const char *name) {
        if (outDir.isEmpty())
            return;
        QDir().mkpath(outDir);
        const QImage frame = m_window->grabWindow();
        QVERIFY2(!frame.isNull(), "grabWindow returned null");
        const QString path = QStringLiteral("%1/%2-%3.png")
                                 .arg(outDir)
                                 .arg(shot++, 2, 10, QLatin1Char('0'))
                                 .arg(QLatin1String(name));
        QVERIFY2(frame.save(path), qPrintable(QStringLiteral("save failed: %1")
                                                  .arg(path)));
    };

    // 1. new doc (init() already made one: 640x400 white).
    QVERIFY(m_controller->hasDocument());
    QCOMPARE(m_controller->docWidth(), 640);
    mark("new-doc");
    checkpoint("new-doc");

    // 2. draw — a brush stroke by press/move/release.
    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(80, 120));
    QTest::mouseMove(m_window, docToView(200, 150));
    QTest::mouseMove(m_window, docToView(320, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(320, 120));
    QVERIFY2(!engineContentBounds(m_controller).isEmpty(),
             "exploratory: the brush leg painted nothing");
    mark("draw");
    checkpoint("draw");

    // 3. select — a rect marquee by drag.
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(360, 220));
    QTest::mouseMove(m_window, docToView(500, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(500, 300));
    QVERIFY2(m_controller->selectionActive(),
             "exploratory: the marquee leg committed no selection");
    mark("select");
    checkpoint("select");

    // 4. fill — through the command door the menu item fires, clipped to
    //    the gesture-made selection (engine truth: inside painted,
    //    outside untouched).
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.1, 0.5, 0.9, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().first().toMap().value(
                      QStringLiteral("layerId")));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok"))
                .toBool());
    QVERIFY2(!isWhitish(enginePixel(m_controller, 430, 260)),
             "exploratory: the fill did not reach inside the selection");
    QVERIFY2(isWhitish(enginePixel(m_controller, 560, 350)),
             "exploratory: the fill leaked OUTSIDE the selection");
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    mark("fill");
    checkpoint("fill");

    // 5. text — RC3 D-6: the Text tool refuses on use for 1.0, so a click
    // adds no layer (the refusal itself is pinned in textClickIsRefused…).
    setTool("text");
    const int beforeText = m_controller->layers().size();
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(120, 320));
    QCOMPARE(m_controller->layers().size(), beforeText);
    mark("text");
    checkpoint("text");

    // 6. shape — an ellipse by drag.
    setTool("shape.ellipse");
    const int beforeShape = m_controller->layers().size();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(400, 60));
    QTest::mouseMove(m_window, docToView(560, 180));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(560, 180));
    QCOMPARE(m_controller->layers().size(), beforeShape + 1);
    mark("shape");
    checkpoint("shape");

    // 7. export — the composed document out through the engine door, then
    //    read back: a non-trivial file that re-imports.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString png = tmp.filePath(QStringLiteral("exploratory.png"));
    QVERIFY2(m_controller->exportDocument(png, QStringLiteral("png")),
             qPrintable(QStringLiteral("exploratory: export refused; status: %1")
                            .arg(m_controller->statusText())));
    QFileInfo fi(png);
    QVERIFY2(fi.exists() && fi.size() > 1024,
             qPrintable(QStringLiteral("exploratory: export wrote %1 bytes")
                            .arg(fi.size())));
    QImage back;
    QVERIFY2(back.load(png), "exploratory: the exported PNG did not re-load");
    QCOMPARE(back.size(), QSize(640, 400));
    mark("export");
    checkpoint("export");

    // 8. transform preview (v1.0.1 item 21, T39) — APPENDED, never inserted:
    //    checkpoint() numbers frames sequentially, so a new leg in the middle
    //    would renumber 00-06 and break every reference hash for no reason.
    //    This is the tick's only LIVE-DRAG frame: the item-19 frame and item
    //    21's preview exist only WHILE the button is down, so a committed
    //    end-state frame cannot see either. Left mid-drag on purpose.
    fillBlockOnNewLayer(QRect(400, 300, 120, 80), 0.1, 0.6, 0.3);
    setTool("transform.free");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(520, 380));
    QTest::mouseMove(m_window, docToView(470, 345));
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "exploratory: the transform frame is not live, so the preview "
             "frame below would be an ordinary canvas grab wearing its name");
    mark("transform-preview");
    checkpoint("transform-preview");
    // Leave no live session behind for the next case in the run.
    QTest::keyClick(m_window, Qt::Key_Escape);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(470, 345));

    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY scripted sequence end-to-end: %1 ms")
               .arg(latency.elapsed());
    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY legs: %1")
               .arg(legs.join(QStringLiteral(" ")));
}

void TstGestureNet::largeCanvasGestureLatency()
{
    // INTERACTIVE-LATENCY charter (v1.0.1 performance head), sized to
    // where the adjudicated shell findings actually bite.
    //
    // The 640x400 probe above cannot see them: O-UI's sweep put F4 (the
    // announcer's full-mask scan per selection commit) at ~140 ms STACKED
    // AT 2048², and F1/F2 at a full-canvas readSelectionImage +
    // revectorize inside paint() re-triggered by every history event.
    // Those costs scale with CANVAS AREA, and a 0.26 MP document hides
    // them — my select leg reads 2 ms there. This case runs the same
    // gesture sequence on a 2048x2048 (4.2 MP) document so the desk has a
    // gesture-level number at the size the findings describe, and so the
    // F1/F2/F4 slice has a before/after instrument that measures a USER
    // SEQUENCE rather than a door.
    //
    // Point probes only — a full-canvas content scan would itself cost
    // more than the thing being measured at 4.2 MP.
    QVERIFY(m_controller->newDocument(2048, 2048, QStringLiteral("white")));
    m_canvas->zoomFit();
    const double zoom = m_canvas->property("zoom").toDouble();
    QVERIFY2(zoom > 0.0 && zoom < 1.0, "zoomFit did not shrink a 2048² doc");
    // zoomFit centres the document, so the offset is derivable.
    const double ox = (m_canvas->width() - 2048.0 * zoom) / 2.0;
    const double oy = (m_canvas->height() - 2048.0 * zoom) / 2.0;
    auto toView = [&](double x, double y) {
        return QPointF(x * zoom + ox, y * zoom + oy).toPoint();
    };
    m_canvas->forceActiveFocus();

    QElapsedTimer leg;
    QStringList legs;
    auto mark = [&](const char *name) {
        legs << QStringLiteral("%1=%2ms").arg(QLatin1String(name))
                    .arg(leg.restart());
    };
    QElapsedTimer total;
    total.start();
    leg.start();

    // draw — brush-stroke latency, the charter's headline probe
    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(200, 600));
    QTest::mouseMove(m_window, toView(700, 620));
    QTest::mouseMove(m_window, toView(1200, 600));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1200, 600));
    QVERIFY2(!isWhitish(enginePixel(m_controller, 700, 620)),
             "large-canvas brush leg painted nothing");
    mark("draw");

    // select — the ants/readSelectionImage path F1/F2 name
    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(300, 1200));
    QTest::mouseMove(m_window, toView(900, 1500));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1400, 1800));
    QVERIFY2(m_controller->selectionActive(),
             "large-canvas marquee committed no selection");
    mark("select");

    // fill inside the selection — a history event, which is what
    // re-triggers the F1/F2 revectorize
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.1, 0.5, 0.9, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().first().toMap().value(
                      QStringLiteral("layerId")));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok")).toBool());
    QVERIFY2(!isWhitish(enginePixel(m_controller, 800, 1400)),
             "large-canvas fill did not reach inside the selection");
    mark("fill");

    // a second marquee ON TOP of a live selection — the stacked case F4
    // describes (announcer re-scans the full mask per selection commit)
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(500, 300));
    QTest::mouseMove(m_window, toView(1100, 700));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1600, 1000));
    QVERIFY(m_controller->selectionActive());
    mark("reselect");

    // move drag — F3's move-press full-doc readLayer stall
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    setTool("move");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(700, 620));
    QTest::mouseMove(m_window, toView(900, 800));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1000, 900));
    mark("move");

    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY-2048 end-to-end: %1 ms")
               .arg(total.elapsed());
    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY-2048 legs: %1")
               .arg(legs.join(QStringLiteral(" ")));
}

void TstGestureNet::maskedLargeCanvasGestureLatency()
{
    // T8 identified this gap: the mask-readback item (routed to O-RENDER,
    // 4x/572ms; the §4.7 occlusion cull merged @ 8b24e2c claims 33x on the
    // occluder case) had NO user-visible instrument. My 2048² probe showed
    // no movement from that merge for a good reason — its document is a
    // single UNMASKED layer, so an occlusion cull has nothing to cull.
    //
    // This is the same 2048² sequence with a real raster MASK on the
    // active layer, so the masked-readback path is exercised as a USER
    // SEQUENCE rather than a door. Reported, not asserted: it is a
    // measurement instrument for the perf train, and an absolute
    // threshold here would fire on machine state.
    QVERIFY(m_controller->newDocument(2048, 2048, QStringLiteral("white")));
    m_canvas->zoomFit();
    const double zoom = m_canvas->property("zoom").toDouble();
    QVERIFY(zoom > 0.0 && zoom < 1.0);
    const double ox = (m_canvas->width() - 2048.0 * zoom) / 2.0;
    const double oy = (m_canvas->height() - 2048.0 * zoom) / 2.0;
    auto toView = [&](double x, double y) {
        return QPointF(x * zoom + ox, y * zoom + oy).toPoint();
    };
    m_canvas->forceActiveFocus();

    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers().first().toMap().value(
                      QStringLiteral("layerId")));
    QVariantMap maskParams;
    maskParams.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    maskParams.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    const QVariantMap mr = m_controller->applyCommand(
        QStringLiteral("mask.add"), target, maskParams, {});
    // Non-vacuity (rule 9): if the mask never attached, this case is just
    // the unmasked probe wearing a different name — say so loudly rather
    // than reporting a masked number that is not masked.
    QVERIFY2(mr.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("mask.add refused, so this probe is "
                                       "NOT measuring a masked document; "
                                       "status: %1")
                            .arg(m_controller->statusText())));

    QElapsedTimer leg, total;
    QStringList legs;
    auto mark = [&](const char *name) {
        legs << QStringLiteral("%1=%2ms").arg(QLatin1String(name))
                    .arg(leg.restart());
    };
    total.start();
    leg.start();

    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(200, 600));
    QTest::mouseMove(m_window, toView(700, 620));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1200, 600));
    mark("draw");

    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(300, 1200));
    QTest::mouseMove(m_window, toView(900, 1500));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1400, 1800));
    QVERIFY(m_controller->selectionActive());
    mark("select");

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(500, 300));
    QTest::mouseMove(m_window, toView(1100, 700));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1600, 1000));
    mark("reselect");

    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    setTool("move");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(700, 620));
    QTest::mouseMove(m_window, toView(900, 800));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1000, 900));
    mark("move");

    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY-2048-MASKED end-to-end: %1 ms")
               .arg(total.elapsed());
    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY-2048-MASKED legs: %1")
               .arg(legs.join(QStringLiteral(" ")));
}

void TstGestureNet::toolOptionReachesTheEngineThroughTheGesture()
{
    // 11d Phase-1 (merged this wave): the DECLARATIVE OPTIONS MODEL adds
    // per-tool option properties to CanvasView (textFontSize,
    // puppetExpansion, puppetRigidity, puppetShowMesh) which the options
    // bar drives. submitText folds textFontSize into a styledRunStyle and
    // the envelope builder carries it verbatim.
    //
    // The defect class this guards is the options bar's own: an option
    // wired to the WIDGET and to the shell property, but never reaching
    // the ENGINE — the control moves, the user sees it move, and the
    // committed document ignores it. That is invisible to a property
    // test and invisible to an engine test; only a gesture that sets the
    // option and then commits can see it.
    //
    // Engine truth without a style readback: text.add commits EMPTY text
    // (contract default) so nothing renders and no pixel probe can work,
    // and layers() does not expose text style. So the document itself is
    // the witness — saved through the engine's own serializer with
    // deterministic ids, and compared byte-wise.
    //
    // The DETERMINISM CONTROL is what makes that legitimate: the same
    // gesture at the same size twice must produce IDENTICAL bytes. Without
    // it, a package embedding a timestamp or a random id would differ
    // every run and the "size changed the document" arm would pass
    // vacuously forever.
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    QStringList armedPresses; // bare canvas: the refusal is the armed signal
    auto runAtSize = [&](double sizePt, const QString &name) -> QByteArray {
        m_controller->setDeterministicIds(0xB0FFEEULL); // same seed each run
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return QByteArray();
        m_canvas->zoomFit();
        m_canvas->zoomOneToOne();
        m_canvas->forceActiveFocus();
        m_canvas->setProperty("textFontSize", sizePt);
        setTool("text");
        QSignalSpy armed(m_canvas, SIGNAL(armedToolPressed(QString)));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(180, 220));
        for (const QList<QVariant> &args : armed)
            armedPresses << args.at(0).toString();
        const QString path = tmp.filePath(name);
        if (!m_controller->saveDocument(path))
            return QByteArray();
        QFile f(path);
        if (!f.open(QIODevice::ReadOnly))
            return QByteArray();
        return f.readAll();
    };

    const QByteArray a = runAtSize(12.0, QStringLiteral("a.appname"));
    const QByteArray aAgain = runAtSize(12.0, QStringLiteral("a2.appname"));
    const QByteArray big = runAtSize(48.0, QStringLiteral("b.appname"));
    QVERIFY2(!a.isEmpty() && !aAgain.isEmpty() && !big.isEmpty(),
             qPrintable(QStringLiteral("a run produced no package — the "
                                       "instrument cannot see anything; "
                                       "status: %1")
                            .arg(m_controller->statusText())));

    // CONTROL first: identical inputs must give identical bytes, or the
    // comparison below means nothing.
    QVERIFY2(a == aAgain,
             qPrintable(QStringLiteral("the same text gesture at the same "
                                       "size produced DIFFERENT packages "
                                       "(%1 vs %2 bytes) — the package is "
                                       "not deterministic under a fixed id "
                                       "seed, so a byte difference cannot be "
                                       "attributed to the option")
                            .arg(a.size()).arg(aAgain.size())));
    // RC3 D-6 (operator WORD 35, sitting S-15): the Text tool REFUSES on
    // use for 1.0, so a click at ANY size commits nothing -- the 48 pt
    // document must now be BYTE-IDENTICAL to the 12 pt one (the control
    // above is what makes that identity meaningful), and the refusal must
    // be SURFACED, never a silent no-op. The option-reaches-engine guard
    // returns with the text-editing slice that re-wires the gesture.
    QVERIFY2(big == a,
             qPrintable(QStringLiteral("a text click at %1 pt CHANGED the "
                                       "document (%2 vs %3 bytes) — the "
                                       "Text tool still commits text.add")
                            .arg(48.0).arg(big.size()).arg(a.size())));
    // This harness hosts a bare CanvasView (no Main.qml), so the refusal is
    // pinned at the canvas: each click emits armedToolPressed("text") --
    // the signal Main.qml surfaces as "Text is not wired to the canvas yet"
    // (pinned end to end in tst_options_bar fontSizeRidesStyleSizeEngineProven).
    QCOMPARE(armedPresses,
             QStringList({QStringLiteral("text"), QStringLiteral("text"),
                          QStringLiteral("text")}));
}

void TstGestureNet::maskedLayerMoveRefusesTyped()
{
    // Q-3 CLASS-KILL (merged this wave, 73c985f/895c5da): the declared
    // §11.2 masked-layer refusals are now enforced on layer.transform and
    // the interactive transform doors. The MOVE tool submits
    // layer.transform, so dragging a MASKED layer is now a refusal path —
    // a behaviour change on a gesture surface I own, arriving without a
    // gesture-level pin.
    //
    // The pin is the honest-refusal shape, not the outcome: the drag must
    // reach the door and come back TYPED AND SURFACED, never a silent
    // no-op, and it must not move pixels or spend a history entry. When
    // the mask-behind arm is eventually built this case is what says the
    // shell half already worked.
    //
    // (This is also why the masked latency probe's move leg collapsed
    // 5973 -> 861 ms this tick: the door now refuses early. That is a
    // short-circuit, NOT a perf win — recorded so the number is not
    // misread as one.)
    fillBlockOnNewLayer(QRect(200, 140, 240, 120), 0.1, 0.2, 0.8);
    const QRect before = engineContentBounds(m_controller);
    QVERIFY(!before.isEmpty());
    QVariantMap target;
    target.insert(QStringLiteral("layerId"),
                  m_controller->layers()
                      .at(m_controller->activeLayerIndex()).toMap()
                      .value(QStringLiteral("layerId")));
    QVariantMap maskParams;
    maskParams.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    maskParams.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("mask.add"), target,
                                maskParams, {})
                 .value(QStringLiteral("ok")).toBool(),
             "setup: mask.add refused, so this is not a masked-layer probe");
    const int historyBefore = m_controller->historyEntries().size();

    setTool("move");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(300, 200));
    QTest::mouseMove(m_window, docToView(360, 240));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(420, 280));
    const QString status = m_controller->statusText();
    qInfo().noquote()
        << QStringLiteral("MASKED-MOVE: status=%1").arg(status);
    QVERIFY2(status.contains(QLatin1String("reject"))
                 || status.contains(QLatin1String("not_implemented")),
             qPrintable(QStringLiteral("a move drag on a MASKED layer "
                                       "neither moved nor surfaced a typed "
                                       "refusal — a silent no-op; status: %1")
                            .arg(status)));
    QCOMPARE(engineContentBounds(m_controller), before);
    QCOMPARE(m_controller->historyEntries().size(), historyBefore);
}

void TstGestureNet::largeCanvasRepaintSensitiveLatency()
{
    // WHY THIS EXISTS. largeCanvasGestureLatency takes NO frames — it uses
    // point probes only, deliberately, so the measurement is not dominated
    // by grab cost. That makes it structurally BLIND to repaint work: with
    // nothing asking the scene graph for a frame, CanvasView::paint() need
    // never run, so a change to what paint() does cannot move its number.
    //
    // The repaint-scoping slice (F1/F2/F4 — the full-canvas
    // readSelectionImage + revectorize inside paint(), and the
    // infoChanged->update() full-viewport repaint) is exactly that class of
    // change. Measuring it needs a probe that FORCES a frame per gesture.
    //
    // Same 2048² sequence, one grabWindow() per leg. Kept SEPARATE from the
    // no-frame probe so the T7..T20 series stays comparable rather than
    // being silently redefined.
    QVERIFY(m_controller->newDocument(2048, 2048, QStringLiteral("white")));
    m_canvas->zoomFit();
    const double zoom = m_canvas->property("zoom").toDouble();
    QVERIFY(zoom > 0.0 && zoom < 1.0);
    const double ox = (m_canvas->width() - 2048.0 * zoom) / 2.0;
    const double oy = (m_canvas->height() - 2048.0 * zoom) / 2.0;
    auto toView = [&](double x, double y) {
        return QPointF(x * zoom + ox, y * zoom + oy).toPoint();
    };
    m_canvas->forceActiveFocus();

    QElapsedTimer leg, total;
    QStringList legs;
    int frames = 0;
    auto frame = [&]() {
        const QImage f = m_window->grabWindow();
        if (!f.isNull())
            ++frames;
    };
    auto mark = [&](const char *name) {
        legs << QStringLiteral("%1=%2ms").arg(QLatin1String(name))
                    .arg(leg.restart());
    };
    total.start();
    leg.start();

    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(200, 600));
    QTest::mouseMove(m_window, toView(700, 620));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1200, 600));
    frame();
    mark("draw");

    setTool("select.rect");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(300, 1200));
    QTest::mouseMove(m_window, toView(900, 1500));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1400, 1800));
    QVERIFY(m_controller->selectionActive());
    frame();
    mark("select");

    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(500, 300));
    QTest::mouseMove(m_window, toView(1100, 700));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1600, 1000));
    frame();
    mark("reselect");

    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    setTool("move");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(700, 620));
    QTest::mouseMove(m_window, toView(900, 800));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(1000, 900));
    frame();
    mark("move");

    // Non-vacuity: if no frame was actually produced, this probe is the
    // blind one it was written to replace, and must say so.
    QVERIFY2(frames == 4,
             qPrintable(QStringLiteral("only %1 of 4 frames were grabbed — "
                                       "this probe is not repaint-sensitive "
                                       "and its number means nothing")
                            .arg(frames)));
    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY-2048-REPAINT end-to-end: %1 ms "
                          "(%2 frames)").arg(total.elapsed()).arg(frames);
    qInfo().noquote()
        << QStringLiteral("GESTURE-LATENCY-2048-REPAINT legs: %1")
               .arg(legs.join(QStringLiteral(" ")));
}

void TstGestureNet::shapeCommitRepaintsTheWholeCanvas()
{
    // FINDING G-5 (MAJOR, REGRESSION) — reds until the repaint scoping is
    // corrected. Committing a shape layer leaves the canvas rendering the
    // TRANSPARENCY CHECKERBOARD over regions the document still holds
    // opaque: the user draws a rectangle and their image appears to
    // vanish. The document is intact (engine truth below); only the
    // presentation is wrong, and it self-heals on any full repaint.
    //
    // Caught by the exploratory reference frames — 05-shape and 06-export
    // both changed, and both stayed changed, so the corruption survives a
    // later checkpoint rather than being a one-frame blip.
    setTool("brush.stroke");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(80, 120));
    QTest::mouseMove(m_window, docToView(200, 150));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(320, 120));
    setTool("shape.ellipse");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(400, 60));
    QTest::mouseMove(m_window, docToView(500, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(560, 180));

    // ENGINE truth: a spot far from the shape is still opaque white.
    const QColor engineBg = enginePixel(m_controller, 60, 350);
    QVERIFY2(engineBg.alpha() >= 250 && isWhitish(engineBg),
             qPrintable(QStringLiteral("setup: the DOCUMENT lost its "
                                       "background (%1, alpha %2) — this is "
                                       "no longer a presentation-only "
                                       "problem")
                            .arg(engineBg.name()).arg(engineBg.alpha())));

    // PRESENTATION truth at the SAME point: the canvas must render what
    // the document holds, not the transparency checkerboard.
    const QImage frame = m_window->grabWindow();
    QVERIFY2(!frame.isNull(), "grabWindow returned null");
    const QColor viewBg = frame.pixelColor(docToView(60, 350));
    QVERIFY2(isWhitish(viewBg),
             qPrintable(QStringLiteral("G-5: after a shape commit the canvas "
                                       "renders %1 where the document holds "
                                       "opaque white — the repaint scoping "
                                       "left the rest of the canvas showing "
                                       "the transparency checkerboard")
                            .arg(viewBg.name())));
}

void TstGestureNet::wandOnTransparentIsAlphaAware()
{
    // OPERATOR RULING 2026-08-20 (@ c0637cb), item-20 design flag:
    // "transparency is a SAMPLEABLE VALUE — clicking fully-transparent
    // selects the contiguous transparent region (alpha-aware tolerance),
    // same as any color... So the wand metric is ALPHA-AWARE (not
    // alpha-blind); the 'no pixels here' notice is the narrow
    // no-pixels-AND-no-transparency edge."
    //
    // Routed to O-UI + the sample-all-layers/wand arm builder — i.e. this
    // is RULED BUT NOT YET BUILT. A plain assertion would red the suite
    // for scheduled work, which is not a finding, so this probe
    // SELF-ACTIVATES the same way armedControlHonestyProbe does: it
    // records today's behaviour and becomes a real pin the moment the arm
    // lands.
    //
    // Setup: the wand samples the ACTIVE layer (§13.3) and refuses
    // typed on MULTI-layer documents today ("headless carries no
    // active-layer state... single-raster-layer documents are live") —
    // observed first-hand this tick. So the probe must stay
    // SINGLE-LAYER: a transparent-background document with one opaque
    // island painted on it, wand-clicked in the transparent part.
    QVERIFY(m_controller->newDocument(640, 400,
                                      QStringLiteral("transparent")));
    m_canvas->zoomFit();
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);
    m_canvas->forceActiveFocus();
    fillBlock(QRect(400, 250, 200, 120), 0.9, 0.2, 0.1); // opaque island
    QVERIFY2(enginePixel(m_controller, 500, 300).alpha() >= 250,
             "setup: the opaque island did not paint");
    QVERIFY2(enginePixel(m_controller, 100, 100).alpha() == 0,
             "setup: the background is not transparent, so this probe "
             "would not be sampling transparency at all");

    setTool("select.wand");
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100)); // deep in the transparent region
    const bool active = m_controller->selectionActive();
    const QRect b = active ? engineSelectionBounds(m_controller) : QRect();
    qInfo().noquote()
        << QStringLiteral("WAND-ON-TRANSPARENT: selectionActive=%1 bounds=%2,%3 "
                          "%4x%5 status=%6")
               .arg(active ? QStringLiteral("true") : QStringLiteral("false"))
               .arg(b.x()).arg(b.y()).arg(b.width()).arg(b.height())
               .arg(m_controller->statusText());
    // VACUITY TRIPWIRE (T32): this self-activating branch was correct
    // while the arm was unbuilt. It has been LIVE since T9, so selecting
    // nothing is now a REGRESSION of ruled behaviour — and returning here
    // would let that regression pass as a green.
    QVERIFY2(active,
             qPrintable(QStringLiteral("the wand selected NOTHING on "
                                       "transparency — the alpha-aware arm "
                                       "was live from T9 and has regressed; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
    // The arm is live: the ruled behaviour is a selection of the
    // CONTIGUOUS TRANSPARENT region — which must NOT include the opaque
    // island, and must cover the clicked corner.
    QImage px;
    QVERIFY(m_controller->readSelectionImage(QRect(100, 100, 1, 1), &px));
    QVERIFY2(px.constScanLine(0)[0] != 0,
             "the wand selected a region that excludes the clicked "
             "transparent point");
    QVERIFY(m_controller->readSelectionImage(QRect(500, 300, 1, 1), &px));
    QVERIFY2(px.constScanLine(0)[0] == 0,
             "the transparent-region selection swallowed the OPAQUE island "
             "— the wand metric is not alpha-aware, it is alpha-blind");
    qInfo().noquote()
        << "WAND-ON-TRANSPARENT: alpha-aware behaviour is LIVE on "
           "single-layer documents (ruled @ c0637cb) — pinned.";

    // The KNOWN gap the ruling routes: the wand samples the ACTIVE layer
    // and pends on multi-layer documents. That arm must refuse TYPED and
    // SURFACE it — never fall silent, and never quietly select something
    // from the wrong layer. Flips to the live assertion when the
    // sample-all-layers arm lands (operator priority P4).
    QVariantMap addParams;
    addParams.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    addParams.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, addParams, {})
                .value(QStringLiteral("ok")).toBool());
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(100, 100));
    const QString multiStatus = m_controller->statusText();
    qInfo().noquote()
        << QStringLiteral("WAND-MULTI-LAYER: active=%1 status=%2")
               .arg(m_controller->selectionActive() ? QStringLiteral("true")
                                                    : QStringLiteral("false"))
               .arg(multiStatus);
    if (!m_controller->selectionActive()) {
        QVERIFY2(multiStatus.contains(QLatin1String("reject"))
                     || multiStatus.contains(QLatin1String("not_implemented")),
                 qPrintable(QStringLiteral("the wand selected nothing on a "
                                           "multi-layer document AND surfaced "
                                           "no typed refusal — a silent "
                                           "no-op; status: %1")
                                .arg(multiStatus)));
    }
}


// ===================================================================
// T39 PER-FEATURE COVERAGE: v1.0.1 items 19/21/13, landed in O-UI's lane
// (e3fbd765). The free-transform frame stopped being canvas-shaped; four
// arms of the new entry path are user-reachable and none had a gesture.
// ===================================================================

void TstGestureNet::transformFrameFramesTheLayerNotTheCanvas()
{
    // ITEM 19's CONTRACT, PINNED FROM BOTH SIDES. My two transform cases
    // failed this tick because they aimed at the CANVAS corner; that made
    // the old behaviour's grave the natural place for a regression guard.
    fillBlockOnNewLayer(QRect(400, 300, 120, 80), 0.8, 0.1, 0.1);
    const QRect before = engineContentBounds(m_controller);
    QCOMPARE(before, QRect(400, 300, 120, 80));

    // NEGATIVE HALF: the canvas BR (640,400) is 156px from the layer's
    // nearest handle. Under the canvas-shaped frame this was a live scale
    // handle; under item 19 it is empty space and must do NOTHING.
    setTool("transform.free");
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(560, 350));
    QTest::mouseMove(m_window, docToView(480, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(480, 300));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect afterCanvasGrab = engineContentBounds(m_controller);
    QVERIFY2(afterCanvasGrab == before,
             qPrintable(QStringLiteral("a drag from the CANVAS corner moved "
                                       "the layer (%1,%2 %3x%4 -> %5,%6 "
                                       "%7x%8): the frame is canvas-shaped "
                                       "again, item 19 regressed")
                            .arg(before.x()).arg(before.y())
                            .arg(before.width()).arg(before.height())
                            .arg(afterCanvasGrab.x()).arg(afterCanvasGrab.y())
                            .arg(afterCanvasGrab.width())
                            .arg(afterCanvasGrab.height())));

    // POSITIVE HALF / LIVENESS FLOOR: "nothing happened" is also what a
    // transform tool that never works produces. The layer's OWN handle must
    // still move pixels, or the negative half above proves nothing.
    setTool("brush.stroke");
    setTool("transform.free"); // re-enter the frame after the committed nudge
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(520, 380));
    QTest::mouseMove(m_window, docToView(490, 360));
    QTest::mouseMove(m_window, docToView(460, 340));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(460, 340));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    const QRect afterLayerGrab = engineContentBounds(m_controller);
    QVERIFY2(afterLayerGrab != before,
             qPrintable(QStringLiteral("LIVENESS FLOOR FAILED: the LAYER's "
                                       "own BR handle committed nothing "
                                       "either, so this case cannot tell an "
                                       "item-19 frame from a dead tool; "
                                       "status: %1")
                            .arg(m_controller->statusText())));
}

void TstGestureNet::transformOnEmptyLayerOpensNoSessionAndSaysSo()
{
    // ITEM 19's EMPTY arm: no session, and - diverging from G-3's mute
    // ruling on purpose - it SPEAKS. An action-shaped control that silently
    // does nothing is exactly the ARMED-SILENT class the census rule makes
    // a MAJOR, so the notice is the feature here, not decoration.
    fillBlock(QRect(100, 100, 60, 60), 0.2, 0.7, 0.2); // something to lose
    const QRect before = engineContentBounds(m_controller);
    QVERIFY(!before.isEmpty());
    QVariantMap params;
    params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY2(m_controller
                 ->applyCommand(QStringLiteral("layer.add"), {}, params, {})
                 .value(QStringLiteral("ok"))
                 .toBool(),
             "layer.add refused - no empty layer, so this case is not "
             "measuring the empty arm");
    m_controller->setActiveLayerIndex(m_controller->layers().size() - 1);
    m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    const QString statusBefore = m_controller->statusText();

    setTool("transform.free");
    const QString statusAfter = m_controller->statusText();
    qInfo() << "empty-layer transform status:" << statusAfter;
    QVERIFY2(statusAfter != statusBefore && !statusAfter.isEmpty(),
             qPrintable(QStringLiteral("ARMED-SILENT: Cmd+T on an empty "
                                       "layer left the status line unchanged "
                                       "at '%1'")
                            .arg(statusAfter)));
    QVERIFY2(statusAfter.contains(QStringLiteral("SENTINEL-EMPTY")),
             qPrintable(QStringLiteral("the empty-layer arm spoke, but not "
                                       "with its own cataloged notice: '%1'")
                            .arg(statusAfter)));

    // NO SESSION: a drag anywhere, including where the canvas-shaped frame
    // used to put handles, must not transform the layer BELOW the empty one.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(500, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(500, 300));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    QCOMPARE(engineContentBounds(m_controller), before);
}

void TstGestureNet::transformCornerHandleAspectLocksAndShiftUnlocks()
{
    // ITEM 13 INVERTED THE MODIFIER: corner handles are aspect-locked by
    // DEFAULT and Shift UNLOCKS free scaling - the reverse of the old
    // hold-for-uniform. An inverted modifier is the quietest kind of
    // regression, because both polarities "work" and only the shape is
    // wrong, so this case discriminates on the RESULT, not on the status.
    auto runScale = [&](Qt::KeyboardModifiers mods, QRect *startOut) -> QRect {
        // Reset through the SAME door init() uses. (An applyCommand
        // "document.new" here silently did nothing — the second arm then
        // measured the first arm's leftovers and reported "no content",
        // which named the symptom and hid the cause.)
        if (!m_controller->newDocument(640, 400, QStringLiteral("white")))
            return QRect();
        m_canvas->zoomOneToOne();
        fillBlockOnNewLayer(QRect(100, 100, 120, 80), 0.9, 0.4, 0.1);
        const QRect start = engineContentBounds(m_controller);
        *startOut = start;
        if (start != QRect(100, 100, 120, 80))
            return QRect(); // caller reports it; every coordinate below
                            // is derived from these exact bounds
        setTool("transform.free");
        // BR handle (220,180) dragged to (280,180): sx=1.5 on x, sy=1.0 on
        // y. LOCKED that is a uniform 1.5 (dominant magnitude); UNLOCKED it
        // stretches x alone. The two land at different HEIGHTS.
        QTest::mousePress(m_window, Qt::LeftButton, mods, docToView(220, 180));
        moveWithModifiers(m_window, docToView(250, 180), mods);
        moveWithModifiers(m_window, docToView(280, 180), mods);
        QTest::mouseRelease(m_window, Qt::LeftButton, mods,
                            docToView(280, 180));
        m_canvas->forceActiveFocus();
        QTest::keyClick(m_window, Qt::Key_Return);
        return engineContentBounds(m_controller);
    };

    QRect lockedStart, unlockedStart;
    const QRect locked = runScale(Qt::NoModifier, &lockedStart);
    QVERIFY2(!locked.isEmpty(),
             qPrintable(QStringLiteral("the aspect-locked arm never ran: the "
                                       "block landed at %1,%2 %3x%4, not "
                                       "100,100 120x80")
                            .arg(lockedStart.x()).arg(lockedStart.y())
                            .arg(lockedStart.width())
                            .arg(lockedStart.height())));
    const QRect unlocked = runScale(Qt::ShiftModifier, &unlockedStart);
    QVERIFY2(!unlocked.isEmpty(),
             qPrintable(QStringLiteral("the Shift arm never ran: the block "
                                       "landed at %1,%2 %3x%4, not "
                                       "100,100 120x80")
                            .arg(unlockedStart.x()).arg(unlockedStart.y())
                            .arg(unlockedStart.width())
                            .arg(unlockedStart.height())));
    qInfo() << "aspect lock: default" << locked << "shift" << unlocked;

    // Default = uniform 1.5 about the TL anchor: 120x80 -> 180x120.
    QVERIFY2(qAbs(locked.height() - 120) <= 2 && qAbs(locked.width() - 180) <= 2,
             qPrintable(QStringLiteral("corner drag WITHOUT Shift gave %1x%2; "
                                       "aspect-locked 1.5 is 180x120. If this "
                                       "is 180x80 the modifier is inverted "
                                       "back to hold-for-uniform")
                            .arg(locked.width()).arg(locked.height())));
    // Shift = free: x alone stretches, height unchanged at 80.
    QVERIFY2(qAbs(unlocked.height() - 80) <= 2 && qAbs(unlocked.width() - 180) <= 2,
             qPrintable(QStringLiteral("corner drag WITH Shift gave %1x%2; "
                                       "unlocked free scale is 180x80")
                            .arg(unlocked.width()).arg(unlocked.height())));
    QVERIFY2(locked.height() != unlocked.height(),
             "Shift changed nothing - the unlock modifier is not wired");
}

void TstGestureNet::transformOnMaskedLayerFramesCanvasAndSaysSo()
{
    // ITEM 19's MASKED arm (F3b). The per-layer readback door refuses typed
    // on a masked layer, so there are no bounds: the frame falls back to
    // CANVAS bounds - which is the exact shape G-3 killed - and the ONLY
    // thing separating this from that defect is that it is LABELLED. So the
    // label is the assertion.
    fillBlockOnNewLayer(QRect(400, 300, 120, 80), 0.8, 0.1, 0.1);
    const QRect before = engineContentBounds(m_controller);
    QCOMPARE(before, QRect(400, 300, 120, 80));
    const QString activeId =
        m_controller->layers()
            .at(m_controller->activeLayerIndex())
            .toMap()
            .value(QStringLiteral("layerId"))
            .toString();
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), activeId);
    QVariantMap maskParams;
    maskParams.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    maskParams.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
    const QVariantMap mr = m_controller->applyCommand(
        QStringLiteral("mask.add"), target, maskParams, {});
    // NON-VACUITY: without the mask this is the ordinary item-19 path
    // wearing a masked name, and it would then FAIL for the wrong reason.
    QVERIFY2(mr.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("mask.add refused, so this case is "
                                       "NOT measuring the masked arm: %1")
                            .arg(mr.value(QStringLiteral("code")).toString())));
    const QString statusBefore = m_controller->statusText();

    setTool("transform.free");
    const QString statusAfter = m_controller->statusText();
    qInfo() << "masked-layer transform status:" << statusAfter;
    QVERIFY2(statusAfter != statusBefore && !statusAfter.isEmpty(),
             "the masked fallback opened a canvas-sized box in SILENCE - "
             "that is G-3's defect with a mask in front of it");
    QVERIFY2(statusAfter.contains(QStringLiteral("SENTINEL-MASKED")),
             qPrintable(QStringLiteral("the masked arm spoke, but not with "
                                       "the notice that names the BOX the "
                                       "user is looking at: '%1'")
                            .arg(statusAfter)));

    // THE SESSION IS LIVE, deliberately (item 19 refuses to wall at entry).
    // The canvas BR is therefore a real handle here - the one place in the
    // suite where that is the CORRECT answer, and the reason this case
    // cannot be folded into the frames-the-layer one.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(640, 400));
    QTest::mouseMove(m_window, docToView(560, 350));
    QTest::mouseMove(m_window, docToView(480, 300));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(480, 300));
    m_canvas->forceActiveFocus();
    QTest::keyClick(m_window, Qt::Key_Return);
    // The ENGINE refuses masked free transform at commit
    // (masked_transform_refusal), so the pixels must be untouched. This is
    // an engine-truth check on a refusal, not a shell echo.
    QVERIFY2(engineContentBounds(m_controller) == before,
             "a masked layer was transformed - the engine's "
             "masked_transform_refusal did not hold");
    QVERIFY2(m_canvas->property("transformActive").toBool()
                 || m_canvas->property("transformWallActive").toBool(),
             "the refusal threw the frame away; item 13 keeps the session "
             "LIVE so the user's aim is not lost to a wall");
}


void TstGestureNet::transformPreviewLeavesNoTrailWhenTheMatrixChanges()
{
    // OQ-5 / C-338 (landed T45): the live transform preview stopped calling
    // a bare update() per drag frame and now invalidates only
    // transformPreviewInvalidateRect(before, after) — 61x less repaint.
    //
    // THE HAZARD ITS OWN COMMENT NAMES: "a rect covering only the new
    // position leaves a trail -- a wrong invalidation rect is a VISIBLE lie
    // about where the layer is, and it is worse than a slow repaint."
    // Nothing asserted that. Reference frame 07 captures ONE mid-drag state,
    // so it cannot see a trail accumulated across several moves. This is
    // RENDER truth — the preview is shell-side only and no readRegionImage
    // will ever show it — so this case reads PIXELS OUT OF THE WINDOW.
    //
    // ★ WHY THE SAMPLE POINT IS OUTSIDE THE LAYER, learned by getting it
    //   wrong first. The obvious probe — shrink the preview and sample a
    //   point it vacated — reported a TRAIL on the very first run. It was
    //   the ORIGIN GHOST: item 19/21 leave the base composite UNMOVED
    //   beneath the preview (recorded T39 as a deliberate limit awaiting
    //   the exclude-layer readback ABI), so every point inside the layer's
    //   own bounds stays red no matter what the preview does. That probe
    //   could not tell "paint the invalidation failed to erase" from "the
    //   layer, correctly still there", and would have filed a MAJOR against
    //   documented behaviour.
    //
    //   So: GROW the preview beyond the layer's bounds first and sample
    //   THERE. Outside the block, the preview is the only thing that can
    //   put ink on the canvas, which makes the reading unambiguous.
    fillBlockOnNewLayer(QRect(360, 240, 120, 80), 0.9, 0.1, 0.1);
    QCOMPARE(engineContentBounds(m_controller), QRect(360, 240, 120, 80));
    setTool("transform.free");

    // BR handle (480,320), anchor = opposite (TL) handle (360,240).
    // Dragging BR out to (560,400) is an aspect-locked 2.0 scale, so the
    // preview covers (360,240)-(600,400) — well past the block's own
    // (360,240)-(480,320).
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      docToView(480, 320));
    QTest::mouseMove(m_window, docToView(520, 360));
    QTest::mouseMove(m_window, docToView(560, 400));
    QCoreApplication::processEvents();

    // OUTSIDE the layer (x>480 and y>320), inside the grown preview.
    const QPoint probeDoc(540, 370);
    const QPoint probeView = docToView(probeDoc.x(), probeDoc.y());
    const QImage grown = m_window->grabWindow();
    QVERIFY2(!grown.isNull(), "grabWindow returned null — no frame, so this "
                              "probe can see neither a trail nor its absence");
    QVERIFY2(grown.rect().contains(probeView),
             qPrintable(QStringLiteral("sample point maps to %1,%2, outside "
                                       "the %3x%4 grab — it would read "
                                       "nothing")
                            .arg(probeView.x()).arg(probeView.y())
                            .arg(grown.width()).arg(grown.height())));
    const QColor atGrown = grown.pixelColor(probeView);

    // ★ LIVENESS FLOOR ON THE INSTRUMENT: "no trail" is also what a probe
    //   sampling empty canvas reports. The point must actually be PAINTED
    //   by the grown preview, or the assertion below is about nothing.
    QVERIFY2(atGrown.red() > atGrown.blue() + 20,
             qPrintable(QStringLiteral("LIVENESS FLOOR FAILED: %1,%2 is not "
                                       "inside the GROWN preview (read %3) — "
                                       "the scale-up leg did not happen, so "
                                       "'no trail' below would be vacuous")
                            .arg(probeDoc.x()).arg(probeDoc.y())
                            .arg(atGrown.name())));

    // Now shrink well back inside the block, several moves so any per-move
    // under-invalidation accumulates rather than cancelling.
    for (const QPoint &to : {QPoint(500, 350), QPoint(460, 320),
                             QPoint(420, 290), QPoint(400, 270)}) {
        QTest::mouseMove(m_window, docToView(to.x(), to.y()));
        QCoreApplication::processEvents();
    }
    const QImage shrunk = m_window->grabWindow();
    QVERIFY2(!shrunk.isNull(), "grabWindow returned null on the second read");
    const QColor atShrunk = shrunk.pixelColor(probeView);

    qInfo() << "OQ-5 trail probe: sample" << probeDoc << "grown"
            << atGrown.name() << "-> shrunk" << atShrunk.name();

    // THE ASSERTION. This point is outside the layer, so nothing but the
    // preview can ink it; the preview is now far up-left of it.
    QVERIFY2(!(atShrunk.red() > atShrunk.blue() + 20),
             qPrintable(QStringLiteral("TRAIL: %1,%2 — outside the layer's "
                                       "own bounds — still carries preview "
                                       "ink (%3) after the preview shrank "
                                       "away from it. The OQ-5 invalidation "
                                       "rect painted the new footprint "
                                       "without erasing the old one")
                            .arg(probeDoc.x()).arg(probeDoc.y())
                            .arg(atShrunk.name())));

    QTest::keyClick(m_window, Qt::Key_Escape);
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        docToView(400, 270));
}


void TstGestureNet::shapeStrokeIsServedOrRefusedTyped()
{
    // THE TRIGGER FOR A RESERVED SHELL CONTROL. Main.qml:890 records
    // `stroke_width` as RESERVED with its condition stated:
    //
    //   "the live compositor routes shape layers through
    //    resolve_parametric_page, which refuses widthPx > 0 TYPED
    //    ('fill-only v1') ... Slots in when §20.3 stroke compositing serves."
    //
    // Main just landed "engine: serve §20.2 SHAPE-LAYER strokes — the
    // contract always accepted one, the compositor refused it". So the
    // stated trigger may now be met, and a RESERVED disposition whose
    // condition has expired is the class that goes stale in silence.
    //
    // This case MEASURES the door rather than reading the commit subject,
    // and it is deliberately written to pass EITHER WAY — its job is to
    // report which regime is live and to red only if the answer stops being
    // one of the two the shell's note contemplates. When the shell grows the
    // control, this becomes its precondition pin.
    // PAYLOAD SHAPE TAKEN FROM THE REGISTRY'S OWN valid_minimal, not
    // invented. My first cut used {shape, rect, fill.content} and the door
    // answered `error.schema.missing_field` — which I was one step from
    // recording as "still refused typed", i.e. reporting MY malformed
    // request as a fact about the compositor. The real shape is
    // {primitive:{kind,geometry}, fill:{type,color}, stroke:{widthPx}}.
    QVariantMap geometry;
    geometry.insert(QStringLiteral("x"), 200);
    geometry.insert(QStringLiteral("y"), 150);
    geometry.insert(QStringLiteral("w"), 200);
    geometry.insert(QStringLiteral("h"), 120);
    QVariantMap primitive;
    primitive.insert(QStringLiteral("kind"), QStringLiteral("rect"));
    primitive.insert(QStringLiteral("geometry"), geometry);
    QVariantMap fcol;
    fcol.insert(QStringLiteral("model"), QStringLiteral("rgba"));
    fcol.insert(QStringLiteral("comps"), QVariantList{0.9, 0.1, 0.1, 1.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("type"), QStringLiteral("solid"));
    fill.insert(QStringLiteral("color"), fcol);
    QVariantMap stroke;
    stroke.insert(QStringLiteral("widthPx"), 6.0);
    QVariantMap params;
    params.insert(QStringLiteral("primitive"), primitive);
    params.insert(QStringLiteral("fill"), fill);
    params.insert(QStringLiteral("stroke"), stroke);

    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("vector.shape_add"), {}, params, {});
    const bool ok = r.value(QStringLiteral("ok")).toBool();
    const QString code = r.value(QStringLiteral("code")).toString();
    qInfo().noquote()
        << QStringLiteral("SHAPE STROKE widthPx=6: ok=%1 code='%2'")
               .arg(ok ? QStringLiteral("true") : QStringLiteral("false"), code);

    // ★ TIGHTENED once the answer was known. This case was first written to
    //   pass in EITHER regime, which was right while "does the compositor
    //   still refuse?" was an open question. It is not open any more:
    //   widthPx=6 is accepted AND painted (measured below). Leaving the
    //   permissive branch in would make this a guard that stops firing the
    //   day the service regresses — the exact class this log keeps finding.
    QVERIFY2(ok,
             qPrintable(QStringLiteral("shape_add with widthPx=6 was REFUSED "
                                       "('%1'). §20.2 shape-layer strokes "
                                       "were served at 8bd5ea17 and measured "
                                       "painting at T58; a refusal here is a "
                                       "REGRESSION of that service")
                            .arg(code)));

    // SERVED. Prove it actually PAINTS the stroke rather than accepting the
    // parameter and dropping it — acceptance is not service, and the whole
    // point of the RESERVED note was compositor behaviour, not validation.
    QImage img;
    QVERIFY2(m_controller->readRegionImage(QRect(0, 0, 640, 400), &img),
             "readRegionImage refused after a stroked shape_add");
    // A 6px stroke centred on the rect edge covers x=200 +/- 3. Sample ON the
    // left edge (dark stroke) and well INSIDE (red fill): if the stroke is
    // dropped, the edge pixel reads as fill or as background.
    const QColor edge = img.pixelColor(200, 210);
    const QColor inside = img.pixelColor(300, 210);
    qInfo().noquote()
        << QStringLiteral("SHAPE STROKE served: edge=%1 inside=%2")
               .arg(edge.name(), inside.name());
    QVERIFY2(inside.red() > inside.blue() + 40,
             qPrintable(QStringLiteral("the shape's FILL did not paint "
                                       "(inside=%1) — this probe is not "
                                       "measuring a rendered shape")
                            .arg(inside.name())));
    QVERIFY2(edge != inside,
             qPrintable(QStringLiteral("SHAPE STROKE ACCEPTED BUT NOT "
                                       "PAINTED: the edge pixel (%1) matches "
                                       "the fill (%2), so widthPx=6 was "
                                       "validated and discarded")
                            .arg(edge.name(), inside.name())));
}


void TstGestureNet::armedCitationsAreStillArmed()
{
    // ★ WHY THIS EXISTS. Twice in two ticks a tool graduated from ARMED to
    //   LIVE — smudge at T64, then dodge/burn/blur/sharpen here — and both
    //   times my roster went on citing `armedControlsAnnouncePendingOnPress`
    //   for a tool that now paints. Both times I noticed by accident, while
    //   reading someone else's commit.
    //
    //   A roster that can only be corrected by its author noticing is a
    //   roster that drifts. This case makes the citation self-checking:
    //   every tool my census roster calls ARMED must still BE armed, which
    //   for a canvas tool means a full press-drag-release reaches the engine
    //   with NOTHING — no journal entry. The moment one is wired, this reds
    //   and names it, instead of me finding out three ticks later.
    struct Armed { const char *tool; };
    static const Armed armed[] = {
        // ★★ T67: heal.stroke and dodge_burn.sponge REMOVED from this list
        // because they GRADUATED (Q-D1(a) and Q-D3(b), both ruled
        // 2026-08-29 and both landed in this range). This lint is what
        // found them: it red'd naming both, which is the first time the
        // roster's drift was caught by the roster instead of by my
        // reading someone else's commit. T64 and T66 were the same defect
        // found by accident; this is the mechanism working.
        {"select.quick"},
        // clone.stroke REMOVED 2026-09-13 (operator WORD 4 (A), merge mechanics).
        // This case named it itself: "ARMED CITATIONS: 4 checked, 1 now LIVE ...
        // STALE CITATION(S): clone.stroke now reach the engine". It is the T64/T66
        // class the case exists to catch, caught by the lint rather than by accident.
        // ⛔ FINDING TO q-gesture, not fixed here: clone.stroke is now LIVE and needs
        // retargeting at a case that covers it live. Removing it from the ARMED
        // roster stops the roster lying; it does not give the tool coverage.
        {"content_aware_fill"}, {"fill"},
        {"smudge.stroke.RETIRED"}, // sentinel: see the guard below
    };
    // The sentinel is deliberately a name no tool has. It keeps THIS list
    // honest about its own maintenance: if the guard below ever stops
    // failing on it, the loop has stopped running.
    QStringList nowLive, checked;
    for (const Armed &a : armed) {
        const QString tool = QLatin1String(a.tool);
        if (tool.endsWith(QLatin1String(".RETIRED")))
            continue;
        setTool(a.tool);
        // NON-VACUITY: if the tool did not take, the drag below is a drag
        // with whatever tool was already selected, and "no journal entry"
        // would be a fact about that other tool.
        QCOMPARE(m_canvas->property("tool").toString(), tool);
        const int before = m_controller->historyEntries().size();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(120, 210));
        QTest::mouseMove(m_window, docToView(220, 210));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(220, 210));
        checked << tool;
        if (m_controller->historyEntries().size() > before)
            nowLive << tool;
    }
    // 4 -> 3 with clone.stroke's removal above (WORD 4 (A)). This guard is the
    // roster's own sentinel and it FIRED on that edit — "the armed list walked 3
    // tools, not 4" — which is the mechanism working, not a nuisance: the count
    // moves only when a human moves it, so a roster that silently empties or a
    // loop that stops running cannot pass.
    QVERIFY2(checked.size() == 3,
             qPrintable(QStringLiteral("the armed list walked %1 tools, not "
                                       "3 — the loop or the list changed "
                                       "shape and every assertion below is "
                                       "about a different set")
                            .arg(checked.size())));
    qInfo().noquote()
        << QStringLiteral("ARMED CITATIONS: %1 checked, %2 now LIVE")
               .arg(checked.size()).arg(nowLive.size());
    QVERIFY2(nowLive.isEmpty(),
             qPrintable(QStringLiteral("STALE CITATION(S): %1 now reach the "
                                       "engine but my census roster still "
                                       "cites armedControlsAnnouncePendingOn"
                                       "Press. Retarget them at the case that "
                                       "covers them live — this is the T64/T66 "
                                       "class, caught by the lint this time "
                                       "instead of by accident")
                            .arg(nowLive.join(QStringLiteral(", ")))));
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QGuiApplication app(argc, argv);
    TstGestureNet tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_gesture_net.moc"
