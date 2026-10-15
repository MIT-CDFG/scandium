// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// PERF PROBE HARNESS — bench-class, runnable ON DEMAND, and deliberately
// NOT part of the default green suite (wall-clock assertions would flake
// CI). Registration idiom: the target always BUILDS, but is registered
// with ctest only behind -DAPPNAME_BENCH_CTEST=ON, and even then carries
// LABELS "bench" so `ctest -LE bench` excludes it (see shell/CMakeLists.txt).
//
// INVOCATION (documented contract):
//   # Release numbers (the reportable config):
//   cmake -S shell -B target/shell-relbench -G Ninja \
//         -DCMAKE_BUILD_TYPE=Release
//   cmake --build target/shell-relbench --target tst_perf_probes
//   target/shell-relbench/tst_perf_probes
//   # or, with -DAPPNAME_BENCH_CTEST=ON at configure time:
//   ctest --test-dir target/shell-relbench -L bench
//   # Debug-config runs (triage only — numbers are NOT reportable): the
//   # unoptimized engine makes the heavy probes exceed QTest's default
//   # 300 s per-function watchdog (probeHistoryGrowth aborts SIGABRT) —
//   # run single light probes, or raise QTEST_FUNCTION_TIMEOUT.
//
// This probe suite is the SHELL-side masked-redraw cover (the desk's
// engine benches are non-mask): N-layer documents (1/4/8) with painted
// content, timed with vs without per-layer raster masks (mask.add door,
// §11.2 slot A serving, from:'selection') at the shell's own composite
// doors — (a) full-composite readback (readRegionImage full-doc, the
// canvas presentation path), (b) a dirty-region recomposite loop
// (repeated small brush strokes + region readbacks, the interactive
// redraw shape). Plus single-call costs of the synchronous ABI calls the
// pointer paths make, history-growth cost of the per-commit
// refreshHistory parse, and paint-invalidation counting on a real
// CanvasView (full-item update() vs the changed region).
//
// MEASUREMENT DISCIPLINE (session triage rules, binding):
//  - warm reps: every timed op runs 1 discarded warm-up + >=5 timed reps;
//    min and median are reported (never a first run, never a mean alone);
//  - in-process timing only (QElapsedTimer around the call) — no piped
//    stdout step timing (block-buffered-pipe artifact class);
//  - A/B/A pairing for config-flip probes: config A is measured, config B
//    is measured, config A is REBUILT (fresh document, identical recipe)
//    and re-measured; the A/A' drift bounds machine-load contamination.
//    Numbers whose A/A' drift exceeds ~10% are INDICATIVE-ONLY;
//  - load-average lines print at suite start and end;
//  - PROBE output lines are machine-grepable:
//      PROBE <name> <k>=<v>... min_us=<min> med_us=<median> reps=<n>
//
// The harness renders no controls (a11y-lint sweeps shell/qml only) and
// changes no product code: the paint-counting canvas is a subclass
// overriding the public virtual paint().

#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "selection_scan.h"

#include <QElapsedTimer>
#include <QGuiApplication>
#include <QPainter>
#include <QQuickWindow>
#include <QtTest>

#include <algorithm>
#include <cstdlib>
#include <optional>
#include <vector>

#ifndef APPNAME_PROBE_BUILD_CONFIG
#define APPNAME_PROBE_BUILD_CONFIG "unknown"
#endif

using namespace appname::shell;

namespace {

// Default (largest) document edge; the composite probes ALSO sweep a
// CANVAS-SIZE AXIS (512/1024/2048 = 0.25/1/4 MP) at a FIXED dirty-region
// size — the scaling curve localizes whether the engine recomposite is
// O(dirty) or O(total tiles) at the shell door (desk aud-perf r2 flag:
// every event re-flattens the tree and iterates ALL composite tiles).
constexpr int kDocW = 2048;
constexpr int kDocH = 2048;
constexpr int kDocEdges[] = {512, 1024, 2048};
constexpr int kReps = 7; // timed reps after 1 discarded warm-up

struct Stats {
    qint64 minUs = 0;
    qint64 medUs = 0;
    int reps = 0;
};

template <typename F>
Stats timeReps(int reps, F &&op)
{
    op(); // warm-up, discarded (cold-cache artifact class)
    std::vector<qint64> us;
    us.reserve(reps);
    QElapsedTimer t;
    for (int i = 0; i < reps; ++i) {
        t.start();
        op();
        us.push_back(t.nsecsElapsed() / 1000);
    }
    std::sort(us.begin(), us.end());
    Stats s;
    s.minUs = us.front();
    s.medUs = us.at(us.size() / 2);
    s.reps = reps;
    return s;
}

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

void probeLine(const QString &name, const QString &kv, const Stats &s)
{
    qInfo("PROBE %s config=%s %s min_us=%lld med_us=%lld reps=%d",
          qPrintable(name), APPNAME_PROBE_BUILD_CONFIG, qPrintable(kv),
          static_cast<long long>(s.minUs), static_cast<long long>(s.medUs),
          s.reps);
}

// Paint-counting canvas: overrides the public virtual paint() to record
// call count and the painter's clip area (QQuickPaintedItem clips paint()
// to the accumulated dirty rect, so the clip area IS the invalidated
// area actually repainted).
class ProbeCanvas : public CanvasView {
public:
    explicit ProbeCanvas(QQuickItem *parent = nullptr)
        : CanvasView(parent)
    {
    }
    void paint(QPainter *painter) override
    {
        ++paintCalls;
        const QRectF clip = painter->clipBoundingRect();
        // An empty clip means "no clip set" = full item.
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

} // namespace

class TstPerfProbes : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanupTestCase();

    void probeMaskedComposite();     // P1: 1/4/8 layers, A/B/A
    void probeDirtyRegionLoop();     // P2: stroke+readback loop, A/B/A
    void probeSingleCallCosts();     // P3: pointer-path sync ABI calls
    void probeHistoryGrowth();       // P4: per-commit cost vs history len
    void probePaintInvalidation();   // P5: paints + invalidated px/gesture
    void probeCommitPathScoping();   // P6: mask reads + repaint per commit class

private:
    Catalog m_catalog = Catalog::fromTexts({});
    DocumentController *m_controller = nullptr;

    // Deterministic document recipe: base layer + (layers-1) added raster
    // layers, each fully filled a distinct color plus two diagonal brush
    // strokes (non-uniform content so no flat-tile fast path can hide
    // compositing cost). withMasks adds ONE raster mask per layer from an
    // elliptical selection (non-trivial coverage incl. feathered edge).
    // docEdge parameterizes the canvas-size axis (square documents).
    bool buildDoc(int layers, bool withMasks, int docEdge = kDocW);
    void paintLayerContent(int layerIndex, double r, double g, double b);
    QString layerIdAt(int index) const;
    bool selectEllipse();

    // The size the CURRENT probe document was built at (set by buildDoc;
    // helpers derive geometry from it, never from the constants).
    int m_docW = kDocW;
    int m_docH = kDocH;
};

void TstPerfProbes::initTestCase()
{
    m_controller = new DocumentController(&m_catalog);
    m_controller->setDeterministicIds(0x9e3779b97f4a7c15ULL);
    qInfo("PROBE-SUITE config=%s doc_max=%dx%d doc_axis=512/1024/2048 "
          "reps=%d",
          APPNAME_PROBE_BUILD_CONFIG, kDocW, kDocH, kReps);
    printLoadLine("suite-start");
}

void TstPerfProbes::cleanupTestCase()
{
    printLoadLine("suite-end");
    delete m_controller;
    m_controller = nullptr;
}

QString TstPerfProbes::layerIdAt(int index) const
{
    const QVariantList rows = m_controller->layers();
    if (index < 0 || index >= rows.size())
        return QString();
    return rows.at(index).toMap().value(QStringLiteral("layerId")).toString();
}

void TstPerfProbes::paintLayerContent(int layerIndex, double r, double g,
                                      double b)
{
    m_controller->setActiveLayerIndex(layerIndex);
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{r, g, b, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerIdAt(layerIndex));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok"))
                .toBool());
    // Two diagonal strokes; offsets vary per layer so content differs.
    for (int s = 0; s < 2; ++s) {
        const double scale = m_docW / 2048.0; // recipe scales with the axis
        const double off = (120.0 * layerIndex + 260.0 * s + 64.0) * scale;
        m_controller->strokeBegin(QStringLiteral("brush.stroke"),
                                  QColor::fromRgbF(1.0 - r, 1.0 - g, b), 24.0,
                                  // O-29: no override => preset hardness, which
                                  // is what this recipe always meant (it used to
                                  // spell it as an explicit 0.0). Content
                                  // unchanged: the preset's hardness IS 0.0.
                                  std::nullopt);
        for (int i = 0; i <= 16; ++i) {
            const double t = i / 16.0;
            m_controller->strokeAppend(off + t * (m_docW - 2 * off),
                                       off * 0.5 + t * (m_docH - off),
                                       1.0, t * 0.3);
        }
        QVERIFY(m_controller->strokeEnd());
    }
}

bool TstPerfProbes::selectEllipse()
{
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), m_docW / 8);
    rect.insert(QStringLiteral("y"), m_docH / 8);
    rect.insert(QStringLiteral("w"), 3 * m_docW / 4);
    rect.insert(QStringLiteral("h"), 3 * m_docH / 4);
    QVariantMap params;
    params.insert(QStringLiteral("source"), QStringLiteral("ellipse"));
    params.insert(QStringLiteral("rect"), rect);
    params.insert(QStringLiteral("op"), QStringLiteral("replace"));
    params.insert(QStringLiteral("antialias"), true);
    return m_controller
        ->applyCommand(QStringLiteral("selection.set"), {}, params, {})
        .value(QStringLiteral("ok"))
        .toBool();
}

bool TstPerfProbes::buildDoc(int layers, bool withMasks, int docEdge)
{
    m_docW = m_docH = docEdge;
    if (!m_controller->newDocument(m_docW, m_docH, QStringLiteral("white")))
        return false;
    for (int i = 1; i < layers; ++i) {
        QVariantMap params;
        params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
        params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
        if (!m_controller->applyCommand(QStringLiteral("layer.add"), {},
                                        params, {})
                 .value(QStringLiteral("ok"))
                 .toBool())
            return false;
    }
    const int count = m_controller->layers().size();
    if (count != layers)
        return false;
    for (int i = 0; i < layers; ++i)
        paintLayerContent(i, 0.15 + 0.1 * i, 0.8 - 0.08 * i, 0.35);
    if (withMasks) {
        if (!selectEllipse())
            return false;
        for (int i = 0; i < layers; ++i) {
            QVariantMap target;
            target.insert(QStringLiteral("layerId"), layerIdAt(i));
            QVariantMap params;
            params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
            params.insert(QStringLiteral("from"), QStringLiteral("selection"));
            params.insert(QStringLiteral("newMaskId"), m_controller->newUuid());
            if (!m_controller->applyCommand(QStringLiteral("mask.add"), target,
                                            params, {})
                     .value(QStringLiteral("ok"))
                     .toBool())
                return false;
        }
        // The probe times COMPOSITE cost, not selection overlay state.
        // C-398: check it, like the six operations above. If this silently
        // fails the selection stays active and the probe times a composite
        // WITH the overlay -- precisely what this line exists to prevent, and
        // it would look like a slow composite rather than a failed reset.
        if (!m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                                        {})
                 .value(QStringLiteral("ok"))
                 .toBool())
            return false;
    }
    return true;
}

// P1 — masked vs unmasked full-composite readback, over BOTH axes:
// canvas size (512/1024/2048 — the r2 scaling-curve requirement) and
// layer count (1/4/8). A/B/A per cell: unmasked doc (A), masked doc (B),
// unmasked doc rebuilt (A').
void TstPerfProbes::probeMaskedComposite()
{
    for (const int edge : kDocEdges) {
        for (const int layers : {1, 4, 8}) {
            Stats a, b, a2;
            QImage img;
            const auto readFull = [&] {
                QVERIFY(m_controller->readRegionImage(
                    QRect(0, 0, m_docW, m_docH), &img));
            };
            const QString kv =
                QStringLiteral("doc=%1 layers=%2").arg(edge).arg(layers);
            QVERIFY(buildDoc(layers, false, edge));
            a = timeReps(kReps, readFull);
            QVERIFY(buildDoc(layers, true, edge));
            b = timeReps(kReps, readFull);
            QVERIFY(buildDoc(layers, false, edge));
            a2 = timeReps(kReps, readFull);
            probeLine(QStringLiteral("composite-full"),
                      kv + QStringLiteral(" masked=0 leg=A"), a);
            probeLine(QStringLiteral("composite-full"),
                      kv + QStringLiteral(" masked=1 leg=B"), b);
            probeLine(QStringLiteral("composite-full"),
                      kv + QStringLiteral(" masked=0 leg=A2"), a2);
            qInfo("PROBE composite-full-ratio doc=%d layers=%d "
                  "masked_over_unmasked_med=%.3f aba_drift=%.3f",
                  edge, layers,
                  a.medUs > 0 ? double(b.medUs) / double(a.medUs) : 0.0,
                  a.medUs > 0 ? double(a2.medUs) / double(a.medUs) : 0.0);
        }
    }
}

// P2 — the interactive redraw shape: small brush stroke commit + the
// dirty-region readback the canvas performs (refreshRegion's shape:
// stroke bbox + brush margin). Per-iteration timing; A/B/A over masks;
// swept over the CANVAS-SIZE AXIS at a FIXED stroke/dirty size — the
// r2 localization: if the fixed-size readback grows with TOTAL canvas
// px, the engine recomposite is O(total tiles) at this door, not
// O(dirty).
void TstPerfProbes::probeDirtyRegionLoop()
{
    const int layers = 8;
    struct LoopStats {
        Stats total, stroke, readback;
    };
    // One iteration = one small stroke COMMIT (strokeEnd: samples ingest +
    // engine apply + pollNow/info/history — the per-commit UI-thread
    // chain) + the dirty-region readback the canvas performs. The two
    // halves are timed separately so mask cost attributes honestly.
    const auto runLoop = [&]() -> LoopStats {
        std::vector<qint64> total, stroke, readback;
        QElapsedTimer t;
        for (int i = 0; i <= kReps; ++i) { // i==0 is the discarded warm-up
            const int cx = 200 + (i * 173) % (m_docW - 400);
            const int cy = 200 + (i * 131) % (m_docH - 400);
            t.start();
            m_controller->strokeBegin(QStringLiteral("brush.stroke"),
                                      QColor(200, 40, 40), 8.0,
                                      // O-29: no override => preset hardness (0.0
                                      // on the built-in) — recipe content unchanged.
                                      std::nullopt);
            for (int p = 0; p <= 8; ++p)
                m_controller->strokeAppend(cx + p * 5, cy + p * 5, 1.0,
                                           p * 0.01);
            if (!m_controller->strokeEnd())
                qFatal("dirty-loop: strokeEnd refused");
            const qint64 strokeUs = t.nsecsElapsed() / 1000;
            const QRect dirty(cx - 8, cy - 8, 8 * 5 + 16, 8 * 5 + 16);
            QImage region;
            t.start();
            if (!m_controller->readRegionImage(
                    dirty.intersected(QRect(0, 0, m_docW, m_docH)), &region))
                qFatal("dirty-loop: readRegionImage refused");
            const qint64 readUs = t.nsecsElapsed() / 1000;
            if (i == 0)
                continue;
            stroke.push_back(strokeUs);
            readback.push_back(readUs);
            total.push_back(strokeUs + readUs);
        }
        const auto stats = [](std::vector<qint64> &v) {
            std::sort(v.begin(), v.end());
            Stats s;
            s.minUs = v.front();
            s.medUs = v.at(v.size() / 2);
            s.reps = static_cast<int>(v.size());
            return s;
        };
        return {stats(total), stats(stroke), stats(readback)};
    };
    for (const int edge : kDocEdges) {
        const auto report = [&](const char *leg, int masked,
                                const LoopStats &s) {
            const QString kv =
                QStringLiteral("doc=%1 layers=%2 masked=%3 leg=%4 ")
                    .arg(edge).arg(layers).arg(masked)
                    .arg(QLatin1String(leg));
            probeLine(QStringLiteral("dirty-loop"),
                      kv + QStringLiteral("part=total"), s.total);
            probeLine(QStringLiteral("dirty-loop"),
                      kv + QStringLiteral("part=stroke"), s.stroke);
            probeLine(QStringLiteral("dirty-loop"),
                      kv + QStringLiteral("part=readback"), s.readback);
        };
        QVERIFY(buildDoc(layers, false, edge));
        const LoopStats a = runLoop();
        QVERIFY(buildDoc(layers, true, edge));
        const LoopStats b = runLoop();
        QVERIFY(buildDoc(layers, false, edge));
        const LoopStats a2 = runLoop();
        report("A", 0, a);
        report("B", 1, b);
        report("A2", 0, a2);
        qInfo("PROBE dirty-loop-ratio doc=%d layers=%d "
              "masked_over_unmasked_med=%.3f stroke_ratio=%.3f "
              "readback_ratio=%.3f aba_drift=%.3f",
              edge, layers,
              a.total.medUs > 0
                  ? double(b.total.medUs) / double(a.total.medUs)
                  : 0.0,
              a.stroke.medUs > 0
                  ? double(b.stroke.medUs) / double(a.stroke.medUs)
                  : 0.0,
              a.readback.medUs > 0
                  ? double(b.readback.medUs) / double(a.readback.medUs)
                  : 0.0,
              a.total.medUs > 0
                  ? double(a2.total.medUs) / double(a.total.medUs)
                  : 0.0);
    }
}

// P3 — single-call costs of the synchronous ABI work the pointer paths
// trigger (all on the UI thread in the app):
//  - readLayer full-doc        = move-tool PRESS echo capture
//  - readSelection full-doc    = ants revectorization input (paint())
//  - selection-bounds scan     = announcer cost per applied selection.*
//  - applyCommand round trip   = per-commit chain (apply + poll +
//                                refreshInfo + refreshHistory)
//  - commandAvailability       = per-keypress chord/nudge gate
void TstPerfProbes::probeSingleCallCosts()
{
    const int layers = 8;
    QVERIFY(buildDoc(layers, true));
    QVERIFY(selectEllipse());
    const QString activeId = layerIdAt(0);
    const QRect full(0, 0, m_docW, m_docH);

    QImage img;
    // ENGINE TRUTH (first-hand, crates/appname-cabi/src/composite.rs
    // layer_region_rgba8): a layer carrying ANY mask refuses per-layer
    // readback typed ("mask application is the render row") — so on
    // masked layers the app's move-echo capture (canvas_view
    // mousePressEvent, item 10) always takes the null fallback. Record
    // the refusal cost here; the SERVED readLayer is timed on an
    // unmasked document below.
    img = m_controller->layerThumbImage(activeId);
    QVERIFY2(img.isNull(),
             "masked-layer readLayer unexpectedly served — the refusal "
             "matrix changed; re-derive this probe");
    probeLine(QStringLiteral("call-readLayer-masked-refusal"),
              QStringLiteral("layers=%1 refused=typed").arg(layers),
              timeReps(kReps, [&] {
                  img = m_controller->layerThumbImage(activeId);
              }));
    probeLine(QStringLiteral("call-readSelection-fulldoc"),
              QStringLiteral("layers=%1").arg(layers), timeReps(kReps, [&] {
                  QVERIFY(m_controller->readSelectionImage(full, &img));
              }));
    probeLine(QStringLiteral("call-selection-bounds-scan"),
              QStringLiteral("layers=%1").arg(layers), timeReps(kReps, [&] {
                  const int maxRows =
                      qMax(1, (1 << 26) / qMax(1, m_docW));
                  QRect bounds;
                  for (int y = 0; y < m_docH; y += maxRows) {
                      QImage strip;
                      QVERIFY(m_controller->readSelectionImage(
                          QRect(0, y, m_docW, qMin(maxRows, m_docH - y)),
                          &strip));
                      bounds = accumulateMaskBounds(strip, y, bounds);
                  }
                  QVERIFY(!bounds.isNull());
              }));
    probeLine(QStringLiteral("call-applyCommand-selection-set"),
              QStringLiteral("layers=%1").arg(layers),
              timeReps(kReps, [&] { QVERIFY(selectEllipse()); }));
    probeLine(QStringLiteral("call-commandAvailability"),
              QStringLiteral("layers=%1").arg(layers), timeReps(kReps, [&] {
                  const QVariantMap m = m_controller->commandAvailability();
                  QVERIFY(!m.isEmpty());
              }));
    // The SERVED per-layer readback (the move-echo capture cost on an
    // unmasked layer): full-doc readLayer, 8-layer unmasked document.
    QVERIFY(buildDoc(layers, false));
    const QString unmaskedId = layerIdAt(0);
    probeLine(QStringLiteral("call-readLayer-fulldoc"),
              QStringLiteral("layers=%1 masked=0").arg(layers),
              timeReps(kReps, [&] {
                  img = m_controller->layerThumbImage(unmaskedId);
                  QVERIFY(!img.isNull());
              }));
}

// P4 — per-commit UI-thread cost vs history length: refreshHistory
// re-parses the FULL entries array after every history event, so the
// per-commit chain grows with session length. Timed op is an identical
// selection.set at history length ~10 vs ~310. (Filler op is a small
// selection-scoped fill; fill_apply is a known-noisy ENGINE bench — the
// filler is not timed here, only the selection.set round trip is.)
void TstPerfProbes::probeHistoryGrowth()
{
    QVERIFY(buildDoc(1, false));
    const Stats early =
        timeReps(kReps, [&] { QVERIFY(selectEllipse()); });
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.4, 0.4, 0.4, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerIdAt(0));
    for (int i = 0; i < 300; ++i)
        QVERIFY(m_controller
                    ->applyCommand(QStringLiteral("fill"), target, fill, {})
                    .value(QStringLiteral("ok"))
                    .toBool());
    const Stats late = timeReps(kReps, [&] { QVERIFY(selectEllipse()); });
    probeLine(QStringLiteral("history-growth"),
              QStringLiteral("hist=early(~10)"), early);
    probeLine(QStringLiteral("history-growth"),
              QStringLiteral("hist=late(~310)"), late);
    qInfo("PROBE history-growth-ratio late_over_early_med=%.3f",
          early.medUs > 0 ? double(late.medUs) / double(early.medUs) : 0.0);
}

// P5 — paint-invalidation truth on a REAL CanvasView (software scene
// graph, offscreen): counts paint() calls and the invalidated area the
// scene graph actually repaints, per synthesized gesture. Contrasts the
// gesture paths (full-item update() per move) with the refreshRegion
// path (partial update(rect)) and the post-commit infoChanged repaint.
// Frame pacing offscreen differs from live cocoa/Metal — the COUNTS and
// AREAS are structural evidence; the per-paint timings are indicative.
void TstPerfProbes::probePaintInvalidation()
{
    QQuickWindow window;
    window.resize(1280, 800);
    ProbeCanvas *canvas = new ProbeCanvas(window.contentItem());
    canvas->setSize(QSizeF(1280, 800));
    canvas->setController(m_controller);
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));
    QVERIFY(buildDoc(4, false));
    canvas->zoomOneToOne();
    QTest::qWait(60); // settle initial paints
    const double itemPx = 1280.0 * 800.0;

    const auto drag = [&](const char *tool, const QPoint &from,
                          const QPoint &to, int moves) {
        canvas->setProperty("tool", QLatin1String(tool));
        QTest::qWait(30);
        canvas->resetCounters();
        QTest::mousePress(&window, Qt::LeftButton, Qt::NoModifier, from);
        for (int i = 1; i <= moves; ++i) {
            const QPoint p = from + (to - from) * i / moves;
            QTest::mouseMove(&window, p);
            QTest::qWait(4); // ~high-frequency input against timer frames
        }
        QTest::mouseRelease(&window, Qt::LeftButton, Qt::NoModifier, to);
        QTest::qWait(40); // drain pending frames
        qInfo("PROBE paint-gesture config=%s tool=%s moves=%d paints=%d "
              "painted_px_per_paint=%.0f item_px=%.0f full_frac=%.3f "
              "paint_us_total=%lld",
              APPNAME_PROBE_BUILD_CONFIG, tool, moves, canvas->paintCalls,
              canvas->paintCalls > 0
                  ? canvas->paintedPx / canvas->paintCalls
                  : 0.0,
              itemPx,
              canvas->paintCalls > 0
                  ? (canvas->paintedPx / canvas->paintCalls) / itemPx
                  : 0.0,
              static_cast<long long>(canvas->paintUs));
    };
    drag("select.rect", QPoint(200, 200), QPoint(500, 420), 60);
    drag("brush.stroke", QPoint(220, 240), QPoint(520, 460), 60);

    // The partial path: a small selection-scoped fill commit — dirtyRects
    // arrive via pollNow -> refreshRegion -> update(viewRect) (partial).
    // HISTORY (pre-repaint-scoping): the SAME commit's history event
    // fired the blanket infoChanged -> update() (full item), subsuming
    // the partial path — the sweep's F2 finding, full_frac 1.000. The
    // scoped overlayChanged now leaves composite dirt on the partial
    // path; the measured area shows which era this build is in.
    canvas->setProperty("tool", QLatin1String("select.rect"));
    QTest::qWait(30);
    QTest::mousePress(&window, Qt::LeftButton, Qt::NoModifier,
                      QPoint(300, 300));
    QTest::mouseMove(&window, QPoint(340, 340));
    QTest::mouseRelease(&window, Qt::LeftButton, Qt::NoModifier,
                        QPoint(340, 340));
    QTest::qWait(40);
    canvas->resetCounters();
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.9, 0.2, 0.2, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerIdAt(0));
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok"))
                .toBool());
    QTest::qWait(60);
    qInfo("PROBE paint-small-commit config=%s dirty_doc_px=%d paints=%d "
          "painted_px_per_paint=%.0f full_frac=%.3f",
          APPNAME_PROBE_BUILD_CONFIG, 40 * 40, canvas->paintCalls,
          canvas->paintCalls > 0 ? canvas->paintedPx / canvas->paintCalls
                                 : 0.0,
          canvas->paintCalls > 0
              ? (canvas->paintedPx / canvas->paintCalls) / itemPx
              : 0.0);
    canvas->setController(nullptr);
}

// P6 — commit-path scoping truth (the F1/F2/F4 repaint-scoping slice's
// acceptance instrument): with a REAL CanvasView AND a wired announcer
// (announceSelectionState runs its bounds scan whenever the pointer is
// set; texts resolve through synthetic rows so the emission path is
// faithful), counts the synchronous selection-mask ABI reads
// (DocumentController::selectionMaskReads), paint calls, and the
// repainted fraction per commit class, at the 2048^2 doc the sweep's
// headline numbers used:
//  - selection-commit: selection.set (the marquee-release shape) — the
//    ants revectorize AND the announcer bounds scan both consume mask
//    reads (pre-fix: one full-doc read EACH; post-fix: ONE shared);
//  - fill-commit-live-selection: a small selection-scoped fill with the
//    selection LIVE — the F1 finding's shape (pre-fix: the ants re-read
//    the full mask on the next paint because the revision bumps on ANY
//    history event, and infoChanged forces a full-item repaint;
//    post-fix: zero reads, partial repaint);
//  - pan-steps: middle-drag pan with the selection live (reads stay 0
//    in both eras — regression guard for the windowed-ants follow-on).
void TstPerfProbes::probeCommitPathScoping()
{
    QQuickWindow window;
    window.resize(1280, 800);
    ProbeCanvas *canvas = new ProbeCanvas(window.contentItem());
    canvas->setSize(QSizeF(1280, 800));
    canvas->setController(m_controller);
    const Catalog annCatalog = Catalog::fromTexts(
        {{QStringLiteral("t"),
          QStringLiteral("{ \"a11y.canvas.selection_bounds\": "
                         "\"{width} x {height}\","
                         "  \"a11y.canvas.selection_cleared\": "
                         "\"cleared\" }")}});
    Announcer announcer(&annCatalog);
    m_controller->setAnnouncer(&announcer);
    window.show();
    QVERIFY(QTest::qWaitForWindowExposed(&window));
    QVERIFY(buildDoc(4, false)); // 2048^2 — the sweep's headline size
    canvas->zoomOneToOne();
    QTest::qWait(60);
    const double itemPx = 1280.0 * 800.0;

    int readsBase = 0;
    QElapsedTimer wall;
    const auto beginScenario = [&] {
        QTest::qWait(40); // drain pending frames from setup
        canvas->resetCounters();
        readsBase = m_controller->selectionMaskReads();
        wall.start();
    };
    const auto report = [&](const char *scenario) {
        const qint64 wallUs = wall.nsecsElapsed() / 1000;
        QTest::qWait(60); // let queued repaints land before counting
        qInfo("PROBE commit-scoping config=%s scenario=%s reads=%d "
              "paints=%d painted_px_per_paint=%.0f full_frac=%.3f "
              "paint_us_total=%lld wall_us=%lld",
              APPNAME_PROBE_BUILD_CONFIG, scenario,
              m_controller->selectionMaskReads() - readsBase,
              canvas->paintCalls,
              canvas->paintCalls > 0
                  ? canvas->paintedPx / canvas->paintCalls
                  : 0.0,
              canvas->paintCalls > 0
                  ? (canvas->paintedPx / canvas->paintCalls) / itemPx
                  : 0.0,
              static_cast<long long>(canvas->paintUs),
              static_cast<long long>(wallUs));
    };

    // S1: the marquee-release shape (large ellipse, the P3 recipe).
    beginScenario();
    QVERIFY(selectEllipse());
    report("selection-commit");

    // S2: a small selection (the P5 40x40 shape) then a fill commit with
    // the selection LIVE — only the fill is measured.
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), 300);
    rect.insert(QStringLiteral("y"), 300);
    rect.insert(QStringLiteral("w"), 40);
    rect.insert(QStringLiteral("h"), 40);
    QVariantMap selParams;
    selParams.insert(QStringLiteral("source"), QStringLiteral("rect"));
    selParams.insert(QStringLiteral("rect"), rect);
    selParams.insert(QStringLiteral("op"), QStringLiteral("replace"));
    selParams.insert(QStringLiteral("antialias"), true);
    QVERIFY(m_controller
                ->applyCommand(QStringLiteral("selection.set"), {}, selParams,
                               {})
                .value(QStringLiteral("ok"))
                .toBool());
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{0.9, 0.2, 0.2, 0.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("content"), QStringLiteral("color"));
    fill.insert(QStringLiteral("color"), color);
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerIdAt(0));
    beginScenario();
    QVERIFY(m_controller->applyCommand(QStringLiteral("fill"), target, fill, {})
                .value(QStringLiteral("ok"))
                .toBool());
    report("fill-commit-live-selection");

    // S3: pan with the selection live (middle-drag, 10 steps).
    beginScenario();
    QTest::mousePress(&window, Qt::MiddleButton, Qt::NoModifier,
                      QPoint(600, 400));
    for (int i = 1; i <= 10; ++i) {
        QTest::mouseMove(&window, QPoint(600 - 30 * i, 400 - 20 * i));
        QTest::qWait(4);
    }
    QTest::mouseRelease(&window, Qt::MiddleButton, Qt::NoModifier,
                        QPoint(300, 200));
    report("pan-steps");

    m_controller->setAnnouncer(nullptr);
    canvas->setController(nullptr);
}

// Offscreen + the software scene graph (QQuickPaintedItem paint() runs
// deterministically there); the engine links in (cabi).
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    if (qgetenv("QT_QUICK_BACKEND").isEmpty())
        qputenv("QT_QUICK_BACKEND", "software");
    QGuiApplication app(argc, argv);
    TstPerfProbes tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_perf_probes.moc"
