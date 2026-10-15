// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// G-5 FIRST-HAND REPRODUCTION + a COMMIT-PATH LATENCY PROBE THAT CANNOT
// REPORT A NUMBER IT DID NOT RENDER.
//
// WHY THIS FILE EXISTS. The repaint-scoping slice (merge 1bcb5bb) rides
// two claims that are in tension, and only one of them was ever measured:
//
//   COST  — G-5: after a shape commit through the REAL gesture path the
//           canvas renders the transparency checkerboard over regions the
//           document still holds opaque. Bisected, reproducible, and
//           user-visible. Case A below reproduces it first-hand rather
//           than relaying it, and case B pins the self-heal.
//
//   BENEFIT — "the top shell perf win". UNMEASURED at the gesture level.
//           The standing interactive probe (tst_gesture_net's
//           largeCanvasGestureLatency, on qa/q-gesture) fires
//           press/move/release back to back under one QElapsedTimer and
//           barely yields to the render loop, so almost no
//           QQuickPaintedItem::paint() work lands inside the measured
//           window. Its 568 ms / 576 ms is flat BY CONSTRUCTION: it is
//           timing envelope submission and engine readback, and the
//           repaint scoping is precisely the thing it excludes. Case D
//           REPORTS that shape's paint cost instead of asserting a frame
//           count (its first draft asserted zero paints and was
//           falsified — see the case), and case C is the replacement.
//
// THE INSTRUMENT RULE (house rule C-11, "no row may report 'no result'";
// three instruments here have already reported a fast time for work they
// silently stopped doing). This probe is built so that an UNRENDERED leg
// is not representable as a number:
//   - every measured leg runs through recordLeg(), which HARD-FAILS on a
//     zero paint() count before the sample can enter the series;
//   - every leg's timed window contains a grabWindow() whose null return
//     hard-fails, so "the scene graph stopped rendering" cannot present
//     as "the commit got faster";
//   - the RAN vs DECLARED partition check — two assertions written
//     inline in commitPathFrameLatency2048() ahead of its qInfo() rows —
//     blocks the report unless the number of RECORDED samples equals the
//     number of DECLARED reps and every recorded sample carries
//     paints > 0.
// A probe that stops rendering therefore reds; it cannot go fast.
//
// PORTABILITY ACROSS THE TWO SIDES. This file is measured BOTH with the
// repaint-scoping slice present and with it reverted, so it touches ONLY
// API that predates the slice: no overlayChanged, no selectionMaskFull,
// no selectionMaskReads. The paint counter rides the public virtual
// CanvasView::paint(), which exists identically on both sides.

#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"

#include <QGuiApplication>
#include <QPainter>
#include <QQuickWindow>
#include <QtTest>

#include <algorithm>
#include <cstdlib>

using namespace appname::shell;

namespace {

// ---------------------------------------------------------------- load
// Wall-clock work on a contended host is contention, not measurement.
// Every run prints the 1/5/15-minute averages at its own boundaries so a
// mid-run load move is visible in the artifact itself, not only in the
// operator's shell.
void printLoadLine(const char *tag)
{
    double load[3] = {0, 0, 0};
#ifdef _WIN32
    const int got = -1; // W-09 (O-WIN): Windows has no load average
#else
    const int got = getloadavg(load, 3);
#endif
    if (got == 3)
        qInfo("MACHINE-LOAD %s load1=%.2f load5=%.2f load15=%.2f", tag,
              load[0], load[1], load[2]);
    else
        qInfo("MACHINE-LOAD %s unavailable", tag);
}

// ------------------------------------------------------- paint counting
// QQuickPaintedItem clips paint() to the accumulated dirty rect, so the
// painter's clip bounding rect IS the area the scene graph is actually
// repainting. An EMPTY clip means "no clip set" = the whole item, which
// is exactly the pre-slice full-item repaint shape.
class FrameCanvas : public CanvasView {
public:
    explicit FrameCanvas(QQuickItem *parent = nullptr)
        : CanvasView(parent)
    {
    }

    void paint(QPainter *painter) override
    {
        ++paintCalls;
        const QRectF clip = painter->clipBoundingRect();
        paintedPx += clip.isEmpty() ? width() * height()
                                    : clip.width() * clip.height();
        QElapsedTimer t;
        t.start();
        CanvasView::paint(painter);
        paintUs += t.nsecsElapsed() / 1000;
    }

    void resetCounters()
    {
        paintCalls = 0;
        paintedPx = 0.0;
        paintUs = 0;
    }

    int paintCalls = 0;
    double paintedPx = 0.0;
    qint64 paintUs = 0;
};

// ------------------------------------------------------------- samples
struct Sample {
    qint64 wallUs = 0;   // gesture submit + forced frame, end to end
    qint64 paintUs = 0;  // time INSIDE CanvasView::paint() for that leg
    int paints = 0;      // paint() calls in the leg (0 => not a sample)
    double paintedPx = 0.0;
};

qint64 medianOf(QVector<qint64> v)
{
    if (v.isEmpty())
        return -1;
    std::sort(v.begin(), v.end());
    return v[v.size() / 2];
}

} // namespace

class TstG5FrameProbe : public QObject {
    Q_OBJECT

private slots:
    void initTestCase();
    void cleanupTestCase();

    // JOB 1 — the regression, reproduced through the real gesture path.
    void g5ShapeCommitRendersCheckerboardOverImage();
    void g5SelfHealsOnForcedFullRepaint();

    // JOB 2 — the benefit, measured where the slice actually acts.
    void commitPathFrameLatency2048();      // the replacement instrument
    void noFrameControlReportsPaintCost();  // why the old number was flat

    // JOB 3 — OQ-5: the per-drag-frame repaint footprint of item 21.
    void transformDragRepaintFootprint();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    DocumentController *m_controller = nullptr;
    QQuickWindow *m_window = nullptr;
    FrameCanvas *m_canvas = nullptr;

    // Engine truth at a document point (the composite the document
    // actually holds), read through the same door the canvas cache uses.
    QColor enginePixel(int x, int y)
    {
        QImage px;
        if (!m_controller->readRegionImage(QRect(x, y, 1, 1), &px)
            || px.isNull())
            return QColor();
        return px.pixelColor(0, 0);
    }

    void settle(int ms = 120) { QTest::qWait(ms); }
};

void TstG5FrameProbe::initTestCase()
{
    printLoadLine("suite-start");
    qInfo("PROBE-BUILD config=%s", APPNAME_PROBE_BUILD_CONFIG);
    m_controller = new DocumentController(&m_catalog);
    m_window = new QQuickWindow;
    m_window->resize(1280, 800);
    m_canvas = new FrameCanvas(m_window->contentItem());
    m_canvas->setSize(QSizeF(1280, 800));
    m_canvas->setController(m_controller);
    m_window->show();
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
}

void TstG5FrameProbe::cleanupTestCase()
{
    printLoadLine("suite-end");
    if (m_canvas)
        m_canvas->setController(nullptr);
    delete m_window;
    m_window = nullptr;
    delete m_controller;
    m_controller = nullptr;
}

// ============================================================ JOB 1 ====

void TstG5FrameProbe::g5ShapeCommitRendersCheckerboardOverImage()
{
    // A 640x400 white document at 1:1, centred in the 1280x800 view, so
    // doc (x,y) maps to view (x+320, y+200). The mapping is DERIVED from
    // the live zoom/geometry below rather than assumed.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvas->property("zoomPercent").toInt(), 100);
    const double ox = (1280.0 - 640.0) / 2.0;
    const double oy = (800.0 - 400.0) / 2.0;
    const auto toView = [&](int x, int y) {
        return QPoint(int(x + ox), int(y + oy));
    };
    m_canvas->forceActiveFocus();
    settle();

    // Baseline: BEFORE any shape commit the canvas already renders the
    // document's white background at the point the sweep below will
    // probe. Without this the case cannot tell a repaint regression
    // apart from a probe that never rendered the document at all.
    //
    // Same predicate and same limitation as the sweep below (see the
    // block there): this asserts screen == #ffffff, NOT screen ==
    // engine; `eng0` is reported for the artifact, not compared.
    {
        const QImage f0 = m_window->grabWindow();
        QVERIFY2(!f0.isNull(), "grabWindow returned null (baseline)");
        const QColor eng0 = enginePixel(60, 350);
        const QColor view0 = f0.pixelColor(toView(60, 350));
        qInfo("G5-BASELINE doc=(60,350) engine=%s a=%d screen=%s",
              qPrintable(eng0.name()), eng0.alpha(),
              qPrintable(view0.name()));
        QVERIFY2(view0 == QColor(Qt::white),
                 qPrintable(QStringLiteral("baseline canvas already wrong "
                                           "(%1) — the rest of this case "
                                           "would be meaningless")
                                .arg(view0.name())));
    }

    // THE GESTURE. Not applyCommand: press / move / release on the canvas
    // with a shape tool, which is the path the tour structurally never
    // takes and the reason this defect survived every battery this week.
    m_canvas->setProperty("tool", QLatin1String("shape.rect"));
    settle(40);
    const int layersBefore = m_controller->layers().size();
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(400, 60));
    QTest::mouseMove(m_window, toView(480, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(560, 180));
    settle();
    QVERIFY2(m_controller->layers().size() > layersBefore,
             "the shape gesture committed no layer — the repro never ran");

    const QImage frame = m_window->grabWindow();
    QVERIFY2(!frame.isNull(), "grabWindow returned null after the commit");

    // Sweep a grid of document points WELL AWAY from the committed shape
    // and report, for each one, engine truth beside what is on screen.
    //
    // WHAT THE ASSERTION ACTUALLY CHECKS. Exactly one thing: that the
    // screen pixel is #ffffff — the white background of the fixture
    // document created above; that predicate is what the per-point
    // AGREE / MISMATCH token reports. It does NOT compare `scr` to `eng`.
    // `eng` is READ AND REPORTED, not asserted on, and it earns its
    // place that way: with both values in the artifact a reader can tell
    // a PRESENTATION fault (engine still #ffffff, screen checker — G-5)
    // apart from actual DATA LOSS (engine itself no longer #ffffff),
    // which is the distinction the operator needs and the one the
    // self-heal case below settles.
    //
    // WHY WHITE IS SUFFICIENT FOR G-5 SPECIFICALLY. The artifact this
    // case hunts is the transparency checker tiled over opaque document
    // area. That checker is built from QColor(0xc8,0xc8,0xc8) and
    // QColor(0xf2,0xf2,0xf2) (canvas_view.cpp, the m_checkerTile block)
    // and NEITHER TILE IS #ffffff, so "the screen is white" genuinely
    // discriminates the artifact — a probe point landing on any checker
    // square, light or dark, scores MISMATCH. Measured, not reasoned:
    // with the clip op mutated back to Qt::ReplaceClip this sweep reds
    // with screen=#f2f2f2 / #c8c8c8 at these coordinates.
    //
    // LIMITATION, AND ITS DIRECTION. This is coupled to a WHITE-document
    // fixture. Change the fixture background and a CORRECT render reds —
    // a FALSE RED, never a false green: the check cannot pass a build
    // that renders the checker. Generalising it (comparing `scr` to
    // `eng`) is NOT a free upgrade and was deliberately not taken: on a
    // white fixture the two predicates are IDENTICAL, so nothing here
    // measures the difference, while `eng` carries alpha from the engine
    // composite and `scr` is already composited over the checker — so on
    // any document with transparency `scr != eng` at a correct render by
    // design. `toView` likewise hardcodes the 1:1 mapping asserted
    // above. A generalised comparison needs both of those handled and a
    // fixture that can actually measure it.
    const QVector<QPoint> probes = {
        {60, 350},  {160, 350}, {320, 350}, {480, 350}, {600, 350},
        {60, 200},  {60, 60},   {160, 250}, {320, 250}, {600, 60},
    };
    int mismatches = 0;
    QStringList firstMismatch;
    for (const QPoint &p : probes) {
        const QColor eng = enginePixel(p.x(), p.y());
        const QColor scr = frame.pixelColor(toView(p.x(), p.y()));
        const bool screenIsDocWhite = (scr == QColor(Qt::white));
        if (!screenIsDocWhite) {
            ++mismatches;
            if (firstMismatch.isEmpty())
                firstMismatch << QStringLiteral("%1,%2").arg(p.x()).arg(p.y())
                              << eng.name() << QString::number(eng.alpha())
                              << scr.name();
        }
        qInfo("G5-POINT doc=(%d,%d) engine=%s a=%d screen=%s %s", p.x(), p.y(),
              qPrintable(eng.name()), eng.alpha(), qPrintable(scr.name()),
              screenIsDocWhite ? "AGREE" : "MISMATCH");
    }
    qInfo("G5-SUMMARY probed=%d mismatches=%d", int(probes.size()), mismatches);

    QVERIFY2(mismatches == 0,
             qPrintable(QStringLiteral("G-5 REPRODUCED: after a shape commit "
                                       "through the gesture path the canvas "
                                       "renders %4 at doc (%1) where the "
                                       "document holds %2 alpha %3 — %5 of "
                                       "%6 probed points disagree")
                            .arg(firstMismatch.value(0))
                            .arg(firstMismatch.value(1))
                            .arg(firstMismatch.value(2))
                            .arg(firstMismatch.value(3))
                            .arg(mismatches)
                            .arg(probes.size())));
}

void TstG5FrameProbe::g5SelfHealsOnForcedFullRepaint()
{
    // Same commit, then a FORCED FULL REPAINT (zoomFit -> zoomOneToOne,
    // both of which call update() on the whole item). If the document is
    // intact and only the invalidation scope is wrong, the pixel comes
    // back. That is the difference between a presentation bug and data
    // loss, and it is the half the operator needs to weigh the cost.
    //
    // Same predicate and same limitation as the sweep in the case above
    // (see the block there): the assertion is screen == #ffffff, the
    // fixture's white background, NOT screen == engine. `eng` is read
    // and logged beside before/after so the artifact shows whether the
    // document ever lost the pixel; it is not compared against.
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    m_canvas->zoomOneToOne();
    const double ox = (1280.0 - 640.0) / 2.0;
    const double oy = (800.0 - 400.0) / 2.0;
    const auto toView = [&](int x, int y) {
        return QPoint(int(x + ox), int(y + oy));
    };
    m_canvas->forceActiveFocus();
    settle();

    m_canvas->setProperty("tool", QLatin1String("shape.rect"));
    settle(40);
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(400, 60));
    QTest::mouseMove(m_window, toView(480, 120));
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(560, 180));
    settle();

    const QColor before =
        m_window->grabWindow().pixelColor(toView(60, 350));

    m_canvas->zoomFit();
    settle();
    m_canvas->zoomOneToOne();
    settle();

    const QColor after = m_window->grabWindow().pixelColor(toView(60, 350));
    const QColor eng = enginePixel(60, 350);
    qInfo("G5-RECOVERY doc=(60,350) engine=%s a=%d before_repaint=%s "
          "after_forced_full_repaint=%s",
          qPrintable(eng.name()), eng.alpha(), qPrintable(before.name()),
          qPrintable(after.name()));

    QVERIFY2(after == QColor(Qt::white),
             qPrintable(QStringLiteral("a forced full repaint did NOT restore "
                                       "the pixel (%1) — this is worse than "
                                       "G-5 as filed: not presentation-only")
                            .arg(after.name())));
}

// ============================================================ JOB 2 ====

void TstG5FrameProbe::commitPathFrameLatency2048()
{
    // THE REPLACEMENT INSTRUMENT. Same size and same commit-path scenario
    // the slice claims to improve (2048^2, the existing comparison
    // point), driven through the GESTURE path, with a rendered frame
    // INSIDE the measured window.
    //
    // The timed leg is: press + moves + release (the envelope reaches the
    // engine, the dirty rects come back, the item invalidates) AND THEN a
    // grabWindow() that forces the scene graph to sync and render, which
    // is what actually runs CanvasView::paint(). The pre-slice build
    // repaints the full 1280x800 item on that frame; the post-slice build
    // repaints only the scoped rect. That difference is invisible to any
    // probe that stops at the release.
    const int kDeclaredReps = 9;
    const int kWarmup = 2;

    printLoadLine("latency-run-start");

    QVERIFY(m_controller->newDocument(2048, 2048, QStringLiteral("white")));
    m_canvas->zoomFit();
    const double zoom = m_canvas->property("zoom").toDouble();
    QVERIFY2(zoom > 0.0 && zoom < 1.0, "zoomFit did not shrink a 2048^2 doc");
    const double ox = (1280.0 - 2048.0 * zoom) / 2.0;
    const double oy = (800.0 - 2048.0 * zoom) / 2.0;
    const auto toView = [&](double x, double y) {
        return QPointF(x * zoom + ox, y * zoom + oy).toPoint();
    };
    m_canvas->forceActiveFocus();
    settle(250);

    QVector<Sample> series;
    series.reserve(kDeclaredReps);

    // recordLeg: the ONLY door a timing sample may enter the series
    // through. A leg with zero paint() calls is a FAILURE, never a fast
    // number — the whole point of the instrument.
    const auto recordLeg = [&](int rep, qint64 wallUs) {
        const int paints = m_canvas->paintCalls;
        QVERIFY2(paints > 0,
                 qPrintable(QStringLiteral("UNRENDERED LEG rep=%1: zero "
                                           "paint() calls inside the measured "
                                           "window — this probe refuses to "
                                           "report a duration it did not "
                                           "render")
                                .arg(rep)));
        Sample s;
        s.wallUs = wallUs;
        s.paintUs = m_canvas->paintUs;
        s.paints = paints;
        s.paintedPx = m_canvas->paintedPx;
        series.append(s);
    };

    for (int rep = 0; rep < kWarmup + kDeclaredReps; ++rep) {
        const bool warm = rep < kWarmup;
        // Drain any queued repaint from the previous rep's undo so it
        // cannot land inside the next counted window.
        settle(150);
        m_canvas->setProperty("tool", QLatin1String("shape.rect"));
        settle(40);
        m_canvas->resetCounters();

        // Stagger each rep's shape so no two commits share a dirty rect
        // and the engine cannot serve a degenerate no-op.
        const double x0 = 300.0 + (rep % 5) * 90.0;
        const double y0 = 300.0 + (rep % 3) * 90.0;

        QElapsedTimer t;
        t.start();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          toView(x0, y0));
        QTest::mouseMove(m_window, toView(x0 + 120, y0 + 90));
        QTest::mouseMove(m_window, toView(x0 + 240, y0 + 180));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            toView(x0 + 240, y0 + 180));
        // FORCE THE FRAME. Without this line the leg measures envelope
        // submission only — the existing probe's exact blind spot.
        const QImage f = m_window->grabWindow();
        const qint64 wallUs = t.nsecsElapsed() / 1000;

        QVERIFY2(!f.isNull(),
                 qPrintable(QStringLiteral("grabWindow returned null at rep "
                                           "%1 — the scene graph stopped "
                                           "rendering; no timing may be "
                                           "reported from this run")
                                .arg(rep)));
        if (!warm)
            recordLeg(rep, wallUs);

        // Restore the layer stack OUTSIDE the timed window so every rep
        // commits against an identical document.
        m_controller->undo();
        settle(120);
    }

    // ---- RAN vs DECLARED partition check ---------------------------
    // These two assertions gate the qInfo() rows below: nothing prints
    // unless every declared rep produced a RENDERED sample. A silently
    // shortened series cannot masquerade as a clean median.
    QCOMPARE(series.size(), kDeclaredReps);
    for (const Sample &s : series)
        QVERIFY2(s.paints > 0, "a sample with zero paints reached the series");

    QVector<qint64> wall, paint;
    QVector<int> paints;
    double pxPerPaint = 0.0;
    for (const Sample &s : series) {
        wall << s.wallUs;
        paint << s.paintUs;
        paints << s.paints;
        pxPerPaint += s.paintedPx / s.paints;
    }
    pxPerPaint /= series.size();
    std::sort(wall.begin(), wall.end());
    std::sort(paint.begin(), paint.end());
    const double itemPx = 1280.0 * 800.0;

    qInfo("PROBE commit-path-frame config=%s doc=2048x2048 reps=%d "
          "wall_med_us=%lld wall_min_us=%lld wall_max_us=%lld "
          "paint_med_us=%lld paint_min_us=%lld paint_max_us=%lld "
          "paints_med=%d painted_px_per_paint=%.0f item_px=%.0f "
          "full_frac=%.3f",
          APPNAME_PROBE_BUILD_CONFIG, int(series.size()),
          static_cast<long long>(medianOf(wall)),
          static_cast<long long>(wall.first()),
          static_cast<long long>(wall.last()),
          static_cast<long long>(medianOf(paint)),
          static_cast<long long>(paint.first()),
          static_cast<long long>(paint.last()),
          paints[paints.size() / 2], pxPerPaint, itemPx,
          pxPerPaint / itemPx);

    // Per-rep rows so the operator can see the spread, not just a median.
    for (int i = 0; i < series.size(); ++i)
        qInfo("PROBE-REP i=%d wall_us=%lld paint_us=%lld paints=%d "
              "painted_px=%.0f",
              i, static_cast<long long>(series[i].wallUs),
              static_cast<long long>(series[i].paintUs), series[i].paints,
              series[i].paintedPx);

    printLoadLine("latency-run-end");
}

void TstG5FrameProbe::noFrameControlReportsPaintCost()
{
    // THE CONTROL — the exact shape of largeCanvasGestureLatency: the
    // same gesture on the same document, with NOTHING forcing a frame
    // before the timer is read.
    //
    // CORRECTION, MEASURED (my own first draft of this case asserted
    // paints == 0 and was FALSIFIED on its first run — recorded here
    // rather than quietly re-tuned). The no-frame shape is not literally
    // frameless: one paint slips in per gesture, because QTest's mouse
    // helpers pump the event loop enough for the render loop to service a
    // single frame. So the case ASSERTS NOTHING about the count and
    // instead REPORTS it, over reps, on both sides of the slice. The
    // question it answers is not "are there zero frames" but "does this
    // shape carry enough repaint work to tell the two builds apart" —
    // and that is settled by comparing its paint_us across the sides,
    // not by any assertion inside one run.
    const int kDeclaredReps = 7;

    printLoadLine("control-run-start");
    QVERIFY(m_controller->newDocument(2048, 2048, QStringLiteral("white")));
    m_canvas->zoomFit();
    const double zoom = m_canvas->property("zoom").toDouble();
    QVERIFY2(zoom > 0.0 && zoom < 1.0, "zoomFit did not shrink a 2048^2 doc");
    const double ox = (1280.0 - 2048.0 * zoom) / 2.0;
    const double oy = (800.0 - 2048.0 * zoom) / 2.0;
    const auto toView = [&](double x, double y) {
        return QPointF(x * zoom + ox, y * zoom + oy).toPoint();
    };
    m_canvas->forceActiveFocus();
    settle(250);

    QVector<qint64> wall, paint;
    QVector<int> paints;
    for (int rep = 0; rep < kDeclaredReps; ++rep) {
        settle(150);
        m_canvas->setProperty("tool", QLatin1String("shape.rect"));
        settle(40);
        m_canvas->resetCounters();

        const double x0 = 300.0 + (rep % 5) * 90.0;
        const double y0 = 300.0 + (rep % 3) * 90.0;

        QElapsedTimer t;
        t.start();
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          toView(x0, y0));
        QTest::mouseMove(m_window, toView(x0 + 120, y0 + 90));
        QTest::mouseMove(m_window, toView(x0 + 240, y0 + 180));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            toView(x0 + 240, y0 + 180));
        // NO grabWindow here — that omission IS the control.
        wall << t.nsecsElapsed() / 1000;
        paint << m_canvas->paintUs;
        paints << m_canvas->paintCalls;

        m_controller->undo();
        settle(120);
    }

    QCOMPARE(wall.size(), kDeclaredReps);
    QVector<qint64> w = wall, p = paint;
    std::sort(w.begin(), w.end());
    std::sort(p.begin(), p.end());
    QVector<int> pc = paints;
    std::sort(pc.begin(), pc.end());

    qInfo("PROBE no-frame-control config=%s doc=2048x2048 reps=%d "
          "wall_med_us=%lld wall_min_us=%lld wall_max_us=%lld "
          "paint_med_us=%lld paint_min_us=%lld paint_max_us=%lld "
          "paints_med=%d paints_min=%d paints_max=%d",
          APPNAME_PROBE_BUILD_CONFIG, int(wall.size()),
          static_cast<long long>(medianOf(w)),
          static_cast<long long>(w.first()),
          static_cast<long long>(w.last()),
          static_cast<long long>(medianOf(p)),
          static_cast<long long>(p.first()),
          static_cast<long long>(p.last()),
          pc[pc.size() / 2], pc.first(), pc.last());
    printLoadLine("control-run-end");
}

// ============================================================ JOB 3 ====

void TstG5FrameProbe::transformDragRepaintFootprint()
{
    // OQ-5 (item 21): the per-drag-frame repaint FOOTPRINT, which is the
    // number §E.4's perf bar for item 21 asks for -- "counts rather than
    // latencies where latencies are noisy". The count is `paintedPx`: the
    // area the scene graph actually repaints, read off the painter's clip
    // bounding rect, with an EMPTY clip scored as the FULL ITEM, because
    // that is exactly what an unscoped update() produces. Same instrument
    // on both sides of the change, and it predates the change (G-38: a
    // control measured with a different instrument than the subject is not
    // a control).
    //
    // THE LAYER IS DELIBERATELY A SMALL BLOCK ON A LARGE CANVAS. With a
    // full-canvas layer the transform frame IS the canvas, a scoped rect
    // can win nothing, and the case would report a clean null it had no
    // capacity to disagree with (G-115).
    printLoadLine("transform-drag-start");
    QVERIFY(m_controller->newDocument(2048, 2048, QStringLiteral("white")));
    m_canvas->zoomFit();
    const double zoom = m_canvas->property("zoom").toDouble();
    QVERIFY2(zoom > 0.0 && zoom < 1.0, "zoomFit did not shrink a 2048^2 doc");
    const double ox = (1280.0 - 2048.0 * zoom) / 2.0;
    const double oy = (800.0 - 2048.0 * zoom) / 2.0;
    const auto toView = [&](double x, double y) {
        return QPointF(x * zoom + ox, y * zoom + oy).toPoint();
    };

    // One raster layer carrying a single 256x256 block at doc (200,200).
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
    m_controller->setActiveLayerIndex(idx);
    {
        QVariantMap rect{{QStringLiteral("x"), 200}, {QStringLiteral("y"), 200},
                         {QStringLiteral("w"), 256}, {QStringLiteral("h"), 256}};
        QVariantMap sel{{QStringLiteral("source"), QStringLiteral("rect")},
                        {QStringLiteral("rect"), rect}};
        m_controller->applyCommand(QStringLiteral("selection.set"), {}, sel, {});
        QVariantMap color{{QStringLiteral("model"), QStringLiteral("rgb")},
                          {QStringLiteral("comps"),
                           QVariantList{0.8, 0.1, 0.1, 0.0}}};
        QVariantMap fill{{QStringLiteral("content"), QStringLiteral("color")},
                         {QStringLiteral("color"), color}};
        QVariantMap target{
            {QStringLiteral("layerId"),
             m_controller->layers().at(idx).toMap().value(
                 QStringLiteral("layerId"))}};
        QVERIFY(m_controller
                    ->applyCommand(QStringLiteral("fill"), target, fill, {})
                    .value(QStringLiteral("ok"))
                    .toBool());
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {}, {});
    }
    m_canvas->forceActiveFocus();
    settle(150);

    m_canvas->setProperty("tool", QLatin1String("transform.free"));
    settle(80);
    QVERIFY2(m_canvas->property("transformActive").toBool(),
             "transform.free opened no frame — the case never ran");
    QVERIFY2(!m_canvas->property("transformPreviewUnavailable").toBool(),
             "no preview snapshot — this case would then be measuring the "
             "chrome alone, not item 21");

    // Press at the BLOCK's own centre. Item 19 made the frame the LAYER,
    // so a press outside the block grabs nothing, the drag produces no
    // gesture, and the probe would report a clean zero for a drag that
    // never happened.
    QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                      toView(328, 328));

    const int kFrames = 8;
    const double itemPx = m_canvas->width() * m_canvas->height();
    QVector<double> perFrame;
    for (int i = 1; i <= kFrames; ++i) {
        settle(40); // drain the previous frame's repaint out of this window
        m_canvas->resetCounters();
        QTest::mouseMove(m_window, toView(328 + i * 40, 328 + i * 30));
        const QImage f = m_window->grabWindow();
        QVERIFY2(!f.isNull(),
                 "grabWindow returned null — the scene graph stopped "
                 "rendering; no footprint may be reported from this run");
        QVERIFY2(m_canvas->paintCalls > 0,
                 qPrintable(QStringLiteral("UNRENDERED DRAG FRAME %1: zero "
                                           "paint() calls inside the counted "
                                           "window — this probe refuses to "
                                           "report a footprint it did not "
                                           "render")
                                .arg(i)));
        perFrame.append(m_canvas->paintedPx);
    }
    QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                        toView(328 + kFrames * 40, 328 + kFrames * 30));

    // THE ANTI-TRAIL PIN (G-118), and it is the reason a scoped rect needs
    // one at all: a too-small invalidation rect does not go SLOW, it leaves
    // the PREVIOUS frame's pixels in the backing store — a visible lie about
    // where the layer is, which no timing number would ever show.
    //
    // C-352 — THE POINT PROBE BELOW IS NOT ENOUGH, MEASURED. O-INT's STEP-4
    // review found that `transformPreviewInvalidateRect` had no test naming
    // it, and that its ONE non-conservative path -- someone changing how
    // `m_transformBase` is derived -- was guarded by a comment. I constructed
    // that failure (mutation M-A: halve `m_transformBase` inside the helper so
    // the rect no longer covers the snapshot's painted extent): the point
    // probe stayed GREEN and the footprint number IMPROVED, 16,764 px to
    // 6,636 px. A single-point probe discriminates at one location and is
    // silent everywhere else.
    //
    // The whole-item check below replaces the chosen pixel with ground truth:
    // after the scoped drag, force a FULL repaint and grab again. If every
    // scoped rect covered what it painted, the incrementally-painted item and
    // the fully-repainted item are THE SAME IMAGE. Any rect that was ever too
    // small leaves stale pixels the full repaint corrects, and the two grabs
    // differ. It cannot be dodged by probing the wrong pixel.
    // doc (490,350) is inside the preview's footprint at drag frame 4 and
    // outside it at frame 8, and it is outside the origin block
    // (200,200,256x256) whose ghost legitimately stays (OQ-6). So after
    // the drag it must be the bare canvas again.
    // C-352: ground truth first -- scoped result vs full-repaint result.
    {
        const QImage scoped = m_window->grabWindow();
        QVERIFY2(!scoped.isNull(), "grabWindow returned null (scoped)");
        m_canvas->update(); // whole item, no rect
        settle(60);
        const QImage full = m_window->grabWindow();
        QVERIFY2(!full.isNull(), "grabWindow returned null (full repaint)");
        QCOMPARE(scoped.size(), full.size());
        int differing = 0;
        int firstX = -1, firstY = -1;
        for (int y = 0; y < scoped.height(); ++y) {
            for (int x = 0; x < scoped.width(); ++x) {
                if (scoped.pixel(x, y) != full.pixel(x, y)) {
                    if (differing == 0) { firstX = x; firstY = y; }
                    ++differing;
                }
            }
        }
        QVERIFY2(differing == 0,
                 qPrintable(QStringLiteral("STALE PIXELS: %1 pixel(s) differ "
                                           "between the scoped-repaint item and "
                                           "a full repaint of the same state "
                                           "(first at %2,%3) -- some invalidation "
                                           "rect did not cover what it painted")
                                .arg(differing).arg(firstX).arg(firstY)));
    }
    {
        const QImage f = m_window->grabWindow();
        QVERIFY2(!f.isNull(), "grabWindow returned null at the trail probe");
        const QColor vacated = f.pixelColor(toView(490, 350));
        QVERIFY2(vacated == QColor(Qt::white),
                 qPrintable(QStringLiteral("TRAIL: doc (490,350) reads %1 "
                                           "after the preview left it — the "
                                           "invalidation rect is too small "
                                           "and the canvas is showing a "
                                           "position the layer no longer has")
                                .arg(vacated.name())));
    }
    QTest::keyClick(m_window, Qt::Key_Escape); // leave no frame behind

    QCOMPARE(perFrame.size(), kFrames); // declared == recorded
    QVector<double> sorted = perFrame;
    std::sort(sorted.begin(), sorted.end());
    const double med = sorted[sorted.size() / 2];
    qInfo("OQ5-DRAG-FOOTPRINT config=%s frames=%d itemPx=%.0f "
          "medianPaintedPx=%.0f min=%.0f max=%.0f fractionOfItem=%.4f",
          APPNAME_PROBE_BUILD_CONFIG, int(perFrame.size()), itemPx, med,
          sorted.first(), sorted.last(), med / itemPx);
    printLoadLine("transform-drag-end");
}

// Offscreen + the software scene graph (deterministic QQuickPaintedItem
// paint + window grabs); the engine links in (cabi).
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    if (qgetenv("QT_QUICK_BACKEND").isEmpty())
        qputenv("QT_QUICK_BACKEND", "software");
    QGuiApplication app(argc, argv);
    TstG5FrameProbe tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_g5_frameprobe.moc"
