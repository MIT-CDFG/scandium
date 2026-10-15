// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.5 (repaint only dirty regions) + §31.3 (selection-bounds
// announcement) — the regression net for the commit-path repaint-scoping
// slice (perf sweep findings F1+F2+F4, shell/tests/PERF-SWEEP-FINDINGS.md):
//  - F1: the selection revision bumps ONLY through the two-sided gate
//    (polled selection-writer kind OR (active,bounds) info delta), so a
//    non-selection commit triggers ZERO selection-mask reads;
//  - F1c: the marching-ants vector recomputes OFF the paint path,
//    windowed to viewport + half-viewport margin, with the pan/zoom
//    escape check re-vectorizing before stale geometry can show;
//  - F2: commit-path invalidation is scoped — composite dirt rides the
//    partial regionsDirty path (small commit => small repaint), overlay
//    truth (selection/guides/grid) rides overlayChanged => full item,
//    info-only commits repaint nothing;
//  - F4: ants + announcer share ONE mask read per selection revision,
//    and the announcement text is BYTE-EQUAL to the independent strip
//    scan's (same accumulateMaskBounds over the same Grayscale8 bytes).
// REAL CanvasView + REAL engine document (cabi-linked, offscreen window,
// software scene graph); assertions read engine truth and instrument
// counters (DocumentController::selectionMaskReads — the
// submissionAttempts precedent), never shell echo state.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "selection_scan.h"

#include <QGuiApplication>
#include <QPainter>
#include <QQuickWindow>
#include <QtTest>

using namespace appname::shell;

namespace {

// Paint-counting canvas (the tst_perf_probes ProbeCanvas shape): the
// painter's clip bounding rect IS the invalidated area the scene graph
// actually repaints (empty clip = full item).
class CountingCanvas : public CanvasView {
public:
    explicit CountingCanvas(QQuickItem *parent = nullptr)
        : CanvasView(parent)
    {
    }
    void paint(QPainter *painter) override
    {
        ++paintCalls;
        const QRectF clip = painter->clipBoundingRect();
        const double px = clip.isEmpty() ? width() * height()
                                         : clip.width() * clip.height();
        paintedPx += px;
        maxPaintPx = qMax(maxPaintPx, px);
        CanvasView::paint(painter);
    }
    void resetCounters()
    {
        paintCalls = 0;
        paintedPx = 0.0;
        maxPaintPx = 0.0;
    }
    int paintCalls = 0;
    double paintedPx = 0.0;
    double maxPaintPx = 0.0;
};

// Exact ants-dash pixel count (the over pen is rgb(0x10,0x10,0x10),
// cosmetic, AA off — no other painted element uses this value: doc
// bounds pen is 0x202020, background 0x404245, default brush 0x141414).
int dashPixelCount(const QImage &img, const QRect &within = QRect())
{
    const QRect scan = within.isNull()
        ? QRect(0, 0, img.width(), img.height())
        : within.intersected(QRect(0, 0, img.width(), img.height()));
    int n = 0;
    for (int y = scan.top(); y <= scan.bottom(); ++y)
        for (int x = scan.left(); x <= scan.right(); ++x)
            if (img.pixel(x, y) == qRgb(0x10, 0x10, 0x10))
                ++n;
    return n;
}

} // namespace

class TstRepaintScoping : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanupTestCase();
    void init(); // fresh document per case

    // Case 1 (F1+F2): fill with a LIVE selection — zero mask reads,
    // partial repaint, engine composite carries the fill.
    void fillWithLiveSelectionIsPartialAndReadFree();
    // Case 2 (F4): selection commit — ONE mask read TOTAL with canvas
    // AND announcer both wired; ants painted at the new boundary.
    void selectionCommitSharesOneRead();
    // Case 3 (F1c): pan past the vectorized window — the escape check
    // re-vectorizes; ants present at the new viewport.
    void panEscapeRefreshesAntsWindow();
    // Case 4 (F2 rows 3/4): guide.add => honest full-item repaint;
    // canvas_resize => reset + full.
    void docWideOverlayAndGeometryStayFull();
    // Case 5 (F4/i18n): announcement text byte-equal to the independent
    // strip-scan ground truth; cleared on deselect; same event stream.
    void announcementsUnchanged();
    // Case 6 (F1 gate, both sides): non-selection command => no read;
    // undo/redo of a writer kind bumps; a same-(active,bounds) CONTENT
    // change (invert of two corner dots) bumps via the KIND side alone.
    void kindGateCoversWritersAndContentChanges();
    // Case 7 (§5 fix round, MAJOR-2 — the dagger kind): history.goto
    // replays steps behind its OWN label (the replayed kinds never reach
    // the host), so it must gate as an unconditional potential selection
    // writer. Pinned on the shape that pierced the gate before the fix:
    // a goto that changes selection CONTENT while leaving (active,
    // bounds) identical — arm (b) is blind there by construction.
    void historyGotoReplayBumpsThroughTheKindGate();
    // Case 8 (§5 fix round, MAJOR-1): a bounds-MOVING history.goto bumps
    // and busts the shared cache. Lifted from the reviewer's probe
    // (viii), which they named armBBoundsDeltaCatchesNonWriterKind;
    // RENAMED here because MAJOR-2's fix made both halves of that name
    // false — history.goto is now IN the writer set, so this no longer
    // isolates arm (b). See the case body for what it does and does NOT
    // pin, and the gate comment in document_controller.cpp for why the
    // compare side is retained anyway.
    void boundsMovingGotoBumpsAndBustsTheCache();

private:
    Catalog m_catalog = Catalog::fromTexts({});
    const Catalog m_annCatalog = Catalog::fromTexts(
        {{QStringLiteral("t"),
          QStringLiteral("{ \"a11y.canvas.selection_bounds\": "
                         "\"{width} x {height}\","
                         "  \"a11y.canvas.selection_cleared\": "
                         "\"cleared\" }")}});
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQuickWindow *m_window = nullptr;
    CountingCanvas *m_canvas = nullptr;
    int m_readsBase = 0;

    // The view transform after init: zoom 1 with the 640x400 doc
    // centered in the 800x600 view — doc (x,y) -> view (x+80, y+100)
    // (the tst_selection_gestures derivation, pinned by the
    // zoomPercent==100 assert in init()).
    QPoint docToView(int x, int y) const { return QPoint(x + 80, y + 100); }

    // Settle until no paint lands across two consecutive 50 ms windows
    // (cap 3 s): under external machine load a queued repaint from setup
    // can arrive LATE and poison the counted window (first seen as a
    // full-suite-under-load flake; a fixed 40 ms drain raced it).
    void drainPaints()
    {
        int last = -1;
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 3000) {
            const int now = m_canvas->paintCalls;
            if (now == last)
                break;
            last = now;
            QTest::qWait(50);
        }
    }
    void beginCount()
    {
        drainPaints();
        m_canvas->resetCounters();
        m_readsBase = m_controller->selectionMaskReads();
    }
    int readsDelta() const
    {
        return m_controller->selectionMaskReads() - m_readsBase;
    }
    bool selectRect(int x, int y, int w, int h,
                    const char *op = "replace")
    {
        QVariantMap rect;
        rect.insert(QStringLiteral("x"), x);
        rect.insert(QStringLiteral("y"), y);
        rect.insert(QStringLiteral("w"), w);
        rect.insert(QStringLiteral("h"), h);
        QVariantMap params;
        params.insert(QStringLiteral("source"), QStringLiteral("rect"));
        params.insert(QStringLiteral("rect"), rect);
        params.insert(QStringLiteral("op"), QLatin1String(op));
        params.insert(QStringLiteral("antialias"), true);
        return m_controller
            ->applyCommand(QStringLiteral("selection.set"), {}, params, {})
            .value(QStringLiteral("ok"))
            .toBool();
    }
    bool fillLayer0(double r, double g, double b)
    {
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"), QVariantList{r, g, b, 0.0});
        QVariantMap fill;
        fill.insert(QStringLiteral("content"), QStringLiteral("color"));
        fill.insert(QStringLiteral("color"), color);
        QVariantMap target;
        target.insert(QStringLiteral("layerId"),
                      m_controller->layers().first().toMap().value(
                          QStringLiteral("layerId")));
        return m_controller
            ->applyCommand(QStringLiteral("fill"), target, fill, {})
            .value(QStringLiteral("ok"))
            .toBool();
    }
    // Undo-stack position = the count of ACTIVE history rows (§40.8:
    // `position` is an absolute index in the undo stack, 0..=undoDepth;
    // † markers journal but occupy no stack slot — this suite creates
    // none, so the active-row count IS the position).
    int undoStackPosition() const
    {
        int n = 0;
        for (const QVariant &v : m_controller->historyEntries())
            if (v.toMap().value(QStringLiteral("active")).toBool())
                ++n;
        return n;
    }
    bool historyGoto(int position)
    {
        QVariantMap params;
        params.insert(QStringLiteral("position"), position);
        return m_controller
            ->applyCommand(QStringLiteral("history.goto"), {}, params, {})
            .value(QStringLiteral("ok"))
            .toBool();
    }
    // Engine coverage truth at one doc pixel (Grayscale8 through the
    // same door the ants/announcer use; 0 = uncovered, 255 = full).
    // NOTE: this bumps selectionMaskReads — never call it inside a
    // beginCount()/readsDelta() window.
    int coverageAt(int x, int y)
    {
        QImage px;
        if (!m_controller->readSelectionImage(QRect(x, y, 1, 1), &px))
            return -1;
        return qGray(px.pixel(0, 0));
    }
    // INDEPENDENT strip-scan ground truth (the pre-slice announcer
    // mechanism, kept test-local so case 5 never asserts the shared
    // read against itself).
    QRect stripScanBounds()
    {
        const int w = m_controller->docWidth();
        const int h = m_controller->docHeight();
        if (w <= 0 || h <= 0)
            return QRect();
        const int maxRows = qMax(1, (1 << 26) / qMax(1, w));
        QRect bounds;
        for (int y = 0; y < h; y += maxRows) {
            QImage strip;
            if (!m_controller->readSelectionImage(
                    QRect(0, y, w, qMin(maxRows, h - y)), &strip))
                return QRect();
            bounds = accumulateMaskBounds(strip, y, bounds);
        }
        return bounds;
    }
};

void TstRepaintScoping::initTestCase()
{
    m_controller = new DocumentController(&m_catalog);
    m_controller->setDeterministicIds(0x51CE7A11u);
    m_announcer = new Announcer(&m_annCatalog);
    m_controller->setAnnouncer(m_announcer);
    m_window = new QQuickWindow;
    m_window->resize(800, 600);
    m_canvas = new CountingCanvas(m_window->contentItem());
    m_canvas->setSize(QSizeF(800, 600));
    m_canvas->setController(m_controller);
    m_window->show();
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
}

void TstRepaintScoping::cleanupTestCase()
{
    m_controller->setAnnouncer(nullptr);
    m_canvas->setController(nullptr);
    delete m_window;
    delete m_announcer;
    delete m_controller;
}

void TstRepaintScoping::init()
{
    QVERIFY(m_controller->newDocument(640, 400, QStringLiteral("white")));
    QTest::qWait(60);
    m_canvas->zoomOneToOne();
    QCOMPARE(m_canvas->zoomPercent(), 100); // pins docToView()
    QTest::qWait(40);
}

void TstRepaintScoping::fillWithLiveSelectionIsPartialAndReadFree()
{
    // The selection sits INSIDE one 256-px engine tile column/row pair:
    // the engine's dirtyRects are §3.7 tile-coalesced (measured first
    // try here: a (100,100,200,100) rect straddling tile borders came
    // back as the (0,0,512,256)-shaped bbox, frac 0.277 — an honest
    // partial, but a mis-derived threshold; recorded). (270,270,40,40)
    // coalesces to at most one (256,256,256,144) doc-clipped tile rect
    // => <= ~0.08 of the 800x600 item.
    QVERIFY(selectRect(270, 270, 40, 40));
    beginCount();
    QVERIFY(fillLayer0(0.9, 0.2, 0.2));
    // F1: the fill is NOT a selection writer and moved no selection
    // state — the revision gate stays closed, so neither the ants nor
    // the announcer touched the mask door (synchronous truth).
    QCOMPARE(readsDelta(), 0);
    // F2: composite dirt rode the partial path — the repainted area is
    // the commit's (tile-coalesced) view rect, not the item. The repaint
    // delivery is async: QTRY rides out machine load.
    QTRY_VERIFY2_WITH_TIMEOUT(m_canvas->paintCalls >= 1,
                              "fill commit repainted nothing", 10000);
    drainPaints(); // count EVERY paint of this commit before the frac
    const double frac =
        m_canvas->maxPaintPx / (m_canvas->width() * m_canvas->height());
    QVERIFY2(frac < 0.1,
             qPrintable(QStringLiteral("fill-with-live-selection repaint "
                                       "full_frac=%1 (expected < 0.1)")
                            .arg(frac)));
    // Engine truth: the selection-scoped fill composed (inside red,
    // outside untouched white).
    QImage px;
    QVERIFY(m_controller->readRegionImage(QRect(280, 280, 1, 1), &px));
    QVERIFY2(px.pixelColor(0, 0).red() > 200
                 && px.pixelColor(0, 0).green() < 120,
             "fill did not land inside the live selection");
    QVERIFY(m_controller->readRegionImage(QRect(10, 10, 1, 1), &px));
    QCOMPARE(px.pixelColor(0, 0), QColor(Qt::white));
    QVERIFY(m_controller->selectionActive());
}

void TstRepaintScoping::selectionCommitSharesOneRead()
{
    beginCount();
    QVERIFY(selectRect(100, 100, 200, 100));
    QTest::qWait(80);
    // F4: ants revectorize AND announcer bounds scan both consumed the
    // mask — through ONE shared read (ants fill the per-revision cache
    // during pollNow's overlayChanged; the announcer's scan hits it).
    QCOMPARE(readsDelta(), 1);
    // The ants painted at the new boundary: exact dash pixels along the
    // rect's top edge (doc y=100 -> view y=200, doc x 100..300 -> view
    // 180..380; the 4-on/4-off cosmetic dash leaves exact 0x101010
    // pixels there).
    const QImage grab = m_window->grabWindow();
    QVERIFY(!grab.isNull());
    QVERIFY2(dashPixelCount(grab, QRect(185, 196, 190, 9)) > 0,
             "no ants dash pixels at the committed selection's top edge");
}

void TstRepaintScoping::panEscapeRefreshesAntsWindow()
{
    // Zoom in FIRST so the vectorization window is a sub-doc rect
    // (viewport 200x150 doc px + half-margin => ~400x300 window), then
    // commit a full-width band whose outlines cross every viewport.
    m_canvas->setZoomCentered(4.0);
    QTest::qWait(30);
    QVERIFY(selectRect(0, 180, 640, 40));
    QTest::qWait(80);
    const QImage before = m_window->grabWindow();
    QVERIFY2(dashPixelCount(before) > 0,
             "sanity: no ants visible after the band commit at 4x");
    // Pan LEFT by 1300 view px (325 doc px) in two middle-drags: the
    // final viewport (doc x ~545..640) lies FULLY outside the original
    // window (x <= ~520), so a stale vector would paint NOTHING there —
    // the escape check must have re-vectorized mid-pan.
    const auto drag = [&](const QPoint &from, int steps, int stepDx) {
        QTest::mousePress(m_window, Qt::MiddleButton, Qt::NoModifier, from);
        for (int i = 1; i <= steps; ++i) {
            QTest::mouseMove(m_window,
                             from + QPoint(stepDx * i, 0));
            QTest::qWait(4);
        }
        QTest::mouseRelease(m_window, Qt::MiddleButton, Qt::NoModifier,
                            from + QPoint(stepDx * steps, 0));
    };
    drag(QPoint(700, 300), 6, -100);  // -600 view px
    drag(QPoint(700, 300), 7, -100);  // -700 view px
    QTest::qWait(80);
    const QImage after = m_window->grabWindow();
    QVERIFY2(dashPixelCount(after) > 0,
             "ants missing after panning past the vectorized window — "
             "the escape recompute did not fire (F1c)");
}

void TstRepaintScoping::docWideOverlayAndGeometryStayFull()
{
    // Row 3: a guide is a view-spanning overlay — full item is the
    // honest scope, and it costs no mask read (no selection here).
    beginCount();
    QVariantMap params;
    params.insert(QStringLiteral("orientation"),
                  QStringLiteral("horizontal"));
    params.insert(QStringLiteral("position"), 120.0);
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("guide.add"), {}, params, {})
                .value(QStringLiteral("ok"))
                .toBool());
    QCOMPARE(readsDelta(), 0); // synchronous truth
    QTRY_VERIFY2_WITH_TIMEOUT(m_canvas->paintCalls >= 1,
                              "guide.add repainted nothing", 10000);
    drainPaints();
    double frac =
        m_canvas->maxPaintPx / (m_canvas->width() * m_canvas->height());
    QVERIFY2(frac > 0.9,
             qPrintable(QStringLiteral("guide.add full_frac=%1 "
                                       "(overlay must repaint the item)")
                            .arg(frac)));
    // Row 4: doc geometry change rides documentReset -> rebuild + full.
    beginCount();
    QVariantMap resize;
    resize.insert(QStringLiteral("width"), 800);
    resize.insert(QStringLiteral("height"), 400);
    resize.insert(QStringLiteral("anchor"), QStringLiteral("center"));
    resize.insert(QStringLiteral("relative"), false);
    resize.insert(QStringLiteral("background"), QStringLiteral("white"));
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("doc.canvas_resize"), {},
                               resize, {})
                .value(QStringLiteral("ok"))
                .toBool());
    QCOMPARE(m_controller->docWidth(), 800);
    QTRY_VERIFY2_WITH_TIMEOUT(m_canvas->paintCalls >= 1,
                              "canvas_resize repainted nothing", 10000);
    drainPaints();
    frac = m_canvas->maxPaintPx / (m_canvas->width() * m_canvas->height());
    QVERIFY2(frac > 0.9,
             qPrintable(QStringLiteral("canvas_resize full_frac=%1 "
                                       "(reset must repaint the item)")
                            .arg(frac)));
}

void TstRepaintScoping::announcementsUnchanged()
{
    // Drain any pending rate-limited announcements from earlier cases,
    // then pin THIS case's stream from a known log index.
    QTest::qWait(700);
    const int logBase = m_announcer->announcementLog().size();
    QVERIFY(selectRect(37, 53, 211, 97));
    // Independent ground truth: the pre-slice strip scan over the same
    // engine bytes (test-local; never the shared read).
    const QRect gt = stripScanBounds();
    QVERIFY(!gt.isNull());
    // The bounds emit MUST land before the deselect announces: both ride
    // source "selection", so an unflushed first entry would be
    // latest-wins-coalesced away. Condition-based (the flush timer can
    // fire late under machine load).
    QTRY_VERIFY_WITH_TIMEOUT(
        m_announcer->announcementLog().size() - logBase >= 1, 10000);
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("selection.none"), {}, {}, {})
                .value(QStringLiteral("ok"))
                .toBool());
    // The rate-limit flush timer can fire late under machine load: ride
    // it out (entries only ever append; the ==2 pin below stays exact).
    QTRY_VERIFY_WITH_TIMEOUT(
        m_announcer->announcementLog().size() - logBase >= 2, 10000);
    const QVariantList log = m_announcer->announcementLog();
    QVERIFY2(log.size() - logBase == 2,
             qPrintable(QStringLiteral("expected exactly 2 announcements "
                                       "(bounds, cleared), got %1")
                            .arg(log.size() - logBase)));
    const QVariantMap first = log.at(logBase).toMap();
    QCOMPARE(first.value(QStringLiteral("source")).toString(),
             QStringLiteral("selection"));
    QCOMPARE(first.value(QStringLiteral("id")).toString(),
             QStringLiteral("a11y.canvas.selection_bounds"));
    // BYTE-EQUAL to the strip-scan truth: same {width} x {height} text.
    QCOMPARE(first.value(QStringLiteral("text")).toString(),
             QStringLiteral("%1 x %2").arg(gt.width()).arg(gt.height()));
    const QVariantMap second = log.at(logBase + 1).toMap();
    QCOMPARE(second.value(QStringLiteral("source")).toString(),
             QStringLiteral("selection"));
    QCOMPARE(second.value(QStringLiteral("id")).toString(),
             QStringLiteral("a11y.canvas.selection_cleared"));
    QCOMPARE(second.value(QStringLiteral("text")).toString(),
             QStringLiteral("cleared"));
}

void TstRepaintScoping::kindGateCoversWritersAndContentChanges()
{
    // (a) A non-selection command with a live selection: no mask read
    // (the case-1 assert on a DIFFERENT kind class — info-only commit).
    QVERIFY(selectRect(100, 100, 200, 100));
    beginCount();
    QVariantMap add;
    add.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    add.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("layer.add"), {}, add, {})
                .value(QStringLiteral("ok"))
                .toBool());
    QTest::qWait(80);
    QCOMPARE(readsDelta(), 0);
    // (b) undo/redo of a WRITER kind bumps (labelMessageId carries the
    // STEP's kind on undo and redo — engine events.rs/doc.rs truth).
    const int revB = m_controller->selectionRevision();
    beginCount();
    m_controller->undo(); // undoes layer.add — NOT a writer, no bump
    QTest::qWait(40);
    QCOMPARE(m_controller->selectionRevision(), revB);
    QCOMPARE(readsDelta(), 0);
    m_controller->undo(); // undoes selection.set — writer kind: bump
    QTest::qWait(40);
    const int revAfterUndo = m_controller->selectionRevision();
    QVERIFY2(revAfterUndo > revB,
             "undo of selection.set did not bump the selection revision");
    m_controller->redo(); // redoes selection.set — bump again
    QTest::qWait(40);
    QVERIFY2(m_controller->selectionRevision() > revAfterUndo,
             "redo of selection.set did not bump the selection revision");
    // (c) THE KIND-SIDE DISCRIMINATOR: a content change with IDENTICAL
    // (active, bounds). Two corner dots => nonzero-coverage bounds span
    // the whole canvas; inverting selects the complement — same active
    // flag, same tight bounds, different pixels. Only the writer-kind
    // side of the gate can see it.
    QVERIFY(selectRect(0, 0, 10, 10));
    QVERIFY(selectRect(630, 390, 10, 10, "add"));
    QTest::qWait(80);
    const QRect boundsBefore = stripScanBounds();
    const int revC = m_controller->selectionRevision();
    beginCount();
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("selection.invert"), {}, {},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    QTest::qWait(80);
    QCOMPARE(stripScanBounds(), boundsBefore); // the state side IS blind
    QVERIFY(m_controller->selectionActive());
    QVERIFY2(m_controller->selectionRevision() > revC,
             "selection.invert with identical (active,bounds) did not "
             "bump — the writer-kind gate side is broken");
}

void TstRepaintScoping::historyGotoReplayBumpsThroughTheKindGate()
{
    // THE DAGGER SHAPE (§5 fix round, MAJOR-2 — reproduced twice by the
    // reviewer before the fix). Two corner dots put the tight nonzero
    // hull across the whole canvas; inverting keeps that hull AND the
    // active flag while flipping every covered pixel. So a history.goto
    // back across the invert changes selection CONTENT with (active,
    // bounds) IDENTICAL — the compare side of the gate is blind here BY
    // CONSTRUCTION — and the engine hands the shell exactly ONE
    // history.v1 row labelled "history.goto": the position form's
    // internal undo/redo run keeps only the replayed rows' dirty rects
    // and DISCARDS their kinds (engine mod.rs:1050-1135), and the cabi
    // emits receipt.kind (doc.rs:249). "selection.invert" never reaches
    // the host. Only the KIND side can see this, and only because
    // history.goto is in the writer set.
    QVERIFY(selectRect(0, 0, 20, 20));
    QVERIFY(selectRect(620, 380, 20, 20, "add"));
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("selection.invert"), {}, {},
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    QTest::qWait(80);
    const int posAfterInvert = undoStackPosition();
    const QRect boundsBefore = stripScanBounds();
    QVERIFY(!boundsBefore.isNull());
    QCOMPARE(coverageAt(5, 5), 0); // the invert zeroed dot 1
    const bool activeBefore = m_controller->selectionActive();
    const int revBefore = m_controller->selectionRevision();
    // Counted window opens LAST: coverageAt()/stripScanBounds() read
    // through the instrumented door themselves.
    beginCount();
    QVERIFY2(historyGoto(posAfterInvert - 1),
             "history.goto position form was refused");
    QTest::qWait(80);
    // THE PIN (synchronous truth, asserted before any test-local read):
    // the gate fired, so the shared mask cache is busted and the ants
    // re-vectorize. A dash-pixel grab CANNOT discriminate this shape — a
    // set and its complement share the same boundary, which is exactly
    // why it slipped past a pixel-level net.
    QVERIFY2(m_controller->selectionRevision() > revBefore,
             "history.goto changed selection content with identical "
             "(active,bounds) and the revision did not bump — the dagger "
             "kind pierced the gate's kind side (§5 MAJOR-2)");
    QTRY_VERIFY2_WITH_TIMEOUT(readsDelta() >= 1,
                              "no mask read after the goto — the shared "
                              "cache was not busted",
                              10000);
    // Engine truth, after the counted window: CONTENT moved back...
    QCOMPARE(coverageAt(5, 5), 255);
    // ...while both observables the compare side watches did NOT.
    QCOMPARE(m_controller->selectionActive(), activeBefore);
    QCOMPARE(stripScanBounds(), boundsBefore);
}

void TstRepaintScoping::boundsMovingGotoBumpsAndBustsTheCache()
{
    // Lifted from the §5 reviewer's probe (viii), which they ran against
    // the PRE-FIX writer set and named after arm (b). WHAT IT PINS
    // TODAY, stated honestly: a history.goto that MOVES the selection's
    // bounds bumps the revision, busts the shared cache and
    // re-vectorizes the ants — the bounds-moving half of the dagger
    // exposure, and the reviewer's measured MITIGATION bound on it.
    // WHAT IT NO LONGER PINS: arm (b)'s `prevSelBounds !=
    // m_selectionBounds` compare as the SOLE cause of the bump. The
    // probe discriminated only while history.goto sat OUTSIDE the writer
    // set; MAJOR-2's fix moved it in, so the kind side now fires here
    // too. Re-derived over all 102 registered kinds: every kind that can
    // move (active,bounds) at all carries a writer-set label, so no
    // shell-drivable case can isolate the compare side against THIS
    // engine. It is retained by derivation for the two things the kind
    // side cannot see — a future kind not yet in the list, and the
    // pollNow coalescing residual — and that reasoning lives at the gate
    // in document_controller.cpp.
    // MEASURED, not assumed (§5 fix-round mutation battery): deleting
    // the compare alone leaves this case GREEN (M2'); removing
    // history.goto from the writer set alone ALSO leaves it green (M1' —
    // the bump falls back to the compare, which is the reviewer's
    // original reading); removing BOTH reds it (M3'). So the case is a
    // live pin on the union, not on either side by itself.
    QVERIFY(selectRect(40, 40, 60, 60));
    QTest::qWait(40);
    const QRect boundsA = stripScanBounds();
    QVERIFY(!boundsA.isNull());
    QVERIFY(selectRect(300, 200, 120, 80));
    QTest::qWait(40);
    const QRect boundsB = stripScanBounds();
    QVERIFY2(boundsA != boundsB,
             "sanity: the two selection.set commits must differ in bounds");
    const int pos = undoStackPosition();
    const int revBefore = m_controller->selectionRevision();
    beginCount();
    QVERIFY2(historyGoto(pos - 1),
             "history.goto position form was refused");
    QTest::qWait(80);
    QVERIFY2(m_controller->selectionRevision() > revBefore,
             "a history.goto moved (active,bounds) and the revision did "
             "not bump");
    QTRY_VERIFY2_WITH_TIMEOUT(readsDelta() >= 1,
                              "no mask read after the bounds-moving goto",
                              10000);
    // The state really did move back to the first commit's bounds.
    QCOMPARE(stripScanBounds(), boundsA);
    QVERIFY(m_controller->selectionActive());
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
    TstRepaintScoping tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_repaint_scoping.moc"
