// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 11d Phase 1 — THE OPTIONS BAR NET. Boots the REAL
// Main.qml offscreen (the item-7/14b harness: this binary compiles its
// own copy of the AppnameShell QML module and wires the real
// controller/announcer like shell/src/main.cpp) against REAL engine
// documents and asserts:
//   - the tool×control PRESENCE/ABSENCE matrix over the declarative
//     per-tool model (empty bar for optionless tools — the common
//     case), including the RESERVED absences (font_family/font_style,
//     puppet_density, warp_bend) whose triggers are recorded in the
//     lane log, and — Pro-Wave row 4 — the EXACT per-tool presence of
//     stroke_width, whose reservation the row deliberately lifted;
//   - Repeater-created controls sit in toolbarPane's PARENT CHAIN (the
//     §31.3 no-double-speak guard's walk is load-bearing — the 11d
//     structure rule);
//   - binding truth BOTH directions per control family, with ENGINE
//     truth where the wire is observable: pen commit-as by JOURNAL KIND
//     (vector.shape_add vs vector.path_new), text style.size and the
//     puppet knobs by the engine's own typed out-of-range refusal (the
//     param must have RIDDEN the envelope for the engine to name it —
//     the state properties carry values verbatim, range duty is the
//     control's, so an illegal programmatic value is the wire spy);
//   - T6 brush/eraser/wand rehouse parity (ranges, bindings, the D3
//     announcement guard) — old behavior preserved;
//   - show_mesh is a REAL overlay (canvas-region pixel diff, ruled
//     presentation-only under D2 — the engine's §45.5 triangulation has
//     no readback door and is NOT faked);
//   - item 4, the hardness row: shape/binding/clamp, the autosave wire
//     spy (brush carries the set value, eraser carries NO hardness key),
//     engine-silence + no invented announcement, and ENGINE-readback
//     pixel truth at the schema extremes (soft transect strictly wider
//     than hard — the raster.rs smoothstep direction).

#include "announcer.h"
#include "click_focus_parity.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

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

#include <functional>

using namespace appname::shell;

namespace {

// Visible interactive controls of one class inside a container.
QList<QQuickItem *> visibleByClass(QQuickItem *rootItem, const char *className)
{
    QList<QQuickItem *> hits;
    std::function<void(QQuickItem *)> walk = [&](QQuickItem *node) {
        const auto children = node->childItems();
        for (QQuickItem *c : children) {
            if (c->isVisible() && c->inherits(className))
                hits << c;
            walk(c);
        }
    };
    walk(rootItem);
    return hits;
}

QStringList visibleLabelTexts(QQuickItem *rootItem)
{
    QStringList out;
    for (QQuickItem *l : visibleByClass(rootItem, "QQuickLabel"))
        out << l->property("text").toString();
    return out;
}

} // namespace

class TstOptionsBar : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void cleanup();
    void catalogGround();
    void toolControlPresenceMatrix();
    void controlsSitInGuardParentChain();
    void brushEraserWandRehouseParity();
    void sizeRowWRITESForEveryToolThatShowsIt();
    // The SAME question asked with the user's own input device (C-784).
    void sizeRowWritesUnderAMouseDragNotJustAKeyPress();
    // Row 5 (C-666): ONE property behind three tools' rows — binding in
    // both directions, and the SHARED state visible across tool switches.
    void sampleAllRowIsCloneOnly();
    void brushSizeAnnouncementGuardPreserved();
    // Item 4 — the hardness row (the FI-9 payoff): shape/binding, the
    // autosave wire spy, engine-silence + no invented announcement, and
    // ENGINE PIXEL TRUTH at the schema extremes (direction pinned by
    // raster.rs: t = 1 - smoothstep(hardness, 1, r)).
    void hardnessRowShapeAndBinding();
    void hardnessRidesTheStrokeEnvelopeEngineProven();
    void hardnessEngineSilentUntilStrokeAndUnannounced();
    void hardnessExtremesEnginePixelTruth();
    void penCommitAsSegmentJournalKinds();
    void fontSizeRidesStyleSizeEngineProven();
    void puppetKnobsRideTheEnvelopeEngineProven();
    void showMeshIsARealOverlay();
    // Pro-Wave row 4 — the SHAPE STROKE rows. The hazard proof runs
    // FIRST and stands alone: a control is not built on a door that
    // fails downstream of the command.
    void strokeSurvivesCompositeAndExportDoors();
    void strokeWidthRowShapeAndBinding();
    void strokeWidthGestureReachesTheCompositeAndExport();
    void strokeWidthZeroIsTheNoStrokePath();
    void strokeWidthEngineRefusalSurfacesTyped();
    void strokeKnobsWithoutCatalogedIdsAreHeld();
    void strokeBandOutsideTheShapeBoundsIsRepainted();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QQuickItem *m_canvas = nullptr;
    QQuickItem *m_toolbar = nullptr;

    QString catTr(const char *key) const
    {
        return m_i18n->tr(QLatin1String(key));
    }
    QVariant evalJs(const QString &code) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "evalJs failed:" << expr.error().toString();
        return undefined ? QVariant() : v;
    }
    void setTool(const char *wireId)
    {
        m_canvas->setProperty("tool", QLatin1String(wireId));
        QCOMPARE(m_canvas->property("tool").toString(), QLatin1String(wireId));
        // The Repeater rebuilds its delegates on the model swap —
        // process the pending events before any lookup.
        QCoreApplication::processEvents();
    }
    void freshDoc()
    {
        QVERIFY(m_controller->newDocument(320, 200, QStringLiteral("white")));
        focusCanvas();
    }
    void focusCanvas()
    {
        m_canvas->forceActiveFocus();
        QTRY_VERIFY(m_canvas->hasActiveFocus());
    }
    QPoint docToView(double x, double y) const
    {
        const double zoom = m_canvas->property("zoom").toDouble();
        const double dw = m_controller->docWidth();
        const double dh = m_controller->docHeight();
        const double offX = (m_canvas->width() - dw * zoom) / 2.0;
        const double offY = (m_canvas->height() - dh * zoom) / 2.0;
        return m_canvas->mapToScene(QPointF(offX + x * zoom, offY + y * zoom))
            .toPoint();
    }
    void clickDoc(double x, double y)
    {
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(x, y));
    }
    void dragStroke(double x0, double y0, double x1, double y1)
    {
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier,
                          docToView(x0, y0));
        const int steps = 6;
        for (int i = 1; i <= steps; ++i)
            QTest::mouseMove(m_window,
                             docToView(x0 + (x1 - x0) * i / steps,
                                       y0 + (y1 - y0) * i / steps));
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier,
                            docToView(x1, y1));
    }
    // Item 4: the hardness slider is the one whose bounds are the
    // CONTRACT range (to == 1; brush size's is 200) — identity by
    // bounds, not child order.
    QQuickItem *hardnessSlider() const
    {
        QQuickItem *hard = nullptr;
        int matches = 0;
        const auto sliders = visibleByClass(m_toolbar, "QQuickSlider");
        for (QQuickItem *s : sliders)
            if (qFuzzyCompare(s->property("to").toDouble(), 1.0)) {
                hard = s;
                ++matches;
            }
        return matches == 1 ? hard : nullptr;
    }
    void clickItem(QQuickItem *item)
    {
        QVERIFY(item);
        QTRY_VERIFY(item->width() > 0 && item->height() > 0);
        // C-666 layout-polish guard, MEASURED live: Repeater-created
        // option rows are born at the layout ORIGIN and stack there
        // until the RowLayout's polish positions them — two visible
        // checkboxes both mapped to scene (8,40), and the synthesized
        // click landed on whichever sibling stacked on top, not on the
        // mapped target (red only after prior-case delegate churn, so
        // the race hid from single-case runs). A mapped position is
        // echo until the target is the TOPMOST item at its own center;
        // wait for that fact, not for a timer.
        // NOT a topmost hit-test: QQuickItem::childAt dead-ends at a
        // Control's full-size BACKGROUND rectangle (measured — the
        // ToolBar's childAt answers its background, which has no
        // children), so a descent can never certify the target. Encode
        // the measured defect itself instead: the click may only be
        // synthesized once NO OTHER control's scene rect covers the
        // target's center — while rows stack pre-polish, the covering
        // sibling is exactly what would swallow the click.
        const auto centerUncovered = [this, item]() -> bool {
            const QPointF c = item->mapToScene(
                QPointF(item->width() / 2.0, item->height() / 2.0));
            for (QQuickItem *ctl : visibleByClass(m_toolbar, "QQuickControl")) {
                if (ctl == item || ctl->isAncestorOf(item)
                    || item->isAncestorOf(ctl))
                    continue;
                const QRectF r(ctl->mapToScene(QPointF(0, 0)),
                               QSizeF(ctl->width(), ctl->height()));
                if (r.contains(c))
                    return false;
            }
            return true;
        };
        QTRY_VERIFY2(centerUncovered(),
                     "another control still covers the click target's center "
                     "(pre-polish sibling stacking never resolved)");
        const QPointF scene = item->mapToScene(
            QPointF(item->width() / 2.0, item->height() / 2.0));
        QTest::mouseClick(m_window, Qt::LeftButton, Qt::NoModifier,
                          scene.toPoint());
    }
    QQuickItem *toolbarControl(const char *className, int index = 0) const
    {
        const QList<QQuickItem *> hits = visibleByClass(m_toolbar, className);
        return index < hits.size() ? hits.at(index) : nullptr;
    }
    QQuickItem *toolbarButtonByText(const char *catalogKey) const
    {
        const QString label = catTr(catalogKey);
        const auto buttons = visibleByClass(m_toolbar, "QQuickAbstractButton");
        for (QQuickItem *b : buttons)
            if (b->property("text").toString() == label)
                return b;
        return nullptr;
    }
    QStringList journalKinds() const
    {
        QStringList out;
        const QVariantList rows =
            m_controller->property("historyEntries").toList();
        for (const QVariant &v : rows)
            out << v.toMap().value(QStringLiteral("kind")).toString();
        return out;
    }
    int announcementsFrom(const char *source) const
    {
        int n = 0;
        const QVariantList log = m_announcer->announcementLog();
        for (const QVariant &v : log)
            if (v.toMap().value(QStringLiteral("source")).toString()
                == QLatin1String(source))
                ++n;
        return n;
    }
    QImage canvasRegionGrab()
    {
        QImage frame = m_window->grabWindow();
        const QPointF tl = m_canvas->mapToScene(QPointF(0, 0));
        const qreal dpr = frame.devicePixelRatio() > 0
            ? frame.devicePixelRatio() : 1.0;
        const QRect r(qRound(tl.x() * dpr), qRound(tl.y() * dpr),
                      qRound(m_canvas->width() * dpr),
                      qRound(m_canvas->height() * dpr));
        return frame.copy(r);
    }
};

void TstOptionsBar::initTestCase()
{
    // Mirror shell/src/main.cpp's wiring (the tst_palette_pins harness).
    m_i18n = new CatalogQml(Catalog::load(QStringLiteral("en")));
    m_controller = new DocumentController(&m_i18n->catalog());
    m_announcer = new Announcer(&m_i18n->catalog());
    m_controller->setAnnouncer(m_announcer);

    m_engine = new QQmlApplicationEngine;
    m_engine->addImageProvider(QStringLiteral("lucide"),
                               new LucideIconProvider);
    m_engine->addImageProvider(QStringLiteral("layerthumb"),
                               new LayerThumbProvider(m_controller));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                m_controller);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppA11y"),
                                                m_announcer);
    // AppUnits: main.cpp:132 registers it and this harness did not, so
    // NewDocumentDialog threw two ReferenceErrors and a TypeError on every
    // boot — this suite was exercising a tree the application never has.
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                new UnitsQml(m_engine));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppKeys"),
                                                new QObject(m_engine));
    m_engine->loadFromModule("AppnameShell", "Main");
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "Main.qml failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    m_announcer->setSubject(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));

    m_canvas = m_window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    QVERIFY2(m_canvas, "canvasView not found");
    m_toolbar = qobject_cast<QQuickItem *>(
        evalJs(QStringLiteral("toolbarPane")).value<QObject *>());
    QVERIFY2(m_toolbar, "toolbarPane not resolvable");
}

void TstOptionsBar::cleanup()
{
    if (m_canvas)
        m_canvas->setProperty("tool", QStringLiteral("brush.stroke"));
    if (m_controller && m_controller->hasDocument())
        m_controller->applyCommand(QStringLiteral("selection.none"), {}, {},
                                   QString());
}

void TstOptionsBar::catalogGround()
{
    // Echo-proofing (RULE 9): every consumed ui.options.* id resolves to
    // its REAL checked-in en.json value — a key-echo here would turn the
    // matrix below into self-confirmation.
    QCOMPARE(catTr("ui.options.brush_size"), QStringLiteral("Size"));
    QCOMPARE(catTr("ui.options.brush_hardness"), QStringLiteral("Hardness"));
    QCOMPARE(catTr("ui.options.brush_color"), QStringLiteral("Brush color"));
    QCOMPARE(catTr("ui.options.wand_tolerance"), QStringLiteral("Tolerance"));
    QCOMPARE(catTr("ui.options.wand_contiguous"), QStringLiteral("Contiguous"));
    // Row 5: the 11d Phase-2 mint goes EMITTED (it waited cataloged-
    // unemitted until C-666).
    QCOMPARE(catTr("ui.options.sample_all_layers"),
             QStringLiteral("Sample All Layers"));
    QCOMPARE(catTr("ui.options.commit_as"), QStringLiteral("Commit as"));
    QCOMPARE(catTr("ui.options.commit_shape"), QStringLiteral("Shape layer"));
    QCOMPARE(catTr("ui.options.commit_path"), QStringLiteral("Path"));
    QCOMPARE(catTr("ui.options.font_size"), QStringLiteral("Size"));
    QCOMPARE(catTr("ui.options.puppet_expansion"), QStringLiteral("Expansion"));
    QCOMPARE(catTr("ui.options.puppet_rigidity"), QStringLiteral("Rigidity"));
    QCOMPARE(catTr("ui.options.puppet_show_mesh"), QStringLiteral("Show Mesh"));
    // Pro-Wave row 4: stroke_width is now a CONSUMED id, not a reserved
    // one — echo-proofed here like the rest.
    QCOMPARE(catTr("ui.options.stroke_width"), QStringLiteral("Stroke"));
    // RESERVED ids stay cataloged (regression guard on the mint) — their
    // CONTROLS must not exist (matrix below).
    QCOMPARE(catTr("ui.options.font_family"), QStringLiteral("Font"));
    QCOMPARE(catTr("ui.options.font_style"), QStringLiteral("Style"));
    QCOMPARE(catTr("ui.options.puppet_density"), QStringLiteral("Density"));
    QCOMPARE(catTr("ui.options.warp_bend"), QStringLiteral("Bend"));
}

void TstOptionsBar::toolControlPresenceMatrix()
{
    freshDoc();

    struct Row {
        const char *tool;
        int sliders;
        int spinboxes;
        int checkboxes;
        int switches;
        int buttons; // ToolButtons: swatch or segment values
        QStringList captions; // visible Label texts (subset check)
    };
    const QList<Row> matrix = {
        // Item 4: brush gains the hardness slider; eraser does NOT (its
        // contract is un-extended) — the slider COUNT is the absence
        // proof, and the parity test asserts the caption directly.
        {"brush.stroke", 2, 0, 0, 0, 1,
         {catTr("ui.options.brush_size"), catTr("ui.options.brush_hardness")}},
        {"eraser.stroke", 1, 0, 0, 0, 0, {catTr("ui.options.brush_size")}},
        // Row 5: clone carries the sample-all checkbox (1.0 final S-08: heal's
        // was removed -- it had no effect)
        // (checkbox text is the control's own, so captions stay
        // label-only). 1.0 final D-8: the wand does NOT; it always samples
        // the composite, so its one box is Contiguous.
        {"select.wand", 0, 1, 1, 0, 0, {catTr("ui.options.wand_tolerance")}},
        {"heal.stroke", 1, 0, 0, 0, 0, {catTr("ui.options.brush_size")}},
        {"clone.stroke", 1, 0, 1, 0, 0, {catTr("ui.options.brush_size")}},
        {"vector.pen", 0, 0, 0, 0, 2, {catTr("ui.options.commit_as")}},
        {"text", 0, 1, 0, 0, 0, {catTr("ui.options.font_size")}},
        {"transform.puppet", 1, 1, 0, 1, 0,
         {catTr("ui.options.puppet_expansion"),
          catTr("ui.options.puppet_rigidity")}},
        // The COMMON empty-options state (load-bearing for the C2 skip
        // rule): a representative sweep of optionless tools.
        {"move", 0, 0, 0, 0, 0, {}},
        {"select.rect", 0, 0, 0, 0, 0, {}},
        {"select.lasso", 0, 0, 0, 0, 0, {}},
        {"crop", 0, 0, 0, 0, 0, {}}, // crop stays dialog-housed (recorded)
        {"transform.free", 0, 0, 0, 0, 0, {}},
        {"transform.perspective", 0, 0, 0, 0, 0, {}},
        {"transform.warp", 0, 0, 0, 0, 0, {}}, // warp_bend RESERVED
        // Pro-Wave row 4: the shape tools gain the stroke-WIDTH
        // spinbox. Deliberate flip of a RESERVED absence — see the
        // stroke_width note below.
        {"shape.rect", 0, 1, 0, 0, 0, {catTr("ui.options.stroke_width")}},
        {"shape.ellipse", 0, 1, 0, 0, 0, {catTr("ui.options.stroke_width")}},
    };

    for (const Row &row : matrix) {
        setTool(row.tool);
        const auto ctx = QStringLiteral("tool=%1").arg(QLatin1String(row.tool));
        QVERIFY2(visibleByClass(m_toolbar, "QQuickSlider").size() == row.sliders,
                 qPrintable(ctx + QStringLiteral(" slider count")));
        QVERIFY2(visibleByClass(m_toolbar, "QQuickSpinBox").size()
                     == row.spinboxes,
                 qPrintable(ctx + QStringLiteral(" spinbox count")));
        QVERIFY2(visibleByClass(m_toolbar, "QQuickCheckBox").size()
                     == row.checkboxes,
                 qPrintable(ctx + QStringLiteral(" checkbox count")));
        QVERIFY2(visibleByClass(m_toolbar, "QQuickSwitch").size()
                     == row.switches,
                 qPrintable(ctx + QStringLiteral(" switch count")));
        QVERIFY2(visibleByClass(m_toolbar, "QQuickToolButton").size()
                     == row.buttons,
                 qPrintable(ctx + QStringLiteral(" tool-button count")));
        const QStringList labels = visibleLabelTexts(m_toolbar);
        for (const QString &want : row.captions)
            QVERIFY2(labels.contains(want),
                     qPrintable(ctx + QStringLiteral(" missing caption ")
                                + want));
        // stroke_width: THE RESERVED ABSENCE THIS ROW DELIBERATELY
        // FLIPPED. It was pinned absent because the compositor refused
        // widthPx > 0 typed ("fill-only v1"), so a width control would
        // have wedged every subsequent composite. That trigger has
        // FIRED (pro/shape-stroke), and the flip was proven at the door
        // that used to fail — composite readback AND the exported PNG,
        // on pixels (strokeSurvivesCompositeAndExportDoors below). The
        // assertion is not deleted: it is UPGRADED from a blanket
        // absence to an EXACT per-tool presence/absence pin, which is
        // strictly stronger — it still catches a leak onto any of the
        // 13 tools that must not carry it, AND now catches its
        // disappearance from the two that must.
        const bool rowOwnsStroke =
            row.captions.contains(catTr("ui.options.stroke_width"));
        QVERIFY2(labels.contains(catTr("ui.options.stroke_width"))
                     == rowOwnsStroke,
                 qPrintable(ctx
                            + (rowOwnsStroke
                                   ? QStringLiteral(" stroke_width vanished")
                                   : QStringLiteral(" stroke_width leaked"))));
        // The still-RESERVED absences (honest-state rule; triggers
        // recorded in the lane log — flip these WITH the trigger, never
        // before): font_family/font_style (style.font inert in the text
        // door), puppet_density (value ids unminted), warp_bend (no live
        // session).
        QVERIFY2(!labels.contains(catTr("ui.options.font_family")),
                 qPrintable(ctx + QStringLiteral(" font_family leaked")));
        QVERIFY2(!labels.contains(catTr("ui.options.puppet_density")),
                 qPrintable(ctx + QStringLiteral(" puppet_density leaked")));
        QVERIFY2(!labels.contains(catTr("ui.options.warp_bend")),
                 qPrintable(ctx + QStringLiteral(" warp_bend leaked")));
        QVERIFY2(visibleByClass(m_toolbar, "QQuickComboBox").isEmpty(),
                 qPrintable(ctx + QStringLiteral(" unexpected combo")));
    }
}

void TstOptionsBar::controlsSitInGuardParentChain()
{
    // 11d structure rule: Repeater-created controls sit in toolbarPane's
    // PARENT CHAIN — the §31.3 no-double-speak guard walks exactly that
    // chain (focusInsideToolbar), so a housing regression (Loader/popup
    // re-hosting) reds HERE, not in a screen reader.
    freshDoc();
    setTool("brush.stroke");
    QQuickItem *slider = toolbarControl("QQuickSlider");
    QVERIFY(slider);
    bool inChain = false;
    for (QQuickItem *n = slider; n; n = n->parentItem())
        if (n == m_toolbar) {
            inChain = true;
            break;
        }
    QVERIFY2(inChain, "options slider is NOT a toolbarPane descendant");
    slider->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(slider->hasActiveFocus());
    QVERIFY2(evalJs(QStringLiteral("root.focusInsideToolbar()")).toBool(),
             "the no-double-speak guard does not see the rehoused control");
    focusCanvas();
    QVERIFY(!evalJs(QStringLiteral("root.focusInsideToolbar()")).toBool());
}

void TstOptionsBar::brushEraserWandRehouseParity()
{
    // T6: the three covered tools rehouse BEHAVIOR-IDENTICAL — ranges,
    // both binding directions, control identity.
    freshDoc();

    // Brush: slider 1..200 step 1 + the color swatch.
    setTool("brush.stroke");
    QQuickItem *slider = toolbarControl("QQuickSlider");
    QVERIFY(slider);
    QCOMPARE(slider->property("from").toDouble(), 1.0);
    QCOMPARE(slider->property("to").toDouble(), 200.0);
    QCOMPARE(slider->property("stepSize").toDouble(), 1.0);
    m_canvas->setProperty("brushSize", 57.0);
    QCOMPARE(slider->property("value").toDouble(), 57.0); // state -> control
    slider->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(slider->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Right);
    QCOMPARE(m_canvas->property("brushSize").toDouble(), 58.0); // control -> state
    QVERIFY2(toolbarControl("QQuickToolButton"), "brush swatch missing");

    // Eraser: same slider row, NO swatch, NO hardness (item 4: the
    // contract is un-extended — the row must not leak here).
    setTool("eraser.stroke");
    QVERIFY(toolbarControl("QQuickSlider"));
    QVERIFY2(!toolbarControl("QQuickToolButton"), "eraser must not show the swatch");
    QCOMPARE(toolbarControl("QQuickSlider")->property("value").toDouble(),
             58.0); // one shared brushSize state, as before 11d
    QVERIFY2(!visibleLabelTexts(m_toolbar).contains(
                 catTr("ui.options.brush_hardness")),
             "hardness leaked onto the eraser bar");

    // Wand: spinbox 0..255 + contiguous checkbox. Ordering note (the
    // QQC2 interaction-breaks-binding fact, pre-11d parity): state ->
    // control asserts run BEFORE interactive input on a delegate; a
    // tool-switch recreates delegates with fresh bindings.
    setTool("select.wand");
    QQuickItem *spin = toolbarControl("QQuickSpinBox");
    QVERIFY(spin);
    QCOMPARE(spin->property("from").toInt(), 0);
    QCOMPARE(spin->property("to").toInt(), 255);
    m_canvas->setProperty("wandTolerance", 99.0);
    QCOMPARE(spin->property("value").toInt(), 99); // state -> control
    spin->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(spin->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Up);
    QCOMPARE(m_canvas->property("wandTolerance").toDouble(), 100.0);
    QQuickItem *contig = toolbarControl("QQuickCheckBox");
    QVERIFY(contig);
    QCOMPARE(contig->property("checked").toBool(), true); // contract default
    clickItem(contig);
    QCOMPARE(m_canvas->property("wandContiguous").toBool(), false);
    // state -> control on a FRESH delegate (the click above consumed
    // this one's binding — the same pre-11d construction).
    m_canvas->setProperty("wandContiguous", true);
    setTool("move");
    setTool("select.wand");
    contig = toolbarControl("QQuickCheckBox");
    QVERIFY(contig);
    QCOMPARE(contig->property("checked").toBool(), true);

    // Restore the launch default for later frames.
    m_canvas->setProperty("brushSize", 16.0);
    m_canvas->setProperty("wandTolerance", 32.0);
}

void TstOptionsBar::sampleAllRowIsCloneOnly()
{
    freshDoc();
    // Pick the row's OWN checkbox by its rendered text — the wand bar
    // holds two checkboxes and class-order is a layout accident.
    const auto sampleAllBox = [this]() -> QQuickItem * {
        const QString want = catTr("ui.options.sample_all_layers");
        for (QQuickItem *it : visibleByClass(m_toolbar, "QQuickCheckBox"))
            if (it->property("text").toString() == want)
                return it;
        return nullptr;
    };

    // 1.0 final D-8: the wand's bar has NO sample-all box (it always
    // samples the composite).
    setTool("select.wand");
    QVERIFY2(!sampleAllBox(), "wand bar still offers the removed sample-all choice");

    // 1.0 final S-08 (O-SIT RC5 sitting): heal's bar has NO sample-all box
    // either -- it changed nothing (heal samples the active layer).
    setTool("heal.stroke");
    QVERIFY2(!sampleAllBox(), "heal bar still offers the removed sample-all choice");

    // Clone's box is the one that remains, and it drives the canvas state
    // both ways.
    m_canvas->setProperty("sampleAllLayers", false);
    setTool("clone.stroke");
    QQuickItem *box = sampleAllBox();
    QVERIFY2(box, "clone bar lacks the sample-all checkbox");
    QCOMPARE(box->property("checked").toBool(), false); // engine default
    clickItem(box);
    QCOMPARE(m_canvas->property("sampleAllLayers").toBool(), true);
    m_canvas->setProperty("sampleAllLayers", false);
    QTRY_COMPARE(box->property("checked").toBool(), false);
}

void TstOptionsBar::brushSizeAnnouncementGuardPreserved()
{
    // T6, the review-D3 class: KEYBOARD-FOCUSED option changes stay
    // suppressed (the platform speaks the focused control); changes with
    // canvas focus announce through the live region — pre-11d parity.
    freshDoc();
    setTool("brush.stroke");
    QQuickItem *slider = toolbarControl("QQuickSlider");
    QVERIFY(slider);
    slider->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(slider->hasActiveFocus());
    QTest::qWait(600); // drain any pending rate-limited window
    const int before = announcementsFrom("brush");
    QTest::keyClick(m_window, Qt::Key_Right);
    QTest::qWait(700); // rule-7 bound: an emission would have landed
    QCOMPARE(announcementsFrom("brush"), before); // suppressed
    focusCanvas();
    m_canvas->setProperty("brushSize", 42.0);
    QTRY_VERIFY_WITH_TIMEOUT(announcementsFrom("brush") == before + 1, 1500);
    m_canvas->setProperty("brushSize", 16.0);
}

void TstOptionsBar::hardnessRowShapeAndBinding()
{
    // Item 4: the hardness row lands in the brush list with the CONTRACT
    // range as the control's bounds (0..=1, step 0.01 — the rigidity
    // slider idiom: QQC2 Slider is double-native, no integer scaling;
    // labeled caption + Accessible.name via the shared delegate, value
    // announced as the control's own accessible value per the recorded
    // O-I18N survey).
    freshDoc();
    setTool("brush.stroke");
    QCOMPARE(visibleByClass(m_toolbar, "QQuickSlider").size(), 2);
    QQuickItem *hard = hardnessSlider();
    QVERIFY2(hard, "no single slider with the 0..1 contract bounds");
    QCOMPARE(hard->property("from").toDouble(), 0.0);
    QCOMPARE(hard->property("to").toDouble(), 1.0);
    QCOMPARE(hard->property("stepSize").toDouble(), 0.01);

    // Default 0.0 = the built-in preset's own hardness; fixed2 readout.
    QCOMPARE(m_canvas->property("brushHardness").toDouble(), 0.0);
    QVERIFY(visibleLabelTexts(m_toolbar).contains(QStringLiteral("0.00")));

    m_canvas->setProperty("brushHardness", 0.85);
    QCOMPARE(hard->property("value").toDouble(), 0.85); // state -> control
    QVERIFY(visibleLabelTexts(m_toolbar).contains(QStringLiteral("0.85")));
    hard->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(hard->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Right);
    QCOMPARE(m_canvas->property("brushHardness").toDouble(),
             0.86); // control -> state

    // The stroke-family clamp (the brushSize posture): a stray
    // programmatic write cannot arm an out-of-range envelope param.
    m_canvas->setProperty("brushHardness", 1.7);
    QCOMPARE(m_canvas->property("brushHardness").toDouble(), 1.0);
    m_canvas->setProperty("brushHardness", -0.3);
    QCOMPARE(m_canvas->property("brushHardness").toDouble(), 0.0);
}

void TstOptionsBar::hardnessRidesTheStrokeEnvelopeEngineProven()
{
    // The §7.8 autosave wire spy (the fontSize pattern): with a saved
    // package the engine appends every APPLIED envelope, so the
    // engine-written bytes prove what rode. The SET hardness must ride
    // the brush envelope at value; the eraser envelope must not carry
    // the key at all (un-extended contract, additionalProperties:false
    // would red the whole stroke at stage 3).
    freshDoc();
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = tmp.path() + QStringLiteral("/h.appname");
    QVERIFY(m_controller->saveDocument(pkg)); // binds the autosave log

    setTool("brush.stroke");
    m_canvas->setProperty("brushHardness", 0.85);
    focusCanvas();
    const int before = journalKinds().size();
    dragStroke(60, 60, 200, 80);
    QTRY_COMPARE(journalKinds().size(), before + 1);
    QCOMPARE(journalKinds().last(), QStringLiteral("brush.stroke"));

    setTool("eraser.stroke");
    focusCanvas();
    dragStroke(60, 120, 200, 140);
    QTRY_COMPARE(journalKinds().size(), before + 2);
    QCOMPARE(journalKinds().last(), QStringLiteral("eraser.stroke"));

    QFile log(pkg + QStringLiteral(".autosave"));
    QVERIFY2(log.exists(), "autosave log was not bound by the save");
    QVERIFY(log.open(QIODevice::ReadOnly));
    const QByteArray bytes = log.readAll();
    QCOMPARE(bytes.count("\"kind\":\"brush.stroke\""), 1);
    QCOMPARE(bytes.count("\"kind\":\"eraser.stroke\""), 1);
    QVERIFY2(bytes.contains("\"hardness\":0.85"),
             "the set hardness did not ride the brush envelope at value");
    QCOMPARE(bytes.count("\"hardness\":"), 1); // the eraser carried NONE

    m_canvas->setProperty("brushHardness", 0.0); // restore launch default
}

void TstOptionsBar::hardnessEngineSilentUntilStrokeAndUnannounced()
{
    // D4 honest state, both directions: the row is options-bar STATE —
    // a value change is engine-silent until a stroke rides it
    // (submissionAttempts is the recorded entry counter of both shell
    // submission doors), and it invents NO announcement: the recorded
    // O-I18N survey resolved hardness announcements to the control's
    // own accessible value (the opacity/flow/rigidity precedent;
    // a11y.tool.brush_size's golden is the canvas-chord path). One-id
    // follow-up only if 11e's {/} chords prove silent — nothing is
    // wired ahead of that.
    freshDoc();
    setTool("brush.stroke");
    const int subs = m_controller->submissionAttempts();
    const int hist = journalKinds().size();
    QTest::qWait(600); // drain any pending rate-limited window
    // SOURCE-SCOPED counting (the D3 pattern): ambient sources (tool/
    // zoom/status) land on the rate-limiter's own clock, so a total-log
    // compare races cross-test spillover (probed: solo-green,
    // full-run +1 from the preceding cleanup's tool change).
    const int annBrush = announcementsFrom("brush");
    const int annAll = m_announcer->announcementLog().size();

    focusCanvas();
    m_canvas->setProperty("brushHardness", 0.42); // canvas-focused change
    QQuickItem *hard = hardnessSlider();
    QVERIFY(hard);
    hard->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(hard->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Right); // focused-control change
    QTest::qWait(700); // rule-7 bound: an emission would have landed

    QCOMPARE(m_controller->submissionAttempts(), subs); // engine-silent
    QCOMPARE(journalKinds().size(), hist);
    // No repurpose: the brush_size source must not carry hardness.
    QCOMPARE(announcementsFrom("brush"), annBrush);
    // No invention under ANY source: nothing new may speak hardness.
    const QVariantList log = m_announcer->announcementLog();
    for (int i = annAll; i < log.size(); ++i) {
        const QVariantMap row = log.at(i).toMap();
        const QString sid = row.value(QStringLiteral("source")).toString()
            + QLatin1Char('|') + row.value(QStringLiteral("id")).toString();
        QVERIFY2(!sid.contains(QLatin1String("hardness"), Qt::CaseInsensitive),
                 qPrintable(QStringLiteral("invented announcement: ") + sid));
    }

    focusCanvas();
    m_canvas->setProperty("brushHardness", 0.0);
}

void TstOptionsBar::hardnessExtremesEnginePixelTruth()
{
    // Schema extremes (min 0 / max 1), strokes otherwise identical, and
    // the assertion direction DERIVED from the engine's own math
    // (raster.rs: t = 1 - smoothstep(hardness, 1, r); hardness == 1 is
    // the hard-disk limit with ~1 px analytic AA): the soft stroke's
    // edge transect holds strictly more intermediate pixels than the
    // hard stroke's. Seed is inert for coverage geometry here (the
    // built-in preset has no scatter; its dynamics curves are linear in
    // pressure, and mouse pressure is constant 1), so hardness is the
    // only live variable. NOTE the hard arm is itself the override
    // proof: preset hardness is 0.0, so a dropped param would leave
    // BOTH arms soft and collapse the margin below.
    const auto transectIntermediates = [this]() {
        // ENGINE readback truth (not a window grab): vertical transect
        // from the stroke centerline (y=100) down past the edge
        // (radius 60 at sizePx 120, pressure 1) at a mid-stroke
        // column. Composite is near-black paint (gray 20) over white
        // (255): intermediate = strictly inside with ~5% margins.
        QImage img;
        if (!m_controller->readRegionImage(QRect(160, 100, 1, 100), &img))
            return -1;
        int n = 0;
        for (int y = 0; y < img.height(); ++y) {
            const int v = qGray(img.pixel(0, y));
            if (v > 32 && v < 243)
                ++n;
        }
        return n;
    };
    const auto paintAt = [this](double hardness) {
        freshDoc();
        setTool("brush.stroke");
        m_canvas->setProperty("brushSize", 120.0);
        m_canvas->setProperty("brushHardness", hardness);
        focusCanvas();
        const int before = journalKinds().size();
        dragStroke(80, 100, 240, 100);
        QTRY_COMPARE(journalKinds().size(), before + 1);
        QCOMPARE(journalKinds().last(), QStringLiteral("brush.stroke"));
    };

    paintAt(1.0); // schema max: the hard-disk limit
    if (QTest::currentTestFailed())
        return;
    const int hardCount = transectIntermediates();
    paintAt(0.0); // schema min: softest falloff
    if (QTest::currentTestFailed())
        return;
    const int softCount = transectIntermediates();
    QVERIFY2(hardCount >= 0 && softCount >= 0, "engine readback refused");

    // The discriminating relation, direction per the schema semantics;
    // the +8 margin is structural (a falloff band spanning tens of px
    // vs ~1 px AA), kept conservative against resampling wobble.
    QVERIFY2(softCount > hardCount,
             qPrintable(QStringLiteral("soft=%1 hard=%2 — direction wrong")
                            .arg(softCount).arg(hardCount)));
    QVERIFY2(softCount >= hardCount + 8,
             qPrintable(QStringLiteral("soft=%1 hard=%2 — margin collapsed")
                            .arg(softCount).arg(hardCount)));

    m_canvas->setProperty("brushSize", 16.0);
    m_canvas->setProperty("brushHardness", 0.0);
}

void TstOptionsBar::penCommitAsSegmentJournalKinds()
{
    // Binding truth by ENGINE JOURNAL KIND: the segment flips the
    // EXISTING penCommitsShape state, and the committed door follows —
    // vector.shape_add (shape arm) vs vector.path_new (path arm).
    freshDoc();
    setTool("vector.pen");
    QQuickItem *shapeBtn = toolbarButtonByText("ui.options.commit_shape");
    QQuickItem *pathBtn = toolbarButtonByText("ui.options.commit_path");
    QVERIFY(shapeBtn);
    QVERIFY(pathBtn);
    // Default state: shape arm checked, path unchecked (bound visuals).
    QCOMPARE(m_canvas->property("penCommitsShape").toBool(), true);
    QCOMPARE(shapeBtn->property("checked").toBool(), true);
    QCOMPARE(pathBtn->property("checked").toBool(), false);

    // Path arm: click the segment (control -> state), draw, commit.
    clickItem(pathBtn);
    QCOMPARE(m_canvas->property("penCommitsShape").toBool(), false);
    QCOMPARE(pathBtn->property("checked").toBool(), true);
    QCOMPARE(shapeBtn->property("checked").toBool(), false);
    focusCanvas();
    clickDoc(40, 40);
    clickDoc(120, 60);
    clickDoc(90, 120);
    QTest::keyClick(m_window, Qt::Key_Return); // commits the open path
    QTRY_VERIFY(journalKinds().contains(QStringLiteral("vector.path_new")));
    QVERIFY2(!journalKinds().contains(QStringLiteral("vector.shape_add")),
             "path arm must not land a shape layer");

    // Shape arm: state -> control direction first, then the door truth.
    m_canvas->setProperty("penCommitsShape", true);
    QCOMPARE(shapeBtn->property("checked").toBool(), true);
    QCOMPARE(pathBtn->property("checked").toBool(), false);
    focusCanvas();
    clickDoc(150, 40);
    clickDoc(230, 60);
    clickDoc(200, 120);
    QTest::keyClick(m_window, Qt::Key_Return);
    QTRY_VERIFY(journalKinds().contains(QStringLiteral("vector.shape_add")));
}

void TstOptionsBar::fontSizeRidesStyleSizeEngineProven()
{
    // The WIRE spy is the §7.8 autosave log: with a saved package the
    // engine appends every APPLIED envelope to <pkg>.autosave — the
    // engine-written bytes carry (or omit) style.size exactly as the
    // interactive submission built it. (The out-of-range refusal spy is
    // NOT usable here — probed first-hand: stage 5 does NOT enforce the
    // nested styledRunStyle.size bound and empty text never layouts, so
    // the engine ACCEPTS an out-of-range size at add; recorded as an
    // engine-side finding for the desk, outside this lane's fence.)
    freshDoc();
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = tmp.path() + QStringLiteral("/t.appname");
    QVERIFY(m_controller->saveDocument(pkg)); // binds the autosave log

    setTool("text");
    QQuickItem *spin = toolbarControl("QQuickSpinBox");
    QVERIFY(spin);
    // x100 presentation scale (the FilterParamDialog idiom); default
    // displays the engine's own default run size. state -> control
    // FIRST (interaction consumes the delegate's value binding — the
    // QQC2 fact), then control -> state by real key.
    QCOMPARE(spin->property("value").toInt(), 1200);
    QCOMPARE(spin->property("displayText").toString(),
             QStringLiteral("12.00"));
    m_canvas->setProperty("textFontSize", 300.0); // state -> control
    QCOMPARE(spin->property("displayText").toString(),
             QStringLiteral("300.00"));
    spin->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(spin->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Up); // +1 pt (step 1 * scale)
    QCOMPARE(m_canvas->property("textFontSize").toDouble(), 301.0);

    // RC3 D-6 (operator WORD 35, sitting S-15): the Text tool REFUSES on
    // use for 1.0 -- a click at any size submits NOTHING (no text.add, no
    // journal row, nothing in the autosave log) and the typed tool-pending
    // refusal is surfaced on the status bar. The option row above stays
    // (nothing else changes for Text); its wire proof returns with the
    // text-editing slice.
    focusCanvas();
    const int before = journalKinds().size();
    clickDoc(60, 60);
    m_canvas->setProperty("textFontSize", 12.0);
    focusCanvas();
    clickDoc(120, 120);
    QTest::qWait(200);
    QCOMPARE(journalKinds().size(), before);
    QVERIFY2(m_controller->statusText() == QStringLiteral("Text is not wired to the canvas yet"),
             qPrintable(QStringLiteral("the Text refusal was not surfaced; "
                                       "status: %1")
                            .arg(m_controller->statusText())));

    QFile log(pkg + QStringLiteral(".autosave"));
    QVERIFY2(log.exists(), "autosave log was not bound by the save");
    QVERIFY(log.open(QIODevice::ReadOnly));
    QCOMPARE(log.readAll().count("\"kind\":\"text.add\""), 0);
}

void TstOptionsBar::puppetKnobsRideTheEnvelopeEngineProven()
{
    // Two engine-truth spies for the §45.5 knobs: (a) the engine
    // validates expansionPx 0..=32 and rigidity 0..=1 BEFORE the
    // pins/identity arms (kind_impls.rs), so an ILLEGAL knob refuses
    // typed even on a no-rotation commit — only possible if the
    // interactive commit carried the bound state; (b) the §7.8 autosave
    // log's engine-written envelope bytes carry (or omit at the
    // contract defaults) the exact values. A no-rotation commit is the
    // wire-identity arm and STILL JOURNALS (history: always — the §40.1
    // "empty push" is empty deltas, not no-journal; CLI-probed
    // first-hand).
    freshDoc();
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QString pkg = tmp.path() + QStringLiteral("/p.appname");
    QVERIFY(m_controller->saveDocument(pkg)); // binds the autosave log
    setTool("transform.puppet");
    QQuickItem *spin = toolbarControl("QQuickSpinBox");
    QQuickItem *slider = toolbarControl("QQuickSlider");
    QVERIFY(spin);
    QVERIFY(slider);
    QCOMPARE(spin->property("from").toInt(), 0);
    QCOMPARE(spin->property("to").toInt(), 32);
    QCOMPARE(spin->property("value").toInt(), 2); // contract default
    QCOMPARE(slider->property("value").toDouble(), 0.7);
    // Both directions on the knob bindings — state -> control BEFORE
    // interactive input (the QQC2 interaction-breaks-binding fact).
    m_canvas->setProperty("puppetExpansion", 8);
    QCOMPARE(spin->property("value").toInt(), 8);
    m_canvas->setProperty("puppetRigidity", 0.3);
    QCOMPARE(slider->property("value").toDouble(), 0.3);
    spin->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(spin->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Up);
    QCOMPARE(m_canvas->property("puppetExpansion").toInt(), 9);
    slider->forceActiveFocus(Qt::TabFocusReason);
    QTRY_VERIFY(slider->hasActiveFocus());
    QTest::keyClick(m_window, Qt::Key_Right);
    QCOMPARE(m_canvas->property("puppetRigidity").toDouble(), 0.31);

    // Wire leg A: clean non-default knobs ride the journaled envelope.
    m_canvas->setProperty("puppetExpansion", 8);
    m_canvas->setProperty("puppetRigidity", 0.35);
    focusCanvas();
    clickDoc(80, 80);
    clickDoc(160, 120);
    int before = journalKinds().size();
    QTest::keyClick(m_window, Qt::Key_Return); // commitPuppet
    QTRY_COMPARE(journalKinds().size(), before + 1);
    QCOMPARE(journalKinds().last(), QStringLiteral("transform.puppet"));

    // Illegal expansion refuses typed at commit — the param RODE.
    m_canvas->setProperty("puppetExpansion", 99);
    setTool("transform.puppet"); // re-arm (the committed session closed)
    focusCanvas();
    clickDoc(80, 80);
    clickDoc(160, 120);
    QTest::keyClick(m_window, Qt::Key_Return);
    QString status = m_controller->property("statusText").toString();
    QVERIFY2(status.contains(QStringLiteral("out_of_range"))
                 && status.contains(QStringLiteral("expansionPx")),
             qPrintable(QStringLiteral("expansionPx did not ride: ") + status));

    // Illegal rigidity refuses typed at commit.
    m_canvas->setProperty("puppetExpansion", 2);
    m_canvas->setProperty("puppetRigidity", 2.0);
    setTool("transform.puppet"); // fresh session (the refused one closed)
    focusCanvas();
    clickDoc(80, 80);
    clickDoc(160, 120);
    QTest::keyClick(m_window, Qt::Key_Return);
    status = m_controller->property("statusText").toString();
    QVERIFY2(status.contains(QStringLiteral("out_of_range"))
                 && status.contains(QStringLiteral("rigidity")),
             qPrintable(QStringLiteral("rigidity did not ride: ") + status));

    // Wire leg B: contract defaults commit clean AND the envelope OMITS
    // both knobs (the default->omit sentinel rule) — the identity arm
    // still journals (history: always).
    m_canvas->setProperty("puppetRigidity", 0.7);
    setTool("transform.puppet");
    focusCanvas();
    clickDoc(80, 80);
    clickDoc(160, 120);
    before = journalKinds().size();
    QTest::keyClick(m_window, Qt::Key_Return);
    QTRY_COMPARE(journalKinds().size(), before + 1);
    QCOMPARE(journalKinds().last(), QStringLiteral("transform.puppet"));
    const QString clean = m_controller->property("statusText").toString();
    QVERIFY2(!clean.contains(QStringLiteral("out_of_range")),
             qPrintable(QStringLiteral("default knobs refused: ") + clean));

    QFile log(pkg + QStringLiteral(".autosave"));
    QVERIFY2(log.exists(), "autosave log was not bound by the save");
    QVERIFY(log.open(QIODevice::ReadOnly));
    const QByteArray bytes = log.readAll();
    QCOMPARE(bytes.count("\"kind\":\"transform.puppet\""), 2);
    QVERIFY2(bytes.contains("\"expansionPx\":8"),
             "expansion 8 did not ride the journaled envelope");
    QVERIFY2(bytes.contains("\"rigidity\":0.35"),
             "rigidity 0.35 did not ride the journaled envelope");
    QCOMPARE(bytes.count("\"expansionPx\""), 1); // default omitted
    QCOMPARE(bytes.count("\"rigidity\""), 1);    // default omitted
}

void TstOptionsBar::showMeshIsARealOverlay()
{
    // The D2 ruling demands a REAL overlay, never a dead toggle: with a
    // live 2-pin session the toggle must CHANGE canvas pixels (the pin
    // adjacency graph) and restore them off — byte-compared on the
    // canvas region so toolbar focus rings cannot mask the diff.
    freshDoc();
    setTool("transform.puppet");
    focusCanvas();
    clickDoc(60, 60);
    clickDoc(200, 140);
    QTRY_COMPARE(m_canvas->property("puppetShowMesh").toBool(), false);
    QCoreApplication::processEvents();
    const QImage off1 = canvasRegionGrab();
    QQuickItem *toggle = toolbarButtonByText("ui.options.puppet_show_mesh");
    QVERIFY(toggle);
    clickItem(toggle);
    QCOMPARE(m_canvas->property("puppetShowMesh").toBool(), true);
    QCoreApplication::processEvents();
    const QImage on = canvasRegionGrab();
    QVERIFY2(off1 != on, "show_mesh ON changed no canvas pixel — dead toggle");
    clickItem(toggle);
    QCOMPARE(m_canvas->property("puppetShowMesh").toBool(), false);
    QCoreApplication::processEvents();
    const QImage off2 = canvasRegionGrab();
    QVERIFY2(off1 == off2, "show_mesh OFF did not restore the frame");
    QTest::keyClick(m_window, Qt::Key_Escape); // drop the pin session
}

// ---------------------------------------------------------------------
// Pro-Wave row 4 — shape stroke options
// ---------------------------------------------------------------------

namespace {

// A §22 solid FillContent over the §7.3 `color` object (model rgb ⇒
// alpha 1.0 whatever the 4th comp carries — appname-text door.rs
// page_fill_of, the fill-math seam this composite leg is routed to).
QVariantMap solidFill(double r, double g, double b)
{
    QVariantMap color;
    color.insert(QStringLiteral("model"), QStringLiteral("rgb"));
    color.insert(QStringLiteral("comps"), QVariantList{r, g, b, 1.0});
    QVariantMap fill;
    fill.insert(QStringLiteral("type"), QStringLiteral("solid"));
    fill.insert(QStringLiteral("color"), color);
    return fill;
}

QVariantMap rectPrimitive(double x, double y, double w, double h)
{
    QVariantMap geometry;
    geometry.insert(QStringLiteral("x"), x);
    geometry.insert(QStringLiteral("y"), y);
    geometry.insert(QStringLiteral("w"), w);
    geometry.insert(QStringLiteral("h"), h);
    QVariantMap primitive;
    primitive.insert(QStringLiteral("kind"), QStringLiteral("rect"));
    primitive.insert(QStringLiteral("geometry"), geometry);
    return primitive;
}

// INK, ASSERTED ON BOTH CHANNELS — and the reason it has to be.
// `qGray()` alone provably CANNOT tell opaque black from a fully
// TRANSPARENT pixel. Measured on this platform (Qt 6.11.1, ARGB32):
//     qGray(0,0,0,a0)  = 0     qGray(0,0,0,a255)  = 0    <-- EQUAL
//     qAlpha(0,0,0,a0) = 0     qAlpha(0,0,0,a255) = 255
// so a bare `qGray(...) < 32` passes IDENTICALLY on a band that arrived
// transparent. That is not a hypothetical regression class: the hazard
// this whole row is built on was a composite leg mishandling this exact
// shape path, and a leg that returned transparency instead of ink would
// sail straight through a gray-only predicate while every assertion
// stayed green. Each ink probe below therefore asserts alpha as well.
//
// The alpha assertion is itself only worth something on a format that
// HAS an alpha channel — on Format_RGB32 qAlpha is a constant 255 and
// the pin would be vacuous in its own right. Every image is checked
// with hasAlphaChannel() before it is probed. Measured today: the
// composite readback arrives Format_RGBA8888 and the exported PNG
// reloads as Format_ARGB32; both carry alpha.
bool isOpaqueInk(QRgb p)
{
    return qAlpha(p) == 255 && qGray(p) < 32;
}

// BACKGROUND, ASSERTED ON BOTH CHANNELS — the exact complement of the
// hazard above, and it went unclosed for five rounds while the ink half
// was fixed. `qGray()` alone provably cannot tell OPAQUE WHITE from a
// FULLY TRANSPARENT pixel either. Measured on this platform (Qt 6.11.1;
// the composite readback arrives Format_RGBA8888 and the reloaded PNG
// Format_ARGB32, both NON-premultiplied, so RGB survives alpha 0):
//     qGray(255,255,255,a0)  = 255   qGray(255,255,255,a255)  = 255  <-- EQUAL
//     qAlpha(255,255,255,a0) = 0     qAlpha(255,255,255,a255) = 255
// A composite leg that dropped the page out from under the shape, or an
// export that wrote a hole where the white fill belongs, produces a
// transect that is byte-for-byte "white" to a gray-only predicate. That
// is not hypothetical here: the mutation that forced x=90 and x=110 to
// transparent white left this suite GREEN. Every background probe below
// therefore asserts alpha as well, on images already guarded by
// hasAlphaChannel() — whose own message promised these pins would
// otherwise be vacuous, which for the background half they were.
bool isOpaqueWhite(QRgb p)
{
    return qAlpha(p) == 255 && qGray(p) == 255;
}

QString pixelDesc(QRgb p)
{
    return QStringLiteral("(%1,%2,%3,a%4)")
        .arg(qRed(p))
        .arg(qGreen(p))
        .arg(qBlue(p))
        .arg(qAlpha(p));
}

} // namespace

void TstOptionsBar::strokeSurvivesCompositeAndExportDoors()
{
    // THE HAZARD, and why this test exists BEFORE any control:
    // pre-`pro/shape-stroke`, the COMMAND door accepted `widthPx > 0`
    // (on the command this row actually emits: contracts/commands/
    // vector.shape_add.yaml:55 `stroke: {$ref: #/$defs/vectorStroke}` →
    // :135 vectorStroke → :139 `widthPx: {type: number, minimum: 0}`,
    // accepted since freeze) while the COMPOSITE door refused it typed
    // (png_io.rs resolve_parametric_page's shape leg, "fill-only v1").
    // A stroke could therefore be SET successfully and then fail at
    // readback and at export. An envelope assertion cannot see that —
    // only PIXELS past the composite and past the PNG writer can. The
    // options rows below are built ON this proof; if it reds, the rows
    // must not ship.
    //
    // Geometry chosen so the stroke is the ONLY possible source of
    // ink: the shape's FILL is WHITE on a WHITE document, so every
    // non-white pixel in the transect came from the stroke band. The
    // rect's left edge sits at x = 100 and `align` defaults to CENTER
    // (vector_ops.rs's recorded election, pinned to stroke_path's
    // `position: {default: center}`), so widthPx 10 paints the band
    // x ∈ [95, 105) at any y strictly inside the rect.
    QVERIFY(m_controller->newDocument(320, 200, QStringLiteral("white")));

    const auto addShape = [this](double widthPx) {
        QVariantMap stroke;
        stroke.insert(QStringLiteral("widthPx"), widthPx);
        stroke.insert(QStringLiteral("color"), solidFill(0.0, 0.0, 0.0)
                                                   .value(QStringLiteral("color")));
        QVariantMap params;
        params.insert(QStringLiteral("primitive"), rectPrimitive(100, 60, 120, 80));
        params.insert(QStringLiteral("fill"), solidFill(1.0, 1.0, 1.0));
        params.insert(QStringLiteral("stroke"), stroke);
        params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
        return m_controller->applyCommand(QStringLiteral("vector.shape_add"), {},
                                          params, QString());
    };

    // Darkest pixel of a horizontal transect crossing the left edge.
    const auto darkestOn = [](const QImage &img) {
        int lo = 255;
        for (int x = 0; x < img.width(); ++x)
            lo = qMin(lo, qGray(img.pixel(x, 0)));
        return lo;
    };
    const QRect transect(90, 100, 26, 1); // x ∈ [90, 116), mid-height

    // --- (a) BASELINE: widthPx 0 is the contract's own shape_add
    // default and must leave the transect pure white (no stroke, and
    // the white fill is invisible). This is the "before" the arm
    // promises is byte-untouched.
    QVariantMap r = addShape(0.0);
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("shape_add widthPx 0 refused: %1")
                            .arg(r.value(QStringLiteral("message")).toString())));
    QImage zero;
    QVERIFY2(m_controller->readRegionImage(transect, &zero),
             "composite readback refused at widthPx 0");
    QCOMPARE(darkestOn(zero), 255);
    QTemporaryDir dir;
    QVERIFY(dir.isValid());
    // The EXPORT baseline, taken here so the black assertions in (c)
    // are discriminating on the STROKE PARAM and not merely on "a
    // shape layer exists": an export door that dropped the stroke
    // would produce this same all-white frame at width 10.
    const QString zeroPng = dir.filePath(QStringLiteral("row4-nostroke.png"));
    QVERIFY2(m_controller->exportDocument(zeroPng, QStringLiteral("png")),
             "export refused at widthPx 0");
    QImage zeroExport;
    QVERIFY2(zeroExport.load(zeroPng), "widthPx-0 PNG did not load");
    QVERIFY2(isOpaqueWhite(zeroExport.pixel(100, 100)),
             qPrintable(QStringLiteral("zeroExport.pixel(100, 100) is not OPAQUE white: %1")
                            .arg(pixelDesc(zeroExport.pixel(100, 100)))));
    QVERIFY2(isOpaqueWhite(zeroExport.pixel(160, 60)),
             qPrintable(QStringLiteral("zeroExport.pixel(160, 60) is not OPAQUE white: %1")
                            .arg(pixelDesc(zeroExport.pixel(160, 60)))));

    // --- (b) THE COMPOSITE DOOR. Same document, stroke width 10.
    m_controller->undo();
    r = addShape(10.0);
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("shape_add widthPx 10 refused at the "
                                       "COMMAND door: %1")
                            .arg(r.value(QStringLiteral("message")).toString())));
    QImage composited;
    QVERIFY2(m_controller->readRegionImage(transect, &composited),
             "COMPOSITE DOOR REFUSED a widthPx > 0 shape — the hazard is OPEN; "
             "no stroke control may ship on this door");
    QCOMPARE(composited.size(), QSize(26, 1));
    QVERIFY2(darkestOn(composited) < 32,
             qPrintable(QStringLiteral("composite carries no stroke ink "
                                       "(darkest=%1 of 255)")
                            .arg(darkestOn(composited))));
    // The band is POSITIONED, not merely present: outside x ∈ [95, 105)
    // the transect is still white, inside it is black. (x=90..94 are
    // outside the shape entirely; x=106..115 are white FILL.)
    QVERIFY2(composited.hasAlphaChannel(),
             "the composite readback carries no alpha channel — the opacity "
             "pins below would be vacuous");
    QVERIFY2(isOpaqueWhite(composited.pixel(0, 0)),
             qPrintable(QStringLiteral("composited.pixel(0, 0) is not OPAQUE white: %1")
                            .arg(pixelDesc(composited.pixel(0, 0)))));  // x = 90
    QVERIFY2(isOpaqueWhite(composited.pixel(4, 0)),
             qPrintable(QStringLiteral("composited.pixel(4, 0) is not OPAQUE white: %1")
                            .arg(pixelDesc(composited.pixel(4, 0)))));  // x = 94
    QVERIFY2(isOpaqueInk(composited.pixel(10, 0)), // x = 100, band centre
             qPrintable(QStringLiteral("x=100 band centre is not OPAQUE ink: %1")
                            .arg(pixelDesc(composited.pixel(10, 0)))));
    QVERIFY2(isOpaqueWhite(composited.pixel(20, 0)),
             qPrintable(QStringLiteral("composited.pixel(20, 0) is not OPAQUE white: %1")
                            .arg(pixelDesc(composited.pixel(20, 0))))); // x = 110, white fill
    QVERIFY2(isOpaqueWhite(composited.pixel(25, 0)),
             qPrintable(QStringLiteral("composited.pixel(25, 0) is not OPAQUE white: %1")
                            .arg(pixelDesc(composited.pixel(25, 0))))); // x = 115, white fill
    // The CENTRE election, pinned instead of narrated. The comment at
    // the top of this case reasons from `align` defaulting to CENTER —
    // band x ∈ [95,105) — and that reasoning is load-bearing for where
    // the probes are aimed. The five probes above are ALSO satisfied by
    // `inside` (band [100,110)), so on their own they do not pin the
    // election they cite. These two do: x=97 is ink under center and
    // white under inside; x=107 is the mirror. The measured transect is
    // crisp (255 outside, 0 inside, no intermediate AA at this
    // geometry), so a 2 px margin off each candidate edge is ample.
    QVERIFY2(isOpaqueInk(composited.pixel(7, 0)), // x = 97
             qPrintable(QStringLiteral("x=97 is not ink — the band is not the "
                                       "CENTER band [95,105): %1")
                            .arg(pixelDesc(composited.pixel(7, 0)))));
    QVERIFY2(isOpaqueWhite(composited.pixel(17, 0)),
             qPrintable(QStringLiteral("composited.pixel(17, 0) is not OPAQUE white: %1")
                            .arg(pixelDesc(composited.pixel(17, 0))))); // x = 107, outside CENTER

    // --- (c) THE EXPORT DOOR. The hazard's own second half: the same
    // stroke must survive the PNG writer, not just the readback. Read
    // the WRITTEN FILE back off disk and probe the same columns.
    const QString png = dir.filePath(QStringLiteral("row4-stroke.png"));
    QVERIFY2(m_controller->exportDocument(png, QStringLiteral("png")),
             "EXPORT DOOR REFUSED a widthPx > 0 shape — the hazard is OPEN "
             "at export; no stroke control may ship");
    QImage exported;
    QVERIFY2(exported.load(png), "exported PNG did not load");
    QCOMPARE(exported.size(), QSize(320, 200));
    QVERIFY2(exported.hasAlphaChannel(),
             "the reloaded PNG carries no alpha channel — the opacity pins "
             "below would be vacuous");
    QVERIFY2(isOpaqueWhite(exported.pixel(90, 100)),
             qPrintable(QStringLiteral("exported.pixel(90, 100) is not OPAQUE white: %1")
                            .arg(pixelDesc(exported.pixel(90, 100)))));
    QVERIFY2(isOpaqueWhite(exported.pixel(94, 100)),
             qPrintable(QStringLiteral("exported.pixel(94, 100) is not OPAQUE white: %1")
                            .arg(pixelDesc(exported.pixel(94, 100)))));
    QVERIFY2(isOpaqueInk(exported.pixel(100, 100)),
             qPrintable(QStringLiteral("the stroke reached the composite but NOT "
                                       "the exported PNG as opaque ink: %1")
                            .arg(pixelDesc(exported.pixel(100, 100)))));
    QVERIFY2(isOpaqueWhite(exported.pixel(110, 100)),
             qPrintable(QStringLiteral("exported.pixel(110, 100) is not OPAQUE white: %1")
                            .arg(pixelDesc(exported.pixel(110, 100)))));
    // Top edge too — one edge could pass by accident, two adjacent
    // edges plus their corner cannot.
    QVERIFY2(isOpaqueInk(exported.pixel(160, 60)),
             qPrintable(QStringLiteral("top edge band missing (or not opaque) in "
                                       "the export: %1")
                            .arg(pixelDesc(exported.pixel(160, 60)))));
    QVERIFY2(isOpaqueWhite(exported.pixel(160, 100)),
             qPrintable(QStringLiteral("export interior is not OPAQUE white fill: %1")
                            .arg(pixelDesc(exported.pixel(160, 100)))));

    m_controller->undo();
}


void TstOptionsBar::strokeWidthRowShapeAndBinding()
{
    // Shape: the FRACTIONAL spinbox (the font_size delegate — the wire
    // type is `number` and sub-pixel widths are real geometry, so an
    // integer control would silently coarsen the contract). The
    // fractional delegate presents x100, so `from`/`to` are the model
    // bounds x100: 0 .. 20000 for min 0 / max 200.
    freshDoc();
    setTool("shape.rect");
    QQuickItem *spin = toolbarControl("QQuickSpinBox");
    QVERIFY2(spin, "no stroke-width spinbox on shape.rect");
    QCOMPARE(spin->property("from").toInt(), 0);
    QCOMPARE(spin->property("to").toInt(), 20000);
    QCOMPARE(spin->property("editable").toBool(), true);
    QVERIFY(visibleLabelTexts(m_toolbar).contains(catTr("ui.options.stroke_width")));
    // The caption is ASSOCIATED (A11Y-3, the house rule for a captioned
    // control) — not merely adjacent. Attached properties are not
    // readable through QObject::property; the tst_palette_pins reader
    // (a QQmlExpression in the item's own context) is the house idiom.
    {
        QQmlExpression expr(qmlContext(spin), spin,
                            QStringLiteral("Accessible.name"));
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        QVERIFY2(!expr.hasError(), qPrintable(expr.error().toString()));
        QVERIFY2(!undefined, "the stroke-width spinbox has no Accessible.name");
        QCOMPARE(v.toString(), catTr("ui.options.stroke_width"));
    }

    // Binding, BOTH directions. Default first: 0.0 is the CONTRACT's
    // own shape_add default, so the row opens on "no stroke".
    QCOMPARE(m_canvas->property("shapeStrokeWidth").toDouble(), 0.0);
    QCOMPARE(spin->property("value").toInt(), 0);
    // state -> control
    m_canvas->setProperty("shapeStrokeWidth", 6.5);
    QCoreApplication::processEvents();
    QCOMPARE(spin->property("value").toInt(), 650);
    // control -> state (the delegate's own onValueModified conversion)
    spin->setProperty("value", 325);
    QMetaObject::invokeMethod(spin, "valueModified");
    QCoreApplication::processEvents();
    QCOMPARE(m_canvas->property("shapeStrokeWidth").toDouble(), 3.25);

    // The SAME row serves shape.ellipse (one descriptor, two tools) and
    // NO other tool grew a spinbox from it.
    setTool("shape.ellipse");
    QVERIFY(toolbarControl("QQuickSpinBox"));
    QCOMPARE(visibleByClass(m_toolbar, "QQuickSpinBox").size(), 1);
    setTool("move");
    QCOMPARE(visibleByClass(m_toolbar, "QQuickSpinBox").size(), 0);
    m_canvas->setProperty("shapeStrokeWidth", 0.0);
}

void TstOptionsBar::strokeWidthGestureReachesTheCompositeAndExport()
{
    // THE CENSUS CASE: a REAL shape.rect drag, with the width set
    // through the REAL control, asserted on ENGINE pixels at the
    // composite AND at the export door — not on the envelope, and not
    // on shell echo. The shape's fill is the brush colour, so the fill
    // is made WHITE and the stroke is the only ink that can differ from
    // the white document.
    freshDoc();
    setTool("shape.rect");
    m_canvas->setProperty("brushColor", QColor(Qt::white));
    QQuickItem *spin = toolbarControl("QQuickSpinBox");
    QVERIFY(spin);
    spin->setProperty("value", 1000); // presented x100 => widthPx 10
    QMetaObject::invokeMethod(spin, "valueModified");
    QCoreApplication::processEvents();
    QCOMPARE(m_canvas->property("shapeStrokeWidth").toDouble(), 10.0);

    focusCanvas();
    const int before = journalKinds().size();
    dragStroke(100, 60, 220, 140); // rect (100,60)-(220,140)
    QTRY_COMPARE(journalKinds().size(), before + 1);
    QCOMPARE(journalKinds().last(), QStringLiteral("vector.shape_add"));

    // Composite: the centred band straddles the left edge at x = 100.
    QImage strip;
    QVERIFY2(m_controller->readRegionImage(QRect(90, 100, 26, 1), &strip),
             "composite readback refused after a stroked shape gesture");
    QVERIFY2(strip.hasAlphaChannel(),
             "the composite readback carries no alpha channel — the opacity "
             "pin below would be vacuous");
    // RE-AIMED 2026-09-05, Q-D37 (a). x = 94 is one pixel outside the band
    // (widthPx 10 centred on the edge at x = 100 spans 95..105), and the
    // band's anti-aliased fringe lands there: measured (235,235,235,a255).
    // The ruling tolerates that fringe. What must still hold is that the
    // band's SOLID ink has stopped, so this asserts NOT-ink rather than
    // opaque white; the far-outside control at x = 110 below still pins
    // opaque white and is what keeps this case honest about the band's edge.
    QVERIFY2(!isOpaqueInk(strip.pixel(4, 0)),
             qPrintable(QStringLiteral("strip.pixel(4, 0) is SOLID INK one pixel "
                                       "outside the band, so the band is wider "
                                       "than the requested widthPx 10: %1")
                            .arg(pixelDesc(strip.pixel(4, 0)))));  // x = 94
    QVERIFY2(isOpaqueInk(strip.pixel(10, 0)), // x = 100, band centre
             qPrintable(QStringLiteral("the gesture's stroke width never reached "
                                       "the composite as OPAQUE ink: %1")
                            .arg(pixelDesc(strip.pixel(10, 0)))));
    QVERIFY2(isOpaqueWhite(strip.pixel(20, 0)),
             qPrintable(QStringLiteral("strip.pixel(20, 0) is not OPAQUE white: %1")
                            .arg(pixelDesc(strip.pixel(20, 0))))); // x = 110, white fill

    // Export: the same band in the written file.
    QTemporaryDir dir;
    QVERIFY(dir.isValid());
    const QString png = dir.filePath(QStringLiteral("row4-gesture.png"));
    QVERIFY2(m_controller->exportDocument(png, QStringLiteral("png")),
             "export refused after a stroked shape gesture");
    QImage out;
    QVERIFY(out.load(png));
    QVERIFY2(out.hasAlphaChannel(),
             "the reloaded PNG carries no alpha channel — the opacity pin "
             "below would be vacuous");
    // RE-AIMED 2026-09-05, Q-D37 (a) — the EXPORT door's half of the same
    // subject the composite pin above carries. It surfaced only after the
    // other two were fixed, because a QTest case stops at its first failed
    // assertion: same band, same x, same 235-grey fringe, measured through
    // the export door instead of the composite. The far-outside control at
    // x = 110 below still pins opaque white.
    QVERIFY2(!isOpaqueInk(out.pixel(94, 100)),
             qPrintable(QStringLiteral("out.pixel(94, 100) is SOLID INK one pixel "
                                       "outside the band, so the exported band is "
                                       "wider than the requested widthPx 10: %1")
                            .arg(pixelDesc(out.pixel(94, 100)))));
    QVERIFY2(isOpaqueInk(out.pixel(100, 100)),
             qPrintable(QStringLiteral("the gesture's stroke reached the composite "
                                       "but not the PNG as OPAQUE ink: %1")
                            .arg(pixelDesc(out.pixel(100, 100)))));
    QVERIFY2(isOpaqueWhite(out.pixel(110, 100)),
             qPrintable(QStringLiteral("out.pixel(110, 100) is not OPAQUE white: %1")
                            .arg(pixelDesc(out.pixel(110, 100)))));

    m_canvas->setProperty("shapeStrokeWidth", 0.0);
    m_canvas->setProperty("brushColor", QColor(Qt::black));
}

void TstOptionsBar::strokeWidthZeroIsTheNoStrokePath()
{
    // The row's OWN zero: the identical gesture at width 0 must leave
    // the composite ink-free. This is what makes the case above
    // discriminating on the ROW rather than on "a shape was drawn", and
    // it pins the engine's `width <= 0 => Ok(None)` arm through the
    // whole shell path.
    freshDoc();
    setTool("shape.rect");
    m_canvas->setProperty("brushColor", QColor(Qt::white));
    m_canvas->setProperty("shapeStrokeWidth", 0.0);
    focusCanvas();
    const int before = journalKinds().size();
    dragStroke(100, 60, 220, 140);
    QTRY_COMPARE(journalKinds().size(), before + 1);
    QImage strip;
    QVERIFY(m_controller->readRegionImage(QRect(90, 100, 26, 1), &strip));
    for (int x = 0; x < strip.width(); ++x)
        QVERIFY2(isOpaqueWhite(strip.pixel(x, 0)),
                 qPrintable(QStringLiteral("width 0 left x=%1 not OPAQUE white: "
                                           "%2 — either it painted ink, or the "
                                           "page is not there at all")
                                .arg(90 + x)
                                .arg(pixelDesc(strip.pixel(x, 0)))));
    m_canvas->setProperty("brushColor", QColor(Qt::black));
}

void TstOptionsBar::strokeWidthEngineRefusalSurfacesTyped()
{
    // ARMED-HONEST, not silent: the contract pins NO maximum on
    // `widthPx`, so the ENGINE owns the ceiling (its band plane is the
    // path's bounds INFLATED BY THE WIDTH — one wire number scales the
    // allocation without limit, so it refuses typed rather than
    // aborting). The control's own 0..200 range cannot reach it; a
    // PROGRAMMATIC value is the house wire spy here (the same posture
    // the text/puppet rows use — the state property carries values
    // verbatim, range duty is the control's). What this pins is that
    // the refusal REACHES THE USER on the existing ui.status.rejected
    // path and is never dressed up as success.
    freshDoc();
    setTool("shape.rect");
    m_canvas->setProperty("shapeStrokeWidth", 9000.0); // past the band ceiling
    focusCanvas();
    const int before = journalKinds().size();
    dragStroke(100, 60, 220, 140);
    QCoreApplication::processEvents();
    // WHICH DOOR REFUSES, recorded rather than assumed: the COMMAND
    // door ACCEPTS the width (the contract pins no range, so there is
    // nothing for stage-5 validation to reject) — the journal carries
    // the shape_add — and the COMPOSITE door is where the engine's own
    // band-plane ceiling answers. That is precisely the shape of the
    // hazard this row was built under, now behaving correctly: the
    // refusal is TYPED and VISIBLE instead of silent.
    QTRY_COMPARE(journalKinds().size(), before + 1);
    QCOMPARE(journalKinds().last(), QStringLiteral("vector.shape_add"));
    QImage strip;
    const bool read = m_controller->readRegionImage(QRect(90, 100, 26, 1), &strip);
    const QString status = m_controller->statusText();
    QVERIFY2(!read, "an unservable stroke width composited anyway");
    QVERIFY2(status.startsWith(QStringLiteral("Rejected")),
             qPrintable(QStringLiteral("refusal did not reach the status "
                                       "line; statusText=%1").arg(status)));
    QVERIFY2(status.contains(QStringLiteral("error.")),
             qPrintable(QStringLiteral("status carries no TYPED code: %1")
                            .arg(status)));
    QVERIFY2(status.contains(QStringLiteral("widthPx")),
             qPrintable(QStringLiteral("the refusal does not name the "
                                       "offending field: %1").arg(status)));
    qInfo() << "typed refusal on the status line:" << status;
    m_canvas->setProperty("shapeStrokeWidth", 0.0);
    // Undo the unservable layer: a document that cannot composite would
    // poison every later case in this binary.
    m_controller->undo();
}

void TstOptionsBar::strokeKnobsWithoutCatalogedIdsAreHeld()
{
    // THE HOLD PIN (the row-7 pattern), in TWO lists — because "held"
    // is two different dispositions and a doorbell that conflates them
    // instructs the next builder to ship a dead control.
    //
    // RULE 8, AND THE MISS THIS TEST WAS WRITTEN ON. The first version
    // of this comment derived "the engine serves it" from
    // ShapeStrokeStyle::from_wire. from_wire is the PARSER. A parser
    // that accepts a param is not the engine serving it, and the two
    // answers differ for exactly one field. Every line below is now
    // derived AT THE RENDERER (crates/appname-vector/src/stroke.rs
    // shape_stroke_coverage, reached through op_bridge.rs), by DRIVING
    // the field through vector.shape_add and reading the composite back
    // on pixels.
    //
    // LIST 1 — SERVED AT THE RENDERER, held ONLY on the missing label.
    // Measured on this row's own geometry (doc 320x200, rect
    // 100,60,120,80, white fill on white so the band is the only ink):
    //   align   (width 16, left edge x=100; probes x88/x96/x104/x112)
    //       center  -> 255/  0/  0/255   band [92,108)
    //       inside  -> 255/255/  0/  0   band [100,116)
    //       outside ->   0/  0/255/255   band [84,100)
    //       absent  == center  (the engine's own recorded election)
    //       THREE DISTINCT BANDS.
    //   join + miterLimit (width 40, corner (100,60); P1=(88,48) sits
    //       outside the bevel chord and inside the round disc,
    //       P2=(83,43) outside both and inside the miter quad)
    //       miter/limit 4 -> P1 0   P2 0      absent == miter/limit 4
    //       round         -> P1 0   P2 255
    //       bevel         -> P1 255 P2 255
    //       miter/limit 1 -> P1 255 P2 255    (ratio 1.414 > 1 => the
    //                        §20.2 rule-2 bevel fallback)
    //       join's three arms are distinct AND miterLimit moves the
    //       raster on its own at a fixed join.
    //   stroke PAINT — `color` black -> (0,0,0,a255); `color` red ->
    //       (255,0,0,a255); `fill` {solid,red} -> (255,0,0,a255); both
    //       present (color black + fill red) -> (255,0,0,a255), i.e.
    //       `fill` WINS. All three arms move the pixel.
    // These are genuine shipping candidates the day their ids land.
    const char *heldOnLabelOnly[] = {
        "ui.options.stroke_color", "ui.options.stroke_join",
        "ui.options.stroke_align", "ui.options.stroke_miter",
        "ui.join.miter",           "ui.join.round",
        "ui.join.bevel",           "ui.align.inside",
        "ui.align.center",         "ui.align.outside",
    };
    for (const char *id : heldOnLabelOnly)
        QVERIFY2(catTr(id) == QLatin1String(id),
                 qPrintable(QStringLiteral("%1 IS NOW CATALOGED — this knob is "
                                           "SERVED AT THE RENDERER on this row's "
                                           "own tools (driven and measured), so "
                                           "the surface is unblocked; ship the "
                                           "control and drop the id from this "
                                           "list").arg(QLatin1String(id))));

    // LIST 2 — `cap`. NOT a labelling hold. INERT on everything this
    // row ships, and it would still be inert the day its ids are minted.
    //   stroke.rs polyline_pieces emits cap geometry only inside
    //   `if !poly.closed`, i.e. on OPEN ends only. Every `primitive`
    //   kind vector.shape_add can build comes back CLOSED: primitives.rs
    //   wraps rect / rounded_rect / ellipse / polygon / line in its own
    //   `close()` helper (`closed: true`), and `custom` refuses typed
    //   (NotImplemented, op_bridge.rs). shape.rect and shape.ellipse —
    //   the only two tools carrying this row — are therefore both closed.
    //   MEASURED: width 24, a 120x120 readback covering the whole
    //   top-left corner of a shape.rect,
    //       butt == round   -> TRUE      butt == square -> TRUE
    //   BYTE-IDENTICAL RASTERS. A cap control here could not move one
    //   pixel, which is precisely the D4 lie ("a control that does
    //   nothing is a lie") this row invokes to keep vector.pen out.
    //
    //   SCOPE OF THE CONDITION, so it is not over-stated: `cap` is NOT
    //   dead in the engine — it is dead on CLOSED subpaths. Driven
    //   through this SAME command's `path` form with closed:false (an
    //   open segment (100,100)->(220,100) at width 24) it moves pixels:
    //       probe             butt   round  square
    //       (94,100)          255      0      0
    //       (90,108)          255    255      0
    //   three distinct rasters. So `cap` becomes a live control the day
    //   a tool ships that emits an OPEN subpath (vector.pen's path mode,
    //   or any shape_add `path` payload with closed:false) — and not
    //   before. It belongs to that row, not to this one.
    const char *heldInertOnClosedShapes[] = {
        "ui.options.stroke_cap", "ui.cap.butt",
        "ui.cap.round",          "ui.cap.square",
    };
    for (const char *id : heldInertOnClosedShapes)
        QVERIFY2(catTr(id) == QLatin1String(id),
                 qPrintable(QStringLiteral("%1 IS NOW CATALOGED — do NOT read "
                                           "that as unblocking a cap control on "
                                           "shape.rect/shape.ellipse. `cap` is "
                                           "INERT on closed subpaths (stroke.rs "
                                           "polyline_pieces caps only "
                                           "`if !poly.closed`) and every "
                                           "shape_add primitive kind is closed; "
                                           "butt/round/square rasterise "
                                           "identically there. Ship it only on a "
                                           "tool that emits an OPEN subpath")
                                .arg(QLatin1String(id))));
    // The one id that IS minted is the one the row ships — the hold is
    // about missing labels, not about the engine.
    QVERIFY(catTr("ui.options.stroke_width") != QStringLiteral("ui.options.stroke_width"));
}


void TstOptionsBar::strokeBandOutsideTheShapeBoundsIsRepainted()
{
    // REPAINT SCOPING, probed rather than assumed, and aimed at the unit
    // that OWNS the claim.
    //
    // THE CITATION, STATED STRAIGHT — the first version of this comment
    // quoted the wrong command, the error reached a desk, and the block
    // it caused was re-grounded. Both files, verbatim, in this repo
    // (contracts/, not spec/):
    //   contracts/commands/vector.shape_add.yaml:163
    //       dirtyRect: "D = shape bounds"
    //     — NO inflate clause. This IS the command the shell emits: the
    //       width is a pre-creation option, so submitShapePrimitive
    //       sends vector.shape_add and nothing else. Its `events.dirty`
    //       key says "shape bounds" too.
    //   contracts/commands/vector.shape_set_style.yaml:98
    //       dirtyRect: "D = shape bounds (stroke width changes inflate;
    //                   use max of before/after, §40.7)"
    //     — the inflate clause lives HERE, on a command THIS ROW NEVER
    //       EMITS.
    // So what shape_add actually promises is the un-inflated rect it
    // delivers. Read only against its own text, the engine keeps its
    // word, and this case would have nothing to say.
    //
    // WHY IT IS STILL A DEFECT, on ground that does not borrow the
    // clause. A CENTER-aligned stroke paints half its width OUTSIDE the
    // primitive's bounds — engine-true, export-true ink, proven on
    // pixels in the cases above. "D = shape bounds" therefore leaves
    // real ink outside the rect the canvas is told to repaint, whatever
    // the yaml says. What changed is REACHABILITY: before the
    // pro/shape-stroke arm the composite door refused widthPx > 0
    // outright, so no shape_add could ever paint outside its own bounds
    // and the gap was latent. This slice is the first thing that can
    // reach it, which is exactly why it is recorded here and now rather
    // than left for the engine side to notice. The §40.7 inflation rule
    // is cited as the SHAPE OF THE FIX the contract already knows how to
    // describe, not as a clause shape_add is breaking.
    //
    // FIRST AIM DISCARDED, recorded because the reason matters: probing
    // the CANVAS GRAB cannot answer this. After ANY vector.shape_add in
    // this harness the cache is transparent everywhere outside the
    // primitive's bounds — measured identically at widthPx 0 (the
    // byte-identical pre-row envelope) and at widthPx 20, x-runs
    // [0,99) and [220,320) both times. That is a PRE-EXISTING cache
    // lifecycle behaviour independent of the stroke, and this row
    // neither causes nor fixes it; a stroke assertion layered on top of
    // it would have been measuring the wrong thing. The engine's
    // regionsDirty payload is the truth this case is actually about.
    const auto dirtyUnionForWidth = [this](double widthPx) {
        freshDoc();
        setTool("shape.rect");
        m_canvas->setProperty("brushColor", QColor(Qt::white));
        m_canvas->setProperty("shapeStrokeWidth", widthPx);
        focusCanvas();
        QCoreApplication::processEvents();
        QSignalSpy spy(m_controller, &DocumentController::regionsDirty);
        const int before = journalKinds().size();
        dragStroke(100, 60, 220, 140); // primitive bounds (100,60)-(220,140)
        [&] { QTRY_COMPARE(journalKinds().size(), before + 1); }();
        QCoreApplication::processEvents();
        QRect u;
        for (const QList<QVariant> &sig : spy)
            for (const QVariant &rv : sig.at(0).toList())
                u = u.united(rv.toRect());
        return u;
    };

    // widthPx 0 — the contract's own shape_add default: D is the shape
    // bounds, un-inflated.
    const QRect plain = dirtyUnionForWidth(0.0);
    if (QTest::currentTestFailed())
        return;
    QVERIFY2(plain.isValid() && !plain.isEmpty(), "no dirty rect for a plain shape_add");
    QVERIFY2(plain.left() >= 100 - 2,
             qPrintable(QStringLiteral("un-stroked D already reaches x=%1 — this "
                                       "case cannot discriminate").arg(plain.left())));

    // A SWEEP, not a two-point reading. If D grew with the width but
    // fell short, a single stroked measurement would look the same as D
    // ignoring the width entirely; only a sweep separates "under-
    // inflated" from "FLAT", and the two want different fixes. Widths
    // 4 / 20 / 40 against the same gesture. A centred band at width w
    // puts ink w/2 outside the primitive on every side, so D must reach
    // x = 100 - w/2. The margins are structural, kept slack against tile
    // coalescing — which can only GROW D, never shrink it below the ink.
    const QRect w4 = dirtyUnionForWidth(4.0);
    if (QTest::currentTestFailed())
        return;
    const QRect stroked = dirtyUnionForWidth(20.0);
    if (QTest::currentTestFailed())
        return;
    const QRect w40 = dirtyUnionForWidth(40.0);
    if (QTest::currentTestFailed())
        return;
    QVERIFY2(stroked.isValid() && !stroked.isEmpty(),
             "no dirty rect for a stroked shape_add");
    QVERIFY2(w40.isValid() && !w40.isEmpty(),
             "no dirty rect for a stroked shape_add at width 40");
    qInfo() << "shape_add dirty rect — w=0:" << plain << " w=4:" << w4
            << " w=20:" << stroked << " w=40:" << w40;

    m_canvas->setProperty("shapeStrokeWidth", 0.0);
    m_canvas->setProperty("brushColor", QColor(Qt::black));

    // MEASURED, AND IT DOES NOT HOLD TODAY. Over these bounds the engine
    // reports the SAME D = (100,59) 120x82 at every width in the sweep —
    // 0, 4, 20 and 40 alike. Not under-inflated: FLAT. It is the
    // primitive's own bounds, byte-for-byte the un-stroked answer. At
    // width 40 the band paints out to x = 80 and y = 40, so 20 doc px of
    // engine-true, export-true ink on every side falls OUTSIDE the rect
    // the canvas is told to repaint, and the shortfall scales with the
    // one number the new control puts in the user's hands.
    //
    // The assertion below is aimed at the width-40 case for that reason:
    // it is the widest separation the sweep offers, so if D ever starts
    // arriving inflated at all this is where it shows first.
    //
    // This is ENGINE-side truth and this lane's write fence is shell/**,
    // so the check stays here. It is a DOORBELL, not an accepted state.
    // Nothing about the row is faked in the meantime — the composite and
    // the exported PNG carry the whole band, proven on pixels above; what
    // is under-scoped is the LIVE canvas invalidation only.
    //
    // ★ WHY THIS IS A LIVE PIN ON THE MEASURED TRUTH AND NOT A
    // QEXPECT_FAIL ON THE WISHED-FOR ONE. The earlier form asserted the
    // INFLATION and marked it expected-failing. It did convert — a
    // simulated fix produced XPASS and a red suite — but it converted
    // ILLEGIBLY: QEXPECT_FAIL's reason string is the only text naming
    // both contract anchors, and Qt prints that reason on XFAIL and
    // SUPPRESSES it on XPASS. So on the one run that matters the reader
    // got the QVERIFY2 message instead, which asserted D does NOT
    // inflate at the exact moment it had started to. The doorbell rang
    // with a message contradicting the event that rang it.
    //
    // Pinning the flatness instead inverts all three properties. The
    // text that names the anchors is now on the arm that PRINTS. The
    // message describes what was OBSERVED rather than what was hoped.
    // And the trigger widens: XPASS needed all four bounds correct at
    // once, so a partial or wrong inflation stayed silently XFAIL, while
    // an equality pin reds on ANY movement in D — the fix, a partial
    // fix, and a regression alike. `plain` is guarded valid, non-empty
    // and discriminating above, so this equality has a liveness floor
    // and cannot pass by both sides being empty.
    // RE-AIMED 2026-09-05 on the operator's Q-D37 ruling, option (a): THE
    // INFLATION IS CORRECT. A dirty rect must cover every pixel the command
    // changes, and a centred stroke's ink lies outside the geometric bounds
    // (a mitred join reaches up to 2w beyond a vertex). SS40.7 already
    // prescribes stroke-width inflation for vector.shape_set_style, so
    // "D = shape bounds" at vector.shape_add.yaml:163 is the STALE WORDING
    // and the merged, countersigned engine (entry 594) is the behaviour of
    // record. The wording gap is filed as SD-v5.7-04 for the 09-08 ceremony;
    // if that does not open, KNOWN-ISSUES carries it.
    //
    // The pin now holds the INFLATION the doorbell itself specified, and it
    // keeps the property the flatness form bought: it reds on ANY wrong
    // movement in D, because a partial inflation fails the bounds and a
    // non-monotonic one fails the containment.
    const auto fmt = [](const QRect &r) {
        return QStringLiteral("%1,%2 %3x%4")
            .arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height());
    };
    QVERIFY2(w40.left() <= 80 && w40.top() <= 40
                 && w40.right() >= 239 && w40.bottom() >= 159,
             qPrintable(QStringLiteral(
                            "vector.shape_add's dirty rect is NOT inflated to "
                            "cover the stroke band at widthPx 40. A centred band "
                            "at width w puts ink w/2 outside the primitive on "
                            "every side, so D must reach x<=80, y<=40, "
                            "right>=239, bottom>=159. Measured D at widthPx "
                            "0/4/20/40 = %1 / %2 / %3 / %4")
                            .arg(fmt(plain), fmt(w4), fmt(stroked), fmt(w40))));
    QVERIFY2(w4.contains(plain) && stroked.contains(w4) && w40.contains(stroked),
             qPrintable(QStringLiteral(
                            "the dirty rect is not MONOTONIC in the stroke "
                            "width: each wider band must cover the narrower "
                            "one. Measured D at widthPx 0/4/20/40 = "
                            "%1 / %2 / %3 / %4")
                            .arg(fmt(plain), fmt(w4), fmt(stroked), fmt(w40))));
}

// ADDED C-784 — THE FINDING FOR WORD 4's ROUTED DEFECT, BEFORE ANY FIX.
// WORD 4 routes one product defect from Q-D41 ii's gate: "heal.stroke renders
// a size row that does not write canvas.brushSize", caught on qa/q-gesture by
// tst_gesture_chrome::sizeRowShipsForEveryStrokeToolThatTakesASize — which
// does not exist in this tree, so the defect is reproduced here first.
//
// GENERALISED ON PURPOSE. The bar is declarative: ten tools share ONE row
// descriptor (optRowBrushSize) and ONE delegate whose write is
// `canvas[modelData.binding] = value`. A defect reaching heal by that path
// reaches every tool on it, so testing heal alone answers a narrower question
// than the one asked. BOTH directions per tool, because state->control
// passing while control->state fails is exactly the shape of "renders a row
// that does not write".
void TstOptionsBar::sizeRowWRITESForEveryToolThatShowsIt()
{
    freshDoc();
    static const char *kSizeTools[] = {
        "brush.stroke", "eraser.stroke", "heal.stroke", "clone.stroke",
        "dodge_burn.dodge", "dodge_burn.burn", "dodge_burn.sponge",
        "blur_sharpen.blur", "blur_sharpen.sharpen", "smudge.stroke",
    };
    QStringList noRow, noRead, noWrite;
    for (const char *t : kSizeTools) {
        setTool(t);
        QQuickItem *slider = toolbarControl("QQuickSlider");
        if (!slider) { noRow << QLatin1String(t); continue; }

        m_canvas->setProperty("brushSize", 57.0);                  // state -> control
        if (!qFuzzyCompare(slider->property("value").toDouble(), 57.0))
            noRead << QLatin1String(t);

        // control -> state, driven through the REAL control (Q-D6) rather
        // than by assigning the slider's value, which would prove only that
        // QML assignment works.
        slider->forceActiveFocus(Qt::TabFocusReason);
        QTRY_VERIFY(slider->hasActiveFocus());
        QTest::keyClick(m_window, Qt::Key_Right);
        if (!qFuzzyCompare(m_canvas->property("brushSize").toDouble(), 58.0))
            noWrite << QLatin1String(t);
    }
    qInfo() << "  no size row:" << noRow << " no READ:" << noRead << " no WRITE:" << noWrite;
    QVERIFY2(noRow.isEmpty(), qPrintable(QStringLiteral("size row ABSENT for: ") + noRow.join(QStringLiteral(", "))));
    QVERIFY2(noRead.isEmpty(), qPrintable(QStringLiteral("size row does not READ canvas.brushSize: ") + noRead.join(QStringLiteral(", "))));
    QVERIFY2(noWrite.isEmpty(), qPrintable(QStringLiteral("size row does not WRITE canvas.brushSize: ") + noWrite.join(QStringLiteral(", "))));
}


// C-784 — WORD 4's routed defect, asked with THEIR instrument rather than
// mine. My key-press arm above passed for all ten tools, and the routed
// finding says heal.stroke ships a size row that does not write
// canvas.brushSize. The two claims can only both be true if the ARM is the
// difference, so this case is a deliberate copy of
// tst_gesture_chrome::sizeRowShipsForEveryStrokeToolThatTakesASize
// (qa/q-gesture, blob read at C-784): same eight-tool table, same seed,
// same 10%->90% drag, same from/to identification.
//
// WHY THE DEVICE COULD BE THE WHOLE DEFECT: Key_Right reaches a Slider
// through the FOCUS path and calls increase() regardless of where the
// control sits or what covers it. A drag reaches it through HIT TESTING.
// A size row that renders, focuses and increments but cannot be grabbed by
// the mouse is broken for every real user and green under my arm -- and
// "renders a row that does not write" is exactly how that reads from
// outside. This case is the control that tells those two apart.
static const char *kQaSizeTools[] = {
    "heal.stroke", "dodge_burn.dodge", "dodge_burn.burn", "dodge_burn.sponge",
    "blur_sharpen.blur", "blur_sharpen.sharpen", "smudge.stroke",
    "clone.stroke",
};

void TstOptionsBar::sizeRowWritesUnderAMouseDragNotJustAKeyPress()
{
    freshDoc();
    // Non-vacuity on the table, their T66 idiom: a table that shrank to
    // nothing leaves every arm below vacuous and the case green.
    QCOMPARE(int(sizeof(kQaSizeTools) / sizeof(kQaSizeTools[0])), 8);

    // THE NEGATIVE CONTROL FIRST. The strip is one Repeater over
    // toolOptionsModel[canvas.tool] with a [] fallback, so a tool with no
    // rows renders an EMPTY bar. If a stray slider were in scope, every arm
    // below would pass without a size row existing at all.
    setTool("select.rect");
    const int stray = visibleByClass(m_toolbar, "QQuickSlider").size();
    QVERIFY2(stray == 0,
             qPrintable(QStringLiteral("CONTROL FAILED: select.rect takes no "
                                       "options yet %1 Slider(s) are visible")
                            .arg(stray)));

    QStringList noRow, notTheSizeRow, noDrag;
    for (const char *t : kQaSizeTools) {
        setTool(t);
        QQuickItem *slider = nullptr;
        QTRY_VERIFY_WITH_TIMEOUT(
            (slider = toolbarControl("QQuickSlider")) != nullptr, 2000);
        if (!slider) { noRow << QLatin1String(t); continue; }

        // It must be THE size row, not merely a slider: optRowBrushSize is
        // declared once as min 1 max 200, so the range identifies it.
        if (!qFuzzyCompare(slider->property("from").toDouble(), 1.0)
            || !qFuzzyCompare(slider->property("to").toDouble(), 200.0)) {
            notTheSizeRow << QLatin1String(t);
            continue;
        }

        // ONE EVENT-LOOP TURN BEFORE THE PRESS, AND THIS LINE IS THE FINDING
        // (C-784). Without it this arm reds on heal.stroke and ONLY on
        // heal.stroke -- the routed defect, reproduced here exactly. It is
        // not a product defect, and the measurement that says so:
        //   * THE DEAF WINDOW IS ~2 MILLISECONDS. Measured cold, one value
        //     per PROCESS: deaf at no-wait, 0ms and 1ms; live at 2, 4, 8, 16,
        //     32, 64, 128, 250 and 500. A press cannot be DELIVERED without
        //     the loop spinning at all, and a human cannot land inside 2ms,
        //     so there is no sequence a user can perform that reaches this.
        //     The settle above is 16ms -- one 60Hz frame, 8x the measured
        //     window -- chosen as a real interval rather than tuned to the
        //     threshold.
        //     THE FIRST BISECT OF THIS WAS WRONG and is recorded because the
        //     error is the reusable part: it looped the wait values IN ONE
        //     PROCESS, so each iteration rebuilt the strip and a later
        //     iteration could reuse a delegate an earlier one had already
        //     made live. Wait-length and iteration index moved together and
        //     it read a threshold of 1ms off contaminated arms. Cold start
        //     per value is the only form of this measurement that means
        //     anything.
        //   * the control is innocent in every readable respect: the same
        //     object across the settle, sceneRect identical at 37,40 110x40,
        //     enabled, opacity 1, and the correct 1..200 range.
        //   * the same wiring driven through the FOCUS path writes brushSize
        //     for all ten tools with no settle at all -- the case above.
        // heal.stroke is singled out structurally, not by a defect: the six
        // single-row tools all REUSE one delegate object, so their arms drag
        // a row that has been live since the previous arm. Only heal is
        // entered from an EMPTY strip (the negative control), so it is the
        // one arm forced onto a delegate created in that same turn.
        // GAP NAMED: this does not explain why clone.stroke, which also gets
        // a fresh delegate, passes in table order. The one-turn result is
        // measured; that asymmetry is not.
        QTest::qWait(16);   // one frame at 60 Hz

        // SEED LOW, INDEPENDENTLY PER ARM. The drag runs to the far end and
        // saturates brushSize at 200; comparing against whatever the previous
        // arm left would report seven working rows as broken (their recorded
        // first cut, and the same trap as the T62 mask arms).
        m_canvas->setProperty("brushSize", 12.0);
        QCoreApplication::processEvents();
        const double before = m_canvas->property("brushSize").toDouble();

        const QPointF lo = slider->mapToScene(
            QPointF(slider->width() * 0.10, slider->height() / 2));
        const QPointF hi = slider->mapToScene(
            QPointF(slider->width() * 0.90, slider->height() / 2));
        QTest::mousePress(m_window, Qt::LeftButton, Qt::NoModifier, lo.toPoint());
        QTest::mouseMove(m_window, hi.toPoint());
        QTest::mouseRelease(m_window, Qt::LeftButton, Qt::NoModifier, hi.toPoint());
        QTest::qWaitFor([&] {
            return m_canvas->property("brushSize").toDouble() != before;
        }, 1500);
        if (m_canvas->property("brushSize").toDouble() == before)
            noDrag << QLatin1String(t);
    }
    // DRAIN BEFORE LEAVING, and this is not defensive padding -- without it
    // this arm fails a test three cases downstream. Measured: at the end of
    // the loop the announcement count is 0, still 0 at +300ms, and reaches 1
    // at +1000ms, stable through +2500ms. Every drag above changes brushSize
    // while activeFocusItem is the CanvasView, which is the ANNOUNCING half
    // of the T6 rule (a keyboard-focused option change is suppressed because
    // the platform speaks the focused control; a change with canvas focus
    // goes through the live region), so this arm necessarily emits -- and it
    // emits RATE-LIMITED, landing about a second after the last drag.
    // brushSizeAnnouncementGuardPreserved drains for 600ms and then asserts
    // that a Key_Right with the slider focused announces NOTHING. My stray
    // landed inside its window and broke a compare that is about suppression,
    // not about this case at all. A test that leaves a timer in flight is not
    // finished, and 600ms was a fair assumption before this arm existed.
    QTest::qWait(1500);
    qInfo().noquote()
        << QStringLiteral("MOUSE ARM: %1 of 8 rendered the size row, %2 of "
                          "those drove canvas.brushSize under a DRAG")
               .arg(8 - noRow.size())
               .arg(8 - noRow.size() - notTheSizeRow.size() - noDrag.size());
    QVERIFY2(noRow.isEmpty(), qPrintable(QStringLiteral("no size row: ")
                                         + noRow.join(QStringLiteral(", "))));
    QVERIFY2(notTheSizeRow.isEmpty(),
             qPrintable(QStringLiteral("a slider that is not optRowBrushSize "
                                       "(1..200): ")
                        + notTheSizeRow.join(QStringLiteral(", "))));
    QVERIFY2(noDrag.isEmpty(),
             qPrintable(QStringLiteral("size row did NOT write canvas.brushSize "
                                       "under a mouse drag: ")
                        + noDrag.join(QStringLiteral(", "))));
}


// TEMPORARY PROBE (C-784). Splits the reproduced heal.stroke failure into
// its two possible causes, which want opposite fixes:
//   (1) the control MOVED and the write did not happen  -> a binding defect
//   (2) the control never moved                         -> the drag never
//       reached it (hit testing / overlay / geometry)
// clone.stroke is the CONTROL ARM: identical row list, passes the drag.

// Offscreen + the app's own Basic style, forced before the app exists.
int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    installClickFocusParity(app); // mirrors shell/src/main.cpp (Windows only)
    TstOptionsBar tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_options_bar.moc"
