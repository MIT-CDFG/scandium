// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.2 "Qt/QML shell v1" row — canvas item blitting composite
// readback (appname_doc_read_region -> QImage; shared-texture path is
// CP-0.5), zoom/pan (scroll/drag + fit/100%), brush + eraser tools with
// LIVE stroke echo. ES §9.5 (repaint only dirty regions).
//
// SEPARATION (round-1 audit rule, O-QA re-greps): this item displays what
// readback gives it. The only pixel writes are verbatim byte copies of
// engine-delivered RGBA8888 regions into a cache image; the stroke echo is
// a PREVIEW overlay drawn by QPainter in view space (presentation, not
// document math). No blend-mode/color math exists here; committed strokes
// are repainted from engine dirty-rects, replacing the preview.

#pragma once

#include "document_controller.h"

#include <QImage>
#include <QPainterPath>
#include <QPixmap>
#include <QQuickPaintedItem>
#include <QVariantList>

#include <optional>

namespace appname::shell {

class CanvasView : public QQuickPaintedItem {
    Q_OBJECT
    Q_PROPERTY(appname::shell::DocumentController *controller READ controller
                   WRITE setController NOTIFY controllerChanged)
    Q_PROPERTY(double zoom READ zoom NOTIFY zoomChanged)
    Q_PROPERTY(int zoomPercent READ zoomPercent NOTIFY zoomChanged)
    // W-18 (Windows port): an AUTOMATIC re-fit (a viewport resize while the
    // user has not navigated) that leaves zoomPercent unchanged emits no
    // zoomChanged, so the shell does not announce a zoom level that did not
    // move. User zoom actions (Fit, 1:1, +/-, wheel) are unaffected. On by
    // default on Windows only -- the frozen macOS tree keeps its behaviour;
    // tests may switch it on.
    Q_PROPERTY(bool refitSilentWhenUnchanged MEMBER m_refitSilentWhenUnchanged)
    Q_PROPERTY(QString tool READ tool WRITE setTool NOTIFY toolChanged)
    Q_PROPERTY(QColor brushColor READ brushColor WRITE setBrushColor NOTIFY brushColorChanged)
    Q_PROPERTY(double brushSize READ brushSize WRITE setBrushSize NOTIFY brushSizeChanged)
    // Item 4: brush.stroke's hardness override (contract 0..=1, absent =
    // preset hardness). STROKE-family state, so it takes the brushSize
    // posture (setter clamps; strokeBegin sanitizes) — NOT the 11d
    // verbatim rule below: a stray out-of-range stroke param would void
    // the WHOLE committed gesture at stage 3, the recorded sizePx
    // rationale (document_controller.cpp strokeBegin). Default 0.0 = the
    // built-in soft-round-pressure preset's own hardness (§15.7).
    // O-29: this is the DISPLAYED value. What reaches the wire is
    // brushHardnessOverride() — unset until the row is stated, so an
    // untouched row omits the param and the preset governs.
    Q_PROPERTY(double brushHardness READ brushHardness WRITE setBrushHardness
                   NOTIFY brushHardnessChanged)
    Q_PROPERTY(int pendingPreviewCount READ pendingPreviewCount NOTIFY pendingPreviewCountChanged)
    Q_PROPERTY(double wandTolerance READ wandTolerance WRITE setWandTolerance
                   NOTIFY wandToleranceChanged)
    Q_PROPERTY(bool wandContiguous READ wandContiguous WRITE setWandContiguous
                   NOTIFY wandContiguousChanged)
    // Tier-1 row 5: the shared sample-scope toggle for the three tools
    // the order names (select.wand -> selection.set, heal.stroke,
    // clone.stroke) — ONE property across their options rows, the
    // brushSize shared-row precedent. The value is intent only; each
    // emission site wires it per ITS contract (wand / spot heal:
    // sampleAllLayers:true; clone: source:"composite" +
    // sampleAllLayers:true, the §45.2 "all" scope). Unchecked = the key
    // is OMITTED (engine default false — the default-omission rule).
    Q_PROPERTY(bool sampleAllLayers READ sampleAllLayers WRITE setSampleAllLayers
                   NOTIFY sampleAllLayersChanged)
    // Pen commit mode: true -> vector.shape_add with the inline path (a
    // visible shape layer); false -> vector.path_new (doc-level path +
    // transient from-path action via the controller).
    Q_PROPERTY(bool penCommitsShape READ penCommitsShape WRITE setPenCommitsShape
                   NOTIFY penCommitsShapeChanged)
    // v1.0.1 item 11d Phase 1 — options-bar bound state. Values are
    // carried VERBATIM (no clamp): contract exactness is the CALLER's
    // duty (the envelope_builder.h builder rule) — the options-bar
    // controls own their ranges, and the engine's typed refusal is the
    // honest surface for anything else (tests exercise exactly that).
    //
    // textFontSize: text.add style.size in pt (contract 0.1..=1296).
    // Default 12.0 = the ENGINE's own default run size (CharStyle::
    // default, crates/appname-text/src/style.rs) — submitText OMITS
    // style.size at the default (the FilterParamDialog default->omit
    // sentinel rule: engine defaults are never re-stated), so default
    // submissions stay byte-identical to pre-11d envelopes.
    Q_PROPERTY(double textFontSize READ textFontSize WRITE setTextFontSize
                   NOTIFY textFontSizeChanged)
    // Pro-Wave row 4 — shapeStrokeWidth: vector.shape_add's
    // `stroke.widthPx` (§20.2 VectorStroke). Cited on the command this
    // property actually emits, not on its sibling: contracts/commands/
    // vector.shape_add.yaml:55 `stroke: {$ref: #/$defs/vectorStroke}` →
    // :135 vectorStroke → :139 `widthPx: {type: number, minimum: 0}`.
    // (vector.shape_set_style.yaml carries a vectorStroke of its own at
    // :80/:84; submitShapePrimitive never emits that command.) The
    // contract pins NO MAXIMUM — vector.shape_add.yaml:139 says so
    // outright ("no range pinned for VectorStroke") — so the control's
    // upper bound is a PRESENTATION election, not a contract bound (the
    // brush_size precedent: contract 0.1..=10000, options bar shows
    // 0..200). Default 0.0 IS the contract's own shape_add default
    // (`stroke: {widthPx: 0}`), so a default submission is byte-identical
    // to the pre-row envelope — the same default->unchanged posture the
    // text/puppet rows hold.
    Q_PROPERTY(double shapeStrokeWidth READ shapeStrokeWidth WRITE
                   setShapeStrokeWidth NOTIFY shapeStrokeWidthChanged)
    // transform.puppet knobs (§45.5; contract transform.puppet.yaml):
    // expansionPx 0..=32 default 2, rigidity 0..=1 default 0.7 — both
    // ride buildPuppetParams' sentinel omission at their contract
    // defaults (same default->omit rule as above).
    Q_PROPERTY(int puppetExpansion READ puppetExpansion WRITE setPuppetExpansion
                   NOTIFY puppetExpansionChanged)
    Q_PROPERTY(double puppetRigidity READ puppetRigidity WRITE setPuppetRigidity
                   NOTIFY puppetRigidityChanged)
    // PRESENTATION-ONLY (the 11d D2 ruling: a real canvas overlay, not
    // a dead control; no wire param): draws the live session's pin
    // adjacency graph — see drawToolOverlays for what is honestly
    // drawable (the engine's §45.5 triangulation has no readback door
    // and is NOT faked).
    Q_PROPERTY(bool puppetShowMesh READ puppetShowMesh WRITE setPuppetShowMesh
                   NOTIFY puppetShowMeshChanged)
    // v1.0.1 item 13: the free-transform frame's QUERYABLE state — the
    // commit decomposition's REFUSED half (the availability-truth
    // pattern: locks/kind-mismatch/§18.3-NI all surface the same way).
    // transformActive mirrors the live frame; the wall names the
    // engine's typed refusal code for the latest commit attempt on it
    // (cleared when the frame's matrix changes or the frame ends).
    // Presentation state only — the refusal itself is engine truth from
    // the applyCommand result; the visible text rides the EXISTING
    // status-line refusal path (ui.status.rejected), no new strings.
    Q_PROPERTY(bool transformActive READ transformActive NOTIFY transformStateChanged)
    Q_PROPERTY(bool transformWallActive READ transformWallActive
                   NOTIFY transformStateChanged)
    Q_PROPERTY(QString transformWallCode READ transformWallCode
                   NOTIFY transformStateChanged)
    // v1.0.1 items 19 + 21: the two HONEST-STATE flags for the halves of
    // the frame that could not be wired to a served route. Neither is a
    // wall — a wall says "you cannot do this", and both of these states
    // are ones where the user CAN still transform. Disabling here would
    // be a false surface with its polarity reversed.
    //
    // transformFrameIsCanvasFallback — the frame is CANVAS-shaped because
    //   the per-layer readback door refused this layer typed (masked
    //   layers, the F3b class). The engine applies transform.free's
    //   matrix as-is in document space and validates-then-discards
    //   `reference`, so a canvas-shaped frame commits the SAME PIXELS as
    //   a layer-shaped one — it is loose, not wrong. That is exactly why
    //   this is a labelled fallback and not the wall perspective takes:
    //   there the frame is load-bearing on the homography.
    // transformPreviewUnavailable — no live preview under the frame.
    //   Two causes, distinguishable by the flag above: masked (both
    //   true) or a layer past the 2^26-px readback cap (only this one).
    //
    // Both are ALSO stated to the user, on the status line, through the
    // existing statusPending residual door — queryable-only would leave
    // a user staring at a canvas-sized box with no idea why.
    Q_PROPERTY(bool transformFrameIsCanvasFallback READ
                   transformFrameIsCanvasFallback NOTIFY transformStateChanged)
    Q_PROPERTY(bool transformPreviewUnavailable READ transformPreviewUnavailable
                   NOTIFY transformStateChanged)
    // G-3 (CRITICAL): the perspective session's queryable state — the
    // item-13 availability-truth pattern applied at session ENTRY. The
    // session frames its quad at the ACTIVE LAYER's derived source
    // bounds (the engine maps source bounds -> quad; a doc-bounds frame
    // flooded every non-full-canvas layer). When the entry-time
    // derivation cannot see the layer (typed 1.4 readback refusal —
    // masked layers et al.) or sees NO opaque pixels, the session does
    // not open and the wall names why: perspectiveWallCode carries the
    // door's typed code (masked -> error.command.not_implemented,
    // already on the status line via ui.status.rejected), or stays
    // EMPTY for the empty-layer arm (no wire door refused — the empty
    // bounds are the answer; no cataloged text/MessageId exists to
    // name it and minting one is outside this lane's fence — the
    // cataloged notice is the queued fold-in). Presentation state; the
    // refusal itself is door truth, never a shell guess.
    Q_PROPERTY(bool perspectiveActive READ perspectiveActive
                   NOTIFY perspectiveStateChanged)
    Q_PROPERTY(bool perspectiveWallActive READ perspectiveWallActive
                   NOTIFY perspectiveStateChanged)
    Q_PROPERTY(QString perspectiveWallCode READ perspectiveWallCode
                   NOTIFY perspectiveStateChanged)
    QML_ELEMENT

public:
    explicit CanvasView(QQuickItem *parent = nullptr);

    DocumentController *controller() const { return m_controller; }
    void setController(DocumentController *controller);

    double zoom() const { return m_zoom; }
    int zoomPercent() const { return static_cast<int>(m_zoom * 100.0 + 0.5); }
    QString tool() const { return m_tool; }
    void setTool(const QString &tool);
    QColor brushColor() const { return m_brushColor; }
    void setBrushColor(const QColor &color);
    double brushSize() const { return m_brushSize; }
    void setBrushSize(double size);
    double brushHardness() const { return m_brushHardness; }
    void setBrushHardness(double hardness);
    // O-29: what the row actually contributes to a stroke envelope —
    // nullopt until the row has been SET to something other than the value
    // it launched with. See setBrushHardness for why this is a stated-vs-
    // unstated flag and not a compare of m_brushHardness against a
    // constant.
    std::optional<double> brushHardnessOverride() const
    {
        return m_brushHardnessStated ? std::optional<double>(m_brushHardness)
                                     : std::nullopt;
    }
    int pendingPreviewCount() const { return m_pendingEchoes.size(); }
    double wandTolerance() const { return m_wandTolerance; }
    void setWandTolerance(double tolerance);
    bool penCommitsShape() const { return m_penCommitsShape; }
    void setPenCommitsShape(bool shape);
    bool wandContiguous() const { return m_wandContiguous; }
    void setWandContiguous(bool contiguous);

    bool sampleAllLayers() const { return m_sampleAllLayers; }
    void setSampleAllLayers(bool sampleAll);
    double textFontSize() const { return m_textFontSize; }
    void setTextFontSize(double sizePt);
    double shapeStrokeWidth() const { return m_shapeStrokeWidth; }
    void setShapeStrokeWidth(double widthPx);
    int puppetExpansion() const { return m_puppetExpansion; }
    void setPuppetExpansion(int px);
    double puppetRigidity() const { return m_puppetRigidity; }
    void setPuppetRigidity(double rigidity);
    bool puppetShowMesh() const { return m_puppetShowMesh; }
    void setPuppetShowMesh(bool show);
    bool transformActive() const { return m_transformActive; }
    bool transformWallActive() const { return m_transformWallActive; }
    QString transformWallCode() const { return m_transformWallCode; }
    bool transformFrameIsCanvasFallback() const
    {
        return m_transformFrameIsCanvasFallback;
    }
    bool transformPreviewUnavailable() const
    {
        return m_transformPreviewUnavailable;
    }
    bool perspectiveActive() const { return m_perspActive; }
    bool perspectiveWallActive() const { return m_perspWallActive; }
    QString perspectiveWallCode() const { return m_perspWallCode; }
    // In-flight preview matrix readback in WIRE order (column-major Mat3
    // for column vectors — the exact convention commitTransform submits).
    // Presentation-only: tests derive expected preview numbers against
    // it; a future QML overlay (scale/angle readout) is its other
    // consumer. No document math — this is the shell's own gesture state.
    Q_INVOKABLE QVariantList transformPreviewMatrix() const;

    Q_INVOKABLE void zoomIn();
    Q_INVOKABLE void zoomOut();
    Q_INVOKABLE void zoomFit();
    // v1.0.1 item 12: fit-and-center ONLY when the document exceeds the
    // viewport at the current zoom — the import/paste call sites use it
    // (doc open already fits via onDocumentReset's unconditional
    // zoomFit).
    Q_INVOKABLE void fitIfOverflow();
    Q_INVOKABLE void zoomOneToOne();
    Q_INVOKABLE void setZoomCentered(double zoom);

    // Deterministic scripted stroke for --screenshot-tour: same echo +
    // commit path as interactive input. Each point: {x, y, pressure, t}.
    // Takes a PALETTE SPELLING (as m_tool holds), not a wire kind: it
    // resolves through the same (tool -> kind, mode) table the mouse path
    // uses. It took a wire kind until 2026-08-29, which made the tour
    // unable to exercise any mode tool. Returns false for a tool that
    // does not dispatch, so an undispatched tool cannot read as a commit.
    Q_INVOKABLE bool tourStroke(const QString &tool, const QVariantList &points);

    // v1.0.1 item 7 (§31.1 keyboard invocation): where the canvas
    // context menu opens for Shift+F10 / the Menu key — the selection's
    // view-space bounds center when the ants outline is visible (the
    // same vectorized truth the overlay draws), else the center of the
    // visible document region, else the view center; always clamped
    // into the item. Non-const: refreshes the revision-cached ants.
    Q_INVOKABLE QPointF contextMenuAnchor();

    // v1.0.1 item 7 §5 fix round (review F1, MAJOR): TRUE while a
    // BUTTON-HELD canvas interaction is in flight — a tool drag gesture,
    // a pan, or a brush/eraser stroke. The context-menu open paths POLL
    // this at event time and refuse to open mid-gesture (Qt 6 delivers a
    // new button press through the item stack even while another
    // button's drag holds the grab; the accepting overlay's popup then
    // steals the exclusive grab and the release never reaches the
    // canvas — the gesture dies silently). Live state, not a shadow
    // flag: it reads the same members the input handlers drive
    // (m_strokeInProgress is cleared in THREE places, not two: in
    // onStrokeCommitted, synchronously inside strokeEnd's emission, so
    // there is no lingering-true window after a release; in cancelGesture,
    // which abandons a live stroke on Esc (G-1); and in the infoChanged
    // lambda in setController(), when the document VANISHES without a
    // reset — undoing doc.new. A "button held implies in flight" reading is
    // therefore wrong, and was ALREADY wrong before G-1: that third site
    // could clear the flag mid-capture too. G-1 did not introduce that
    // window, it added a second way in.
    //
    // ★ ROUND-5 MINOR-1 — THE THIRD SITE'S OTHER CONSEQUENCE, which this
    // note framed as a gestureInFlight/context-menu window only. Clearing
    // the flag OUTSIDE cancelGesture() also kills cancelGesture's stroke
    // arm for every LATER caller: the next document's documentReset finds
    // `if (m_strokeInProgress)` false and never disarms the controller, so
    // the vanished document's samples stay buffered and committable into
    // the NEW document (V-D4a) — MUT-F's exact shape, in shipped code. The
    // lambda therefore calls strokeCancel() itself, and the invariant to
    // preserve is: ANY site that clears m_strokeInProgress outside
    // cancelGesture() must disarm the controller in the same breath,
    // because it has just made cancelGesture unable to.)
    // Click-SESSIONS (polygon/pen) hold no button —
    // there is no release to lose — and are deliberately NOT in-flight.
    Q_INVOKABLE bool gestureInFlight() const;

    void paint(QPainter *painter) override;

signals:
    void controllerChanged();
    void zoomChanged();
    void toolChanged();
    void brushColorChanged();
    void brushSizeChanged();
    void brushHardnessChanged();
    void pendingPreviewCountChanged();
    void wandToleranceChanged();
    void wandContiguousChanged();
    void sampleAllLayersChanged();
    void penCommitsShapeChanged();
    // v1.0.1 item 11d Phase 1: options-bar bound state.
    void textFontSizeChanged();
    void shapeStrokeWidthChanged();
    void puppetExpansionChanged();
    void puppetRigidityChanged();
    void puppetShowMeshChanged();
    // Crop tool drag result (doc space, rounded, w/h >= 1): the QML layer
    // opens the crop dialog pre-filled — the canvas never submits doc.crop.
    void cropRectDragged(int x, int y, int w, int h);
    // Emitted after a selection.* envelope submit. Item 25b narrowed the
    // WAND arm: submitWand now emits only when the engine ACCEPTED the
    // command (a refused wand click announces pending instead — see
    // armedToolRefusedPending). The marquee/lasso/polygon arms still
    // emit unconditionally; aligning all four is a recorded follow-on,
    // deliberately outside this slice's fence. The signal has zero
    // consumers today (census row 19), so the split is a contract note,
    // not a behaviour change for any caller.
    void selectionSubmitted();
    // v1.0.1 item 14b: the plain Delete/Backspace chord (canvas-focus
    // scope). The DISPOSITION lives in QML (Main.qml
    // deleteLayerContentWithConfirm — the ONE implementation the menu
    // rows share): selection -> immediate edit.cut-sans-copy; none ->
    // the §32.5 confirm dialog. Emitted only when the edit.delete
    // availability row is enabled (the chord consumes the 14a surface
    // at the emit site).
    void deleteChordPressed();
    // v1.0.1 item 13: any transition of the transform frame's queryable
    // state (frame begin/close/cancel, wall set/clear).
    void transformStateChanged();
    // G-3: any transition of the perspective session's queryable state
    // (session open/close/cancel, entry wall set/clear).
    void perspectiveStateChanged();
    // ---- item 25b HONESTY SWEEP (honest-state rule D4) ----
    // A press landed while the armed tool has NO branch in
    // mousePressEvent's if-chain: the tool is armed, the canvas is not
    // wired for it, and the press would otherwise be swallowed by the
    // bare event->accept(). DERIVED, never enumerated: the emit site is
    // the chain's final `else`, so membership in the chain IS the
    // wiring predicate — wiring a tool (adding its branch) stops the
    // announcement with no second edit anywhere. Carries the PALETTE
    // wire id (not an engine kind: the two are not 1:1, and three
    // Toning tools share one kind), because the id is what names the
    // tool to the user. VOCABULARY-FREE by design — the reason-id and
    // display-name mapping lives beside toolNameKeys in Main.qml.
    void armedToolPressed(const QString &tool);
    // A WIRED tool's press reached the engine and the engine answered a
    // typed error.command.not_implemented — the gesture is honest, the
    // engine arm is the pending half. Kept distinct from
    // armedToolPressed so the two cannot be conflated: this one means
    // "wired, engine held it", the other means "not wired".
    void armedToolRefusedPending(const QString &tool);

protected:
    void mousePressEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void hoverMoveEvent(QHoverEvent *event) override;
    void mouseUngrabEvent() override;
    void focusOutEvent(QFocusEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void keyReleaseEvent(QKeyEvent *event) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private slots:
    void onRegionsDirty(const QVariantList &rects);
    void onDocumentReset();
    void onStrokeCommitted(bool ok);
    // Repaint-scoping slice (sweep F2): overlay truth (selection ants /
    // guides / grid) moved — refresh the ants vector if the selection
    // revision moved, then repaint the full item (overlays span the
    // view). The ONLY full-item repaint a command commit can trigger;
    // composite dirt rides the partial regionsDirty path.
    void onOverlayChanged();

private:
    struct EchoStroke {
        QString kind;
        QColor color;
        double sizePx = 16.0;
        QVector<QPointF> points;   // document space
        QVector<float> pressures;
    };

    QPointF viewToDoc(const QPointF &viewPos) const;
    QPointF docToView(const QPointF &docPos) const;
    void clampZoom();
    void refreshRegion(const QRect &docRect);
    // OQ-5 (item 21): the ITEM-space rect the live transform preview and
    // its chrome occupy across a matrix change, for a scoped update().
    // Both matrices are passed because the OLD footprint must be erased
    // as well as the new one painted — an invalidation rect that covers
    // only the new position leaves a trail, which is a visible lie about
    // where the layer is.
    QRect transformPreviewInvalidateRect(const QTransform &before,
                                         const QTransform &after) const;
    void rebuildCache();
    void drawEcho(QPainter *painter, const EchoStroke &stroke, bool pending) const;
    void strokePointFromEvent(QMouseEvent *event);

    // ---- CP-0.5 tool gestures (all submits ride applyCommand) ----
    // Gesture rubber bands are input echo of an IN-FLIGHT gesture — they
    // vanish at submit. Marching ants for the committed selection need
    // geometry readback (ABI-growth request 4) and are NOT drawn.
    enum class Gesture { None, Marquee, Lasso, MoveDrag, CropDrag,
                         TransformTranslate, TransformScale, TransformRotate,
                         PenHandleDrag, ShapeDrag, TextDrag,
                         PerspectiveCorner, PuppetRotate };
    static QString selectionOpForModifiers(Qt::KeyboardModifiers mods);
    // items 9 + 16: the ONE click-outside deselect, shared by the three
    // drag-shaped selection tools (rect, ellipse, lasso). See the
    // definition for the §39.4 gate and the polygon/wand exclusions.
    void deselectIfClickOutside(const QPointF &docPoint);
    void submitShapeSelection(); // rect|ellipse from the marquee gesture
    void submitLasso();
    void submitPolygon();
    void submitWand(const QPointF &docPos, Qt::KeyboardModifiers mods);
    void submitMove(const QPoint &deltaPx);
    void commitPen(bool closed);
    void submitShapePrimitive();
    void submitText(); // click = point layout, drag = paragraph rect (slice T)
    void drawFocusRing(QPainter *painter, const QTransform &entryTransform);
    void beginTransformFrame();
    void commitTransform();
    // item 13 helpers: frame teardown (active/matrix/wall reset in one
    // place) and the wall's set/clear lifecycle.
    void closeTransformFrame();
    void setTransformWall(const QString &code);
    void clearTransformWall();
    // Perspective frame (transform.perspective): four INDEPENDENT corner
    // handles over the base rect; commit maps base corners -> quad.
    void beginPerspectiveFrame();
    void commitPerspective();
    // Puppet session (transform.puppet): pin placement + per-pin
    // rotation dial + depth — the WIRE-EXPRESSIBLE deformation set (the
    // engine's recorded ruling: drag destinations are a contract gap;
    // no drag-deform UX until the fold-in lands).
    void beginPuppetSession();
    void commitPuppet();
    void cancelGesture();
    // G-2 (MAJOR): the STROKE-ONLY arm of cancelGesture, extracted so hygiene
    // handlers can abandon a live stroke WITHOUT tearing down the rest of the
    // gesture ladder. Focus loss must not discard a polygon the user is
    // mid-placement on; it must not commit a stroke either.
    void cancelStroke();
    int transformHandleAt(const QPointF &docPos) const; // -1 none, 0..7 handles
    void drawToolOverlays(QPainter *painter) const;     // inside doc transform

    Gesture m_gesture = Gesture::None;
    QPointF m_gestureStartDoc;
    QPointF m_gestureCurrentDoc;
    QVector<QPointF> m_lassoPoints;      // also polygon vertices
    bool m_polygonActive = false;
    QPointF m_hoverDoc;                  // polygon preview segment target

    bool m_transformActive = false;
    QRectF m_transformBase;              // original frame, doc space
    QTransform m_transformNow;           // accumulated, doc space
    // item 13 named wall: the engine's typed refusal for the latest
    // commit attempt on the LIVE frame (empty/false = no wall).
    bool m_transformWallActive = false;
    QString m_transformWallCode;
    // item 19: the layer the frame was DERIVED FOR, captured at entry.
    // G-3's idiom one level down — see commitTransform for why resolving
    // the active layer at commit time is a live defect once the frame
    // stops being canvas-shaped for every layer alike.
    QString m_transformLayerId;
    bool m_transformFrameIsCanvasFallback = false;
    // item 21: the entry-time layer content the live preview draws, and
    // the honest flag for when there is none. Doc-origin framed
    // RGBA8888, straight alpha — the pixels item 19's derivation already
    // read (one readback, not two).
    QImage m_transformSnapshot;
    bool m_transformPreviewUnavailable = false;

    bool m_perspActive = false;
    QRectF m_perspBase;                  // the ACTIVE layer's derived source
                                         // bounds, doc space (G-3: never the
                                         // doc rect unless they coincide)
    QString m_perspLayerId;              // entry-time layer capture — the
                                         // commit target (the frame's truth
                                         // is per-layer; a commit-time
                                         // active-layer swap would remap a
                                         // DIFFERENT layer's bounds onto
                                         // this frame's quad)
    QPointF m_perspQuad[4];              // TL,TR,BR,BL destinations, doc space
    int m_perspHandle = -1;              // dragged corner index
    // G-3 entry wall (item-13 pattern at session entry): set when the
    // session refuses to open — the derivation's typed refusal code,
    // or code-less (empty) for the no-opaque-pixels arm. Cleared on
    // any successful open / cancel.
    bool m_perspWallActive = false;
    QString m_perspWallCode;

    bool m_puppetActive = false;
    QVector<PuppetPin> m_puppetPins;     // placements + rotation/depth
    int m_puppetSelected = -1;
    int m_transformHandle = -1;
    QPointF m_transformAnchorDoc;        // scale anchor / rotate center
    double m_rotateStartAngle = 0.0;
    QTransform m_transformAtGestureStart;

    double m_wandTolerance = 32.0;       // contract default
    bool m_wandContiguous = true;        // contract default
    bool m_sampleAllLayers = false;      // engine default in all three schemas

    // Pen tool session (anchors -> envelope_builder's cubic JSON).
    QVector<PenAnchor> m_penAnchors;
    bool m_penActive = false;
    bool m_penCommitsShape = true;

    // v1.0.1 item 11d Phase 1 — options-bar bound state (defaults are
    // the ENGINE/contract defaults; each is omitted from the wire AT
    // its default — see the Q_PROPERTY block).
    double m_textFontSize = 12.0; // CharStyle::default size_pt
    // Pro-Wave row 4: vector.shape_add `stroke.widthPx`. 0.0 is the
    // CONTRACT's own shape_add default (vector.shape_add.yaml:199
    // `stroke: {widthPx: 0}`) and the engine's own no-visible-stroke
    // arm (vector_ops.rs from_wire: width <= 0 => Ok(None)) — the field
    // is REQUIRED on shape_add, so it is always emitted; at the default
    // the envelope is byte-identical to the pre-row one.
    double m_shapeStrokeWidth = 0.0;
    int m_puppetExpansion = 2;    // contract default (transform.puppet.yaml)
    double m_puppetRigidity = 0.7; // contract default
    bool m_puppetShowMesh = false; // presentation-only overlay, default off

    // Marching-ants overlay (ABI 1.2 selection readback): Gray8 coverage
    // >=50% vectorized into a doc-space outline path ONCE per selection
    // revision, OFF the paint path (repaint-scoping slice, sweep F1c —
    // recompute rides overlayChanged and the pan/zoom/geometry escape
    // check; paint() only consumes). The vectorization is WINDOWED to
    // the viewport + a half-viewport margin clipped to the doc
    // (m_antsWindow): a false outline where a selection run crosses the
    // window edge exists only in the margin, and ensureAntsWindow
    // recomputes before the viewport can reach it. Static dashes
    // (deterministic tour); animation is a later nicety.
    QPainterPath m_antsPath;
    QRect m_antsWindow; // doc-space region the current path is true for
    int m_antsRevision = -1;
    bool m_antsValid = false;
    void recomputeAnts();
    void ensureAntsWindow();
    QPointF snapDocPoint(const QPointF &doc) const;
    void applyToolCursor();

    DocumentController *m_controller = nullptr;
    // Engine readback, straight-alpha RGBA8888, sized to the OUTPUT FRAME
    // (DocumentController::imageRect), not to the canvas.
    QImage m_cache;
    // ★★ THE ABSOLUTE DOCUMENT COORDINATE OF m_cache's PIXEL (0,0).
    //
    // Before ABI 1.7 the cache was canvas-sized at doc origin, so
    // cache-index and absolute-doc-coord were THE SAME NUMBER and every
    // blit could use either. That identity is what an offset crop breaks,
    // and it breaks SILENTLY — the code still compiles, still paints, and
    // still produces a plausibly-sized image of the wrong pixels. Every
    // conversion between the two spaces now goes through this offset:
    //     cache index = absolute doc coord - m_cacheOrigin
    // Kept in lockstep with m_cache by rebuildCache(), its only writer.
    //
    // ★ doc<->view (viewToDoc/docToView) stay ABSOLUTE and are NOT rebased
    // on this origin: every tool envelope — selection rects, stroke
    // points, wand seed, guides — is absolute document coords, and
    // rebasing doc-space would corrupt all of them silently. The origin
    // enters ONLY at blit and extent time.
    QPoint m_cacheOrigin;
    // v1.0.1 item 10: the ACTIVE layer's 1.4 readback, captured at
    // MoveDrag press — the drag echo draws THIS at the offset over the
    // unmoved composite base (null = the typed-refusal layer classes:
    // the old whole-composite echo is the recorded fallback).
    QImage m_moveEchoLayer;
    QPixmap m_checkerTile; // transparency checker, built once (hot paint path)

    double m_zoom = 1.0;
    QPointF m_offset; // view position of doc (0,0)
    bool m_userNavigated = false;
#ifdef Q_OS_WIN
    bool m_refitSilentWhenUnchanged = true;
#else
    bool m_refitSilentWhenUnchanged = false;
#endif
    bool m_autoRefit = false; // set only around geometryChange's zoomFit()

    QString m_tool = QStringLiteral("brush.stroke");
    QColor m_brushColor = QColor(20, 20, 20);
    double m_brushSize = 16.0;
    // The row's DISPLAYED value; only meaningful to the wire once stated.
    double m_brushHardness = 0.0; // = soft-round-pressure's own hardness (§15.7)
    bool m_brushHardnessStated = false; // O-29: has the row been set? (see .cpp)

    bool m_strokeInProgress = false;
    quint64 m_strokeStartMs = 0;
    EchoStroke m_activeEcho;
    QVector<EchoStroke> m_pendingEchoes; // typed-rejection previews, capped

    bool m_panning = false;
    bool m_spaceHeld = false;
    // B-02: the clone source anchor, in DOCUMENT space, armed by
    // Option/Alt-click (Photoshop convention). Unset = unarmed: clone
    // refuses to paint and says why, which is Q-D14's VOICE moved from
    // tool SELECTION (where it now reads falsely, the tool works) to the
    // moment it is actually true.
    bool m_cloneArmed = false;
    QPointF m_cloneAnchorDoc;
    QPointF m_panAnchorView;
    QPointF m_panAnchorOffset;
};

} // namespace appname::shell
