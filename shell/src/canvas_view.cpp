// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header.

#include "canvas_view.h"

#include <QCursor>
#include <QGuiApplication>
#include <QHoverEvent>
#include <QJsonArray>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QWheelEvent>

#include <algorithm>
#include <QtMath>

#include <cmath>
#include <utility>

namespace appname::shell {

namespace {

constexpr double kMinZoom = 1.0 / 64.0;
constexpr double kMaxZoom = 64.0;
constexpr int kMaxPendingEchoes = 64;
constexpr int kReadChunk = 2048; // chunked full refresh; each read ≤ 2^22 px (« the 2^26 ABI cap)

// ---- THE PALETTE-SPELLING -> (WIRE KIND, MODE) TABLE ----
// ONE table, walked by a test (tst_gesture_net::
// paletteModeToolsReachTheEngineAsTheirFamilyKind), because this is the
// FIRST place the palette spelling and the wire kind diverge: m_tool is
// "dodge_burn.dodge" and the engine kind is "dodge_burn.stroke". A second
// spelling of this mapping anywhere is how the next tool added gets it
// wrong.
//
// THE MODE IS NOT A PREFERENCE - it is the tool the user already picked.
// No options surface is needed to know it, which is why these palette
// entries light up with no new UI.
//
// ★★★ MOVED OUT OF mousePressEvent 2026-08-29, AND THE MOVE WAS A BUG FIX,
// NOT TIDYING. While the table lived inside the mouse handler it was
// reachable ONLY from a mouse press, and `tourStroke` - the path the
// screenshot tour drives, and the ONLY path that produces operator-facing
// evidence - took a wire kind straight from its caller and called
// strokeBegin with NO mode at all. So the tour could never have shown a
// single mode tool working: dodge, burn, blur, sharpen (live since
// 08-28) and heal (Q-D1(a)) would each have been sent with mode absent
// and refused, and the frame would have recorded the pre-fix failure
// while the real mouse path was fine. ★★ A SECOND DISPATCH PATH THAT
// SILENTLY DISAGREES WITH THE FIRST IS WORSE THAN NO SECOND PATH, because
// the one that disagrees is the one that gets photographed.
struct ModeTool { const char *tool; const char *kind; const char *mode; };
constexpr ModeTool kModeTools[] = {
    {"dodge_burn.dodge",     "dodge_burn.stroke",   "dodge"},
    {"dodge_burn.burn",      "dodge_burn.stroke",   "burn"},
    {"blur_sharpen.blur",    "blur_sharpen.stroke", "blur"},
    {"blur_sharpen.sharpen", "blur_sharpen.stroke", "sharpen"},
    // Q-D1 RULED (a) BY THE OPERATOR 2026-08-29: `J` heals with the
    // auto-chosen content-aware source. It is the first row whose palette
    // spelling EQUALS its wire kind - heal is here not because the name
    // diverges but because the MODE must be stated, and until now every
    // member exercised the kind half, which made the two halves look like
    // one thing.
    //
    // WHY `spot` NEEDS NOTHING ELSE, from the contract and not from hope:
    // heal.stroke requires ONLY brushPreset, and its mode enum is
    // [healing, spot]. The `healing` branch demands source+offset (or
    // pattern); the `spot` branch REMOVES that requirement - and FORBIDS
    // source/offset outright (allOf[1]). buildStrokeEnvelope has no
    // source-or-offset path at all, so the envelope satisfies the `not`
    // by construction rather than by care. ** IF A SOURCE-POINT
    // AFFORDANCE IS EVER ADDED FOR CLONE, THAT FORBID TURNS THIS WORKING
    // ROW INTO A REFUSAL. **
    //
    // The operator's "content-aware" is the SCHEMA DEFAULT (spotFill:
    // content_aware), not a param we send.
    {"heal.stroke",          "heal.stroke",         "spot"},
    // ★★ Q-D3 RULED (b) BY THE OPERATOR 2026-08-29: sponge -> desaturate,
    // "the classic default", WITH A LABEL so the interim mapping is never
    // silent. This row used to be a deliberate ABSENCE with its
    // measurement written beside it: dodge_burn.stroke's mode enum is
    // [dodge, burn, saturate, desaturate] and there is NO `sponge`, so
    // the palette spelled a tool the wire cannot name, and guessing would
    // have shipped a tool that silently does the wrong one of two
    // OPPOSITE things. It was routed instead of guessed, and the ruling
    // is what changed - not the measurement.
    //
    // ★ THE LABEL IS PART OF THE RULING, NOT DECORATION ON IT. The
    // palette renders "Sponge (desaturate)" so a user is never told
    // "Sponge" while getting one arbitrary half of a two-way choice. The
    // saturate/desaturate sub-control is filed as its own row.
    {"dodge_burn.sponge",    "dodge_burn.stroke",   "desaturate"},
};

// Every palette spelling that reaches the engine as a stroke, with what
// it reaches it AS. `dispatches` is false for a tool that is armed but
// not live - and that distinction is the one smudge was on the wrong side
// of for a whole slice: the palette could select it, the cursor changed,
// and a drag produced NOTHING. A TOOL THAT IS NEVER DISPATCHED CANNOT
// EVEN BE REFUSED, because the refusal channel fires when the ENGINE
// answers and the engine was never asked.
struct StrokeDispatch { QString kind; QString mode; bool dispatches = false; };
StrokeDispatch resolveStrokeDispatch(const QString &tool)
{
    for (const ModeTool &mt : kModeTools)
        if (tool == QLatin1String(mt.tool))
            return {QLatin1String(mt.kind), QLatin1String(mt.mode), true};
    // Kinds whose palette spelling IS the wire kind and which need no
    // mode. ZERO NEW PARAMS for these, measured rather than hoped:
    // buildStrokeEnvelope emits brushPreset/sizePx/opacity/flow for ANY
    // kind (branching on kind only for brush's hardness), and each of
    // these schemas - additionalProperties:false, so a stray param is a
    // rejection and not a tolerance - lists exactly those.
    if (tool == QLatin1String("brush.stroke")
        || tool == QLatin1String("eraser.stroke")
        || tool == QLatin1String("smudge.stroke")
        // B-02. Clone could not join kModeTools above: that table maps a
        // palette spelling onto a kind AND A MODE, and clone has no mode
        // that removes its source requirement — which is exactly why it
        // outlived dodge/burn/blur/sharpen/sponge as the dead one. It
        // belongs HERE, where the palette spelling IS the wire kind; its
        // extra required params (source, offset) are not a mode but ARMED
        // STATE, supplied at the call site from the Alt-click anchor.
        || tool == QLatin1String("clone.stroke"))
        return {tool, QString(), true};
    return {};
}

} // namespace

CanvasView::CanvasView(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    setAcceptedMouseButtons(Qt::LeftButton | Qt::MiddleButton);
    setAcceptHoverEvents(true); // polygon preview segment tracks the cursor
    setOpaquePainting(true);
    setFlag(ItemHasContents, true);
    // Review A11Y-2 D1: QQuickPaintedItem does NOT repaint on focus
    // change — without this, the focus ring goes stale (painted around
    // an unfocused pane, or missing on a focused one) until an
    // unrelated repaint.
    connect(this, &QQuickItem::activeFocusChanged, this,
            [this] { update(); });
    applyToolCursor(); // the default tool's cursor, before any switch (P4-D2)
}

void CanvasView::setController(DocumentController *controller)
{
    if (m_controller == controller)
        return;
    if (m_controller)
        disconnect(m_controller, nullptr, this, nullptr);
    m_controller = controller;
    if (m_controller) {
        connect(m_controller, &DocumentController::regionsDirty, this,
                &CanvasView::onRegionsDirty);
        connect(m_controller, &DocumentController::documentReset, this,
                &CanvasView::onDocumentReset);
        // A document swap or geometry change invalidates every in-flight
        // tool session's coordinates (review V-D4a: committing a pen/
        // polygon session started in the PREVIOUS document would land
        // stale geometry in the new one).
        connect(m_controller, &DocumentController::documentReset, this,
                &CanvasView::cancelGesture);
        connect(m_controller, &DocumentController::strokeCommitted, this,
                &CanvasView::onStrokeCommitted);
        // Overlay truth (ants/guides/grid) changes with no composite
        // dirty rect (selection kinds are dirty:null; guide/grid are
        // overlay-scope) — the controller's refreshInfo detects exactly
        // those transitions and emits overlayChanged (repaint-scoping
        // slice, sweep F2: the old blanket infoChanged->update() made
        // EVERY commit a full-item repaint, subsuming the partial
        // regionsDirty path; review D3's overlay-repaint need is met by
        // the scoped signal).
        connect(m_controller, &DocumentController::overlayChanged, this,
                &CanvasView::onOverlayChanged);
        // The document can VANISH without a reset: undoing doc.new (the
        // engine allows it — it is a command like any other) leaves a
        // pre-init handle. Drop the stale cache and the previews that
        // referenced the gone document.
        connect(m_controller, &DocumentController::infoChanged, this, [this]() {
            if (m_controller && !m_controller->hasDocument() && !m_cache.isNull()) {
                m_cache = QImage();
                m_cacheOrigin = QPoint(); // never outlive the cache it indexes
                m_pendingEchoes.clear();
                // ★★ ROUND-5 MINOR-1 — THE THIRD FLAG CLEAR MUST DISARM
                // THE CONTROLLER ITSELF. This clear happens OUTSIDE
                // cancelGesture(), which is the shape round 2 removed
                // from onDocumentReset: with the flag already false, the
                // NEXT document's documentReset finds cancelGesture's
                // `if (m_strokeInProgress)` arm DEAD, so nothing disarms
                // the controller and the VANISHED document's samples ride
                // into the new one — V-D4a, one strokeEnd() away, and
                // strokeEnd() is Q_INVOKABLE (document_controller.h:389).
                // Not reachable through the canvas today (the only
                // strokeEnd() callers are mouseReleaseEvent, gated on the
                // flag, and tourStroke, which calls strokeBegin first),
                // but "one call away" is this class's own stated reason
                // for strokeCancel() existing, so it is closed rather
                // than recorded. Flag FIRST, controller second — the
                // same ordering cancelGesture states below.
                m_strokeInProgress = false;
                // NARROW ON PURPOSE, not cancelGesture(). cancelGesture()
                // is the single-owner form and it is what onDocumentReset
                // got, but its driver does not exist here: onDocumentReset
                // and cancelGesture are BOTH in documentReset's fan-out, so
                // a clear in the first made the second's arm dead in the
                // same emission. This lambda is in no fan-out with
                // cancelGesture. Calling it here would additionally drop
                // transform/perspective/puppet/lasso/polygon/pen sessions
                // and emit transformStateChanged + perspectiveStateChanged
                // from INSIDE an infoChanged emission — a new signal-in-
                // signal edge on a path with no coverage, i.e. exactly the
                // kind of scope expansion round 4 could only document
                // rather than measure. Disarming is the whole finding.
                // (No null check: the enclosing condition already
                // established m_controller.)
                m_controller->strokeCancel();
                m_activeEcho = EchoStroke{};
                emit pendingPreviewCountChanged();
                update();
            }
        });
    }
    // A controller attached AFTER a selection exists (late connect) gets
    // its ants seeded here — consume-only paint() never recomputes.
    recomputeAnts();
    emit controllerChanged();
}

void CanvasView::setTool(const QString &tool)
{
    if (m_tool == tool) {
        // Re-arm an INACTIVE session on same-tool re-selection (review
        // A11Y-1 MAJOR-2): commit/cancel ends the session but leaves the
        // tool selected, so a second menu trigger or button press was a
        // silent no-op — an action-shaped control must act.
        if (tool == QLatin1String("transform.puppet") && !m_puppetActive)
            beginPuppetSession();
        else if (tool == QLatin1String("transform.perspective") && !m_perspActive)
            beginPerspectiveFrame();
        else if (tool == QLatin1String("transform.free") && !m_transformActive)
            beginTransformFrame();
        return;
    }
    cancelGesture(); // includes dropping an un-committed transform frame
    m_tool = tool;
    if (m_tool == QLatin1String("transform.free"))
        beginTransformFrame();
    else if (m_tool == QLatin1String("transform.perspective"))
        beginPerspectiveFrame();
    else if (m_tool == QLatin1String("transform.puppet"))
        beginPuppetSession();
    applyToolCursor();
    emit toolChanged();
    update();
}

void CanvasView::applyToolCursor()
{
    // Per-tool pointer feel (P4): pan overrides while space is held or a
    // middle-drag is live; precision tools get the crosshair; move gets
    // the four-way arrow. Pure presentation.
    if (m_panning) {
        setCursor(Qt::ClosedHandCursor);
        return;
    }
    if (m_spaceHeld) {
        setCursor(Qt::OpenHandCursor);
        return;
    }
    if (m_tool == QLatin1String("move")) {
        setCursor(Qt::SizeAllCursor);
    } else if (m_tool == QLatin1String("transform.free")
               || m_tool == QLatin1String("transform.perspective")) {
        setCursor(Qt::ArrowCursor);
    } else if (m_tool == QLatin1String("text")) {
        setCursor(Qt::IBeamCursor);
    } else {
        setCursor(Qt::CrossCursor); // marquees/lasso/polygon/wand/crop/brush/eraser/pen/shapes
    }
}

void CanvasView::setWandTolerance(double tolerance)
{
    const double clamped = std::clamp(tolerance, 0.0, 255.0); // contract range
    if (qFuzzyCompare(m_wandTolerance, clamped))
        return;
    m_wandTolerance = clamped;
    emit wandToleranceChanged();
}

void CanvasView::setWandContiguous(bool contiguous)
{
    if (m_wandContiguous == contiguous)
        return;
    m_wandContiguous = contiguous;
    emit wandContiguousChanged();
}

void CanvasView::setSampleAllLayers(bool sampleAll)
{
    if (m_sampleAllLayers == sampleAll)
        return;
    m_sampleAllLayers = sampleAll;
    emit sampleAllLayersChanged();
}

void CanvasView::setBrushColor(const QColor &color)
{
    if (m_brushColor == color)
        return;
    m_brushColor = color;
    emit brushColorChanged();
}

void CanvasView::setBrushSize(double size)
{
    const double clamped = std::clamp(size, 0.1, 10000.0); // §10.3 sizePx range
    if (qFuzzyCompare(m_brushSize, clamped))
        return;
    m_brushSize = clamped;
    emit brushSizeChanged();
}

void CanvasView::setBrushHardness(double hardness)
{
    // Item 4: the brushSize posture (stroke-family state clamps; the
    // controller sanitizes again at strokeBegin). Contract range 0..=1.
    // Plain equality, not qFuzzyCompare: the range legitimately holds
    // 0.0, where relative fuzz is unreliable.
    const double clamped = std::clamp(hardness, 0.0, 1.0);
    // O-29 — THE PREDICATE. brush.stroke's `hardness` is optional with no
    // schema default ("absent = preset hardness"), so the shell has to
    // decide, per stroke, whether the row is stating an override at all.
    // The signal is "the row HAS BEEN SET", latched here, in the one place
    // a set can arrive.
    //
    // THE LATCH RUNS BEFORE THE NO-CHANGE EARLY-RETURN, and the ordering
    // is load-bearing (§5 review minor-1, which found it the other way
    // round). Reaching this setter IS the statement; the value carried is
    // a separate question. A statement does not stop being one because
    // the number happens to equal what the row already displays — and
    // that is not a corner case, it is the FIRST case that breaks: under
    // a preset picker without display seeding, the row shows 0.0 while a
    // hard-round preset carries 1.0, so a user who drags straight to the
    // slider minimum wanting SOFT states 0.0, has it swallowed, the param
    // omitted, the preset consulted — and paints HARD. That is precisely
    // the mirrored silent-wrong argued against below, re-entering through
    // the early-return instead of through the predicate. Latching first
    // closes that door; the early-return keeps its real job, which is
    // suppressing a redundant CHANGE SIGNAL, and nothing more.
    //
    // Why not the obvious `m_brushHardness == kDefaultHardness` compare:
    //   1. It answers the wrong question. Absent means "no opinion". A
    //      number equal to a constant is not an absence of opinion — it is
    //      an opinion that happens to coincide. Deliberately dragging the
    //      row to 0.0 under a hard preset would be discarded as if it had
    //      never been touched, and the brush would paint hard: the SAME
    //      class of silent-wrong this change exists to remove, mirrored.
    //   2. It would only be reliable by numeric luck. The row's default is
    //      0.0, which is exactly representable AND equals the slider
    //      minimum, so `==` happens to hold today; move the default to a
    //      value the step arithmetic reaches by accumulation and the
    //      compare starts missing by an ulp. A predicate that is correct
    //      because of the current numbers is what this slice is retiring.
    // This flag needs no float reasoning and no knowledge of the preset,
    // which the shell does not have and must not guess.
    //
    // Why this row does NOT follow the house default->omit sentinel rule
    // (puppetExpansion/puppetRigidity above, submitText's style.size,
    // FilterParamDialog). What that rule actually rests on is that
    // "absent" resolves to ONE FIXED CONSTANT THE SHELL CAN NAME, so
    // `value == default` really is equivalent to omitting. That holds for
    // all three, though they get their constant from different places —
    // transform.puppet.yaml carries real schema defaults (`default: 2`
    // for expansionPx, `default: 0.7` for rigidity), whereas text.add
    // .yaml's `style.size` carries NO `default:` key at all and its 12
    // comes from the engine (appname-text style.rs, `impl Default for
    // CharStyle { size_pt: 12.0 }`), as canvas_view.h's own note on that
    // row says. Either way the shell can name the number.
    //
    // `hardness` is the exception the rule was never written for: absent
    // resolves to `preset.hardness`, which VARIES with the preset and
    // which no shell surface can query. The equivalence the rule rests on
    // is simply unavailable here, so the rule cannot be transplanted;
    // intent is the only signal that survives a preset swap.
    //
    // It is deliberately STICKY for the session: once the user has stated
    // a hardness, later returning the row to 0.0 is still a statement, and
    // it rides the wire.
    //
    // KNOWN GAP, flagged not papered over: with a preset picker, an
    // UNSTATED row would show 0.00 while a hard-round preset paints 1.0.
    // That is a display-seeding duty (the picker must seed the row FROM
    // the chosen preset, so the number shown is the one that will actually
    // paint, AND CLEAR THIS FLAG when it does, so a seeded row reads as
    // unstated again) and belongs to the picker slice; it cannot be fixed
    // here, because the hardness of a preset is engine-side knowledge with
    // no query door in this ABI.
    m_brushHardnessStated = true;
    if (m_brushHardness == clamped)
        return; // value unchanged: no signal — but the intent IS latched
    m_brushHardness = clamped;
    emit brushHardnessChanged();
}

void CanvasView::zoomIn()
{
    setZoomCentered(m_zoom * 2.0);
}

void CanvasView::zoomOut()
{
    setZoomCentered(m_zoom / 2.0);
}

void CanvasView::zoomFit()
{
    if (!m_controller || !m_controller->hasDocument() || width() <= 0 || height() <= 0)
        return;
    const double margin = 24.0;
    // ★ Fit the OUTPUT FRAME. Fitting the canvas under an active crop
    // would frame (and centre) a region the user cannot see.
    const QRect img = m_controller->imageRect();
    const double dw = std::max(1, img.width());
    const double dh = std::max(1, img.height());
    const int percentBefore = zoomPercent();
    m_zoom = std::min((width() - margin) / dw, (height() - margin) / dh);
    clampZoom();
    // m_offset is doc(0,0) in view coords, so centring a frame whose
    // origin is not (0,0) has to subtract that origin back out —
    // otherwise the crop lands off-centre by exactly the crop offset.
    m_offset = QPointF((width() - dw * m_zoom) / 2.0 - img.x() * m_zoom,
                       (height() - dh * m_zoom) / 2.0 - img.y() * m_zoom);
    m_userNavigated = false; // fit re-applies on resize until the user navigates
    ensureAntsWindow(); // F1c window escape check
    if (!(m_autoRefit && m_refitSilentWhenUnchanged && zoomPercent() == percentBefore))
        emit zoomChanged(); // W-18: an unchanged automatic re-fit stays silent
    update();
}

void CanvasView::fitIfOverflow()
{
    if (!m_controller || !m_controller->hasDocument())
        return;
    // Overflow of the OUTPUT FRAME — the thing actually painted.
    const double w = m_controller->imageRect().width() * m_zoom;
    const double h = m_controller->imageRect().height() * m_zoom;
    if (w > width() || h > height())
        zoomFit();
}

void CanvasView::zoomOneToOne()
{
    setZoomCentered(1.0);
}

void CanvasView::setZoomCentered(double zoom)
{
    if (!m_controller || !m_controller->hasDocument())
        return;
    const QPointF viewCenter(width() / 2.0, height() / 2.0);
    const QPointF docAtCenter = viewToDoc(viewCenter);
    m_zoom = zoom;
    clampZoom();
    m_offset = viewCenter - docAtCenter * m_zoom;
    m_userNavigated = true;
    ensureAntsWindow(); // F1c window escape check
    emit zoomChanged();
    update();
}

// tourStroke takes a PALETTE SPELLING, exactly like m_tool, and resolves
// it through the SAME table the mouse path uses.
//
// ★★★ IT USED TO TAKE A WIRE KIND AND PASS IT STRAIGHT THROUGH WITH NO
// MODE, which made the tour structurally incapable of showing any mode
// tool working -- dodge, burn, blur, sharpen, and heal would each have
// been sent with `mode` absent and refused by the schema. The bug was
// invisible because every existing caller passes "brush.stroke" or
// "eraser.stroke", where the spelling IS the kind and the mode is
// legitimately empty: THE TWO PATHS AGREED ON EXACTLY THE CASES ANYONE
// HAD TRIED. Those callers are unchanged by this signature -- they
// resolve to themselves -- so the fix costs them nothing and the tour can
// now photograph what the mouse actually does.
bool CanvasView::tourStroke(const QString &tool, const QVariantList &points)
{
    if (!m_controller || !m_controller->hasDocument() || points.isEmpty())
        return false;
    const StrokeDispatch dispatch = resolveStrokeDispatch(tool);
    if (!dispatch.dispatches)
        return false; // an undispatched tool must not look like a commit
    // The ECHO keeps the palette spelling (shell-local chrome); the
    // ENVELOPE gets the family kind, because that is what the engine
    // registers. Same split as the mouse path, deliberately.
    m_activeEcho = EchoStroke{tool, m_brushColor, m_brushSize, {}, {}};
    m_controller->strokeBegin(dispatch.kind, m_brushColor, m_brushSize,
                              brushHardnessOverride(), // O-29: unset = preset governs
                              dispatch.mode,
                              m_sampleAllLayers); // row 5: same as the mouse path, deliberately
    for (const QVariant &pv : points) {
        const QVariantMap p = pv.toMap();
        const double x = p.value(QStringLiteral("x")).toDouble();
        const double y = p.value(QStringLiteral("y")).toDouble();
        const double pressure = p.value(QStringLiteral("pressure"), 1.0).toDouble();
        const double t = p.value(QStringLiteral("t")).toDouble();
        m_activeEcho.points.append(QPointF(x, y));
        m_activeEcho.pressures.append(static_cast<float>(std::clamp(pressure, 0.0, 1.0)));
        m_controller->strokeAppend(x, y, pressure, t);
    }
    m_strokeInProgress = true; // so onStrokeCommitted files the echo
    update();
    return m_controller->strokeEnd();
}

void CanvasView::paint(QPainter *painter)
{
    // The ENTRY transform is the view-coordinate base Qt hands us with
    // the device-pixel-ratio pre-applied — the ring must restore THIS,
    // never bare identity (review A11Y-2 D2: resetTransform discarded
    // the DPR scale, halving the ring on retina displays).
    const QTransform entryTransform = painter->transform();
    // Neutral workspace background (generic; no document math anywhere).
    painter->fillRect(QRectF(0, 0, width(), height()), QColor(0x40, 0x42, 0x45));
    if (!m_controller || !m_controller->hasDocument() || m_cache.isNull()) {
        drawFocusRing(painter, entryTransform); // ring in the no-doc state too (review M1)
        return;
    }

    // The OUTPUT FRAME in view coords. m_offset alone is doc(0,0); under
    // an offset crop the frame starts at m_cacheOrigin, so the checker,
    // the clip and the blit all shift with it. docToView is used rather
    // than open-coding the transform precisely because doc<->view stays
    // absolute — the origin enters here, at extent time, not in there.
    const QRectF docViewRect(docToView(QPointF(m_cacheOrigin)),
                             QSizeF(m_cache.width() * m_zoom, m_cache.height() * m_zoom));

    // Transparency checker under the document (presentation convention).
    // Built once and cached: paint() runs on every stroke-echo update, and
    // a per-frame QPixmap+QPainter is measurable churn (review R7).
    if (m_checkerTile.isNull()) {
        const int checker = 8;
        m_checkerTile = QPixmap(checker * 2, checker * 2);
        QPainter tp(&m_checkerTile);
        tp.fillRect(m_checkerTile.rect(), QColor(0xc8, 0xc8, 0xc8));
        tp.fillRect(0, 0, checker, checker, QColor(0xf2, 0xf2, 0xf2));
        tp.fillRect(checker, checker, checker, checker, QColor(0xf2, 0xf2, 0xf2));
    }
    painter->save();
    // G-5 (repaint-scoping regression): the clip operation MUST be
    // IntersectClip. QPainter::setClipRect defaults to Qt::ReplaceClip,
    // which DISCARDS the scene graph's incoming dirty-rect clip and lets
    // the checker tile paint over the ENTIRE document rect, while the
    // engine composite blitted below stays inside the restored (scoped)
    // clip — so every scoped repaint erased the image to checkerboard
    // outside its own dirty rect and looked like data loss. This was
    // harmless only while every repaint was full-item; the repaint-scoping
    // slice (merge 1bcb5bb) made partial clips the normal case and turned
    // it live. Intersect keeps the checker inside whatever region the
    // scene graph asked us to repaint. On a FULL-item paint no clip is set
    // at all, and QPainter promotes IntersectClip to ReplaceClip when
    // clipping is disabled, so the full-repaint shape is unchanged.
    painter->setClipRect(docViewRect, Qt::IntersectClip);
    painter->drawTiledPixmap(docViewRect, m_checkerTile);
    painter->restore();

    // The engine composite, blitted verbatim (nearest-neighbor: honest
    // pixels at any zoom).
    painter->save();
    painter->setRenderHint(QPainter::SmoothPixmapTransform, false);
    painter->translate(m_offset);
    painter->scale(m_zoom, m_zoom);
    // Inside translate(m_offset)+scale(m_zoom) the painter's coordinates
    // ARE absolute doc coords, so the cache goes at its own origin —
    // (0,0) would place a cropped image at the canvas corner.
    painter->drawImage(QPointF(m_cacheOrigin), m_cache);

    // Stroke echo previews (view-transform space so widths track zoom).
    for (const EchoStroke &stroke : m_pendingEchoes)
        drawEcho(painter, stroke, true);
    if (m_strokeInProgress)
        drawEcho(painter, m_activeEcho, false);
    // Tool gesture overlays (rubber bands / transform frame / move echo) —
    // input echo of in-flight gestures, doc-anchored, cosmetic pens.
    drawToolOverlays(painter);
    // Guides + grid overlays: engine truth from info v2 (write path is
    // the guide.*/grid.set kinds; overlay-scope per contract — the engine
    // never composites them).
    if (abiHasBlock12()) {
        // ★ These overlays live in ABSOLUTE doc coords (this is inside
        // translate+scale), so their EXTENT is the output frame — the
        // old `0 .. m_cache.width()` was correct only while the cache
        // was canvas-sized at doc origin. The grid's PHASE stays anchored
        // to doc 0 (guide/grid VALUES are absolute document coordinates
        // and an active crop must not shift them); only the drawn span
        // is clipped to what the frame actually shows.
        const QRectF frame(QPointF(m_cacheOrigin), QSizeF(m_cache.size()));
        const QVariantMap grid = m_controller->gridSpec();
        if (grid.value(QStringLiteral("show")).toBool()) {
            const double spacing = grid.value(QStringLiteral("spacing")).toDouble();
            if (spacing >= 0.1) {
                QPen gridPen(QColor(0x60, 0x80, 0xa0, 0x60), 0);
                gridPen.setCosmetic(true);
                painter->setPen(gridPen);
                for (double x = std::ceil(frame.left() / spacing) * spacing;
                     x <= frame.right(); x += spacing)
                    painter->drawLine(QLineF(x, frame.top(), x, frame.bottom()));
                for (double y = std::ceil(frame.top() / spacing) * spacing;
                     y <= frame.bottom(); y += spacing)
                    painter->drawLine(QLineF(frame.left(), y, frame.right(), y));
            }
        }
        QPen guidePen(QColor(0x40, 0xc0, 0xd0), 0);
        guidePen.setCosmetic(true);
        painter->setPen(guidePen);
        for (const QVariant &g : m_controller->guidesH())
            painter->drawLine(QLineF(frame.left(), g.toDouble(),
                                     frame.right(), g.toDouble()));
        for (const QVariant &g : m_controller->guidesV())
            painter->drawLine(QLineF(g.toDouble(), frame.top(),
                                     g.toDouble(), frame.bottom()));
    }

    // Marching ants: engine-true selection outline from the ABI 1.2
    // coverage-mask readback (presentation-side vectorization of
    // delivered coverage — no document math). White-under-black double
    // pass keeps the outline visible on any content. CONSUME-ONLY
    // (repaint-scoping slice, sweep F1c): the vector is recomputed off
    // the paint path — on overlayChanged and at pan/zoom/geometry window
    // escape — never here.
    if (m_antsValid && !m_antsPath.isEmpty()) {
        QPen under(QColor(0xff, 0xff, 0xff), 0);
        under.setCosmetic(true);
        painter->setBrush(Qt::NoBrush);
        painter->setPen(under);
        painter->drawPath(m_antsPath);
        QPen over(QColor(0x10, 0x10, 0x10), 0);
        over.setCosmetic(true);
        over.setStyle(Qt::CustomDashLine);
        over.setDashPattern({4.0, 4.0});
        painter->setPen(over);
        painter->drawPath(m_antsPath);
    }
    painter->restore();

    // Document bounds.
    painter->setPen(QPen(QColor(0x20, 0x20, 0x20), 1.0));
    painter->setBrush(Qt::NoBrush);
    painter->drawRect(docViewRect.adjusted(-0.5, -0.5, 0.5, 0.5));

    drawFocusRing(painter, entryTransform);
}

void CanvasView::drawFocusRing(QPainter *painter, const QTransform &entryTransform)
{
    // §30.4 focus.ring (A11Y-2): the canvas is a pane in the F6 cycle —
    // keyboard focus must be visible on EVERY path (with or without a
    // document), DPR-correct, and LIVE (activeFocusChanged -> update()
    // wired in the constructor — review D1: QQuickPaintedItem never
    // repaints on focus change by itself). Color matches Theme.focusRing
    // (the documented C++-constants-match-Theme class).
    if (!hasActiveFocus())
        return;
    painter->save();
    painter->setTransform(entryTransform);
    QPen ring(QColor(0x8e, 0xc0, 0xee), 2);
    ring.setCosmetic(true);
    painter->setPen(ring);
    painter->setBrush(Qt::NoBrush);
    painter->drawRect(boundingRect().adjusted(1, 1, -1, -1));
    painter->restore();
}

void CanvasView::drawEcho(QPainter *painter, const EchoStroke &stroke, bool pending) const
{
    if (stroke.points.isEmpty())
        return;
    const bool eraser = stroke.kind == QLatin1String("eraser.stroke");
    QColor color = eraser ? QColor(0xff, 0xff, 0xff, 0xb4) : stroke.color;
    if (pending)
        color.setAlpha(eraser ? 0x82 : 0xb4); // previews are visibly provisional
    painter->setBrush(Qt::NoBrush);
    if (stroke.points.size() == 1) {
        const double d = std::max(stroke.sizePx * stroke.pressures.value(0, 1.0f), 0.75 / m_zoom);
        painter->setPen(Qt::NoPen);
        painter->setBrush(color);
        painter->drawEllipse(stroke.points.first(), d / 2.0, d / 2.0);
        painter->setBrush(Qt::NoBrush);
        return;
    }
    // One path, one stroke pass: per-segment drawing of a translucent pen
    // double-paints the joins (lens artifacts). v1 preview uses a single
    // width from the mean pressure (mouse input is constant 1.0 anyway);
    // true variable-width dabs are the ENGINE's job — this is only a
    // presentation preview.
    double meanPressure = 0.0;
    for (const float p : stroke.pressures)
        meanPressure += p;
    meanPressure = stroke.pressures.isEmpty() ? 1.0 : meanPressure / stroke.pressures.size();
    const double w = std::max(stroke.sizePx * meanPressure, 0.75 / m_zoom);
    QPainterPath path(stroke.points.first());
    for (int i = 1; i < stroke.points.size(); ++i)
        path.lineTo(stroke.points.at(i));
    painter->setPen(QPen(color, w, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter->drawPath(path);
}

void CanvasView::mousePressEvent(QMouseEvent *event)
{
    forceActiveFocus(); // keyboard zoom/pan modifiers reach the item
    if (!m_controller || !m_controller->hasDocument()) {
        event->ignore();
        return;
    }
    if (event->button() == Qt::MiddleButton
        || (event->button() == Qt::LeftButton && m_spaceHeld)) {
        m_panning = true;
        m_panAnchorView = event->position();
        m_panAnchorOffset = m_offset;
        applyToolCursor();
        event->accept();
        return;
    }
    if (event->button() != Qt::LeftButton) {
        event->ignore();
        return;
    }
    const QPointF doc = viewToDoc(event->position());
    // Resolved by the ONE table (top of file). The handler used to carry
    // its own copy; see the note there for why that was a defect and not
    // merely duplication.
    // B-02: OPTION/ALT-CLICK ARMS THE CLONE SOURCE (§21.1, Photoshop
    // convention). It is a point-set, NOT a stroke: it must run before the
    // dispatch test or the same press would begin painting from the very
    // pixel the user is trying to sample. Re-arming is always allowed —
    // `aligned` (default true) holds C(P) − P constant only UNTIL re-arm.
    if (m_tool == QLatin1String("clone.stroke")
        && (event->modifiers() & Qt::AltModifier)) {
        m_cloneAnchorDoc = doc;
        m_cloneArmed = true;
        applyToolCursor();
        event->accept();
        return;
    }
    const StrokeDispatch dispatch = resolveStrokeDispatch(m_tool);
    if (dispatch.dispatches) {
        // B-02: an UNARMED clone has no source, and its contract requires
        // one — so this refuses at the door rather than building an
        // envelope the engine would reject on stage 3. The canvas states
        // the fact; Main.qml owns the sentence.
        if (dispatch.kind == QLatin1String("clone.stroke") && !m_cloneArmed) {
            // ★★★★★ THE EXISTING DOOR, NOT A NEW ONE — corrected 2026-09-06.
            // B-02 first emitted a bespoke cloneSourceNeeded signal here and
            // that was wrong twice over. `armedToolPressed` already announces
            // on the ONE `tool_pending` announcer source AND moves the status
            // line (Main.qml announcePending), resolving the per-tool reason
            // id from pendingReasonIds — where `clone.stroke` has mapped to
            // `ui.status.clone_needs_source` since Q-D21 (a). A second signal
            // moved the status line while leaving the announcer row silent,
            // which is precisely what tst_palette_pins rule D4 caught.
            //
            // Adding clone to resolveStrokeDispatch DID correctly remove it
            // from the `else` at the foot of this handler — that block's own
            // comment says wiring a tool "removes it from this else
            // atomically". What it does not license is bypassing the door
            // that block was reaching for. An UNARMED clone is still
            // engine-silent by construction: no envelope, no stroke session,
            // history cannot move.
            emit armedToolPressed(m_tool);
            event->accept();
            return;
        }
        m_strokeInProgress = true;
        m_strokeStartMs = event->timestamp();
        // The ECHO keeps the palette spelling (it is shell-local chrome);
        // the ENVELOPE gets the family kind, because that is what the
        // engine registers.
        m_activeEcho = EchoStroke{m_tool, m_brushColor, m_brushSize, {}, {}};
        // B-02: the ARMED OFFSET is anchor − strokeStart, computed ONCE
        // here. §21.1/§45.2's `aligned` invariant is C(P) − P constant, so
        // every dab of this stroke reads from P + offset; recomputing per
        // dab would make the source follow the cursor, which is the
        // NON-aligned mode and not what the default contracts.
        const QPointF cloneOffset = m_cloneArmed
            ? QPointF(m_cloneAnchorDoc.x() - doc.x(), m_cloneAnchorDoc.y() - doc.y())
            : QPointF();
        m_controller->strokeBegin(dispatch.kind, m_brushColor, m_brushSize,
                                  brushHardnessOverride(), // O-29: unset = preset governs
                                  dispatch.mode,
                                  m_sampleAllLayers, // row 5: builder kind-guards the emit
                                  cloneOffset);      // B-02: clone.stroke only
        strokePointFromEvent(event);
    } else if (m_tool == QLatin1String("select.rect")
               || m_tool == QLatin1String("select.ellipse")) {
        m_gesture = Gesture::Marquee;
        m_gestureStartDoc = m_gestureCurrentDoc = doc;
    } else if (m_tool == QLatin1String("select.lasso")) {
        m_gesture = Gesture::Lasso;
        m_lassoPoints.clear();
        m_lassoPoints.append(doc);
    } else if (m_tool == QLatin1String("select.polygon")) {
        // v1.0.1 item 15: clicking the FIRST VERTEX closes the polygon.
        // This is the PEN's grammar (item 8, ~40 lines below) ported
        // verbatim — same 8px/zoom VIEW-space tolerance, same "close
        // instead of appending" shape — so the shell's two click-session
        // tools close identically. It joins the existing double-click
        // (mouseDoubleClickEvent) and Enter (keyPressEvent) paths; none
        // of them is removed.
        // The >= 3 bound is the ENGINE's polygon contract (submitPolygon
        // refuses below 3 points), not a copy of the pen's >= 2 — a pen
        // path closed on 2 anchors is a legal curve, a 2-vertex polygon
        // encloses no area. Below 3 vertices the click falls through and
        // appends as usual, so the first vertex only gains its second
        // meaning once closing is actually possible.
        if (m_polygonActive && m_lassoPoints.size() >= 3
            && QLineF(doc, m_lassoPoints.first()).length() <= 8.0 / m_zoom) {
            submitPolygon();
            event->accept();
            return;
        }
        m_polygonActive = true;
        m_lassoPoints.append(doc);
        m_hoverDoc = doc;
        update();
    } else if (m_tool == QLatin1String("select.wand")) {
        submitWand(doc, event->modifiers());
    } else if (m_tool == QLatin1String("vector.pen")) {
        // v1.0.1 item 8: clicking the FIRST anchor closes the path (the
        // standard pen grammar, alongside double-click and Enter).
        if (m_penActive && m_penAnchors.size() >= 2
            && QLineF(doc, m_penAnchors.first().anchor).length()
                <= 8.0 / m_zoom) {
            commitPen(true);
            event->accept();
            return;
        }
        m_penActive = true;
        PenAnchor a;
        a.anchor = snapDocPoint(doc);
        a.handleOut = a.anchor;
        m_penAnchors.append(a);
        // LIVE store (re-review V2-D1): the hover-segment overlay reads
        // m_hoverDoc, which approach motion never seeded (hover updates
        // are pen/polygon-gated) — seed it at the anchor so no stale
        // segment can flash before the first post-click motion.
        m_hoverDoc = a.anchor;
        m_gesture = Gesture::PenHandleDrag; // drag before release = handles
        update();
    } else if (m_tool == QLatin1String("shape.rect")
               || m_tool == QLatin1String("shape.ellipse")) {
        m_gesture = Gesture::ShapeDrag;
        m_gestureStartDoc = m_gestureCurrentDoc = snapDocPoint(doc);
    // RC3 D-6 (operator WORD 35, sitting S-15): the Text tool has NO press
    // branch for 1.0. A click used to submit text.add with no text (the
    // contract default), which added an invisible "Text" layer that could
    // not be typed into. Text now falls through to the chain's final
    // `else` -- the typed tool-pending refusal Quick Selection already
    // gives -- engine-silent, no layer added. submitText and the TextDrag
    // gesture are left in place, unreachable, for the text-editing slice.
    } else if (m_tool == QLatin1String("transform.puppet")) {
        // Item 25b (honesty sweep): the TOOL ID claims this branch, and
        // the session flag gates only the BODY. Session entry is
        // menu-driven, so a press outside an open session is inert BY
        // DESIGN — the tool is wired, it simply has nothing to do yet.
        // Hoisting the id out of the condition is what lets the chain's
        // final `else` mean exactly "no branch claims this tool id"; a
        // session-gated condition would fall through and make the sweep
        // announce "not wired yet" about a wired tool.
        if (m_puppetActive) {
            // Near-pin: Alt = rotation dial (Gesture::PuppetRotate), plain =
            // select. Free space: place a new pin (selected). No drag-move:
            // drag destinations are not wire-expressible (see header note).
            int hit = -1;
            for (int i = 0; i < m_puppetPins.size(); ++i) {
                if (QLineF(doc, m_puppetPins[i].pos).length() <= 8.0 / m_zoom) {
                    hit = i;
                    break;
                }
            }
            if (hit >= 0) {
                m_puppetSelected = hit;
                if (event->modifiers() & Qt::AltModifier)
                    m_gesture = Gesture::PuppetRotate;
            } else {
                PuppetPin pin;
                pin.pos = doc;
                m_puppetPins.append(pin);
                m_puppetSelected = m_puppetPins.size() - 1;
            }
            update();
        }
    } else if (m_tool == QLatin1String("transform.perspective")) {
        // Item 25b: id-claims-branch, session gates the body (see the
        // puppet branch above for the full reasoning).
        if (m_perspActive) {
            // Corner grab: each of the four quad corners moves
            // INDEPENDENTLY (the perspective distinction from the rigid
            // free-transform handles).
            for (int i = 0; i < 4; ++i) {
                if (QLineF(doc, m_perspQuad[i]).length() <= 8.0 / m_zoom) {
                    m_perspHandle = i;
                    m_gesture = Gesture::PerspectiveCorner;
                    break;
                }
            }
        }
    } else if (m_tool == QLatin1String("crop")) {
        m_gesture = Gesture::CropDrag;
        m_gestureStartDoc = m_gestureCurrentDoc = doc;
    } else if (m_tool == QLatin1String("move")) {
        m_gesture = Gesture::MoveDrag;
        m_gestureStartDoc = m_gestureCurrentDoc = doc;
        // v1.0.1 item 10: capture the active layer for the drag echo
        // (the 1.4 per-layer door — the same full-doc-rect read the
        // thumbnails/cut use; the big-doc chunking carry applies to all
        // three, recorded). Typed refusals yield null → the fallback
        // echo below; never faked.
        m_moveEchoLayer = QImage();
        if (m_controller) {
            const QVariantList rows = m_controller->layers();
            const int idx = m_controller->activeLayerIndex();
            if (idx >= 0 && idx < rows.size())
                m_moveEchoLayer = m_controller->layerThumbImage(
                    rows.at(idx).toMap()
                        .value(QStringLiteral("layerId")).toString());
        }
    } else if (m_tool == QLatin1String("transform.free")) {
        // Item 25b: id-claims-branch, session gates the body (see the
        // puppet branch above for the full reasoning).
        if (m_transformActive) {
            const int handle = transformHandleAt(doc);
            const QPointF center = m_transformNow.map(m_transformBase.center());
            if (handle >= 0) {
                m_gesture = Gesture::TransformScale;
                m_transformHandle = handle;
                // Anchor = the OPPOSITE handle's mapped position (handles are
                // TL,T,TR,R,BR,B,BL,L clockwise; opposite = +4 mod 8).
                const int opposite = (handle + 4) % 8;
                QPointF pts[8];
                const QRectF b = m_transformBase;
                const QPointF corners[8] = {
                    b.topLeft(), QPointF(b.center().x(), b.top()), b.topRight(),
                    QPointF(b.right(), b.center().y()), b.bottomRight(),
                    QPointF(b.center().x(), b.bottom()), b.bottomLeft(),
                    QPointF(b.left(), b.center().y())};
                for (int i = 0; i < 8; ++i)
                    pts[i] = m_transformNow.map(corners[i]);
                m_transformAnchorDoc = pts[opposite];
                m_transformAtGestureStart = m_transformNow;
                m_gestureStartDoc = doc;
            } else {
                // Rotate ring: near any mapped corner but outside its handle.
                bool rotate = false;
                const QPointF baseCorners[4] = {m_transformBase.topLeft(), m_transformBase.topRight(),
                                                m_transformBase.bottomRight(), m_transformBase.bottomLeft()};
                for (const QPointF &c : baseCorners) {
                    const double d = QLineF(doc, m_transformNow.map(c)).length();
                    if (d > 8.0 / m_zoom && d <= 22.0 / m_zoom)
                        rotate = true;
                }
                const QPointF local = m_transformNow.inverted().map(doc);
                if (rotate) {
                    m_gesture = Gesture::TransformRotate;
                    m_transformAnchorDoc = center;
                    m_rotateStartAngle = std::atan2(doc.y() - center.y(), doc.x() - center.x());
                    m_transformAtGestureStart = m_transformNow;
                } else if (m_transformBase.contains(local)) {
                    m_gesture = Gesture::TransformTranslate;
                    m_gestureStartDoc = doc;
                    m_transformAtGestureStart = m_transformNow;
                }
            }
        }
    } else {
        // ---- item 25b HONESTY SWEEP: the fall-through, made honest ----
        // Reaching here means NO branch above claims m_tool. Before this
        // slice the press died on the bare event->accept() below: the
        // user clicked, the slot checked, the name announced, and the
        // canvas did nothing, forever and silently. Under honest-state
        // rule D4 an armed tool must ANNOUNCE its pending state on
        // press, so the silence becomes a statement.
        //
        // THE DERIVATION, and why it is not a list: the pending set is
        // the COMPLEMENT of the if-chain above. There is no roster of
        // armed tool ids anywhere in the shell — wiring a tool means
        // adding its branch, which removes it from this `else`
        // atomically and with no second edit. A hand-written roster
        // would go stale the moment a wiring wave landed, and the two
        // slices would either double-announce or lie; this cannot.
        // (The three session-gated tools are claimed by id above, so an
        // unopened session never reaches here.)
        //
        // ENGINE-SILENT by construction: no envelope is built, no
        // stroke session opens, history cannot move. The announcement
        // is shell-local and rides the existing status/announcer path.
        emit armedToolPressed(m_tool);
    }
    event->accept();
}

void CanvasView::mouseDoubleClickEvent(QMouseEvent *event)
{
    if (m_penActive) {
        commitPen(true); // double-click closes the path (§20.1 closure)
        event->accept();
        return;
    }
    if (m_polygonActive) {
        submitPolygon();
        event->accept();
        return;
    }
    if (m_transformActive) {
        commitTransform();
        event->accept();
        return;
    }
    QQuickPaintedItem::mouseDoubleClickEvent(event);
}

void CanvasView::mouseMoveEvent(QMouseEvent *event)
{
    if (m_panning) {
        m_offset = m_panAnchorOffset + (event->position() - m_panAnchorView);
        m_userNavigated = true;
        ensureAntsWindow(); // F1c: re-vectorize if the pan escaped the window
        update();
        event->accept();
        return;
    }
    if (m_strokeInProgress) {
        strokePointFromEvent(event);
        event->accept();
        return;
    }
    const QPointF doc = viewToDoc(event->position());
    switch (m_gesture) {
    case Gesture::Marquee:
    case Gesture::CropDrag:
        m_gestureCurrentDoc = snapDocPoint(doc);
        update();
        break;
    case Gesture::MoveDrag:
        m_gestureCurrentDoc = doc;
        update();
        break;
    case Gesture::PenHandleDrag: {
        // Dragging off a fresh anchor pulls out symmetric handles.
        PenAnchor &a = m_penAnchors.last();
        const QPointF pulled = viewToDoc(event->position());
        if (QLineF(a.anchor, pulled).length() > 2.0 / m_zoom) {
            a.handleOut = pulled;
            a.hasHandle = true;
        } else {
            a.handleOut = a.anchor;
            a.hasHandle = false;
        }
        update();
        break;
    }
    case Gesture::ShapeDrag:
    case Gesture::TextDrag:
        m_gestureCurrentDoc = snapDocPoint(doc);
        update();
        break;
    case Gesture::PerspectiveCorner:
        if (m_perspHandle >= 0 && m_perspHandle < 4) {
            m_perspQuad[m_perspHandle] = doc;
            update();
        }
        break;
    case Gesture::PuppetRotate:
        if (m_puppetSelected >= 0 && m_puppetSelected < m_puppetPins.size()) {
            PuppetPin &pin = m_puppetPins[m_puppetSelected];
            // Dial: angle pin -> cursor, degrees (§45.5 Alt-drag dial).
            pin.rotationDeg = qRadiansToDegrees(
                std::atan2(doc.y() - pin.pos.y(), doc.x() - pin.pos.x()));
            pin.hasRotation = true;
            update();
        }
        break;
    case Gesture::Lasso:
        // Sub-pixel-throttled append keeps point counts sane at high zoom.
        if (m_lassoPoints.isEmpty()
            || QLineF(m_lassoPoints.constLast(), doc).length() > 1.0 / m_zoom) {
            m_lassoPoints.append(doc);
            update();
        }
        break;
    case Gesture::TransformTranslate: {
        // item 13: the commit rounds translation to whole pixels for the
        // served integer-translate arm, so the preview must not promise
        // sub-pixel placement — snap the delta live (preview == committed
        // placement by construction; commit-time qRound is then inert).
        const QPointF d = doc - m_gestureStartDoc;
        clearTransformWall(); // the wall names a matrix that no longer exists
        const QTransform before = m_transformNow;
        m_transformNow = m_transformAtGestureStart
            * QTransform::fromTranslate(qRound(d.x()), qRound(d.y()));
        update(transformPreviewInvalidateRect(before, m_transformNow));
        break;
    }
    case Gesture::TransformScale: {
        const QPointF a = m_transformAnchorDoc;
        const QPointF s0 = m_gestureStartDoc - a;
        const QPointF s1 = doc - a;
        double sx = qFuzzyIsNull(s0.x()) ? 1.0 : s1.x() / s0.x();
        double sy = qFuzzyIsNull(s0.y()) ? 1.0 : s1.y() / s0.y();
        if (m_transformHandle == 1 || m_transformHandle == 5) {
            // Edge-midpoint handles scale ONE axis by geometry (1=T,5=B
            // vertical, 3=R,7=L horizontal); the aspect lock does not
            // apply to them — the modifier is ignored (item 13, recorded).
            sx = 1.0;
        } else if (m_transformHandle == 3 || m_transformHandle == 7) {
            sy = 1.0;
        } else if (!(event->modifiers() & Qt::ShiftModifier)) {
            // item 13: corner handles are ASPECT-LOCKED by default —
            // uniform incremental scale from the dominant magnitude
            // (preserves the ratio at gesture start == the base ratio
            // absent prior one-axis scaling), per-axis sign preserved so
            // a flip through the anchor stays a flip. Shift UNLOCKS free
            // scaling — the inversion of the old hold-for-uniform; Shift
            // stays THIS gesture's scale modifier (the marquee's
            // Shift=add belongs to a different gesture family).
            const double s = std::max(std::abs(sx), std::abs(sy));
            sx = std::copysign(s, sx);
            sy = std::copysign(s, sy);
        }
        if (qFuzzyIsNull(sx) || qFuzzyIsNull(sy))
            break; // degenerate (det 0) — contract rejects; don't build it
        clearTransformWall(); // the wall names a matrix that no longer exists
        const QTransform before = m_transformNow;
        m_transformNow = m_transformAtGestureStart
            * (QTransform::fromTranslate(-a.x(), -a.y())
               * QTransform::fromScale(sx, sy)
               * QTransform::fromTranslate(a.x(), a.y()));
        update(transformPreviewInvalidateRect(before, m_transformNow));
        break;
    }
    case Gesture::TransformRotate: {
        const QPointF c = m_transformAnchorDoc;
        double angle = std::atan2(doc.y() - c.y(), doc.x() - c.x()) - m_rotateStartAngle;
        double deg = angle * 180.0 / M_PI;
        if (event->modifiers() & Qt::ShiftModifier)
            deg = std::round(deg / 15.0) * 15.0; // snap
        QTransform rot;
        rot.rotate(deg);
        clearTransformWall(); // the wall names a matrix that no longer exists
        const QTransform before = m_transformNow;
        m_transformNow = m_transformAtGestureStart
            * (QTransform::fromTranslate(-c.x(), -c.y()) * rot
               * QTransform::fromTranslate(c.x(), c.y()));
        update(transformPreviewInvalidateRect(before, m_transformNow));
        break;
    }
    case Gesture::None:
        break;
    }
    event->accept();
}

void CanvasView::mouseUngrabEvent()
{
    // A stolen grab mid-pan (e.g. a modal opened by an app shortcut during
    // a drag) would strand m_panning: stale ClosedHand + the next release
    // swallowed (review P4-D3).
    m_panning = false;
    // G-2, the grab-loss half. A stolen grab strands a live stroke the same way
    // it strands m_panning, and the next release commits it.
    //
    // PINNED by brushGrabLossMidStrokeDoesNotCommit in
    // tst_selection_gestures.cpp (ctest #20). This comment previously read
    // "THE HALF NO TEST CAN SEE ... UNVERIFIABLE BY TEST", carried from
    // q-gesture verbatim. Their claim was RIGHT and the conclusion I drew from
    // it was WRONG: QTest MOUSE SYNTHESIS genuinely cannot produce a grab
    // steal, but a test does not need synthesis -- QQuickItem::ungrabMouse()
    // and QQuickWindow::mouseGrabberItem() are both PUBLIC (measured in the
    // installed headers: qquickitem.h:297, qquickwindow.h:102). The "future
    // harness" the old text hoped for was the Qt public API all along.
    //
    // DEMONSTRATED, not assumed: deleting the cancelStroke() below -- and only
    // that one, focusOutEvent's call left intact -- reds that case at
    // "grab loss left the stroke session live". So the probe provably reaches
    // this line.
    cancelStroke();
    applyToolCursor();
    QQuickPaintedItem::mouseUngrabEvent();
}

void CanvasView::focusOutEvent(QFocusEvent *event)
{
    // Space-held focus loss would strand m_spaceHeld (stale OpenHand and
    // a surprise pan on the next left press) — same hygiene family.
    m_spaceHeld = false;
    // G-2 (MAJOR, q-gesture T33, REPRODUCED): the comment above says "same
    // hygiene family" and then handles exactly ONE member of it. A LIVE STROKE
    // is the other. Take focus off the canvas mid-drag, release, and
    // mouseReleaseEvent still called strokeEnd() -- THE STROKE COMMITTED
    // (measured content bounds 93,293 214x14). Same mechanism G-1 fixed for
    // Esc, one hazard over.
    cancelStroke();
    applyToolCursor();
    QQuickPaintedItem::focusOutEvent(event);
}

void CanvasView::hoverMoveEvent(QHoverEvent *event)
{
    if (m_polygonActive || m_penActive) {
        m_hoverDoc = viewToDoc(event->position());
        update();
    }
    QQuickPaintedItem::hoverMoveEvent(event);
}

void CanvasView::mouseReleaseEvent(QMouseEvent *event)
{
    if (m_panning) {
        m_panning = false;
        applyToolCursor();
        event->accept();
        return;
    }
    if (m_strokeInProgress && event->button() == Qt::LeftButton) {
        m_controller->strokeEnd(); // onStrokeCommitted handles the echo
        event->accept();
        return;
    }
    if (event->button() != Qt::LeftButton) {
        event->ignore();
        return;
    }
    const Gesture g = m_gesture;
    m_gesture = Gesture::None;
    switch (g) {
    case Gesture::Marquee:
        submitShapeSelection();
        break;
    case Gesture::Lasso:
        submitLasso();
        break;
    case Gesture::CropDrag: {
        const QRectF r = QRectF(m_gestureStartDoc, m_gestureCurrentDoc).normalized();
        if (r.width() >= 1.0 && r.height() >= 1.0)
            emit cropRectDragged(qRound(r.x()), qRound(r.y()),
                                 std::max(1, qRound(r.width())),
                                 std::max(1, qRound(r.height())));
        update();
        break;
    }
    case Gesture::MoveDrag: {
        const QPointF d = m_gestureCurrentDoc - m_gestureStartDoc;
        m_moveEchoLayer = QImage(); // echo capture ends with the drag
        submitMove(QPoint(qRound(d.x()), qRound(d.y())));
        break;
    }
    case Gesture::ShapeDrag:
        submitShapePrimitive();
        break;
    case Gesture::TextDrag:
        submitText();
        break;
    case Gesture::PenHandleDrag:
        break; // anchor finalized; the pen session continues until Enter/Esc
    default:
        break; // transform gestures end; the frame persists until Enter/Esc
    }
    event->accept();
}

void CanvasView::wheelEvent(QWheelEvent *event)
{
    if (!m_controller || !m_controller->hasDocument()) {
        event->ignore();
        return;
    }
    const double steps = event->angleDelta().y() / 120.0;
    if (qFuzzyIsNull(steps)) {
        event->ignore();
        return;
    }
    const double factor = std::pow(1.25, steps);
    const QPointF anchorView = event->position();
    const QPointF anchorDoc = viewToDoc(anchorView);
    m_zoom *= factor;
    clampZoom();
    m_offset = anchorView - anchorDoc * m_zoom;
    m_userNavigated = true;
    ensureAntsWindow(); // F1c: zoom-out can escape the vectorized window
    emit zoomChanged();
    update();
    event->accept();
}

void CanvasView::keyPressEvent(QKeyEvent *event)
{
    if (event->key() == Qt::Key_Space && !event->isAutoRepeat()) {
        m_spaceHeld = true;
        applyToolCursor();
        event->accept();
        return;
    }
    if (event->key() == Qt::Key_Escape) {
        // §39.2 [NORMATIVE], operator-ruled at v1.0.1 (review D3): Esc
        // resolves the innermost in-progress context ONLY — it NEVER
        // deselects (select.deselect does: Cmd+D, the menu item, and the
        // click-outside path — rect/ellipse/lasso since item 16). The
        // panic key must not destroy committed state; pinned by
        // escapeNeverDeselects.
        cancelGesture();
        event->accept();
        return;
    }
    if (m_puppetActive
        && (event->key() == Qt::Key_Delete || event->key() == Qt::Key_Backspace)) {
        if (m_puppetSelected >= 0 && m_puppetSelected < m_puppetPins.size()) {
            m_puppetPins.removeAt(m_puppetSelected);
            m_puppetSelected = m_puppetPins.isEmpty() ? -1
                                                      : m_puppetPins.size() - 1;
            update();
        }
        event->accept();
        return;
    }
    if (m_puppetActive
        && (event->key() == Qt::Key_Plus || event->key() == Qt::Key_Minus)) {
        // §45.5: depth exposed as bring-forward/send-backward, ±1/press.
        if (m_puppetSelected >= 0 && m_puppetSelected < m_puppetPins.size()) {
            m_puppetPins[m_puppetSelected].depth +=
                event->key() == Qt::Key_Plus ? 1 : -1;
            update();
        }
        event->accept();
        return;
    }
    // ---- v1.0.1 item 14b chords (canvas-focus scope — the §30.2
    // pattern: a focused text control means the canvas has no active
    // focus and these keys never arrive; sessions that own the keyboard
    // yield below). Every chord consumes the 14a availability surface —
    // a recognized chord whose row is disabled is a CONSUMED no-op (the
    // disabled-menu-item semantic); unrecognized modifier combos are
    // left unconsumed. The puppet session's own Delete (pin removal)
    // ran above and returns first. BUTTON-HELD gestures also yield
    // (gestureInFlight — the item-7 keyboard-guard rationale: a modal
    // confirm opening mid-drag is the grab steal one level removed, and
    // a mid-gesture mutation flips state under the live gesture).
    const bool sessionOwnsKeys = m_transformActive || m_perspActive
        || m_puppetActive || m_polygonActive || m_penActive
        || gestureInFlight();
    if ((event->key() == Qt::Key_Delete || event->key() == Qt::Key_Backspace)
        && !sessionOwnsKeys && m_controller) {
        const Qt::KeyboardModifiers mods =
            event->modifiers() & ~Qt::KeypadModifier;
        const auto rowEnabled = [this](const char *action) {
            return m_controller->commandAvailability()
                .value(QLatin1String(action))
                .toMap()
                .value(QStringLiteral("enabled"))
                .toBool();
        };
        if (mods == Qt::AltModifier || mods == Qt::ControlModifier) {
            // Alt+Delete / Cmd+Delete (portable Ctrl = macOS Cmd): fill
            // with the ENGINE's headless-constant arms — content
            // "foreground" (black) / "background" (white), the engine's
            // own defaults (kind_impls.rs). Recorded R15 distinction:
            // these are NOT the shell's swatch state (canvas.brushColor
            // seeds the Fill DIALOG; the chords speak the engine
            // constants until a swatch-state feed is designed). The
            // selection SCOPES the fill (contract: coverage-gated; no
            // selection = whole layer) — it never gates it.
            const bool background = mods == Qt::ControlModifier;
            if (rowEnabled(background ? "fill.background"
                                      : "fill.foreground")) {
                const QVariantList rows = m_controller->layers();
                const int idx = m_controller->activeLayerIndex();
                if (idx >= 0 && idx < rows.size()) {
                    QVariantMap target;
                    target.insert(QStringLiteral("layerId"),
                                  rows.at(idx).toMap()
                                      .value(QStringLiteral("layerId"))
                                      .toString());
                    QVariantMap params;
                    params.insert(QStringLiteral("content"),
                                  background
                                      ? QStringLiteral("background")
                                      : QStringLiteral("foreground"));
                    m_controller->applyCommand(QStringLiteral("fill"),
                                               target, params, {});
                }
            }
            event->accept();
            return;
        }
        if (mods == Qt::NoModifier) {
            // Plain Delete/Backspace: the desk-ruled disposition —
            // relayed to QML (the confirm dialog lives there; ONE
            // disposition implementation shared with the menu rows).
            if (rowEnabled("edit.delete"))
                emit deleteChordPressed();
            event->accept();
            return;
        }
        // Other combos (Shift+Delete, Cmd+Alt+Delete, ...): not ours.
    }
    if ((event->key() == Qt::Key_Left || event->key() == Qt::Key_Right
         || event->key() == Qt::Key_Up || event->key() == Qt::Key_Down)
        && !sessionOwnsKeys && m_controller) {
        const Qt::KeyboardModifiers mods =
            event->modifiers() & ~Qt::KeypadModifier;
        if (mods == Qt::NoModifier || mods == Qt::ShiftModifier) {
            // Arrow nudge: 1 px, Shift = 10 px, through submitMove —
            // the EXACT integer-translate arm (layer.transform) the
            // move tool and item-13's translate decomposition use.
            // Availability rides the nudge row; announcements: no
            // cataloged nudge id exists (checked first-hand) — honest
            // silence; the status line carries the applied/rejected
            // truth as every submit does.
            const bool enabled = m_controller->commandAvailability()
                .value(QStringLiteral("nudge"))
                .toMap()
                .value(QStringLiteral("enabled"))
                .toBool();
            if (enabled) {
                const int step = (mods == Qt::ShiftModifier) ? 10 : 1;
                QPoint d;
                switch (event->key()) {
                case Qt::Key_Left: d = QPoint(-step, 0); break;
                case Qt::Key_Right: d = QPoint(step, 0); break;
                case Qt::Key_Up: d = QPoint(0, -step); break;
                default: d = QPoint(0, step); break;
                }
                submitMove(d);
            }
            event->accept();
            return;
        }
    }
    if (event->key() == Qt::Key_Return || event->key() == Qt::Key_Enter) {
        if (m_puppetActive) {
            commitPuppet();
            event->accept();
            return;
        }
        if (m_perspActive) {
            commitPerspective();
            event->accept();
            return;
        }
        if (m_transformActive) {
            commitTransform();
            event->accept();
            return;
        }
        if (m_polygonActive) {
            submitPolygon();
            event->accept();
            return;
        }
        if (m_penActive) {
            commitPen(false); // Enter commits the open path
            event->accept();
            return;
        }
    }
    QQuickPaintedItem::keyPressEvent(event);
}

void CanvasView::keyReleaseEvent(QKeyEvent *event)
{
    if (event->key() == Qt::Key_Space && !event->isAutoRepeat()) {
        m_spaceHeld = false;
        applyToolCursor();
        event->accept();
        return;
    }
    QQuickPaintedItem::keyReleaseEvent(event);
}

void CanvasView::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (!m_userNavigated && m_controller && m_controller->hasDocument()) {
        m_autoRefit = true;
        zoomFit(); // its own escape check runs after the fit
        m_autoRefit = false;
    } else {
        ensureAntsWindow(); // F1c: a grown viewport can escape the window
    }
}

void CanvasView::onRegionsDirty(const QVariantList &rects)
{
    for (const QVariant &rv : rects) {
        const QRect r = rv.toRect();
        if (r.isValid())
            refreshRegion(r);
    }
}

void CanvasView::onDocumentReset()
{
    m_antsValid = false;
    m_antsPath = QPainterPath();
    m_antsWindow = QRect(); // stale doc-space window dies with the doc
    m_pendingEchoes.clear();
    // Item-10 review D2: a mid-drag reset (undoing a crop, Cmd+N/O —
    // the ungated shortcuts) must not leave a stale old-document
    // capture painting over the rebuilt cache. Item 21's transform
    // snapshot is the same capture class through a different door and
    // inherits the same clear — cancelGesture (which also fires on
    // documentReset) closes the frame, but this function runs FIRST and
    // rebuilds the cache, so the snapshot must not survive into it.
    m_moveEchoLayer = QImage();
    m_transformSnapshot = QImage();
    // m_strokeInProgress is NOT cleared here on purpose. `documentReset` is
    // connected to onDocumentReset FIRST and to cancelGesture immediately
    // after, and direct connections fire in order — so clearing it here made
    // cancelGesture's stroke arm dead on exactly the path that connection
    // exists to serve (V-D4a: a session started in the PREVIOUS document).
    // cancelGesture owns it, and also drops m_activeEcho and disarms the
    // controller, which this function never did.
    emit pendingPreviewCountChanged();
    rebuildCache();
    zoomFit();
}

void CanvasView::onStrokeCommitted(bool ok)
{
    if (!m_strokeInProgress)
        return;
    m_strokeInProgress = false;
    if (ok) {
        // Engine truth replaces presentation: any previews kept from
        // earlier typed rejections are stale the moment commits succeed —
        // drop them; the dirty-rect repaint shows the real strokes
        // (review R2; matches the documented echo lifecycle).
        if (!m_pendingEchoes.isEmpty()) {
            m_pendingEchoes.clear();
            emit pendingPreviewCountChanged();
        }
    } else {
        // Typed rejection (e.g. error.command.not_implemented until the
        // stroke wiring reaches the ABI): keep the echo as a visibly
        // provisional preview so the tool stays live; capped.
        m_pendingEchoes.append(m_activeEcho);
        if (m_pendingEchoes.size() > kMaxPendingEchoes)
            m_pendingEchoes.removeFirst();
        emit pendingPreviewCountChanged();
    }
    m_activeEcho = EchoStroke{};
    update();
}

QPointF CanvasView::viewToDoc(const QPointF &viewPos) const
{
    return (viewPos - m_offset) / m_zoom;
}

QPointF CanvasView::docToView(const QPointF &docPos) const
{
    return docPos * m_zoom + m_offset;
}

void CanvasView::clampZoom()
{
    m_zoom = std::clamp(m_zoom, kMinZoom, kMaxZoom);
}

// ★★ docRect is ABSOLUTE document coordinates — that is the contract of
// every caller (onRegionsDirty forwards the engine's own dirty rects,
// which are absolute) and of the read door (appname_doc_read_region
// frames on absolute coords and does NOT clamp).
//
// The two lines that changed here are the two that used to rely on
// cache-index == absolute-doc-coord. Under an offset crop that identity
// is false, and BOTH failures are silent: clipping against
// QRect(0,0,cache.w,cache.h) would clip the wrong window (dropping every
// dirty rect right of the image and admitting ones left of it), and
// blitting at bounded.topLeft() would land the pixels at the wrong place
// in the cache. Neither crashes; both just paint the wrong picture.
void CanvasView::refreshRegion(const QRect &docRect)
{
    if (!m_controller || m_cache.isNull())
        return;
    // The cache's own footprint, in ABSOLUTE doc coords.
    const QRect cacheDocRect(m_cacheOrigin, m_cache.size());
    const QRect bounded = docRect.intersected(cacheDocRect);
    if (!bounded.isValid() || bounded.isEmpty())
        return;
    QImage region;
    if (!m_controller->readRegionImage(bounded, &region))
        return;
    QPainter p(&m_cache);
    p.setCompositionMode(QPainter::CompositionMode_Source); // verbatim byte replace
    // absolute -> cache index. Identity only while m_cacheOrigin is (0,0).
    p.drawImage(bounded.topLeft() - m_cacheOrigin, region);
    p.end();
    const QPointF tl = docToView(bounded.topLeft());
    const QRectF viewRect(tl, QSizeF(bounded.width() * m_zoom, bounded.height() * m_zoom));
    update(viewRect.toAlignedRect().adjusted(-1, -1, 1, 1));
}

void CanvasView::rebuildCache()
{
    if (!m_controller || !m_controller->hasDocument()) {
        m_cache = QImage();
        m_cacheOrigin = QPoint(); // never outlive the cache it indexes
        update();
        return;
    }
    // ★ THE OUTPUT FRAME, not the canvas: image_rect() =
    // crop_rect.unwrap_or(canvas_rect()). Under a non-destructive crop
    // the canvas is still 200x100 while the frame is 120x60 at (40,20) —
    // sizing to docWidth/docHeight would paint the full canvas and make
    // the active crop invisible, which is the defect this consumes.
    const QRect img = m_controller->imageRect();
    if (img.isEmpty()) {
        m_cache = QImage();
        m_cacheOrigin = QPoint();
        update();
        return;
    }
    m_cacheOrigin = img.topLeft();
    m_cache = QImage(img.width(), img.height(), QImage::Format_RGBA8888);
    m_cache.fill(Qt::transparent);
    // Chunked full readback over ABSOLUTE doc coords starting at the
    // frame's top-left: every request stays far under the ABI's 2^26 px
    // readback cap and keeps first paint responsive. Starting at (0,0)
    // instead would read the right NUMBER of pixels from the wrong place.
    for (int y = img.top(); y <= img.bottom(); y += kReadChunk)
        for (int x = img.left(); x <= img.right(); x += kReadChunk)
            refreshRegion(QRect(x, y, std::min(kReadChunk, img.right() - x + 1),
                                std::min(kReadChunk, img.bottom() - y + 1)));
    update();
}

// ---- CP-0.5 tool gestures ---------------------------------------------------

// Snap a doc-space point to guides / grid / document edges when within
// tolerance (view px). Pure input assistance on engine-true overlay data.
QPointF CanvasView::snapDocPoint(const QPointF &doc) const
{
    if (!m_controller || !abiHasBlock12())
        return doc;
    const double tol = 6.0 / m_zoom;
    QPointF out = doc;
    auto trySnap = [&](double target, double value, double *slot) {
        if (std::abs(value - target) <= tol
            && std::abs(value - target) < std::abs(value - *slot))
            *slot = target;
    };
    double bestX = out.x() + tol * 4;
    double bestY = out.y() + tol * 4;
    trySnap(0.0, out.x(), &bestX);
    trySnap(m_controller->docWidth(), out.x(), &bestX);
    trySnap(0.0, out.y(), &bestY);
    trySnap(m_controller->docHeight(), out.y(), &bestY);
    for (const QVariant &g : m_controller->guidesV())
        trySnap(g.toDouble(), out.x(), &bestX);
    for (const QVariant &g : m_controller->guidesH())
        trySnap(g.toDouble(), out.y(), &bestY);
    const QVariantMap grid = m_controller->gridSpec();
    if (grid.value(QStringLiteral("show")).toBool()) {
        const double spacing = grid.value(QStringLiteral("spacing")).toDouble();
        if (spacing >= 0.1) {
            trySnap(std::round(out.x() / spacing) * spacing, out.x(), &bestX);
            trySnap(std::round(out.y() / spacing) * spacing, out.y(), &bestY);
        }
    }
    if (std::abs(bestX - out.x()) <= tol)
        out.setX(bestX);
    if (std::abs(bestY - out.y()) <= tol)
        out.setY(bestY);
    return out;
}

QString CanvasView::selectionOpForModifiers(Qt::KeyboardModifiers mods)
{
    const bool shift = mods & Qt::ShiftModifier;
    const bool alt = mods & Qt::AltModifier;
    if (shift && alt)
        return QStringLiteral("intersect");
    if (shift)
        return QStringLiteral("add");
    if (alt)
        return QStringLiteral("subtract");
    return QStringLiteral("replace");
}

// v1.0.1 items 9 + 16: THE click-outside deselect. A CLICK (no drag) on
// an UNSELECTED point clears the selection. §39.4 [NORMATIVE] gates this
// on the effective op being NEW — a modifier-click (add/subtract/
// intersect) is a NO-OP, never a deselect (review D2: a slip shift-click
// mid-composition must not destroy the composed selection). Outside-ness
// is ENGINE TRUTH via a 1x1 mask read (zero coverage = outside); a click
// INSIDE stays a no-op — a NAMED §39.4 deviation (the spec deselects on
// any op=new click; reserved here for a future selection-move; review
// D4). A click on the 1..127 feathered band outside the visible ants
// reads as inside — conservative, engine-true (review D9). Pre-1.2 (no
// mask readback) the read fails: no-op.
//
// ITEM 16 COVERAGE, and why it is not "all selection tools": rect,
// ellipse and lasso — the three DRAG-SHAPED tools, whose click carries
// no other meaning and is therefore free to mean "deselect".
// select.polygon and select.wand are EXCLUDED SEMANTICALLY, not
// overlooked: a polygon click APPENDS A VERTEX (that is the tool's whole
// grammar, and item 15 above gives the first vertex a second meaning on
// top of it), and a wand click IS the wand's entire gesture (it floods
// at the click point). Neither tool has a spare click to spend.
void CanvasView::deselectIfClickOutside(const QPointF &docPoint)
{
    if (!m_controller || !m_controller->selectionActive())
        return;
    if (selectionOpForModifiers(QGuiApplication::keyboardModifiers())
        != QLatin1String("replace"))
        return;
    const int cx = qRound(docPoint.x());
    const int cy = qRound(docPoint.y());
    QImage px;
    if (cx >= 0 && cy >= 0 && cx < m_controller->docWidth()
        && cy < m_controller->docHeight()
        && m_controller->readSelectionImage(QRect(cx, cy, 1, 1), &px)
        && px.constScanLine(0)[0] == 0) {
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                                   {});
    }
}

void CanvasView::submitShapeSelection()
{
    const QRectF r = QRectF(m_gestureStartDoc, m_gestureCurrentDoc).normalized();
    update();
    if (r.width() < 1.0 && r.height() < 1.0) {
        // A sub-pixel drag is a CLICK, not a new selection intent.
        // v1.0.1 item 16 moved the deselect body into
        // deselectIfClickOutside() so the lasso shares this exact
        // mechanism; rect and ellipse both arrive here (this function
        // serves both marquee tools).
        deselectIfClickOutside(m_gestureStartDoc);
        return;
    }
    // Contract: source rect|ellipse carries rect{x,y,w,h}, ALL integers.
    QVariantMap rect;
    rect.insert(QStringLiteral("x"), qRound(r.x()));
    rect.insert(QStringLiteral("y"), qRound(r.y()));
    rect.insert(QStringLiteral("w"), std::max(1, qRound(r.width())));
    rect.insert(QStringLiteral("h"), std::max(1, qRound(r.height())));
    QVariantMap params;
    params.insert(QStringLiteral("source"),
                  m_tool == QLatin1String("select.ellipse") ? QStringLiteral("ellipse")
                                                            : QStringLiteral("rect"));
    params.insert(QStringLiteral("rect"), rect);
    params.insert(QStringLiteral("op"),
                  selectionOpForModifiers(QGuiApplication::keyboardModifiers()));
    params.insert(QStringLiteral("antialias"), true);
    m_controller->applyCommand(QStringLiteral("selection.set"), {}, params, {});
    emit selectionSubmitted();
}

void CanvasView::submitLasso()
{
    const QVector<QPointF> pts = std::exchange(m_lassoPoints, {});
    update();
    if (pts.size() < 2) {
        // v1.0.1 item 16: a lasso CLICK. The move handler only appends a
        // point once the cursor has travelled more than one VIEW pixel
        // (1.0 / m_zoom in doc space), so a click with no drag leaves the
        // press point alone in the list — the lasso's exact analogue of
        // the marquee's sub-pixel rect. Same mechanism, same §39.4 gate.
        // RECORDED, not fixed here: the marquee's click test is a
        // 1.0-DOC-pixel bbox while this one is a 1-VIEW-pixel step, so
        // the two tools disagree about what counts as a drag at zooms
        // away from 100%. That is the view-space drag-threshold question
        // the item-16 triage HELD for an operator ruling; unifying the
        // thresholds would change committed-selection behaviour for all
        // three tools and is deliberately out of this slice.
        if (!pts.isEmpty())
            deselectIfClickOutside(pts.first());
        return; // contract: lasso needs >= 2 points
    }
    QVariantList points;
    for (const QPointF &p : pts)
        // v1.0.1 item 6 (THE lasso/polygon break): bare
        // `append(QVariantList{...})` resolves to QList's RANGE-append
        // and SPLICES the pair's elements into the outer list —
        // flattening [[x,y],...] to [x,y,x,y,...], which the engine
        // refuses typed ("[x, y] pairs"). QVariant() forces the
        // single-element overload.
        points.append(QVariant(QVariantList{p.x(), p.y()}));
    QVariantMap params;
    params.insert(QStringLiteral("source"), QStringLiteral("lasso"));
    params.insert(QStringLiteral("points"), points);
    params.insert(QStringLiteral("op"),
                  selectionOpForModifiers(QGuiApplication::keyboardModifiers()));
    params.insert(QStringLiteral("antialias"), true);
    m_controller->applyCommand(QStringLiteral("selection.set"), {}, params, {});
    emit selectionSubmitted();
}

void CanvasView::submitPolygon()
{
    const QVector<QPointF> pts = std::exchange(m_lassoPoints, {});
    m_polygonActive = false;
    update();
    if (pts.size() < 3)
        return; // contract: polygon needs >= 3 points
    QVariantList points;
    for (const QPointF &p : pts)
        // Same range-append splice as submitLasso (item 6) — see there.
        points.append(QVariant(QVariantList{p.x(), p.y()}));
    QVariantMap params;
    params.insert(QStringLiteral("source"), QStringLiteral("polygon"));
    params.insert(QStringLiteral("points"), points);
    params.insert(QStringLiteral("op"),
                  selectionOpForModifiers(QGuiApplication::keyboardModifiers()));
    params.insert(QStringLiteral("antialias"), true);
    m_controller->applyCommand(QStringLiteral("selection.set"), {}, params, {});
    emit selectionSubmitted();
}

void CanvasView::submitWand(const QPointF &docPos, Qt::KeyboardModifiers mods)
{
    if (!m_controller || !m_controller->hasDocument())
        return;
    const int w = m_controller->docWidth();
    const int h = m_controller->docHeight();
    const int sx = qRound(docPos.x());
    const int sy = qRound(docPos.y());
    if (sx < 0 || sy < 0 || sx >= w || sy >= h)
        return; // contract: seed must be inside the canvas — outside = no intent
    QVariantMap params;
    params.insert(QStringLiteral("source"), QStringLiteral("wand"));
    params.insert(QStringLiteral("seed"), QVariantList{sx, sy});
    params.insert(QStringLiteral("op"), selectionOpForModifiers(mods));
    params.insert(QStringLiteral("tolerance"), m_wandTolerance);
    params.insert(QStringLiteral("contiguous"), m_wandContiguous);
    params.insert(QStringLiteral("antialias"), true);
    // Tier-1 row 5: emitted ONLY when true (schema default false — the
    // default-omission rule). `source` stays "wand": selection.set's
    // sample scope is THIS boolean, not the retouch kinds'
    // source:"composite" arm. The engine's wand sampling arm serves the
    // flattened composite with NO active-layer state (kind_impls.rs
    // sampling_source), so checked is also the setting under which the
    // wand answers on multi-raster documents.
    // 1.0 final D-8 (corrigendum (l)): the wand ALWAYS samples the
    // composite. The Sample All Layers choice is removed from its options
    // row: unchecked, the wand refused on every multi-layer document.
    params.insert(QStringLiteral("sampleAllLayers"), true);
    // Item 25b (the wand mechanical fix): the result map was DISCARDED
    // here and selectionSubmitted() fired unconditionally — the signal
    // announced a submitted selection even when the engine refused the
    // command outright. Consume the result and tell the truth.
    const QVariantMap r = m_controller->applyCommand(
        QStringLiteral("selection.set"), {}, params, {});
    if (r.value(QStringLiteral("ok")).toBool()) {
        emit selectionSubmitted();
        return;
    }
    // The wand's own honest limit (census class G): on a document with
    // more than one raster layer `sampling_source`
    // (crates/appname-tools/src/kind_impls.rs:5551-5583) answers a typed
    // NotImplemented — headless carries no active-layer state, so the
    // sample source resolves only on a single-raster-layer document.
    // That is a MILESTONE gap, not a user error, and the raw
    // ui.status.rejected spec jargon is the wrong way to say so; the
    // press announces pending instead.
    // Scope is deliberately narrow — per CODE and per KIND. Only
    // selection.set's not_implemented arm maps here, so a future typed
    // not_implemented from some other kind can never be mislabeled
    // "arrives at a later milestone".
    if (r.value(QStringLiteral("code")).toString()
        == QLatin1String("error.command.not_implemented")) {
        emit armedToolRefusedPending(m_tool);
        return;
    }
    // Every other refusal keeps the universal ui.status.rejected surface
    // applyCommand already set — untouched by this slice.
}

void CanvasView::submitMove(const QPoint &deltaPx)
{
    update();
    if (deltaPx.isNull() || !m_controller)
        return;
    const QVariantList rows = m_controller->layers();
    const int idx = m_controller->activeLayerIndex();
    if (idx < 0 || idx >= rows.size())
        return;
    const QString layerId =
        rows.at(idx).toMap().value(QStringLiteral("layerId")).toString();
    // layer.transform: the engine's CP-0.5 arm is the EXACT integer
    // translate — column-major Mat3, translation in elements 6..7.
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerId);
    QVariantMap params;
    params.insert(QStringLiteral("matrix"),
                  QVariantList{1, 0, 0, 0, 1, 0, deltaPx.x(), deltaPx.y(), 1});
    m_controller->applyCommand(QStringLiteral("layer.transform"), target, params, {});
}

void CanvasView::beginPuppetSession()
{
    if (!m_controller || !m_controller->hasDocument())
        return;
    m_puppetActive = true;
    m_puppetPins.clear();
    m_puppetSelected = -1;
    update();
}

void CanvasView::commitPuppet()
{
    if (!m_puppetActive)
        return;
    const QVector<PuppetPin> pins = std::exchange(m_puppetPins, {});
    m_puppetActive = false;
    m_puppetSelected = -1;
    update();
    if (pins.isEmpty() || !m_controller)
        return; // no pins = no intent (Esc-equivalent), nothing recorded
    const QVariantList rows = m_controller->layers();
    const int idx = m_controller->activeLayerIndex();
    QVariantMap target;
    if (idx >= 0 && idx < rows.size())
        target.insert(QStringLiteral("layerId"),
                      rows.at(idx).toMap().value(QStringLiteral("layerId")).toString());
    // 11d Phase 1: the §45.5 knobs ride the READY builder arguments —
    // the options-bar bound state passes through HERE (the one live
    // submission path). Values equal to the CONTRACT defaults
    // (expansionPx 2, rigidity 0.7 — transform.puppet.yaml) collapse to
    // the builder's omission sentinels: engine defaults are never
    // re-stated client-side (the FilterParamDialog default->omit rule),
    // and default-knob commits stay byte-identical to pre-11d
    // envelopes. Density is NOT passed: its control is RESERVED (the
    // value ids fewer/normal/more are unminted — lane log record).
    const int expansionArg = m_puppetExpansion == 2 ? -1 : m_puppetExpansion;
    const double rigidityArg =
        std::abs(m_puppetRigidity - 0.7) < 1e-9 ? -1.0 : m_puppetRigidity;
    m_controller->applyCommand(
        QStringLiteral("transform.puppet"), target,
        buildPuppetParams(pins, QString(), expansionArg, rigidityArg)
            .toVariantMap(),
        {});
}

void CanvasView::beginPerspectiveFrame()
{
    if (!m_controller || !m_controller->hasDocument())
        return;
    // G-3 CRITICAL fix: the engine — contract-correct — solves the
    // homography from the LAYER'S SOURCE BOUNDS onto the submitted quad
    // (transform.perspective.yaml §18.3; TransformPerspectiveKind maps
    // src_rect corners -> quad), so the old doc-bounds frame made EVERY
    // commit on a non-full-canvas layer stretch the layer across the
    // document (silent-wrong, reported applied). The frame must start
    // at the layer's source bounds: handles render around the LAYER
    // (WYSIWYG) and the identity quad maps bounds -> bounds — the
    // existing commits-nothing drop stays a true no-op.
    //
    // ENTRY-TIME derivation (interim until the per-layer-bounds ABI arm
    // lands — ABI-growth request 3, the filed ask): one full-doc 1.4
    // readLayer + alpha scan (layerContentBounds — the thumbnail/cut
    // door). A TYPED refusal (masked layers — the cabi node-refusal
    // matrix) WALLS the session instead of opening blind: status line
    // through the existing ui.status.rejected path (set door-side),
    // queryable here; the silent doc-bounds fallback WAS the bug and
    // never returns. An EMPTY layer (no opaque pixels) has nothing to
    // frame: no session, wall active with an EMPTY code — no wire door
    // refused (the empty bounds ARE the door's answer) and no cataloged
    // text or MessageId exists to name it (i18n is another lane's
    // fence; the audit sweeps quoted dotted ids, so even a
    // queryable-only id would mint — a cataloged empty-layer notice is
    // the queued fold-in; a zero-rect or doc-rect frame would each be
    // a lie).
    m_perspWallActive = false;
    m_perspWallCode.clear();
    m_perspLayerId.clear();
    const QVariantList rows = m_controller->layers();
    const int idx = m_controller->activeLayerIndex();
    const QString layerId = (idx >= 0 && idx < rows.size())
        ? rows.at(idx).toMap().value(QStringLiteral("layerId")).toString()
        : QString();
    const QVariantMap derived = m_controller->layerContentBounds(layerId);
    if (!derived.value(QStringLiteral("ok")).toBool()) {
        m_perspWallActive = true;
        m_perspWallCode = derived.value(QStringLiteral("code")).toString();
        emit perspectiveStateChanged();
        update();
        return;
    }
    const QRectF bounds(derived.value(QStringLiteral("x")).toInt(),
                        derived.value(QStringLiteral("y")).toInt(),
                        derived.value(QStringLiteral("w")).toInt(),
                        derived.value(QStringLiteral("h")).toInt());
    if (bounds.isEmpty()) {
        m_perspWallActive = true;
        m_perspWallCode.clear(); // the empty-layer arm: walled, no wire code
        emit perspectiveStateChanged();
        update();
        return;
    }
    m_perspActive = true;
    m_perspLayerId = layerId;
    m_perspBase = bounds;
    m_perspQuad[0] = m_perspBase.topLeft();
    m_perspQuad[1] = m_perspBase.topRight();
    m_perspQuad[2] = m_perspBase.bottomRight();
    m_perspQuad[3] = m_perspBase.bottomLeft();
    m_perspHandle = -1;
    emit perspectiveStateChanged();
    update();
}

void CanvasView::commitPerspective()
{
    if (!m_perspActive)
        return;
    const bool identity = m_perspQuad[0] == m_perspBase.topLeft()
        && m_perspQuad[1] == m_perspBase.topRight()
        && m_perspQuad[2] == m_perspBase.bottomRight()
        && m_perspQuad[3] == m_perspBase.bottomLeft();
    QPointF quad[4] = {m_perspQuad[0], m_perspQuad[1], m_perspQuad[2],
                       m_perspQuad[3]};
    // G-3: the commit targets the ENTRY-TIME layer the frame was
    // derived for — resolving the active layer here would remap a
    // mid-session layer switch's DIFFERENT source bounds onto this
    // frame's quad (the defect class one level down). A layer deleted
    // mid-session answers as the engine's typed bad_target — honest.
    const QString layerId = std::exchange(m_perspLayerId, {});
    m_perspActive = false;
    m_perspHandle = -1;
    emit perspectiveStateChanged();
    update();
    if (identity || !m_controller)
        return; // no intent — mirror the free-transform identity drop
    QVariantMap target;
    if (!layerId.isEmpty())
        target.insert(QStringLiteral("layerId"), layerId);
    m_controller->applyCommand(
        QStringLiteral("transform.perspective"), target,
        buildPerspectiveQuadParams(quad).toVariantMap(), {});
}

void CanvasView::beginTransformFrame()
{
    if (!m_controller || !m_controller->hasDocument())
        return;
    // ---- v1.0.1 items 19 + 21 ------------------------------------------
    // ITEM 19: the frame is the ACTIVE LAYER'S CONTENT, not the canvas.
    // The comment that stood here — "per-layer bounds are not readable
    // through the flat info surface" — was true when it was written and
    // expired silently when ABI 1.4 landed per-layer readback. G-3 built
    // the derivation on that door for perspective; this site never caught
    // up, so every Cmd+T on a small layer opened a canvas-sized box with
    // its handles nowhere near the pixels they move.
    //
    // WHY THIS IS UX AND NOT CORRECTNESS, unlike G-3. Perspective's frame
    // is load-bearing on the commit: the engine solves the homography from
    // the layer's source rect onto the submitted quad, so a wrong frame is
    // wrong pixels. transform.free's matrix applies AS-IS in document
    // space and `reference` is validated then discarded (verified in
    // TransformFreeKind), so the doc-bounds frame was never wrong — only
    // dishonest about where the layer is.
    //
    // ENTRY-TIME, ONCE. m_transformBase is read-only for the session (the
    // live matrix lives in m_transformNow), so one derivation per frame
    // entry is all this costs — G-3's cadence exactly. REMOVE-ON-ARRIVAL
    // of the per-layer bounds ABI arm (info v4 contentBounds, filed): the
    // engine already keeps Buffer::bounds as an O(1) field and every
    // transform kind frames on it, it is simply not on the wire. When it
    // lands, this readback and BOTH honest fallbacks below dissolve — the
    // §9.5 set_props repaint interim's remove-marked precedent.
    //
    // ITEM 21 rides the same call: the derivation reads the whole layer
    // and used to throw the pixels away; layerContentSnapshot hands them
    // back, so the live preview costs ZERO extra readbacks.
    m_transformWallActive = false;
    m_transformWallCode.clear();
    m_transformLayerId.clear();
    m_transformFrameIsCanvasFallback = false;
    m_transformPreviewUnavailable = false;
    m_transformSnapshot = QImage();
    const QVariantList rows = m_controller->layers();
    const int idx = m_controller->activeLayerIndex();
    const QString layerId = (idx >= 0 && idx < rows.size())
        ? rows.at(idx).toMap().value(QStringLiteral("layerId")).toString()
        : QString();
    if (layerId.isEmpty()) {
        // NO ACTIVE LAYER RESOLVES. Kept ahead of the derivation because
        // the door answers this with its client-side precondition code,
        // and the masked arm below would then tell the user a MASK
        // blocked the read — a confident wrong answer about a layer that
        // was never found. No session, code-less wall (no wire door
        // refused), and no notice: no cataloged id names "no active
        // layer", and minting one for a state the panel does not
        // currently produce would be minting ahead of a surface.
        m_transformActive = false;
        m_transformWallActive = true;
        m_transformWallCode.clear();
        emit transformStateChanged();
        update();
        return;
    }
    QImage snapshot;
    const QVariantMap derived =
        m_controller->layerContentSnapshot(layerId, &snapshot);
    if (!derived.value(QStringLiteral("ok")).toBool()) {
        // MASKED LAYER (F3b): the readback door refuses typed, so there
        // are no bounds and no pixels. The frame opens at CANVAS bounds
        // — LABELLED, never silently, which is the whole difference from
        // the shape G-3 killed.
        //
        // NOT A WALL, deliberately. The engine refuses masked free
        // transform at COMMIT anyway (layer_has_enabled_mask ->
        // masked_transform_refusal), and that refusal already arrives as
        // the item-13 named wall with the frame left live and the user's
        // work intact. Walling at ENTRY would remove a working affordance
        // and claim "you cannot do this" about something the user can in
        // fact still drag, aim and cancel. A guard that hides a defect is
        // a false surface with its polarity reversed.
        m_transformActive = true;
        m_transformLayerId = layerId;
        m_transformBase = QRectF(0, 0, m_controller->docWidth(),
                                 m_controller->docHeight());
        m_transformNow = QTransform();
        m_transformFrameIsCanvasFallback = true;
        m_transformPreviewUnavailable = true;
        // SAY SO. layerContentSnapshot has already put the door's typed
        // refusal on the status line (ui.status.rejected) — true and
        // MATERIALLY INCOMPLETE: it names the read that failed and says
        // nothing about the box the user is now looking at. This replaces
        // it with the consequence, carrying the same wire code.
        m_controller->statusPending(
            QStringLiteral("ui.pending.transform_masked_layer"),
            {{QStringLiteral("code"),
              derived.value(QStringLiteral("code")).toString()}});
        emit transformStateChanged();
        update();
        return;
    }
    const QRectF bounds(derived.value(QStringLiteral("x")).toInt(),
                        derived.value(QStringLiteral("y")).toInt(),
                        derived.value(QStringLiteral("w")).toInt(),
                        derived.value(QStringLiteral("h")).toInt());
    if (bounds.isEmpty()) {
        // EMPTY LAYER: nothing to frame. A zero-rect box and a doc-rect
        // box are each a lie, so there is no session — G-3's ruling,
        // mirrored exactly, INCLUDING the code-less wall (no wire door
        // refused: the empty bounds ARE the door's answer).
        //
        // WHERE THIS DIVERGES FROM G-3, on purpose: G-3 shipped this arm
        // MUTE because no cataloged text existed and minting was out of
        // its fence, and recorded the notice as a queued fold-in. A Cmd+T
        // that silently does nothing is a control present and inert, so
        // the notice is minted here and the arm speaks.
        m_transformActive = false;
        m_transformWallActive = true;
        m_transformWallCode.clear();
        m_controller->statusPending(
            QStringLiteral("ui.status.transform_layer_empty"));
        emit transformStateChanged();
        update();
        return;
    }
    m_transformActive = true;
    m_transformLayerId = layerId;
    m_transformBase = bounds;
    m_transformNow = QTransform();
    if (derived.value(QStringLiteral("snapshot")).toBool()
        && !snapshot.isNull()) {
        m_transformSnapshot = snapshot;
    } else {
        // ABOVE THE 2^26-px READBACK CAP: the bounds are real (the strip
        // scan chunks) but no single read delivers the pixels and no
        // QImage holds them as one plane. Frame and handles are honest;
        // a preview built from one capped strip would not be. Same choice
        // the ants overlay already makes at gigapixel sizes. The general
        // fix is the viewport-windowed snapshot (m_antsWindow's shape) —
        // recorded, not built here.
        m_transformPreviewUnavailable = true;
        m_controller->statusPending(
            QStringLiteral("ui.pending.transform_preview_too_large"));
    }
    emit transformStateChanged();
    update();
}

void CanvasView::commitTransform()
{
    // v1.0.1 item 13: DECOMPOSE the accumulated matrix instead of
    // submitting it blind.
    //  - TRANSLATE-ONLY matrices (linear part identity within 1e-12 —
    //    the integer-translate arm's OWN gate, kinds_layer_geom.rs,
    //    mirrored so this class equals that arm's servable set exactly;
    //    pure-translate gesture chains hold the cells at exactly 1/0,
    //    the epsilon only forgives sub-1e-12 residue from scale/rotate
    //    round-trips) commit through layer.transform's EXACT
    //    integer-translate arm — the same door the move tool uses: the
    //    §18.3 exactness family's pure remap (a move must be bit-exact,
    //    never resampled). dx/dy round to integers BEFORE submission
    //    (the submitMove qRound convention; the served arm demands
    //    integer tx/ty — rounding makes that exact). The drag preview
    //    already snaps to whole pixels, so preview == committed
    //    placement. A rounded (0,0) is no expressible intent — frame
    //    closes, no submission (extends the exact-identity drop).
    //  - Everything else (scale/rotate/shear component) submits
    //    transform.free and branches on the ENGINE's answer — the
    //    engine is the truth, no shell pre-check replaces it. GROUND
    //    (re-established at this build, superseding the stale
    //    arms-are-NI note): the §18.3 resample arms are SERVED
    //    (appname-tools TransformFreeKind — full affine bake), so the
    //    ok arm is the live path. A typed refusal (POSITION/ALL lock
    //    §11.1, target-kind classes, any future NI) is a NAMED WALL:
    //    the status line already carries it (applyCommand →
    //    ui.status.rejected), the frame STAYS LIVE (the user's work is
    //    not thrown away), and the wall is queryable state
    //    (transformWallActive/transformWallCode — the availability-
    //    truth REFUSED half: lock truth is write-only pre-info-v2, so
    //    it cannot be pre-computed into a disabled state). The
    //    translate door branches identically.
    // RECORDED, NOT BUILT (the item-13 design entry defers it to its
    // own slice): import place-style scale-to-fit placement rides this
    // same commit path.
    if (!m_transformActive)
        return;
    const QTransform t = m_transformNow;
    const QPointF ref = m_transformBase.center();
    if (t.isIdentity() || !m_controller) {
        closeTransformFrame(); // no intent — the existing identity drop
        return;
    }
    constexpr double kLinearEps = 1e-12; // the engine arm's identity gate
    // §5 review minor 1: STRICT < to mirror the engine gate's own
    // operator (identity_linear < 1e-12) — at exactly double(1e-12) the
    // <= form classified translate-only and got walled at the integer
    // door (safe but needless); strict < keeps the classes identical.
    const bool translateOnly = std::abs(t.m11() - 1.0) < kLinearEps
        && std::abs(t.m12()) < kLinearEps && std::abs(t.m21()) < kLinearEps
        && std::abs(t.m22() - 1.0) < kLinearEps;
    // item 19: the commit targets the ENTRY-TIME layer the frame was
    // derived for — G-3's idiom one level down. Resolving the active
    // layer HERE (which is what this did) was harmless only while the
    // frame was canvas-shaped for every layer alike: item 19 makes it
    // live, because a mid-session layer switch would then commit layer
    // A's frame geometry — and A's `reference` pivot — against layer B.
    // A layer deleted mid-session answers as the engine's typed
    // bad_target, which is the honest outcome, not a shell guess.
    // NOT std::exchange'd here: the wall arm below leaves the frame LIVE
    // (item 13's contract), and a live frame that has forgotten its layer
    // would commit against the active one on the next Enter. closeTransformFrame
    // owns the clear, and every path out of this function goes through it
    // or deliberately keeps the frame.
    QVariantMap target;
    if (!m_transformLayerId.isEmpty())
        target.insert(QStringLiteral("layerId"), m_transformLayerId);
    QVariantMap result;
    if (translateOnly) {
        const int dx = qRound(t.dx());
        const int dy = qRound(t.dy());
        if (dx == 0 && dy == 0) {
            closeTransformFrame(); // sub-half-pixel: no expressible intent
            return;
        }
        QVariantMap params;
        params.insert(QStringLiteral("matrix"),
                      QVariantList{1, 0, 0, 0, 1, 0, dx, dy, 1});
        result = m_controller->applyCommand(QStringLiteral("layer.transform"),
                                            target, params, {});
    } else {
        // Wire matrix: column-major Mat3 for column vectors — columns
        // (m11,m12,0),(m21,m22,0),(dx,dy,1); det != 0 is contract-checked.
        QVariantMap params;
        params.insert(QStringLiteral("matrix"),
                      QVariantList{t.m11(), t.m12(), 0.0, t.m21(), t.m22(),
                                   0.0, t.dx(), t.dy(), 1.0});
        params.insert(QStringLiteral("reference"),
                      QVariantList{ref.x(), ref.y()});
        params.insert(QStringLiteral("scaleEffects"), false);
        result = m_controller->applyCommand(QStringLiteral("transform.free"),
                                            target, params, {});
    }
    if (result.value(QStringLiteral("ok")).toBool()) {
        closeTransformFrame();
    } else {
        // NAMED WALL: engine truth said no — document unchanged, frame
        // still live, refusal queryable and already on the status line.
        setTransformWall(result.value(QStringLiteral("code")).toString());
        update();
    }
}

void CanvasView::closeTransformFrame()
{
    m_transformActive = false;
    m_transformNow = QTransform();
    m_transformWallActive = false;
    m_transformWallCode.clear();
    // items 19 + 21: the entry-time capture dies with the frame it was
    // taken for. A surviving m_transformSnapshot would paint the OLD
    // layer under the NEXT session (item 10's D2 lesson, one door over).
    m_transformLayerId.clear();
    m_transformFrameIsCanvasFallback = false;
    m_transformPreviewUnavailable = false;
    m_transformSnapshot = QImage();
    emit transformStateChanged();
    update();
}

void CanvasView::setTransformWall(const QString &code)
{
    m_transformWallActive = true;
    m_transformWallCode = code;
    emit transformStateChanged();
}

// OQ-5 (item 21): the scoped invalidation rect for the live transform
// preview.
//
// WHY THE SNAPSHOT'S OWN SIZE IS NOT THE FOOTPRINT. m_transformSnapshot is a
// FULL-DOC-FRAMED readback drawn at doc origin, so mapping ITS rect would
// invalidate the whole canvas on every drag frame and buy nothing at all.
// What it can actually PAINT is bounded by m_transformBase: the frame and
// the snapshot come from the SAME readback at the SAME moment -- item 19
// derives m_transformBase from the alpha bytes of exactly this image -- so
// every pixel outside the frame carries alpha byte 0 and draws nothing.
// That identity is the whole licence for scoping here; if the frame ever
// stops being derived from the snapshot's own alpha, this rect stops being
// safe. (The canvas-fallback arm has no snapshot to bound: readLayer
// refuses masked layers, so no preview is drawn there in the first place.)
//
// BOTH matrices, because the OLD footprint must be ERASED as well as the
// new one painted. A rect covering only the new position leaves a trail --
// a wrong invalidation rect is a VISIBLE lie about where the layer is, and
// it is worse than a slow repaint.
//
// PADDING is all in ITEM space because all of it is constant on screen: the
// 5 px handle half-extent (hs = 5.0 / m_zoom in doc space is 5 px on
// screen), the cosmetic 1 px dashed frame pen, and 2 px of slack for the
// smoothing kernel's support at rest. Then refreshRegion's own 1 px
// alignment slack, for the same reason it takes it.
QRect CanvasView::transformPreviewInvalidateRect(const QTransform &before,
                                                 const QTransform &after) const
{
    const QRectF doc = before.mapRect(m_transformBase)
                           .united(after.mapRect(m_transformBase));
    const QPointF tl = docToView(doc.topLeft());
    const QRectF view(tl, QSizeF(doc.width() * m_zoom, doc.height() * m_zoom));
    const qreal pad = 5.0 + 1.0 + 2.0; // handle half + cosmetic pen + smoothing
    return view.adjusted(-pad, -pad, pad, pad)
        .toAlignedRect()
        .adjusted(-1, -1, 1, 1);
}

void CanvasView::clearTransformWall()
{
    if (!m_transformWallActive)
        return;
    m_transformWallActive = false;
    m_transformWallCode.clear();
    emit transformStateChanged();
}

QVariantList CanvasView::transformPreviewMatrix() const
{
    const QTransform &t = m_transformNow;
    return {t.m11(), t.m12(), 0.0, t.m21(), t.m22(), 0.0,
            t.dx(),  t.dy(),  1.0};
}

bool CanvasView::gestureInFlight() const
{
    // §5 fix round (F1): the exact set of states where the canvas holds
    // a button-driven interaction whose RELEASE must not be stolen by a
    // popup grab. See the header note for the contract.
    return m_panning || m_strokeInProgress || m_gesture != Gesture::None;
}

QPointF CanvasView::contextMenuAnchor()
{
    // v1.0.1 item 7 (§31.1): the keyboard-open anchor. Preference
    // order — the SELECTION's visible bounds center (ants outline
    // truth), the visible document region's center, the view center;
    // clamped into the item so the popup always opens on-canvas.
    // CONTRACT WORDING (§5 review F2, measured): this point is the
    // ANCHOR half of "anchor, then fit-to-window" — near a window edge
    // Qt's window-bounds clamp repositions the popup's FINAL position
    // (e.g. y = windowH − popupHeight − containerOffset while x stays
    // on the anchor). That clamp is the correct UX (the popup stays
    // fully on-screen), not an anchor defect.
    const QRectF view(0, 0, width(), height());
    QRectF target = view;
    if (m_controller && m_controller->hasDocument()) {
        // The VISIBLE document is the output frame: the popup must anchor
        // on-canvas, and under a crop the canvas rect includes area the
        // user cannot see.
        const QRect img = m_controller->imageRect();
        const QRectF docView(docToView(QPointF(img.topLeft())),
                             docToView(QPointF(img.x() + img.width(),
                                               img.y() + img.height())));
        const QRectF visibleDoc = docView.intersected(view);
        if (!visibleDoc.isEmpty())
            target = visibleDoc;
        // Off-paint ants refresh (repaint-scoping slice): the anchor
        // wants the VISIBLE selection bounds — the windowed vector
        // intersected with the view below is exactly that; the escape
        // check re-vectorizes if the window went stale.
        ensureAntsWindow();
        if (m_antsValid && !m_antsPath.isEmpty()) {
            const QRectF sel = m_antsPath.boundingRect();
            const QRectF selView(docToView(sel.topLeft()),
                                 docToView(sel.bottomRight()));
            const QRectF visibleSel = selView.intersected(view);
            if (!visibleSel.isEmpty())
                target = visibleSel;
        }
    }
    QPointF c = target.center();
    c.setX(qBound(0.0, c.x(), qMax(0.0, width())));
    c.setY(qBound(0.0, c.y(), qMax(0.0, height())));
    return c;
}

void CanvasView::onOverlayChanged()
{
    // Overlay truth moved (selection delta | guides | grid). The ants
    // vector refreshes only when the SELECTION revision moved — a guide
    // or grid change repaints without re-reading the mask.
    if (!m_antsValid
        || (m_controller
            && m_antsRevision != m_controller->selectionRevision()))
        recomputeAnts();
    update(); // overlays span the view: full item is the honest scope
}

void CanvasView::recomputeAnts()
{
    // Repaint-scoping slice (sweep F1c): the ants vectorization, OFF the
    // paint path. Reads through the controller's SHARED per-revision
    // full-doc mask when the doc is under the share cap (F1b/F4 — the
    // announcer consumes the same read); past it, a direct
    // viewport-window read; past the ABI's 2^26 px readback cap the ants
    // stay absent (the existing gigapixel guard's semantics — windowed
    // gigapixel ants remain the recorded follow-up).
    m_antsValid = false;
    m_antsPath = QPainterPath();
    m_antsWindow = QRect();
    if (!m_controller || !m_controller->hasDocument()
        || !m_controller->selectionActive() || !abiHasBlock12())
        return;
    const int w = m_controller->docWidth();
    const int h = m_controller->docHeight();
    const QRect docR(0, 0, w, h);
    if (static_cast<qint64>(w) * h > (qint64(1) << 26))
        return;
    // The vectorization window: viewport + half-viewport margin, clipped
    // to the doc (an unsized item — width/height 0 — falls back to the
    // whole doc, the pre-slice shape). A run clipped by an interior
    // window edge draws a false outline ON that edge — it lives in the
    // margin, and ensureAntsWindow recomputes before the viewport can
    // reach it.
    QRect window = docR;
    if (width() > 0 && height() > 0) {
        const QRectF viewDoc(viewToDoc(QPointF(0, 0)),
                             viewToDoc(QPointF(width(), height())));
        window = viewDoc
                     .adjusted(-viewDoc.width() / 2.0,
                               -viewDoc.height() / 2.0,
                               viewDoc.width() / 2.0, viewDoc.height() / 2.0)
                     .toAlignedRect()
                     .intersected(docR);
    }
    QImage mask; // full-doc (shared) or window-sized (direct)
    QPoint maskOrigin(0, 0);
    if (m_controller->selectionMaskFull(&mask)) {
        // Shared read (doc <= share cap): scanlines are doc-origin.
    } else if (!window.isEmpty()) {
        if (!m_controller->readSelectionImage(window, &mask))
            return; // typed refusal / pre-1.2: no ants beats stale ants
        maskOrigin = window.topLeft();
    }
    // >=128 coverage -> row-run region -> outline path (vectorized once
    // per revision/window; feathered edges outline at half coverage).
    QRegion region;
    if (!mask.isNull() && !window.isEmpty()) {
        for (int y = window.top(); y <= window.bottom(); ++y) {
            const uchar *line = mask.constScanLine(y - maskOrigin.y());
            int runStart = -1;
            for (int x = window.left(); x <= window.right() + 1; ++x) {
                const bool on =
                    x <= window.right() && line[x - maskOrigin.x()] >= 128;
                if (on && runStart < 0)
                    runStart = x;
                else if (!on && runStart >= 0) {
                    region += QRect(runStart, y, x - runStart, 1);
                    runStart = -1;
                }
            }
        }
    }
    QPainterPath path;
    path.addRegion(region);
    m_antsPath = path.simplified();
    m_antsWindow = window;
    m_antsRevision = m_controller->selectionRevision();
    m_antsValid = true;
}

void CanvasView::ensureAntsWindow()
{
    // The pan/zoom/geometry escape check (F1c): recompute when the
    // visible doc rect leaves the vectorized window's trustworthy core.
    if (!m_controller || !m_controller->hasDocument()
        || !m_controller->selectionActive() || !abiHasBlock12())
        return; // nothing to show; overlayChanged/reset paths clear
    if (m_antsValid
        && m_antsRevision == m_controller->selectionRevision()) {
        const QRect docR(0, 0, m_controller->docWidth(),
                         m_controller->docHeight());
        const QRect needed = QRectF(viewToDoc(QPointF(0, 0)),
                                    viewToDoc(QPointF(width(), height())))
                                 .toAlignedRect()
                                 .intersected(docR);
        // Trustworthy up to a 1 px guard band on sides INTERIOR to the
        // doc (the false-outline sides); doc-edge sides carry real
        // outlines and stay inclusive.
        QRect safe = m_antsWindow;
        if (safe.left() > docR.left())
            safe.setLeft(safe.left() + 1);
        if (safe.top() > docR.top())
            safe.setTop(safe.top() + 1);
        if (safe.right() < docR.right())
            safe.setRight(safe.right() - 1);
        if (safe.bottom() < docR.bottom())
            safe.setBottom(safe.bottom() - 1);
        if (needed.isEmpty() || safe.contains(needed))
            return;
    }
    recomputeAnts();
}
// G-2 (MAJOR): the STROKE ARM, MOVED VERBATIM out of cancelGesture so that
// focus loss and grab loss can abandon a live stroke without disturbing
// anything else. The body below is the original bytes, comments included --
// its ordering rationale ("flag first, a controller call can emit into
// onStrokeCommitted, which READS this flag") is load-bearing and was NOT
// paraphrased. Guarded and idempotent: a no-op when no stroke is live.
//
// WHY THE CALLERS DO NOT USE cancelGesture(), stated because the wider call
// is the obvious one and it would be wrong: cancelGesture also clears the
// lasso, polygon, pen, transform and perspective contexts. Losing focus to a
// palette must not discard a polygon the user is mid-placement on. Esc says
// "abandon the innermost context" (SS39.2); focus loss says only "you are no
// longer the one receiving input".
void CanvasView::cancelStroke()
{
    if (m_strokeInProgress) {
        // Flag first: a controller call can emit into onStrokeCommitted,
        // which READS this flag. strokeCancel() emits nothing today, but
        // the ordering should not depend on that staying true.
        m_strokeInProgress = false;
        if (m_controller)
            m_controller->strokeCancel();
        // Dropped, NOT parked in m_pendingEchoes: a pending echo is a
        // provisional preview of something the engine rejected. An
        // abandoned stroke was never offered to the engine at all.
        m_activeEcho = EchoStroke{};
    }
}


void CanvasView::cancelGesture()
{
    // G-1 (MAJOR): a LIVE STROKE is an innermost in-progress context, and
    // §39.2 [NORMATIVE] says Esc resolves the innermost in-progress context.
    // Every other context below honours it — lasso, polygon, pen, transform,
    // perspective, puppet — so its absence here was an omission from that
    // ladder, not a ruled exception. Without this, Esc left
    // m_strokeInProgress true, mouseReleaseEvent still called strokeEnd(),
    // and the release COMMITTED the stroke the user had just cancelled.
    //
    // ★ SCOPE (round-4 MINOR-3) — THIS ARM SERVES THREE CALL SITES, NOT
    // ONE. cancelGesture() is reached from Escape (keyPressEvent), from the
    // documentReset connection in setController(), and from setTool(). The
    // §39.2 argument above is the Escape rationale; the stroke arm changed
    // the other two behaviours as well, deliberately and stated here rather
    // than left to be discovered:
    //
    //   • DOCUMENT RESET WITH A LIVE STROKE now runs strokeCancel() and
    //     drops the echo. Previously the controller stayed armed holding the
    //     PREVIOUS document's samples — the V-D4a hazard this connection
    //     exists to prevent, which onDocumentReset never actually closed.
    //     PINNED, both directions, by brushEscapeMidStrokeDoesNotCommit in
    //     tst_selection_gestures.cpp (ctest #20) — QUALIFIED DELIBERATELY,
    //     because the bare name is AMBIGUOUS: tst_gesture_net.cpp (ctest
    //     #26) defines a case of the SAME NAME carrying ONE assertion,
    //     where this file's carries 24. Both binaries are registered, so a
    //     bare-name citation resolves to either — and the weak one would
    //     flatter this pin without anyone noticing.
    //     and round-5 MINOR-2 corrects HOW that is known. Round 4 wrote
    //     "both directions" while only ONE was measured: MUT-F reds at the
    //     CONTROLLER pin, and no mutation in that battery reached the
    //     CANVAS pin at all (MUT-B, the only other flag-side mutation, reds
    //     on the Esc arm ~180 lines above it and never runs this arm).
    //     The canvas direction was ASSERTED, not demonstrated — the same
    //     "asserted, not measured" defect round 4 itself convened to fix.
    //     Round 5 ran the missing mutation, MUT-G: delete the
    //     documentReset -> cancelGesture connection in setController() and
    //     the canvas pin reds. Both directions are now MEASURED, one
    //     mutation each.
    //
    //     WHICH BINARY MUT-G ACTUALLY REDDENED — asked because the mutation
    //     ran before the ambiguity above was noticed, and its log names only
    //     the bare case. It can only have been the tst_selection_gestures
    //     one. MUT-G deletes the documentReset -> cancelGesture connection;
    //     a case that never reaches the reset path cannot observe its
    //     absence. Measured inside each same-named case body, COMMENTS
    //     STRIPPED FIRST (i18n/tools/audit.py: strip_cxx_comments) --
    //     matching 'documentReset|newDocument|resetDocument':
    //           tst_gesture_net.cpp        -> 0
    //           tst_selection_gestures.cpp -> 6
    //     The stripping is not fastidiousness. A RAW grep answers 2 and 13,
    //     and the twin's 2 are both inside the comment that explains why
    //     that case avoids a reset -- an explanation naming the identifier
    //     it avoids. Run naively, the instrument reports this argument as
    //     EXPIRED at the exact moment the code most carefully preserves it.
    //     This is a STRUCTURAL argument, not a re-run: the twin has no path
    //     to the mutated edge, so no execution of it could have gone red.
    //     It expires if that twin ever gains reset coverage — written here
    //     so the expiry is visible rather than left to be inferred.
    //
    //   • TOOL SWITCH MID-STROKE now ABANDONS the stroke instead of leaving
    //     it live to be committed later under the ORIGINAL m_strokeKind (a
    //     brush stroke landing after the user selected the eraser). More
    //     correct, and consistent with the same switch dropping an
    //     uncommitted transform frame one line below — but it is an
    //     expansion beyond the Esc fix, and it is currently UNPINNED. Any
    //     test that drives setTool() mid-stroke should assert the abandon,
    //     not the commit.
    cancelStroke();
    m_gesture = Gesture::None;
    m_moveEchoLayer = QImage();
    m_lassoPoints.clear();
    m_polygonActive = false;
    m_penAnchors.clear();
    m_penActive = false;
    // item 19: `|| m_transformWallActive` is REQUIRED now, not belt-and-
    // braces. Before item 19 a transform wall could only exist ON a live
    // frame (item 13 sets it from a commit refusal and keeps the frame),
    // so m_transformActive covered it. The empty-layer arm creates the
    // state G-3 already had one door over — WALL WITHOUT SESSION — and
    // without this the wall outlives the tool switch that should drop it.
    // Same shape as the m_perspActive || m_perspWallActive test below.
    if (m_transformActive || m_transformWallActive)
        closeTransformFrame(); // drops any wall with the frame
    if (m_perspActive || m_perspWallActive) {
        m_perspActive = false;
        m_perspHandle = -1;
        m_perspLayerId.clear();
        m_perspWallActive = false;
        m_perspWallCode.clear();
        emit perspectiveStateChanged();
    }
    if (m_puppetActive) {
        m_puppetActive = false;
        m_puppetPins.clear();
        m_puppetSelected = -1;
    }
    update();
}

void CanvasView::setPenCommitsShape(bool shape)
{
    if (m_penCommitsShape == shape)
        return;
    m_penCommitsShape = shape;
    emit penCommitsShapeChanged();
}

// ---- v1.0.1 item 11d Phase 1: options-bar bound state -----------------
// Setters carry values VERBATIM (no clamp): range exactness is the
// options-bar control's duty (the envelope_builder.h caller rule), and
// an out-of-range value submitted anyway meets the ENGINE's typed
// refusal — the honest surface, exercised by the options-bar net.

void CanvasView::setTextFontSize(double sizePt)
{
    if (qFuzzyCompare(m_textFontSize, sizePt))
        return;
    m_textFontSize = sizePt;
    emit textFontSizeChanged();
}

// Pro-Wave row 4: carried VERBATIM like the other 11d bound state (no
// clamp) — the control owns its range, the ENGINE owns the refusal.
// qFuzzyCompare cannot compare against 0, so the guard is an exact
// compare here (0.0 is this property's default and a REACHABLE value:
// it is the contract's no-stroke arm, not a sentinel).
void CanvasView::setShapeStrokeWidth(double widthPx)
{
    if (m_shapeStrokeWidth == widthPx)
        return;
    m_shapeStrokeWidth = widthPx;
    emit shapeStrokeWidthChanged();
}

void CanvasView::setPuppetExpansion(int px)
{
    if (m_puppetExpansion == px)
        return;
    m_puppetExpansion = px;
    emit puppetExpansionChanged();
}

void CanvasView::setPuppetRigidity(double rigidity)
{
    if (qFuzzyCompare(m_puppetRigidity, rigidity))
        return;
    m_puppetRigidity = rigidity;
    emit puppetRigidityChanged();
}

void CanvasView::setPuppetShowMesh(bool show)
{
    if (m_puppetShowMesh == show)
        return;
    m_puppetShowMesh = show;
    emit puppetShowMeshChanged();
    update(); // live overlay flips with the toggle
}

void CanvasView::commitPen(bool closed)
{
    const QVector<PenAnchor> anchors = std::exchange(m_penAnchors, {});
    m_penActive = false;
    m_gesture = Gesture::None;
    update();
    if (anchors.size() < 2 || !m_controller)
        return; // a single click is not a path — no intent
    const QJsonObject path = buildVectorPathJson(anchors, closed);
    // Dedupe (V-D1) may have reduced the session below a drawable path
    // (e.g. two clicks on one snap target): zero segments = no intent.
    if (path.value(QLatin1String("subpaths")).toArray().first().toObject()
            .value(QLatin1String("segments")).toArray().isEmpty())
        return;
    if (m_penCommitsShape) {
        // Inline path -> visible shape layer (fill from the brush color,
        // no stroke). No readback dependency: the layer lands in info.
        QVariantMap fill;
        fill.insert(QStringLiteral("type"), QStringLiteral("solid"));
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"),
                     QVariantList{m_brushColor.redF(), m_brushColor.greenF(),
                                  m_brushColor.blueF(), 0.0});
        fill.insert(QStringLiteral("color"), color);
        QVariantMap stroke;
        stroke.insert(QStringLiteral("widthPx"), 0.0);
        QVariantMap params;
        params.insert(QStringLiteral("path"), path.toVariantMap());
        params.insert(QStringLiteral("fill"), fill);
        params.insert(QStringLiteral("stroke"), stroke);
        params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
        m_controller->applyCommand(QStringLiteral("vector.shape_add"), {}, params, {});
    } else {
        const QString pathId = m_controller->newUuid();
        QVariantMap params;
        params.insert(QStringLiteral("path"), path.toVariantMap());
        params.insert(QStringLiteral("newPathId"), pathId);
        const QVariantMap r = m_controller->applyCommand(
            QStringLiteral("vector.path_new"), {}, params, {});
        if (r.value(QStringLiteral("ok")).toBool())
            m_controller->trackCreatedPath(pathId);
    }
}

void CanvasView::submitShapePrimitive()
{
    const QRectF r = QRectF(m_gestureStartDoc, m_gestureCurrentDoc).normalized();
    update();
    if (r.width() < 1.0 || r.height() < 1.0 || !m_controller)
        return;
    QVariantMap geometry;
    geometry.insert(QStringLiteral("x"), r.x());
    geometry.insert(QStringLiteral("y"), r.y());
    geometry.insert(QStringLiteral("w"), r.width());
    geometry.insert(QStringLiteral("h"), r.height());
    QVariantMap primitive;
    primitive.insert(QStringLiteral("kind"),
                     m_tool == QLatin1String("shape.ellipse")
                         ? QStringLiteral("ellipse")
                         : QStringLiteral("rect"));
    primitive.insert(QStringLiteral("geometry"), geometry);
    QVariantMap fill;
    fill.insert(QStringLiteral("type"), QStringLiteral("solid"));
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"),
                 QVariantList{m_brushColor.redF(), m_brushColor.greenF(),
                              m_brushColor.blueF(), 0.0});
    fill.insert(QStringLiteral("color"), color);
    // Pro-Wave row 4: the options-bar stroke width rides here. `stroke`
    // is REQUIRED on vector.shape_add, so the field is always emitted;
    // at the default (0.0 = the contract's own `stroke: {widthPx: 0}`)
    // this envelope is byte-identical to the pre-row one. NO stroke
    // PAINT is emitted: the §20.2 `color`/`fill` keys have no cataloged
    // label to put on a swatch, so the paint stays HELD and the engine's
    // own recorded absent-paint posture (opaque black — vector_ops.rs
    // stroke_paint, the fill leg's fill_to_linear_rgba(None) arm)
    // stands. A width the engine cannot serve over these path bounds
    // (its band-plane ceiling) refuses TYPED and reaches the user on
    // the existing ui.status.rejected path — never a fake success.
    QVariantMap stroke;
    stroke.insert(QStringLiteral("widthPx"), m_shapeStrokeWidth);
    QVariantMap params;
    params.insert(QStringLiteral("primitive"), primitive);
    params.insert(QStringLiteral("fill"), fill);
    params.insert(QStringLiteral("stroke"), stroke);
    params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    m_controller->applyCommand(QStringLiteral("vector.shape_add"), {}, params, {});
}

void CanvasView::submitText()
{
    // Slice T mechanics (contracts text.add.yaml): a click places POINT
    // text at the anchor; a drag defines a PARAGRAPH box (IRect). The
    // §40.7 layout cross-fields are enforced by the builders. Text is
    // empty — the contract default; the typing surface arrives with the
    // chrome mint batch. Pre-service the door refuses typed-NI and the
    // status line surfaces it (the slice-V flip pattern: zero shell
    // changes at service arrival).
    if (!m_controller)
        return;
    const QRectF drag =
        QRectF(m_gestureStartDoc, m_gestureCurrentDoc).normalized();
    const bool isClick = drag.width() < 3.0 / m_zoom
        && drag.height() < 3.0 / m_zoom;
    // 11d Phase 1: the options-bar font-size state rides style.size
    // (CONSUMED by the door — crates/appname-text/src/door.rs
    // char_style_of, range 0.1..=1296). At the engine's own default run
    // size (12 pt, CharStyle::default) the field is OMITTED — the
    // default->omit sentinel rule; default submissions stay
    // byte-identical to pre-11d envelopes. font_family/font_style ship
    // NO controls this slice: the door validates style.font then
    // DISCARDS it ("font does not resolve in v1" — name matching is
    // the recorded C-6 arrival), so a family/style control would do
    // nothing — the honest-state rule reserves both (lane log record).
    QJsonObject style;
    if (std::abs(m_textFontSize - 12.0) > 1e-9)
        style.insert(QLatin1String("size"), m_textFontSize);
    QJsonObject params;
    if (isClick) {
        params = buildTextAddPointParams(m_gestureStartDoc, QString(),
                                         m_controller->newUuid(), style);
    } else {
        const QRect box = drag.toAlignedRect();
        if (box.width() < 1 || box.height() < 1)
            return;
        params = buildTextAddParagraphParams(box, QString(),
                                             m_controller->newUuid(), style);
    }
    m_controller->applyCommand(QStringLiteral("text.add"), {},
                               params.toVariantMap(), {});
}

int CanvasView::transformHandleAt(const QPointF &docPos) const
{
    if (!m_transformActive)
        return -1;
    const QRectF b = m_transformBase;
    const QPointF handles[8] = {
        b.topLeft(), QPointF(b.center().x(), b.top()), b.topRight(),
        QPointF(b.right(), b.center().y()), b.bottomRight(),
        QPointF(b.center().x(), b.bottom()), b.bottomLeft(),
        QPointF(b.left(), b.center().y())};
    const double tol = 8.0 / m_zoom;
    for (int i = 0; i < 8; ++i)
        if (QLineF(docPos, m_transformNow.map(handles[i])).length() <= tol)
            return i;
    return -1;
}

void CanvasView::drawToolOverlays(QPainter *painter) const
{
    QPen dash(QColor(0x20, 0x60, 0xc0), 0); // width 0 = cosmetic (1px at any zoom)
    dash.setCosmetic(true);
    dash.setStyle(Qt::DashLine);

    if (m_gesture == Gesture::Marquee || m_gesture == Gesture::CropDrag) {
        const QRectF r = QRectF(m_gestureStartDoc, m_gestureCurrentDoc).normalized();
        painter->setBrush(Qt::NoBrush);
        // White underlay first (P4 polish: rubber bands stay visible on
        // any content, matching the ants convention).
        QPen under(QColor(0xff, 0xff, 0xff, 0xa0), 0);
        under.setCosmetic(true);
        painter->setPen(under);
        if (m_tool == QLatin1String("select.ellipse"))
            painter->drawEllipse(r);
        else
            painter->drawRect(r);
        painter->setPen(m_gesture == Gesture::CropDrag ? QPen(QColor(0xc0, 0x90, 0x20), 0,
                                                              Qt::DashLine)
                                                       : dash);
        if (m_tool == QLatin1String("select.ellipse"))
            painter->drawEllipse(r);
        else
            painter->drawRect(r);
    }
    if (m_gesture == Gesture::Lasso && m_lassoPoints.size() >= 2) {
        painter->setPen(dash);
        painter->setBrush(Qt::NoBrush);
        QPainterPath path(m_lassoPoints.first());
        for (int i = 1; i < m_lassoPoints.size(); ++i)
            path.lineTo(m_lassoPoints.at(i));
        painter->drawPath(path);
    }
    if (m_polygonActive && !m_lassoPoints.isEmpty()) {
        painter->setPen(dash);
        painter->setBrush(Qt::NoBrush);
        QPainterPath path(m_lassoPoints.first());
        for (int i = 1; i < m_lassoPoints.size(); ++i)
            path.lineTo(m_lassoPoints.at(i));
        path.lineTo(m_hoverDoc); // live segment to the cursor
        painter->drawPath(path);
    }
    if (m_penActive && !m_penAnchors.isEmpty()) {
        // Path-in-progress: cubic preview with the SAME control rules the
        // commit uses (envelope_builder), plus anchor/handle markers.
        painter->setBrush(Qt::NoBrush);
        painter->setPen(dash);
        QPainterPath preview(m_penAnchors.first().anchor);
        for (int i = 1; i < m_penAnchors.size(); ++i) {
            const PenAnchor &a = m_penAnchors.at(i - 1);
            const PenAnchor &b = m_penAnchors.at(i);
            const QPointF c1 = a.hasHandle ? a.handleOut
                                           : a.anchor + (b.anchor - a.anchor) / 3.0;
            const QPointF hin = b.anchor - (b.handleOut - b.anchor);
            const QPointF c2 = b.hasHandle
                ? hin
                : a.anchor + 2.0 * (b.anchor - a.anchor) / 3.0;
            preview.cubicTo(c1, c2, b.anchor);
        }
        // Live segment from the last committed anchor to the cursor
        // (review V-D2) — same control rule as the commit: handleOut if
        // pulled, else collinear straight.
        if (m_gesture != Gesture::PenHandleDrag) {
            const PenAnchor &last = m_penAnchors.constLast();
            const QPointF c1 = last.hasHandle
                ? last.handleOut
                : last.anchor + (m_hoverDoc - last.anchor) / 3.0;
            const QPointF c2 = last.anchor + 2.0 * (m_hoverDoc - last.anchor) / 3.0;
            QPainterPath live(last.anchor);
            live.cubicTo(c1, c2, m_hoverDoc);
            painter->drawPath(live);
        }
        painter->drawPath(preview);
        const double hs = 3.0 / m_zoom;
        painter->setPen(Qt::NoPen);
        painter->setBrush(QColor(0x20, 0x60, 0xc0));
        for (const PenAnchor &a : m_penAnchors) {
            painter->drawRect(QRectF(a.anchor.x() - hs, a.anchor.y() - hs,
                                     hs * 2, hs * 2));
            if (a.hasHandle) {
                painter->drawEllipse(a.handleOut, hs * 0.8, hs * 0.8);
                painter->drawEllipse(a.anchor - (a.handleOut - a.anchor),
                                     hs * 0.8, hs * 0.8);
            }
        }
    }
    if (m_gesture == Gesture::ShapeDrag || m_gesture == Gesture::TextDrag) {
        // TextDrag shares the rect preview: the drag defines the
        // paragraph box (a click-extent drag draws nothing visible).
        const QRectF r = QRectF(m_gestureStartDoc, m_gestureCurrentDoc).normalized();
        painter->setBrush(Qt::NoBrush);
        QPen under(QColor(0xff, 0xff, 0xff, 0xa0), 0);
        under.setCosmetic(true);
        painter->setPen(under);
        if (m_tool == QLatin1String("shape.ellipse"))
            painter->drawEllipse(r);
        else
            painter->drawRect(r);
        QPen over(QColor(0x30, 0xa0, 0x60), 0);
        over.setCosmetic(true);
        over.setStyle(Qt::DashLine);
        painter->setPen(over);
        if (m_tool == QLatin1String("shape.ellipse"))
            painter->drawEllipse(r);
        else
            painter->drawRect(r);
    }
    if (m_gesture == Gesture::MoveDrag) {
        // v1.0.1 item 10: the composite base stays UNMOVED (painted
        // above — other layers never shift in preview); the ACTIVE
        // layer's captured readback draws at the drag offset. The
        // origin ghost under it is the recorded interim look (the
        // engine-true minus-one-layer base = the exclude-layer readback
        // ABI ask, filed). Null capture (typed-refusal layer classes) =
        // the pre-1.4 whole-composite echo, the recorded fallback.
        // Review D3: the capture is layer CONTENT, not contribution
        // (the read serves opacity/blend/visibility-agnostic pixels —
        // a 30%-opacity or non-normal-blend layer previews at full
        // content strength ×0.6): the same honest-interim class as the
        // ghost, resolved by the same ABI ask.
        const QPointF d = m_gestureCurrentDoc - m_gestureStartDoc;
        painter->save();
        painter->setOpacity(0.6);
        // Absolute doc space. The composite FALLBACK is m_cache, which now
        // starts at m_cacheOrigin rather than doc(0,0), so the drag delta
        // is applied FROM that origin. m_moveEchoLayer is a layer readback
        // and keeps its own doc-origin framing.
        if (m_moveEchoLayer.isNull())
            painter->drawImage(d + QPointF(m_cacheOrigin), m_cache);
        else
            painter->drawImage(d, m_moveEchoLayer);
        painter->restore();
    }
    if (m_transformActive) {
        // ---- v1.0.1 ITEM 21: the live transform preview ----------------
        // The same three moves as item 10's move echo, with a QTransform
        // instead of a delta: capture ONCE at session entry (done in
        // beginTransformFrame, on the readback item 19 was already
        // paying for), paint the capture under the accumulated matrix
        // over the UNMOVED base composite, drop it when the frame ends.
        //
        // WHAT THIS EXPLICITLY DOES NOT INHERIT FROM ITEM 10: its
        // whole-composite fallback. Item 10 falls back to transforming
        // m_cache when the per-layer read refuses, which under a small
        // translate is noisy but locally plausible. Under a SCALE or
        // ROTATE it would make every other layer appear to scale and
        // rotate — the operator-visible lie item 19 exists to kill,
        // amplified. When there is no snapshot there is no preview, and
        // beginTransformFrame says so on the status line.
        //
        // PRESENTATION-ONLY, and the spec licenses exactly this: ES §39
        // "commits are the only mutations" and §12.3 rule 5 "previews
        // never mutate tiles — they render into viewport-side buffers".
        // No engine call happens per drag frame; submitting transform.free
        // per frame would be a mutation per frame. §12.3 rule 4's identity
        // guarantee is about PARAMS, not pixels — the shell commits
        // exactly the m_transformNow it previewed, so the rule holds even
        // though Qt's transform is not §18.2's Lanczos/bicubic.
        //
        // Drawn at doc ORIGIN, not at m_transformBase.topLeft(): the
        // snapshot is a full-doc-framed readback, so its own (0,0) is the
        // document's. Offsetting it by the frame corner would translate
        // the preview by the layer's own offset, twice.
        if (!m_transformSnapshot.isNull()) {
            painter->save();
            painter->setOpacity(0.6);
            painter->setTransform(m_transformNow, true);
            // ES §12.3 rule 2 blesses the drag-vs-settle split
            // ("bilinear during interaction"); smoothing only at rest is
            // the same choice the puppet preview makes.
            painter->setRenderHint(QPainter::SmoothPixmapTransform,
                                   m_gesture == Gesture::None);
            painter->drawImage(QPointF(0, 0), m_transformSnapshot);
            painter->restore();
        }
        // HONEST LIMIT, inherited from item 10 and recorded rather than
        // hidden: the base composite underneath is UNMOVED, so the
        // layer's ORIGIN GHOST stays visible beneath the preview — and it
        // is far more visible under a large scale/rotate than under item
        // 10's small translate. A true minus-one-layer base is the filed
        // exclude-layer readback ABI ask (engine kernels: blend modes,
        // masks, opacity), NOT shell math. Same for the capture being
        // layer CONTENT rather than CONTRIBUTION (opacity/blend-agnostic).
        painter->save();
        painter->setTransform(m_transformNow, true);
        QPen framePen(QColor(0x30, 0xa0, 0x60), 0);
        framePen.setCosmetic(true);
        framePen.setStyle(Qt::DashLine);
        painter->setPen(framePen);
        painter->setBrush(Qt::NoBrush);
        painter->drawRect(m_transformBase);
        painter->restore();
        // Handles in un-transformed doc space (constant screen size).
        const QRectF b = m_transformBase;
        const QPointF handles[8] = {
            b.topLeft(), QPointF(b.center().x(), b.top()), b.topRight(),
            QPointF(b.right(), b.center().y()), b.bottomRight(),
            QPointF(b.center().x(), b.bottom()), b.bottomLeft(),
            QPointF(b.left(), b.center().y())};
        painter->setPen(Qt::NoPen);
        painter->setBrush(QColor(0x30, 0xa0, 0x60));
        const double hs = 5.0 / m_zoom;
        for (const QPointF &h : handles) {
            const QPointF p = m_transformNow.map(h);
            painter->drawRect(QRectF(p.x() - hs, p.y() - hs, hs * 2, hs * 2));
        }
    }
    if (m_perspActive) {
        // Quad frame + four independent corner handles (same palette as
        // the free-transform frame; the quad polygon replaces the rect).
        QPen framePen(QColor(0x30, 0xa0, 0x60), 0);
        framePen.setCosmetic(true);
        framePen.setStyle(Qt::DashLine);
        painter->setPen(framePen);
        painter->setBrush(Qt::NoBrush);
        const QPolygonF quad({m_perspQuad[0], m_perspQuad[1], m_perspQuad[2],
                              m_perspQuad[3]});
        painter->drawPolygon(quad);
        painter->setPen(Qt::NoPen);
        painter->setBrush(QColor(0x30, 0xa0, 0x60));
        const double hs = 5.0 / m_zoom;
        for (int i = 0; i < 4; ++i)
            painter->drawRect(QRectF(m_perspQuad[i].x() - hs,
                                     m_perspQuad[i].y() - hs, hs * 2, hs * 2));
    }
    if (m_puppetActive) {
        // 11d Phase 1 — ui.options.puppet_show_mesh (D2-ruled
        // PRESENTATION-ONLY, a real overlay): the live session's pin
        // ADJACENCY GRAPH — every pin pair joined — i.e. the pending
        // envelope's own pin-set state visualized (one solve couples
        // every pin). The engine's §45.5 triangulation has NO readback
        // door; a shell-invented triangle mesh would fake engine truth
        // and is deliberately NOT drawn (recorded in the lane log).
        // Drawn UNDER the pin dots; default off keeps pre-11d frames
        // byte-identical.
        if (m_puppetShowMesh && m_puppetPins.size() >= 2) {
            QPen mesh(QColor(0x30, 0xa0, 0x60, 0x90), 0);
            mesh.setCosmetic(true);
            mesh.setStyle(Qt::DashLine);
            painter->setPen(mesh);
            painter->setBrush(Qt::NoBrush);
            for (int i = 0; i < m_puppetPins.size(); ++i)
                for (int j = i + 1; j < m_puppetPins.size(); ++j)
                    painter->drawLine(m_puppetPins.at(i).pos,
                                      m_puppetPins.at(j).pos);
        }
        // Pin markers: filled dot, white under-ring for contrast on any
        // content, selected pin ringed in the frame palette; a rotation
        // pin draws its dial tick at the set angle.
        const double pr = 6.0 / m_zoom;
        for (int i = 0; i < m_puppetPins.size(); ++i) {
            const PuppetPin &pin = m_puppetPins.at(i);
            QPen ring(QColor(0xff, 0xff, 0xff, 0xd0), 0);
            ring.setCosmetic(true);
            painter->setPen(ring);
            painter->setBrush(i == m_puppetSelected
                                  ? QColor(0x30, 0xa0, 0x60)
                                  : QColor(0x2a, 0x2b, 0x2e));
            painter->drawEllipse(pin.pos, pr, pr);
            if (pin.hasRotation) {
                const double rad = qDegreesToRadians(pin.rotationDeg);
                QPen tick(QColor(0x30, 0xa0, 0x60), 0);
                tick.setCosmetic(true);
                painter->setPen(tick);
                painter->drawLine(pin.pos,
                                  pin.pos + QPointF(std::cos(rad), std::sin(rad))
                                          * (pr * 2.5));
            }
        }
    }
}

void CanvasView::strokePointFromEvent(QMouseEvent *event)
{
    const QPointF doc = viewToDoc(event->position());
    const double tSeconds = m_strokeStartMs == 0 || event->timestamp() < m_strokeStartMs
        ? 0.0
        : (event->timestamp() - m_strokeStartMs) / 1000.0;
    // v1 pointer source is the mouse: pressure 1.0 (post-calibration rule
    // §15.1; tablet input arrives with the CP-0.5 UI wave).
    const double pressure = 1.0;
    m_activeEcho.points.append(doc);
    m_activeEcho.pressures.append(static_cast<float>(pressure));
    m_controller->strokeAppend(doc.x(), doc.y(), pressure, tSeconds);
    update();
}

} // namespace appname::shell
