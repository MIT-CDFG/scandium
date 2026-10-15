// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header.

#include "tour.h"

#include "abi_client.h"
#include "canvas_view.h"
#include "document_controller.h"

#include <QClipboard>
#include <QCoreApplication>
#include <QGuiApplication>
#include <QColor>
#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QImage>
#include <QJsonObject>
#include <QQuickWindow>
#include <QVariantList>
#include <QVariantMap>

#include <cstdio>

namespace appname::shell {

namespace {

// Settle the scene graph, then grab. grabWindow renders synchronously on
// the offscreen/software path, so one settle pass suffices.
bool capture(QQuickWindow *window, const QDir &dir, const QString &name, QStringList *inventory)
{
    // Repaint-scoping slice (sweep F2) instrument note: an offscreen
    // window renders ONLY inside grabWindow (no expose-driven frames),
    // and a grab-time render replays just the ACCUMULATED dirty clip —
    // content outside a partial clip does not survive between grabs on
    // this path. Pre-slice, the blanket per-commit full repaint hid
    // that; with commit invalidation scoped (partial-only frames exist
    // now: paste/shape/text adds), the grab must render ONE complete
    // frame to stay a faithful engine-truth screenshot. Full-repaint
    // the canvas before grabbing — CONTENT stays the cache/engine truth
    // (byte-equal to the pre-slice frames by construction); sensitivity
    // to missing invalidation lives in tst_repaint_scoping (mask-read
    // counts, paint clip fractions, and post-gesture window grabs).
    if (auto *canvas = window->findChild<CanvasView *>())
        canvas->update();
    for (int i = 0; i < 3; ++i)
        QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
    const QImage frame = window->grabWindow();
    if (frame.isNull()) {
        std::fprintf(stderr, "tour: grabWindow returned null for %s\n", qPrintable(name));
        return false;
    }
    const QString path = dir.filePath(name);
    if (!frame.save(path, "PNG")) {
        std::fprintf(stderr, "tour: failed to save %s\n", qPrintable(path));
        return false;
    }
    std::printf("tour: wrote %s (%dx%d)\n", qPrintable(path), frame.width(), frame.height());
    inventory->append(path);
    return true;
}

QVariantList diagonalStroke(double x0, double y0, double x1, double y1, int steps)
{
    QVariantList points;
    for (int i = 0; i <= steps; ++i) {
        const double f = static_cast<double>(i) / steps;
        QVariantMap p;
        p.insert(QStringLiteral("x"), x0 + (x1 - x0) * f);
        p.insert(QStringLiteral("y"), y0 + (y1 - y0) * f);
        p.insert(QStringLiteral("pressure"), 1.0);
        p.insert(QStringLiteral("t"), 0.008 * i); // 125 Hz, strictly increasing
        points.append(p);
    }
    return points;
}

// ★★★★★ DISMISS THE REFUSAL TOAST BEFORE ANY LATER FRAME SEES IT.
// A refusal raises the P1-2 toast and `refusalToastTimer` (Main.qml) has an
// interval of 6000 ms -- a HUMAN is long past that beat before it clears, so
// nobody driving the app ever sees it linger. THE TOUR BURNS NO WALL-CLOCK, so
// without this the SAME rendered toast survives into every downstream frame.
//
// ★★★★ WHY THIS MATTERS FOR THE GOLDENS AND NOT FOR THE PRODUCT: pinning those
// frames commits a WALL-CLOCK-DEPENDENT artifact. One slower run crosses
// 6000 ms, the toast vanishes, and the goldens diff for a reason that has
// nothing to do with the code. A golden set is the last place to enshrine a
// race.
//
// ⛔ AND THAT PREDICTION CAME TRUE ON 2026-09-01, BECAUSE THE FIX WAS APPLIED
// TO ONE CALL SITE AND NOT TO THE CLASS. The original lived inline after frame
// 21 and covered the PUPPET refusal, protecting frames 22..30. Nobody asked
// whether an EARLIER refusal did the same thing -- step 18's `doc.mode_convert`
// does, and frames 19 and 20 sat inside its 6000 ms window for days. The gate
// went red, green, green, red on an unchanged tree until the evidence was
// preserved and the two frames could be named and opened.
// ★★★ SO IT IS A FUNCTION NOW, NOT A BLOCK. Two call sites, one body: a second
// pasted copy is the two-copy problem this project is built around, and the
// first copy had already drifted from the class it was supposed to cover.
//
// ★★★★★ NO processEvents() HERE, AND THE REASON IS A REGRESSION CAUGHT IN THE
// ORIGINAL FIX. The first version called it, which flushed a pending status-bar
// update and put `Exported /var/folders/.../appname-tour-composite.png` into
// every later frame -- QDir::temp() is USER- AND MACHINE-SPECIFIC, so goldens
// pinned with that string would diff on any other machine. That trades a
// WALL-CLOCK dependency for a MACHINE dependency and calls it a fix.
// ★★★ The visibility change needs no pump: capture() renders the window, and
// that render is what applies it. Verified by running both versions and reading
// the frames.
void dismissRefusalToast(QQuickWindow *window)
{
    if (auto *toast =
            window->findChild<QQuickItem *>(QStringLiteral("refusalToast"))) {
        toast->setProperty("visible", false);
    }
}

} // namespace

int runScreenshotTour(QQuickWindow *window, DocumentController *controller, const QString &outputDir)
{
    QDir dir(outputDir);
    if (!dir.exists() && !QDir().mkpath(outputDir)) {
        std::fprintf(stderr, "tour: cannot create output dir %s\n", qPrintable(outputDir));
        return 1;
    }

    // Fixed geometry: the acceptance artifact must be reproducible.
    window->setWidth(1280);
    window->setHeight(800);

    auto *canvas = window->findChild<CanvasView *>(QStringLiteral("canvasView"));
    if (!canvas) {
        std::fprintf(stderr, "tour: canvasView item not found\n");
        return 1;
    }

    QStringList inventory;
    bool ok = true;

    // ★★★★★ COMPOSITE TRUTH, BECAUSE `committed=true` IS ACCEPTANCE AND NOT
    // VISIBLE EFFECT — a distinction this desk has named three times in the
    // ledger and never closed until now.
    //
    // It is not academic. Demo step §6 was ACCEPTED, grew a history entry, and
    // changed nothing: heal.stroke onto a fully transparent layer is a DEFINED
    // no-op (heal.stroke.yaml:210). Every signal a presenter would check said
    // success. A tour step printing `committed=true` would have said the same.
    //
    // So: export the flattened composite and hash it, either side of the
    // stroke. A changed hash is the only evidence in this file that PIXELS
    // MOVED rather than that an envelope was accepted.
    //
    // ★ Temp path, removed after: frame 11 writes export-check.png beside the
    // frames as a documented non-frame, and a second such file would move a
    // count two desks are reasoning about. ★ An empty hash means the export
    // door refused, and the caller must not read that as "unchanged" — it is
    // reported as UNMEASURED, not as false.
    auto compositeHash = [&]() -> QByteArray {
        const QString cp = QDir::temp().filePath(
            QStringLiteral("appname-tour-composite.png"));
        QFile::remove(cp);
        if (!controller->exportDocument(cp, QStringLiteral("png"), -1))
            return {};
        QByteArray h;
        QFile f(cp);
        if (f.open(QIODevice::ReadOnly))
            h = QCryptographicHash::hash(f.readAll(), QCryptographicHash::Sha256);
        f.close();
        QFile::remove(cp);
        return h;
    };
    auto pixelVerdict = [](const QByteArray &a, const QByteArray &b) -> const char * {
        if (a.isEmpty() || b.isEmpty())
            return "UNMEASURED(export refused)";
        return a == b ? "false" : "true";
    };


    // 01 — launch state: no document, panels empty, catalog resolution
    // visible (key-echo when no catalogs are installed — by design).
    ok = ok && capture(window, dir, QStringLiteral("01-launch.png"), &inventory);

    // 02 — new document (fixed 640x400, white background).
    if (ok) {
        if (!controller->newDocument(640, 400, QStringLiteral("white"))) {
            std::fprintf(stderr, "tour: doc.new failed\n");
            ok = false;
        }
        // ABI 1.6 font feed evidence (request-10 arc): the per-document
        // provisioning ran inside newDocument; the summary is machine
        // inventory (provided/skipped/refused + the engine's own stamp
        // and enumerated face count).
        const QVariantMap fonts = controller->fontFacesSummary();
        std::printf("tour: fonts provided=%d skipped=%d refused=%d stamp=%d faces=%d\n",
                    fonts.value(QStringLiteral("provided")).toInt(),
                    fonts.value(QStringLiteral("skipped")).toInt(),
                    fonts.value(QStringLiteral("refused")).toInt(),
                    fonts.value(QStringLiteral("stamp")).toInt(),
                    fonts.value(QStringLiteral("faces")).toInt());
        ok = ok && capture(window, dir, QStringLiteral("02-new-document.png"), &inventory);
    }

    // 03 — zoom/pan surface: fit, 100%, then a fixed 200% (final state).
    if (ok) {
        canvas->zoomFit();
        canvas->zoomOneToOne();
        canvas->setZoomCentered(2.0);
        ok = capture(window, dir, QStringLiteral("03-zoom-200.png"), &inventory);
    }

    // 04 — brush stroke (fixed diagonal samples), committed through the
    // ABI: blob → appname_asset_create → envelope → engine render →
    // dirty-rect repaint. A typed rejection would instead keep the echo
    // overlay as a pending preview with the status line typed — the
    // pre-flip degraded state this frame used to prove; either outcome is
    // an honest frame.
    if (ok) {
        canvas->setTool(QStringLiteral("brush.stroke"));
        canvas->setBrushColor(QColor(0xc2, 0x3b, 0x22));
        canvas->setBrushSize(24.0);
        // ★★★ THIS LINE IS THE POSITIVE CONTROL FOR pixels_changed.
        // A brush stroke onto a white canvas MUST move pixels. If this
        // prints false, the composite-hash verdict is broken and every
        // `pixels_changed=false` elsewhere in this tour means nothing --
        // which is the vacuous-control failure the verdict was added to
        // prevent, arriving in the verdict itself.
        const QByteArray brBefore = compositeHash();
        const bool committed =
            canvas->tourStroke(QStringLiteral("brush.stroke"),
                               diagonalStroke(80, 80, 560, 320, 24));
        // ★★★★★ CAPTURE BEFORE THE AFTER-HASH. compositeHash() exports
        // through the REAL export door, and exportDocument() sets the status
        // line (`ui.status.exported`) to an ABSOLUTE path under QDir::temp()
        // -- on macOS `/var/folders/<opaque>/T/`, which is USER- AND
        // MACHINE-SPECIFIC. Taking the after-hash first put that string into
        // the frame, so the evidence photographed THE INSTRUMENT instead of
        // the product. ★★★★ Measured: the 08-25 golden for 04 reads
        // `Applied: Brush Stroke`; the current build read `Exported
        // /var/folders/q_/…/appname-tour-composite.png`. Pinning that would
        // have made the goldens machine-specific FOREVER -- they would diff
        // on every other machine and every CI runner, for no code reason.
        // ★★★ The capture does not touch the document, so the after-hash is
        // identical either side of this reorder; only the status line the
        // frame records changes, and it changes back to the product's own.
        ok = capture(window, dir, QStringLiteral("04-brush-stroke.png"), &inventory);
        const QByteArray brAfter = compositeHash();
        std::printf("tour: brush.stroke committed=%s pixels_changed=%s "
                    "(false = typed rejection, preview kept)\n",
                    committed ? "true" : "false",
                    pixelVerdict(brBefore, brAfter));
    }

    // 05 — eraser stroke attempt across the brush path.
    if (ok) {
        canvas->setTool(QStringLiteral("eraser.stroke"));
        canvas->setBrushSize(32.0);
        const bool committed =
            canvas->tourStroke(QStringLiteral("eraser.stroke"),
                               diagonalStroke(120, 300, 520, 120, 20));
        std::printf("tour: eraser.stroke committed=%s\n", committed ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("05-eraser-stroke.png"), &inventory);
    }

    // 06 — undo surface. Until stroke commits reach the ABI, doc.new is
    // the only in-effect command, so undo reverts IT: the document
    // vanishes back to the pre-init state (engine-true; the no-document
    // teardown drops the canvas cache and previews, and the flags rebuild
    // from the journal cursor). Once stroke wiring lands in the ABI this
    // frame flips to "eraser stroke reverted" with no shell change. Both
    // are honest states (review R3: the old comment claimed the
    // nothing_to_undo path, which is not what this frame exercises).
    if (ok) {
        controller->undo();
        ok = capture(window, dir, QStringLiteral("06-undo.png"), &inventory);
    }

    // 07 — CP-0.5 chrome: rect selection + color fill through the generic
    // applyCommand door. Selection state shows via the honest active chip
    // (no marching ants without geometry readback — ABI-growth request 4).
    if (ok) {
        QVariantMap rect;
        rect.insert(QStringLiteral("x"), 200);
        rect.insert(QStringLiteral("y"), 120);
        rect.insert(QStringLiteral("w"), 240);
        rect.insert(QStringLiteral("h"), 140);
        QVariantMap sel;
        sel.insert(QStringLiteral("source"), QStringLiteral("rect"));
        sel.insert(QStringLiteral("rect"), rect);
        sel.insert(QStringLiteral("op"), QStringLiteral("replace"));
        sel.insert(QStringLiteral("antialias"), true);
        const QVariantMap r1 = controller->applyCommand(
            QStringLiteral("selection.set"), {}, sel, {});
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"), QVariantList{0.15, 0.35, 0.8, 0});
        QVariantMap fill;
        fill.insert(QStringLiteral("content"), QStringLiteral("color"));
        fill.insert(QStringLiteral("color"), color);
        fill.insert(QStringLiteral("blend"), QStringLiteral("normal"));
        fill.insert(QStringLiteral("opacity"), 1.0);
        const QString layerId = controller->layers().isEmpty()
            ? QString()
            : controller->layers().first().toMap()
                  .value(QStringLiteral("layerId")).toString();
        QVariantMap target;
        target.insert(QStringLiteral("layerId"), layerId);
        const QVariantMap r2 = controller->applyCommand(
            QStringLiteral("fill"), target, fill, {});
        std::printf("tour: selection.set ok=%s fill ok=%s\n",
                    r1.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    r2.value(QStringLiteral("ok")).toBool() ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("07-select-fill.png"), &inventory);
    }

    // 08 — layers panel v2: add a layer, set its opacity via the class-M
    // door (fresh session uuid), rename it. Panel shows live truth.
    QString tourLayerId; // kept for 08b (hidden-eye evidence frame)
    if (ok) {
        QVariantMap add;
        add.insert(QStringLiteral("kind"), QStringLiteral("raster"));
        const QString newId = controller->newUuid();
        tourLayerId = newId;
        add.insert(QStringLiteral("newLayerId"), newId);
        const QVariantMap r1 = controller->applyCommand(
            QStringLiteral("layer.add"), {}, add, {});
        QVariantMap target;
        target.insert(QStringLiteral("layerId"), newId);
        QVariantMap props;
        props.insert(QStringLiteral("opacity"), 0.75);
        const QString mergeKey = QStringLiteral("layer.set_props:") + newId
            + QLatin1Char(':') + controller->newUuid()
            + QStringLiteral("#opacity");
        const QVariantMap r2 = controller->applyCommand(
            QStringLiteral("layer.set_props"), target, props, mergeKey);
        QVariantMap name;
        name.insert(QStringLiteral("name"), QStringLiteral("Tour Layer"));
        const QVariantMap r3 = controller->applyCommand(
            QStringLiteral("layer.rename"), target, name, {});
        std::printf("tour: layer.add ok=%s set_props ok=%s rename ok=%s\n",
                    r1.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    r2.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    r3.value(QStringLiteral("ok")).toBool() ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("08-layers-v2.png"), &inventory);
    }

    // 08b — hidden-eye truth on the ACTIVE row (review P2-D1/D3): make
    // the tour layer active and hide it through the same set_props door
    // the panel's eye uses. The layer is an EMPTY raster, so the canvas
    // composite is unchanged — the frame's whole payload is the panel:
    // eye-off glyph, textSecondary tint, bgActive ground. State is then
    // restored (re-shown, prior active index): downstream frames keep
    // their canvas and panel state, and the two extra set_props events
    // appear in the HISTORY list — engine truth, rendered honestly.
    if (ok && !tourLayerId.isEmpty()) {
        int tourLayerIndex = -1;
        const QVariantList rows = controller->layers();
        for (int i = 0; i < rows.size(); ++i) {
            if (rows.at(i).toMap().value(QStringLiteral("layerId")).toString()
                == tourLayerId) {
                tourLayerIndex = i;
                break;
            }
        }
        if (tourLayerIndex < 0) {
            std::fprintf(stderr, "tour: tour layer not found in info\n");
            ok = false;
        } else {
            const int prevActive = controller->activeLayerIndex();
            controller->setActiveLayerIndex(tourLayerIndex);
            QVariantMap target;
            target.insert(QStringLiteral("layerId"), tourLayerId);
            QVariantMap hide;
            hide.insert(QStringLiteral("visible"), false);
            const QVariantMap r1 = controller->applyCommand(
                QStringLiteral("layer.set_props"), target, hide,
                QStringLiteral("layer.set_props:") + tourLayerId
                    + QLatin1Char(':') + controller->newUuid()
                    + QStringLiteral("#visible"));
            std::printf("tour: hide ok=%s\n",
                        r1.value(QStringLiteral("ok")).toBool() ? "true" : "false");
            ok = capture(window, dir, QStringLiteral("08b-hidden-eye.png"),
                         &inventory);
            QVariantMap show;
            show.insert(QStringLiteral("visible"), true);
            const QVariantMap r2 = controller->applyCommand(
                QStringLiteral("layer.set_props"), target, show,
                QStringLiteral("layer.set_props:") + tourLayerId
                    + QLatin1Char(':') + controller->newUuid()
                    + QStringLiteral("#visible"));
            std::printf("tour: re-show ok=%s\n",
                        r2.value(QStringLiteral("ok")).toBool() ? "true" : "false");
            controller->setActiveLayerIndex(prevActive);
        }
    }

    // 09 — doc geometry change through a dialog kind: resize to width 320
    // (the in-place documentReset path rebuilds and refits the canvas).
    if (ok) {
        QVariantMap params;
        params.insert(QStringLiteral("width"), 320);
        params.insert(QStringLiteral("resample"), QStringLiteral("lanczos"));
        params.insert(QStringLiteral("constrainAspect"), true);
        params.insert(QStringLiteral("scaleEffects"), true);
        const QVariantMap r = controller->applyCommand(
            QStringLiteral("doc.resize"), {}, params, {});
        std::printf("tour: doc.resize ok=%s\n",
                    r.value(QStringLiteral("ok")).toBool() ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("09-resize.png"), &inventory);
    }

    // 10 — ABI 1.2 doors (version-gated; frames only when the block is
    // live): OS-clipboard paste of a generated image, engine-side export,
    // re-import of the exported file. Proves ingress + egress round-trip.
    if (ok && abiHasBlock12()) {
        QImage clip(48, 48, QImage::Format_RGBA8888);
        clip.fill(QColor(0xd0, 0x60, 0x20));
        QGuiApplication::clipboard()->setImage(clip);
        const bool pasted = controller->pasteFromClipboard();
        std::printf("tour: paste ok=%s\n", pasted ? "true" : "false");
        // ★★★ Q-D26: THESE RETURN VALUES NOW GATE THE VERDICT. They used to be
        // PRINTED and dropped, so `ok` came from capture() alone -- did a PNG get
        // written -- and a failed paste, export or reimport left the step GREEN.
        // The round-trip frame cannot catch it either: exporting the composite and
        // pasting it back at (0,0) is an IDENTITY operation, so 11-export-reimport
        // is byte-identical to 10-paste whether the round-trip works or is totally
        // broken (measured over four runs). The picture is blind by construction;
        // only the return value discriminates.
        //
        // ★★ SAFE AT HEAD AND SAFE WHEN THE ABI IS OLDER, both measured rather than
        // argued: a real tour run prints `paste ok=true`, `export ok=true reimport
        // ok=true`, so this passes today; and the whole block is behind
        // `abiHasBlock12()`, whose else-arm prints the skip line, so a build without
        // the 1.2 block never reaches these asserts and cannot red on them (G-514).
        ok = ok && pasted;
        ok = ok && capture(window, dir, QStringLiteral("10-paste.png"), &inventory);
        const QString exportPath = dir.filePath(QStringLiteral("export-check.png"));
        const bool exported =
            controller->exportDocument(exportPath, QStringLiteral("png"), -1);
        const bool reimported = exported && controller->importImage(exportPath);
        std::printf("tour: export ok=%s reimport ok=%s\n",
                    exported ? "true" : "false", reimported ? "true" : "false");
        ok = ok && exported && reimported;
        ok = ok && capture(window, dir, QStringLiteral("11-export-reimport.png"),
                           &inventory);
    } else if (ok) {
        std::printf("tour: ABI 1.2 block absent — frames 10-11 skipped (version gate)\n");
    }

    // 12 — slice V: shape layer via an ellipse primitive. FLIPPED
    // (Option-A composite fix on main, 2026-08-18): the ellipse RENDERS
    // — model door live since the VectorGeometry train, composite serve
    // since the fix, zero shell changes across both arrivals (the
    // armed-wall design's whole point). The shape_add is still UNDONE
    // after capture: originally required (the wall era froze the
    // composite while a shape existed), retained now because frame 13's
    // committed baseline is the pre-shape state — the undo is
    // engine-true either way.
    if (ok) {
        QVariantMap geometry;
        geometry.insert(QStringLiteral("x"), 40.0);
        geometry.insert(QStringLiteral("y"), 40.0);
        geometry.insert(QStringLiteral("w"), 120.0);
        geometry.insert(QStringLiteral("h"), 80.0);
        QVariantMap primitive;
        primitive.insert(QStringLiteral("kind"), QStringLiteral("ellipse"));
        primitive.insert(QStringLiteral("geometry"), geometry);
        QVariantMap color;
        color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
        color.insert(QStringLiteral("comps"), QVariantList{0.9, 0.6, 0.1, 0.0});
        QVariantMap fill;
        fill.insert(QStringLiteral("type"), QStringLiteral("solid"));
        fill.insert(QStringLiteral("color"), color);
        QVariantMap stroke;
        stroke.insert(QStringLiteral("widthPx"), 0.0);
        QVariantMap params;
        params.insert(QStringLiteral("primitive"), primitive);
        params.insert(QStringLiteral("fill"), fill);
        params.insert(QStringLiteral("stroke"), stroke);
        params.insert(QStringLiteral("newLayerId"), controller->newUuid());
        const QVariantMap r = controller->applyCommand(
            QStringLiteral("vector.shape_add"), {}, params, {});
        std::printf("tour: shape_add ok=%s\n",
                    r.value(QStringLiteral("ok")).toBool() ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("12-vector-shape.png"), &inventory);
        if (r.value(QStringLiteral("ok")).toBool()) {
            controller->undo();
            std::printf("tour: shape_add undone (frame-13 baseline — see comment)\n");
        }
    }

    // 13 — slice V: pen-built path (envelope_builder's cubic JSON) ->
    // doc-level vector.path_new -> selection.from_path on the tracked id;
    // the ants trace the PATH-shaped selection (engine rasterization).
    if (ok) {
        QVector<PenAnchor> anchors;
        PenAnchor a;
        a.anchor = QPointF(70, 30);
        a.handleOut = QPointF(120, 10);
        a.hasHandle = true;
        anchors.append(a);
        PenAnchor b;
        b.anchor = QPointF(250, 60);
        b.handleOut = b.anchor;
        anchors.append(b);
        PenAnchor c;
        c.anchor = QPointF(160, 130);
        c.handleOut = QPointF(110, 140);
        c.hasHandle = true;
        anchors.append(c);
        const QString pathId = controller->newUuid();
        QVariantMap params;
        params.insert(QStringLiteral("path"),
                      buildVectorPathJson(anchors, true).toVariantMap());
        params.insert(QStringLiteral("newPathId"), pathId);
        const QVariantMap r1 = controller->applyCommand(
            QStringLiteral("vector.path_new"), {}, params, {});
        if (r1.value(QStringLiteral("ok")).toBool())
            controller->trackCreatedPath(pathId);
        const bool pathOk = r1.value(QStringLiteral("ok")).toBool();
        const bool sel = pathOk && controller->selectionFromLastPath();
        // When path_new failed the id was never tracked and from_path was
        // never submitted — print skipped, not a phantom ok=false (V-D3).
        std::printf("tour: path_new ok=%s from_path %s\n",
                    pathOk ? "true" : "false",
                    pathOk ? (sel ? "ok=true" : "ok=false") : "skipped");
        ok = capture(window, dir, QStringLiteral("13-path-selection.png"), &inventory);
    }

    // 15 — §26.1 Cut pairing (1.4-gated): cut Background through the
    // path-shaped selection frame 13 left active. The app-side coverage-
    // masked copy lands on the OS clipboard, then edit.cut clears the
    // region (fractional §13.1 edges) and the canvas shows the path-
    // shaped hole. The layers ABOVE Background (the 1.2 block's two
    // full-document opaque paste layers + the empty tour layer) are
    // hidden FIRST through the real eye door — engine-true set_props
    // events, and the interim repaint renders them — so the hole is
    // actually visible. Captured BEFORE the env-gated recovery block so
    // the standing sequence stays self-contained (the number stays 15;
    // 14 is the recovery frame's reserved name).
    if (ok && abiHasBlock14()) {
        const QVariantList rows = controller->layers();
        for (int i = 1; i < rows.size(); ++i) {
            const QString id = rows.at(i).toMap()
                                   .value(QStringLiteral("layerId")).toString();
            QVariantMap t;
            t.insert(QStringLiteral("layerId"), id);
            QVariantMap props;
            props.insert(QStringLiteral("visible"), false);
            controller->applyCommand(QStringLiteral("layer.set_props"), t, props,
                QStringLiteral("layer.set_props:") + id + QLatin1Char(':')
                    + controller->newUuid() + QStringLiteral("#visible"));
        }
        const QString bgId = rows.isEmpty() ? QString()
            : rows.first().toMap().value(QStringLiteral("layerId")).toString();
        const QVariantMap r = controller->cutLayer(bgId);
        std::printf("tour: edit.cut ok=%s\n",
                    r.value(QStringLiteral("ok")).toBool() ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("15-cut.png"), &inventory);
    } else if (ok) {
        std::printf("tour: ABI 1.4 block absent — frame 15 skipped (version gate)\n");
    }

    // 16 — slice T: the text door (contracts text.add.yaml). FLIP
    // STATUS (TEXTVEC wave, 40f326c): the TextLayout service is LIVE —
    // text.add COMMITS (model + layout real) — but NO USABLE FONTS
    // exist on this assembly: the ABI has no font door (Door 10's
    // register_embedded_face is package-internal; system-font discovery
    // COMPLETE (request-10 arc + Option-A composite fix): the shell
    // provisions System faces at every document open, text lays out
    // with REAL glyphs, to_shape serves REAL outlines, and the CANVAS
    // RENDERS THEM — the full pipeline (provision -> layout -> outline
    // -> composite) live end-to-end, zero shell changes at either
    // upstream arrival. The frame shows "Appname 1.0" in rendered
    // glyphs with the cataloged Convert-Text-to-Shape status.
    // 1.0 final (corrigendum (l), O-WATCH F-7): the 16-text.png step is removed --
    // its feature has no 1.0 control, so a frame of it would show the
    // release doing something a user cannot.

    // 17 — the door-slice-3 selection doors, one round-trip: modify
    // (expand — the CP-0.5 dialog's kind, live-wired since then and
    // FLIPPED at the merge with zero shell changes), save to a named
    // channel (transient tracked id — the request-9 interim), deselect,
    // then LOAD the saved channel back. The ants visibly grow at the
    // modify, vanish at the deselect, and RETURN at the load — the
    // channel round-trip is the frame's payload.
    if (ok) {
        QVariantMap expand;
        expand.insert(QStringLiteral("expand"), 6);
        const QVariantMap r1 = controller->applyCommand(
            QStringLiteral("selection.modify"), {}, expand, {});
        const QVariantMap r2 = controller->saveSelectionAsChannel();
        const QVariantMap r3 = controller->applyCommand(
            QStringLiteral("selection.none"), {}, {}, {});
        const QVariantMap r4 = controller->loadSavedChannelSelection();
        std::printf("tour: modify ok=%s save ok=%s none ok=%s load ok=%s\n",
                    r1.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    r2.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    r3.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    r4.value(QStringLiteral("ok")).toBool() ? "true" : "false");
        ok = capture(window, dir, QStringLiteral("17-selection-doors.png"),
                     &inventory);
    }

    // 18 — doc.mode_convert (LIVE at door slice 3; ok=true on clean
    // docs — census + skip-cut discriminator). Since frame 16's text
    // layer (TEXTVEC flip) the doc holds a PARAMETRIC layer, so the
    // convert refuses at the earlier §40.2 staging gate ("needs the
    // rasterize-or-drop compat arm" — door slice 3's recorded honest
    // partial) BEFORE reaching the premul path. The FILED premul
    // defect (edit.cut leaves alpha-zero pixels with nonzero premul
    // color; §6.4 assert; the caught panic POISONS the handle — dead
    // session, review D1) reproduces via the CORRECTED minimal steps
    // — a cut through a MODIFIED (fractional-coverage-bearing)
    // selection is load-bearing: set -> fill -> modify{expand} -> cut
    // -> convert (reviewer three-way bisect; the un-modified minimal
    // passes — review round-2 D1 corrected my filing). The frame
    // flips to grayscale when BOTH the §40.2 arm and the cut fix
    // land. WARNING for frame appenders: on a doc without parametric
    // layers whose cut went through a modified selection, this step
    // would panic-poison the handle (every later call refuses, undo
    // included) — keep this the last live-document step or open a
    // fresh document first (the env-gated recovery block does).
    // 1.0 final (corrigendum (l), O-WATCH F-7): the 18-mode-convert.png step is removed --
    // its feature has no 1.0 control, so a frame of it would show the
    // release doing something a user cannot.

    // 19/20 — the warps row (transform.perspective + transform.warp,
    // LIVE at the warps merge). FRESH DOCUMENT by design: frame 18's
    // document must stay the last step on the old handle (see its
    // appender warning), and a clean canvas makes the geometric
    // evidence unambiguous. 19: a brush stroke, then a keystone quad
    // through the perspective door — the stroke visibly converges
    // toward the pinched edge. 20: warp preset arc 35 on the same
    // layer — the stroke bows.
    if (ok) {
        if (!controller->newDocument(640, 400, QStringLiteral("white"))) {
            std::fprintf(stderr, "tour: doc.new (warps) failed\n");
            ok = false;
        }
    }
    if (ok) {
        canvas->zoomFit();
        canvas->setTool(QStringLiteral("brush.stroke"));
        canvas->setBrushColor(QColor(0x1f, 0x5f, 0xa8));
        canvas->setBrushSize(20.0);
        const bool committed =
            canvas->tourStroke(QStringLiteral("brush.stroke"),
                               diagonalStroke(120, 90, 520, 310, 20));
        const QString layerId = controller->layers().isEmpty()
            ? QString()
            : controller->layers().first().toMap()
                  .value(QStringLiteral("layerId")).toString();
        QVariantMap target;
        target.insert(QStringLiteral("layerId"), layerId);
        const QPointF quad[4] = {QPointF(60, 40), QPointF(580, 90),
                                 QPointF(580, 310), QPointF(60, 360)};
        const QVariantMap r1 = controller->applyCommand(
            QStringLiteral("transform.perspective"), target,
            buildPerspectiveQuadParams(quad).toVariantMap(), {});
        std::printf("tour: stroke committed=%s perspective ok=%s code=%s\n",
                    committed ? "true" : "false",
                    r1.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    qPrintable(r1.value(QStringLiteral("code")).toString()));
        ok = capture(window, dir, QStringLiteral("19-perspective.png"),
                     &inventory);
        // 1.0 final (corrigendum (l), O-WATCH F-7): the warp half of this
        // step (frame 20) is removed; Warp has no 1.0 control.
    }

    // 21 — transform.puppet (warps row). Wire semantics per the ENGINE's
    // recorded ruling: pos = pin PLACEMENT; drag destinations are a
    // recorded contract gap (O-TOOLS escalation); the wire-expressible
    // deformation is ROTATION pins (+depth) — the §45.8 golden's own
    // shape. Three pins, one rotated 30 (the golden's fixture shape),
    // on the warped layer. The solve arm is feature-gated (arap-libigl):
    // ok=true means this build carries the solver and the layer bends
    // around the pins; a typed NI means the default build — either is
    // an honest frame, and the print carries the code.
    // 1.0 final (corrigendum (l), O-WATCH F-7): the 21-puppet.png step is removed --
    // its feature has no 1.0 control, so a frame of it would show the
    // release doing something a user cannot.

    // 22/23 — the filter registry (census-live filter.apply; eight
    // slice-1 filters + the CP-1.0 liquify checkpoint filter registered
    // engine-side). 22: gaussian blur sigma 6 — the warped stroke
    // softens visibly. 23: liquify with a REAL displacement field (the
    // §16.3 complete field payload — the engine's recorded
    // self-describing-params pin): a sinusoidal horizontal push over a
    // 32-px node lattice smears the blurred stroke sideways. Filter
    // menu/dialog chrome (and the liquify brush workspace tier) hold
    // for their mint/workspace waves; these are the doors' mechanics
    // evidence.
    if (ok) {
        const QString layerId = controller->layers().isEmpty()
            ? QString()
            : controller->layers().first().toMap()
                  .value(QStringLiteral("layerId")).toString();
        QVariantMap target;
        target.insert(QStringLiteral("layerId"), layerId);
        QJsonObject blurParams;
        blurParams.insert(QLatin1String("radius"), 18.0);
        const QVariantMap r1 = controller->applyCommand(
            QStringLiteral("filter.apply"), target,
            buildFilterApplyParams(QStringLiteral("filter.gaussian_blur"),
                                   blurParams).toVariantMap(), {});
        std::printf("tour: gaussian_blur ok=%s code=%s\n",
                    r1.value(QStringLiteral("ok")).toBool() ? "true" : "false",
                    qPrintable(r1.value(QStringLiteral("code")).toString()));
        ok = capture(window, dir, QStringLiteral("22-filter-blur.png"),
                     &inventory);
        // 21x13 node lattice at pitch 32 covers 640x384: dx = 14*sin(row),
        // dy = 0 — deterministic, visibly wavy.
        // 1.0 final (corrigendum (l), O-WATCH F-7): the liquify half of
        // this step (frame 23) is removed; Liquify has no 1.0 control.
    }

    // ---- 24-27: THE SUNDAY SIX, DRIVEN THROUGH THEIR REAL CONTROLS ----
    //
    // ★★★ WHY THESE EXIST AND WHY THEY ARE SHAPED LIKE THIS. Four of the
    // six weekend items -- spot heal, the blend control, the fill-layer
    // menu and the mask menu -- were LANDED and tested, and the tour
    // touched NONE of them: it dispatched 20 command kinds, `mask.add`
    // was not among them, and its only `layer.add` was kind:"raster".
    // So the Release binary had never performed any of them.
    //
    // ★★★★ AND THE OBVIOUS STEP WOULD HAVE BEEN A FALSE GREEN (O-UI's
    // catch). Every other step here changes state through
    // `controller->applyCommand(...)`. A step calling
    // applyCommand("layer.add", kind:"fill") would produce a frame and
    // prove the ENGINE ACCEPTS -- while never touching what the feature
    // IS: a MenuItem gated by `commandAvailability`. These steps drive
    // the CONTROLS instead, through the same window a person would.
    //
    // ★★ EACH ONE ASSERTS THE GATE BEFORE IT ACTS. Invoking `triggered`
    // on a MenuItem fires its handler whether or not the item is
    // enabled, so triggering ALONE tests the handler and is silent about
    // the door. `enabled=` is printed beside every action for exactly
    // that reason -- it is the half a trigger cannot see.
    auto qmlItem = [&](const char *name) -> QQuickItem * {
        return window->findChild<QQuickItem *>(QString::fromLatin1(name));
    };
    auto enabledOf = [&](const char *name) -> int {
        QQuickItem *it = qmlItem(name);
        return it ? (it->property("enabled").toBool() ? 1 : 0) : -1; // -1 = absent
    };
    auto triggerItem = [&](const char *name) -> bool {
        QObject *o = window->findChild<QObject *>(QString::fromLatin1(name));
        return o && QMetaObject::invokeMethod(o, "triggered");
    };

    // 24 — SPOT HEAL ON `J` (Q-D1(a)). Driven as a stroke through the
    // palette spelling, so it resolves through the SAME (tool -> kind,
    // mode) table a mouse press uses. Until 2026-08-29 tourStroke took a
    // wire kind and sent NO mode, which is precisely why this frame could
    // not have existed before.
    if (ok) {
        const int before = int(controller->historyEntries().size());
        const QByteArray pxBefore = compositeHash();
        canvas->setTool(QStringLiteral("heal.stroke"));
        canvas->setBrushSize(28.0);
        const bool committed = canvas->tourStroke(QStringLiteral("heal.stroke"),
                                                  diagonalStroke(80, 80, 560, 320, 24));
        // Capture before the after-hash -- see the note at frame 04.
        ok = capture(window, dir, QStringLiteral("24-heal-spot.png"), &inventory);
        const QByteArray pxAfter = compositeHash();
        std::printf("tour: heal.stroke(spot) committed=%s pixels_changed=%s history+%d\n",
                    committed ? "true" : "false",
                    pixelVerdict(pxBefore, pxAfter),
                    int(controller->historyEntries().size()) - before);
    }

    // 25 — THE BLEND CONTROL (Q-D2(a)), THE FLAGSHIP STEP. Set it to
    // Luminosity and put it back, which is what the demo script promises.
    // Driven through the real ComboBox: its `activated` signal is what a
    // click emits, and its model is the contract's 27 modes.
    if (ok) {
        QQuickItem *combo = qmlItem("layerBlendCombo");
        const int before = int(controller->historyEntries().size());
        int count = combo ? combo->property("count").toInt() : -1;
        bool drove = false;
        if (combo) {
            const QVariantList tags = combo->property("tags").toList();
            int lum = -1, nrm = -1;
            for (int i = 0; i < tags.size(); ++i) {
                const QString t = tags.at(i).toString();
                if (t == QLatin1String("luminosity")) lum = i;
                if (t == QLatin1String("normal")) nrm = i;
            }
            if (lum >= 0 && nrm >= 0) {
                QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, lum));
                QCoreApplication::processEvents();
                ok = ok && capture(window, dir, QStringLiteral("25-blend-luminosity.png"),
                                   &inventory);
                QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, nrm));
                QCoreApplication::processEvents();
                drove = true;
            }
        }
        // The count is reported so a shrunken model is visible in the log
        // rather than only in a test: 27 is the contract's enum size.
        std::printf("tour: blend control present=%s modes=%d drove=%s history+%d\n",
                    combo ? "true" : "false", count, drove ? "true" : "false",
                    int(controller->historyEntries().size()) - before);
        ok = ok && capture(window, dir, QStringLiteral("26-blend-restored.png"), &inventory);
    }

    // 27 — NEW FILL LAYER, through the MENU (O-UI's row 6). The gate is
    // read first and printed; then the item is triggered.
    if (ok) {
        const int before = int(controller->historyEntries().size());
        const int gate = enabledOf("fillButton");
        const bool fired = triggerItem("fillMenuLinear");
        QCoreApplication::processEvents();
        std::printf("tour: layer.addFill gate_enabled=%d triggered=%s history+%d\n",
                    gate, fired ? "true" : "false",
                    int(controller->historyEntries().size()) - before);
        ok = capture(window, dir, QStringLiteral("27-fill-layer.png"), &inventory);
    }

    // 28 — ADD MASK, through the MENU (O-UI's row 5).
    if (ok) {
        const int before = int(controller->historyEntries().size());
        const int gate = enabledOf("maskButton");
        const bool fired = triggerItem("maskMenuRevealAll");
        QCoreApplication::processEvents();
        std::printf("tour: mask.add gate_enabled=%d triggered=%s history+%d\n",
                    gate, fired ? "true" : "false",
                    int(controller->historyEntries().size()) - before);
        ok = capture(window, dir, QStringLiteral("28-mask-reveal-all.png"), &inventory);
    }

    // ---- 29-30: THE TWO SUNDAY ROWS WITH NO BINARY EVIDENCE ----
    //
    // Rows 1, 2, 5 and 6 are driven above. Rows 3 (import widening) and 4
    // (sponge with its label) had NO Release-binary evidence at all --
    // measured off the scoreboard, not assumed -- so they get the same
    // treatment rather than resting on unit tests and a CLI transcript.

    // 29 — ROW 4: SPONGE (Q-D3(b)). Driven as a stroke through the PALETTE
    // SPELLING, so it resolves through the same (tool -> kind, mode) table
    // a mouse press uses: "dodge_burn.sponge" must reach the engine as
    // dodge_burn.stroke with mode "desaturate". The operator ruled the
    // mapping AND the label, so both are printed -- the label is the half
    // that stops an interim mapping being silent, and a frame that showed
    // the tool working while the label had rotted would be a false green.
    if (ok) {
        // ★★★ SELECT A RASTER TARGET FIRST, BY MEASUREMENT. The first run of
        // this step inherited the active layer from step 28 -- the FILL layer
        // it had just masked -- and the engine refused:
        // "target.layerId ... is not a raster layer", error.command.bad_target.
        // dodge_burn.stroke paints pixels and a fill layer has none to paint.
        //
        // ★★ THE REFUSED FRAME WAS NOT WASTED AND IS WORTH THE COMMENT: it
        // showed the palette rendering "Sponge (desaturate)" (Q-D3(b)'s label,
        // in the shipped binary), the untruncated refusal toast (P1-2), and the
        // echo held as "1 pending preview" -- the documented degraded state --
        // all in one capture. A step that fails against a real engine still
        // photographs three things that work.
        //
        // The row is found by KIND rather than by index: index 0 is a
        // convention and the preceding steps move the stack.
        {
            const QVariantList rows = controller->layers();
            for (int i = 0; i < rows.size(); ++i) {
                if (rows.at(i).toMap().value(QStringLiteral("kind")).toString()
                    == QLatin1String("raster")) {
                    controller->setActiveLayerIndex(i);
                    break;
                }
            }
            QCoreApplication::processEvents();
        }
        const int before = int(controller->historyEntries().size());
        const QByteArray spBefore = compositeHash();
        canvas->setTool(QStringLiteral("dodge_burn.sponge"));
        canvas->setBrushSize(30.0);
        const bool committed =
            canvas->tourStroke(QStringLiteral("dodge_burn.sponge"),
                               diagonalStroke(80, 80, 560, 320, 24));
        // The LABEL half of Q-D3(b) is not printed here: this function has
        // no catalog handle, and reaching for one would be a worse fix than
        // leaving it to the check that already owns it --
        // tst_palette_pins pins the sponge label to the mode ACTUALLY sent,
        // so a tool relabelled correctly and rewired wrongly still reds
        // there. The tour's job is the dispatch, and kind+mode are it.
        // Capture before the after-hash -- see the note at frame 04.
        ok = capture(window, dir, QStringLiteral("29-sponge.png"), &inventory);
        const QByteArray spAfter = compositeHash();
        std::printf("tour: sponge committed=%s pixels_changed=%s kind=%s mode=%s history+%d\n",
                    committed ? "true" : "false",
                    pixelVerdict(spBefore, spAfter),
                    qPrintable(controller->lastStrokeKind()),
                    qPrintable(controller->lastStrokeMode()),
                    int(controller->historyEntries().size()) - before);
    }

    // 30 — ROW 3: IMPORT WIDENING (Q-D5(a), item 22). Export a NON-PNG and
    // re-import it through DocumentController::importImage -- the very
    // function Main.qml's importFileDialog calls on accept. Mirrors the
    // frame-11 png round trip deliberately: same doors, one format apart,
    // so a difference between them is the widening and nothing else.
    //
    // ★ THE FILE GOES TO A TEMP PATH, NOT INTO THE TOUR DIRECTORY. frame
    // 11 writes export-check.png beside the frames and that file is a
    // documented non-frame the inventory excludes; adding a second such
    // file would move a count two desks are currently reasoning about.
    //
    // ★★ WHAT THIS DOES AND DOES NOT PROVE: it drives the SHELL's import
    // door, not the native file dialog, which no offscreen tour can open.
    // So it proves the path from the dialog's accept handler downward. The
    // dialog's own filter list is pinned separately by tst_import_filter.
    if (ok && abiHasBlock12()) {
        const QString jpg = QDir::temp().filePath(QStringLiteral("appname-tour-reimport.jpg"));
        QFile::remove(jpg);
        const bool exported = controller->exportDocument(jpg, QStringLiteral("jpeg"), 90);
        const bool reimported = exported && controller->importImage(jpg);
        std::printf("tour: import-widening export_jpeg=%s reimport=%s\n",
                    exported ? "true" : "false", reimported ? "true" : "false");
        // Q-D26, same defect as the PNG round-trip above and fixed the same way:
        // measured `export_jpeg=true reimport=true` in a real run, and the block is
        // behind the same `abiHasBlock12()` gate. THERE ARE TWO ROUND-TRIP SITES and
        // the question named one -- fixing the named instance and leaving its twin
        // is the failure this desk keeps recording, so both move together.
        ok = ok && exported && reimported;
        QFile::remove(jpg);
        ok = ok && capture(window, dir, QStringLiteral("30-import-jpeg.png"), &inventory);
    }

    // 14 — W05 recovery restore (env-gated: APPNAME_TOUR_RECOVER names a
    // package whose sidecar holds a crafted crash log; absent = the
    // standing 14-frame tour is unchanged). Exercises the SHELL's
    // queryRecovery -> openRecoveredDocument path; the frame shows the
    // REPLAYED state (content the package on disk never had).
    const QByteArray recoverPkg = qgetenv("APPNAME_TOUR_RECOVER");
    if (ok && !recoverPkg.isEmpty()) {
        const QString pkg = QString::fromUtf8(recoverPkg);
        const QVariantMap q = controller->queryRecovery(pkg);
        std::printf("tour: recovery available=%s commands=%d torn=%s\n",
                    q.value(QStringLiteral("available")).toBool() ? "true" : "false",
                    q.value(QStringLiteral("commands")).toInt(),
                    q.value(QStringLiteral("torn")).toBool() ? "true" : "false");
        bool restored = false;
        if (q.value(QStringLiteral("available")).toBool())
            restored = controller->openRecoveredDocument(
                pkg, q.value(QStringLiteral("logPath")).toString());
        std::printf("tour: recovery restore ok=%s\n", restored ? "true" : "false");
        ok = restored
            && capture(window, dir, QStringLiteral("14-recovery.png"), &inventory);
    }

    // 31 — THE DISCARD CONFIRM. Standing priority (4)-3 shipped this dialog
    // on 2026-09-01 and NOTHING HAD EVER DRAWN IT; a one-off grab showed it
    // rendering correctly and then ceased to exist, which is strictly better
    // than never and strictly worse than coverage.
    //
    // ★★ WHAT THIS FRAME IS FOR, AND WHAT IT IS NOT FOR. It pins a STATE:
    // that the dialog renders, that its three strings RESOLVE, and that
    // nothing clips. Had the catalog lookup missed, the title would read
    // `ui.dialog.discard_confirm.title` verbatim -- a raw MessageId that
    // throws nothing, exits zero, and that no assertion in tst_dirty_flag
    // can name. It does NOT pin the DOOR: that a dirty document actually
    // routes here is an EVENT, pinned by tst_dirty_flag and by
    // tst_gesture_chrome's newDocumentPresetCreatesThatDocument. Different
    // instruments for different claims, chosen deliberately.
    //
    // ★ OPENED DIRECTLY rather than through confirmDiscard(): the helper
    // takes a JS callback, which is awkward to build from here, and the
    // frame's job is the rendering and not the routing. Stated rather than
    // implied, because a frame that looks like it proves the path and does
    // not is the kind of second evidence source this ledger keeps catching.
    //
    // Last capture in the tour: it leaves a MODAL open, so nothing may
    // follow it.
    if (ok) {
        QObject *dlg = nullptr;
        for (QObject *o : window->findChildren<QObject *>()) {
            if (QLatin1String(o->metaObject()->className())
                    .startsWith(QLatin1String("DiscardConfirmDialog"))) {
                dlg = o;
                break;
            }
        }
        if (!dlg) {
            std::fprintf(stderr, "tour: DiscardConfirmDialog not found in the chrome\n");
            ok = false;
        } else {
            QMetaObject::invokeMethod(dlg, "open");
            for (int i = 0; i < 8; ++i)
                QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
            if (!dlg->property("visible").toBool()) {
                std::fprintf(stderr, "tour: DiscardConfirmDialog did not become visible\n");
                ok = false;
            } else {
                ok = capture(window, dir, QStringLiteral("31-discard-confirm.png"),
                             &inventory);
            }
        }
    }

    // ── B-12 / B-03 FRAMES (O-UI, 2026-09-08, on the operator's word) ──
    // Appended rather than inserted: inserting would renumber 29-31 and
    // move goldens that have nothing to do with these features. Frame 31
    // leaves its modal open by design ("nothing may follow it"), so the
    // first thing here is to close it — otherwise every frame below would
    // photograph the discard dialog instead of its own subject.
    if (ok) {
        for (QObject *o : window->findChildren<QObject *>()) {
            if (QLatin1String(o->metaObject()->className())
                    .startsWith(QLatin1String("DiscardConfirmDialog"))) {
                QMetaObject::invokeMethod(o, "close");
                break;
            }
        }
        for (int i = 0; i < 8; ++i)
            QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
    }

    // 32 — THE MASK PANEL (B-12). Its five controls are gated on the
    // ACTIVE layer having a mask, and frames 29-30 moved the selection
    // since the mask added at 28 — so this re-establishes its own state
    // rather than inheriting it. A frame of the collapsed heading would
    // photograph the empty case and prove nothing about the controls.
    if (ok) {
        const int before = int(controller->historyEntries().size());
        const bool fired = triggerItem("maskMenuRevealAll");
        QCoreApplication::processEvents();
        std::printf("tour: mask panel gate triggered=%s history+%d\n",
                    fired ? "true" : "false",
                    int(controller->historyEntries().size()) - before);
        ok = capture(window, dir, QStringLiteral("32-mask-panel.png"), &inventory);
    }

    // 33-38 — THE ADJUSTMENT DIALOGS. B-03's acceptance clause is "one tour
    // frame each" for SIX dialogs, all by 09-15; X-08's floor is ">=3" and
    // was met by the first three. THE FIRST THREE ARE NOT AN ARBITRARY PICK:
    // §7's cut line 3 names exactly Brightness/Contrast, Levels and
    // Hue/Saturation as the three that ship if the set is cut, so they are
    // listed first and remain the survivors' set. 36-38 complete B-03.
    //
    // Each opens through its REAL menu item — the Q-D6 standard, a scripted
    // step driving the real control, never applyCommand — and is closed
    // before the next, so no frame inherits another's modal.
    {
        struct AdjFrame { const char *menu; const char *file; };
        static const AdjFrame kAdj[] = {
            {"adjustMenuBrightnessContrast", "33-adjust-brightness-contrast.png"},
            {"adjustMenuLevels",             "34-adjust-levels.png"},
            {"adjustMenuHueSaturation",      "35-adjust-hue-saturation.png"},
            {"adjustMenuExposure",           "36-adjust-exposure.png"},
            {"adjustMenuThreshold",          "37-adjust-threshold.png"},
            {"adjustMenuCurves",             "38-adjust-curves.png"},
        };
        for (const AdjFrame &f : kAdj) {
            if (!ok)
                break;
            const bool fired = triggerItem(f.menu);
            for (int i = 0; i < 8; ++i)
                QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
            QObject *dlg = nullptr;
            for (QObject *o : window->findChildren<QObject *>()) {
                if (QLatin1String(o->metaObject()->className())
                        .startsWith(QLatin1String("AdjustmentParamDialog"))) {
                    dlg = o;
                    break;
                }
            }
            const bool shown = dlg && dlg->property("visible").toBool();
            std::printf("tour: %s triggered=%s visible=%s\n", f.menu,
                        fired ? "true" : "false", shown ? "true" : "false");
            if (!shown) {
                std::fprintf(stderr, "tour: %s did not open\n", f.menu);
                ok = false;
                break;
            }
            ok = capture(window, dir, QLatin1String(f.file), &inventory);
            QMetaObject::invokeMethod(dlg, "close");
            for (int i = 0; i < 8; ++i)
                QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
        }
    }

    // 39 — THE ABOUT DIALOG (B-09). Its DONE-WHEN is "About shows all;
    // matches THIRD-PARTY-NOTICES", VERIFIED-BY an O-WATCH READ — and until
    // now there was nothing for them to read but source. Opened through the
    // REAL Help menu item, the Q-D6 standard.
    //
    // THE LICENCES VIEWER IS DELIBERATELY NOT PHOTOGRAPHED HERE. In the
    // tour's environment no staged Contents/Resources/licenses resolves, so
    // the frame would depict the UNAVAILABLE state rather than the eleven
    // texts — a picture that documents the dev build while reading as
    // coverage of the feature. An honest gap beats a misleading frame.
    if (ok) {
        const bool fired = triggerItem("helpAboutItem");
        for (int i = 0; i < 8; ++i)
            QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
        QObject *dlg = nullptr;
        for (QObject *o : window->findChildren<QObject *>()) {
            if (QLatin1String(o->metaObject()->className())
                    .startsWith(QLatin1String("AboutDialog"))) {
                dlg = o;
                break;
            }
        }
        const bool shown = dlg && dlg->property("visible").toBool();
        std::printf("tour: helpAboutItem triggered=%s visible=%s\n",
                    fired ? "true" : "false", shown ? "true" : "false");
        if (!shown) {
            std::fprintf(stderr, "tour: helpAboutItem did not open\n");
            ok = false;
        } else {
            // ★ THE BUILD-COMMIT ROW IS BLANKED FOR THE CAPTURE, AND THE
            // FRAME WOULD BE UNUSABLE WITHOUT THIS. AboutDialog renders
            // "Build <sha>-dirty", which changes on EVERY COMMIT — so a
            // golden containing it can never match a fresh run again: the
            // next commit after any re-pin breaks tour_ok's artifact arm
            // permanently. Found by LOOKING at the first capture, not by
            // reasoning about it (C-758's lesson, applied one tick later to
            // the change it prompted).
            //
            // The row's own rendering is NOT lost from coverage: it is
            // pinned two-directionally by tst_about_render
            // (anUnsetVersionRendersNOTHING / aSetVersionAndCommitDORender),
            // which is the right instrument for a VALUE. This frame's job is
            // the credit SET, which is what an O-WATCH read of B-09 needs.
            // LIMIT STATED: the frame therefore documents the dialog WITHOUT
            // that row, and the binding it breaks is not restored — this is
            // the last step of the run.
            dlg->setProperty("buildCommit", QString());
            for (int i = 0; i < 4; ++i)
                QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
            ok = capture(window, dir, QStringLiteral("39-about.png"), &inventory);
            // Closed before the run ends: a modal left open is how one frame
            // poisons the next (C-730), even when it is the last step today.
            QMetaObject::invokeMethod(dlg, "close");
            for (int i = 0; i < 8; ++i)
                QCoreApplication::processEvents(QEventLoop::AllEvents, 50);
        }
    }

    std::printf("tour: %s — %d file(s) in %s\n", ok ? "PASS" : "FAIL",
                static_cast<int>(inventory.size()), qPrintable(dir.absolutePath()));
    return ok ? 0 : 1;
}

} // namespace appname::shell
