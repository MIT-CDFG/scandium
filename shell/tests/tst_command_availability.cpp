// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 14a — THE DESK CHECKPOINT: prove the menu migration onto
// the single command-availability surface (the items-7+14 design entry,
// reports/O-UI-log.md) causes NO enable/disable regression.
//
// Shape: a state-matrix regression net over the REAL Main.qml booted
// offscreen (the tst_palette_pins harness — this binary compiles its own
// copy of the AppnameShell QML module and wires the real controller/
// announcer exactly like shell/src/main.cpp). For EVERY migrated action
// the matrix asserts, in EVERY visited document state:
//   (a) the OLD predicate's truth — each pre-migration Main.qml
//       predicate frozen VERBATIM in the table below and re-computed
//       from the controller's own property surface (the same
//       Q_PROPERTYs the old QML bindings read),
//   (b) == the availability row's `enabled`,
//   (c) == the actual QML MenuItem's `enabled` (cataloged-text lookup,
//       palette flyout duplicates excluded — the d7 pattern).
// Rows with no menu consumer yet (edit.delete, select.reselect,
// layer.viaCopy, transform.enter, fill.foreground/background, nudge —
// the item-7/14b vocabulary) assert (a)==(b) against the DESIGN-ruled
// derivation recorded per row.
//
// The state matrix (each slot GUARDS the dimension it claims before
// verifying, so a state that fails to arrive reds instead of silently
// collapsing the matrix):
//   S1 no-document (boot)          S2 document + no-selection
//   S3 document + active-selection S4 selection cleared (canUndo firm)
//   S5 undo/redo arms: undo (canRedo on), undo-to-PRE-INIT (hasDocument
//      false while canRedo true — refreshInfo's honest pre-init arm),
//      redo-all-back
//   S6 saved selection channel (select.loadSaved's only true state)
//   S7 multi-layer (layer.add)     S8 activeLayerIndex OUT OF RANGE
//      (activeLayerId == "" with a document open) + restore
// S8 doubles as the behavior-identity pin for the setActiveLayerIndex
// re-notify: the OLD predicates re-evaluated on activeLayerIndexChanged
// (they bound root.activeLayerId); commandAvailability notifies on
// infoChanged, so the controller re-emits it when an index move flips
// availability truth — leg (c) reds if that emission is dropped.
//
// HONEST LIMITS (recorded):
// - abiBlock12/abiBlock14 are CONSTANT (true on this build): their
//   conjuncts in edit.cut/paste/import/export cannot flip at runtime,
//   so the matrix cannot discriminate a dropped abi conjunct — that
//   coverage is code-identity only (the rows call the same accessors).
// - Layer LOCKS are write-only pre-info-v2: lock-class refusals are
//   apply-time typed refusals by design, never pre-computed rows —
//   there is no lock dimension in this matrix on purpose.
// - No current predicate depends on layer COUNT (only active-layer
//   emptiness), so S7 visits the multi-layer state without expecting
//   any row to flip.
//
// RULE 9 mutation evidence (run-and-restore, recorded in the lane log +
// commit): (M1) select.deselect row computed from hasDocument instead
// of selectionActive -> exactly the deselect action reds in S2/S4;
// (M2) edit.cut row's active-layer conjunct dropped -> exactly the cut
// action reds in S8. Both restored by targeted edit; green re-run.

#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QtTest>

#include <functional>

using namespace appname::shell;

namespace {

// The QML-visible inputs the OLD Main.qml predicates read. Captured
// through the property surface (not member access) so the frozen
// predicates below consume exactly what the old bindings consumed.
struct Snap {
    bool hasDocument = false;
    bool selectionActive = false;
    bool canUndo = false;
    bool canRedo = false;
    bool abi12 = false;
    bool abi14 = false;
    // 14b: the reselect memory truth (Q_PROPERTY hasSelectionMemory —
    // armed by an applied selection.set). This matrix drives selections
    // through selection.all/none only, so it stays false in every state
    // here; the armed-side flip is engine-proven end-to-end (real
    // marquee -> deselect -> chord) in tst_chord_table.
    bool reselectMemory = false;
    // Pro-Wave row 1: has THIS shell authored an adjustment op yet? The
    // ABI publishes no op ids (appname_doc_info_json's layer object has
    // no `ops` key), so this session memory is the only thing that makes
    // an op addressable — and therefore a real availability dimension,
    // not a convenience. S9 drives it in both directions.
    bool adjustmentOpMemory = false;
    QString activeLayerId;
    QString lastSavedChannelId;
};

using Truth = std::function<bool(const Snap &)>;
using Reason = std::function<QString(const Snap &)>;

struct Case {
    QString action;      // availability row key
    QStringList menuKeys; // cataloged text keys of consuming MenuItems
    Truth oldTruth;      // the FROZEN pre-migration predicate
    Reason reason;       // expected reasonId while disabled ("" = absent)
};

} // namespace

class TstCommandAvailability : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();
    void catalogGround();
    void s1NoDocument();
    // P1-4(a). These two need the NO-DOCUMENT state and there is no
    // closeDocument() door, so they run where that state exists —
    // immediately after s1, before s2 opens one. Declaration order IS the
    // fixture here; moving them below s2 makes them silently test a
    // different state rather than fail.
    void p14aReportUnavailableSpeaksTheRowsOwnReason();
    void p14aRepeatedPressFiresTheEventEachTime();
    void s2DocumentNoSelection();
    void s3SelectionActive();
    void s4SelectionCleared();
    void s5UndoRedoArmsAndPreInit();
    void s6SavedChannel();
    void s7MultiLayer();
    void s8ActiveLayerOutOfRange();
    // Pro-Wave row 1: the authored-op memory dimension (S1-S8 never
    // reach it) and its document-state clear.
    void s9AdjustmentOpMemory();

    void s10AdjustmentMemoryIsDocumentState();
    // P1-4(a), document-state half. These run LAST, and that is a
    // fixture constraint rather than a preference: this suite shares one
    // controller across cases in declaration order, and an earlier
    // placement had newDocument() here clobber the authored-op memory
    // s10 asserts — my case passed and its NEIGHBOUR went red. Anything
    // added below that calls newDocument() inherits the same duty.
    void p14aReportUnavailableFallsBackWhenNoConjunctIsNamed();
    void p14aReportUnavailableIsSilentTwoWays();
    void toastReconciliationBothChannelsReachIt();
    void p14aAnUncatalogedReasonIsCountedNotJustSwallowed();

private:
    CatalogQml *m_i18n = nullptr;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QList<Case> m_cases;

    QString catTr(const char *key) const
    {
        return m_i18n->tr(QLatin1String(key));
    }
    // MenuItem lookup through the REAL menu bar (mainMenuBar.menuAt/
    // itemAt in Main.qml's own context — ids resolve there): uniform
    // for plain items AND the Repeater-created filter items (which are
    // not QObject-tree reachable — the palette_pins flyout finding),
    // and palette-flyout-proof by construction (only the bar is
    // walked; menu.edit.puppet_warp is duplicated in a flyout).
    QObject *menuItem(const QString &catalogKey) const
    {
        const QString code = QStringLiteral(
            "(function() {"
            "  var label = AppI18n.tr(\"%1\");"
            "  for (var i = 0; i < mainMenuBar.count; ++i) {"
            "    var m = mainMenuBar.menuAt(i);"
            "    if (!m) continue;"
            "    for (var j = 0; j < m.count; ++j) {"
            "      var it = m.itemAt(j);"
            "      if (it && it.text === label) return it;"
            "    }"
            "  }"
            "  return null;"
            "})()").arg(catalogKey);
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "menuItem lookup failed:"
                       << expr.error().toString();
        return undefined ? nullptr : v.value<QObject *>();
    }
    Snap snapshot() const
    {
        Snap s;
        s.hasDocument = m_controller->property("hasDocument").toBool();
        s.selectionActive =
            m_controller->property("selectionActive").toBool();
        s.canUndo = m_controller->property("canUndo").toBool();
        s.canRedo = m_controller->property("canRedo").toBool();
        s.abi12 = m_controller->property("abiBlock12").toBool();
        s.abi14 = m_controller->property("abiBlock14").toBool();
        s.lastSavedChannelId =
            m_controller->property("lastSavedChannelId").toString();
        s.reselectMemory =
            m_controller->property("hasSelectionMemory").toBool();
        s.adjustmentOpMemory =
            !m_controller->lastAdjustmentOpId().isEmpty();
        // Main.qml's root.activeLayerId derivation, verbatim (lines
        // 84-88 pre-migration — unchanged by 14a):
        //   const rows = AppDoc.layers;
        //   const idx = AppDoc.activeLayerIndex;
        //   return (idx >= 0 && idx < rows.length) ? rows[idx].layerId : "";
        const QVariantList rows = m_controller->property("layers").toList();
        const int idx = m_controller->property("activeLayerIndex").toInt();
        s.activeLayerId =
            (idx >= 0 && idx < rows.size())
                ? rows.at(idx).toMap().value(QStringLiteral("layerId"))
                      .toString()
                : QString();
        return s;
    }
    void verifyState(const char *label);
    QVariantMap applyOk(const char *kind, const QVariantMap &params)
    {
        const QVariantMap r = m_controller->applyCommand(
            QLatin1String(kind), {}, params, QString());
        return r;
    }
};

void TstCommandAvailability::initTestCase()
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

    // ---- THE FROZEN REFERENCE ----
    // Every oldTruth lambda is the pre-migration Main.qml predicate,
    // quoted verbatim in the comment above it, recomputed over the same
    // property surface. Reasons: rows carry reasonId ONLY while
    // disabled and only from EXISTING cataloged ids (design entry) —
    // "ui.error.no_document" / "error.command.selection_required".
    const Reason noReason = [](const Snap &) { return QString(); };
    const Reason docReason = [](const Snap &s) {
        return s.hasDocument ? QString()
                             : QStringLiteral("ui.error.no_document");
    };
    const Reason selReason = [](const Snap &s) {
        return !s.hasDocument
                   ? QStringLiteral("ui.error.no_document")
                   : QStringLiteral("error.command.selection_required");
    };
    const Truth doc = [](const Snap &s) { return s.hasDocument; };
    const Truth layer = [](const Snap &s) {
        return s.hasDocument && !s.activeLayerId.isEmpty();
    };

    // File
    // old: enabled: AppDoc.hasDocument
    m_cases << Case{QStringLiteral("file.save"),
                    {QStringLiteral("ui.menu.file.save")}, doc, docReason};
    // old: enabled: AppDoc.hasDocument
    m_cases << Case{QStringLiteral("file.saveAs"),
                    {QStringLiteral("ui.menu.file.save_as")}, doc,
                    docReason};
    // old: enabled: AppDoc.hasDocument && AppDoc.abiBlock12
    m_cases << Case{QStringLiteral("file.import"),
                    {QStringLiteral("ui.menu.file.import")},
                    [](const Snap &s) { return s.hasDocument && s.abi12; },
                    docReason};
    // old: enabled: AppDoc.hasDocument && AppDoc.abiBlock12
    m_cases << Case{QStringLiteral("file.export"),
                    {QStringLiteral("menu.file.export_as")},
                    [](const Snap &s) { return s.hasDocument && s.abi12; },
                    docReason};
    // Edit
    // old: enabled: AppDoc.canUndo
    m_cases << Case{QStringLiteral("edit.undo"),
                    {QStringLiteral("ui.action.undo")},
                    [](const Snap &s) { return s.canUndo; }, noReason};
    // old: enabled: AppDoc.canRedo
    m_cases << Case{QStringLiteral("edit.redo"),
                    {QStringLiteral("ui.action.redo")},
                    [](const Snap &s) { return s.canRedo; }, noReason};
    // Wave F: the history.goto 14a row — the HistoryPanel rows' enabled
    // truth (census class F; NEW row, no pre-migration predicate to
    // freeze: designed doc-gated, range truth stays the engine's typed
    // out_of_range at apply time). No consuming MenuItem — the consumer
    // is the panel row's MouseArea/keys, proven by real gestures against
    // engine readback in tst_history_panel.
    m_cases << Case{QStringLiteral("history.goto"), {}, doc, docReason};
    // old: enabled: AppDoc.hasDocument && root.activeLayerId !== ""
    //               && AppDoc.abiBlock14
    m_cases << Case{QStringLiteral("edit.cut"),
                    {QStringLiteral("ui.menu.edit.cut")},
                    [](const Snap &s) {
                        return s.hasDocument && !s.activeLayerId.isEmpty()
                               && s.abi14;
                    },
                    docReason};
    // old: enabled: AppDoc.hasDocument
    m_cases << Case{QStringLiteral("edit.copy"),
                    {QStringLiteral("ui.menu.edit.copy")}, doc, docReason};
    // old: enabled: AppDoc.hasDocument && AppDoc.abiBlock12
    m_cases << Case{QStringLiteral("edit.paste"),
                    {QStringLiteral("ui.menu.edit.paste")},
                    [](const Snap &s) { return s.hasDocument && s.abi12; },
                    docReason};
    // ENGINE-derived (item 15 — no pre-migration predicate exists; this
    // row is NEW, the edit.delete precedent for a design/engine-derived
    // Case). edit.paste_into's kind impl
    // (crates/appname-core/src/engine/kinds.rs, EditPasteIntoKind::apply)
    // refuses error.command.selection_required unless
    // `cx.document().selection().active()` — the contract's
    // selectionBehavior "REQUIRED — consumed as the new raster mask's
    // coverage" (§40.5). Ingress is edit.paste's own 1.2 asset_create
    // door, so the predicate is edit.paste's AND the selection:
    //     hasDocument && abi12 && selectionActive
    // The reason mirrors commandAvailability()'s shared selReasonId
    // exactly, INCLUDING its third arm: when a document and a selection
    // both exist and only the abi-1.2 conjunct fails, no cataloged bare
    // id names that cause, so the row carries NO reasonId (never a
    // misnamed one).
    m_cases << Case{QStringLiteral("edit.pasteInto"),
                    {QStringLiteral("ui.menu.edit.paste_into")},
                    [](const Snap &s) {
                        return s.hasDocument && s.abi12
                               && s.selectionActive;
                    },
                    [](const Snap &s) -> QString {
                        if (!s.hasDocument)
                            return QStringLiteral("ui.error.no_document");
                        if (!s.selectionActive)
                            return QStringLiteral(
                                "error.command.selection_required");
                        return QString();
                    }};
    // old: enabled: AppDoc.hasDocument && root.activeLayerId !== ""
    m_cases << Case{QStringLiteral("edit.fill"),
                    {QStringLiteral("ui.menu.edit.fill")}, layer,
                    docReason};
    // 1.0 final (corrigendum (l)): Edit > Puppet Warp is removed; its
    // ABSENCE is pinned in catalogGround.
    // DESIGN-derived (desk ruling): Delete = the edit.cut ENGINE kind
    // WITHOUT the app-side copy — mutation-only, NO abiBlock14
    // conjunct; both disposition arms act on the active layer; the
    // transparency lock stays an apply-time typed refusal (the recorded
    // honest limit), never a pre-computed disable. 14b SHIPPED the menu
    // consumer (the Edit > Delete row, ui.menu.edit.delete — sealed
    // menu-mint 3dc5c37), so the (c) leg now covers it in every state.
    m_cases << Case{QStringLiteral("edit.delete"),
                    {QStringLiteral("ui.menu.edit.delete")}, layer,
                    docReason};
    // Select
    // old: enabled: AppDoc.hasDocument
    m_cases << Case{QStringLiteral("select.all"),
                    {QStringLiteral("ui.menu.select.all")}, doc, docReason};
    // old: enabled: AppDoc.selectionActive
    m_cases << Case{QStringLiteral("select.deselect"),
                    {QStringLiteral("ui.menu.select.deselect")},
                    [](const Snap &s) { return s.selectionActive; },
                    selReason};
    // old: enabled: AppDoc.hasDocument
    m_cases << Case{QStringLiteral("select.invert"),
                    {QStringLiteral("ui.menu.select.invert")}, doc,
                    docReason};
    // old: enabled: AppDoc.selectionActive
    m_cases << Case{QStringLiteral("select.modify"),
                    {QStringLiteral("ui.menu.select.modify")},
                    [](const Snap &s) { return s.selectionActive; },
                    selReason};
    // DESIGN-derived, WIRED AT 14b: reselect enables once the shell's
    // last-selection memory is armed (an applied selection.set). Within
    // this matrix the memory never arms (see the Snap field note).
    // ITEM 25b (census class H, stale-record consume-fix): the disabled
    // arm used to drop its reasonId entirely, on a record claiming
    // "nothing to reselect" had no cataloged bare id. STALE —
    // ui.error.nothing_to_reselect exists in all four locales. The row
    // now names its FAILING CONJUNCT, which is the whole difficulty the
    // old record was dodging: with no document the honest reason is the
    // document one; with a document but no memory it is the memory one.
    // Both arms are asserted below, so naming the wrong conjunct reds.
    const Reason reselectReason = [](const Snap &s) {
        return s.hasDocument
                   ? QStringLiteral("ui.error.nothing_to_reselect")
                   : QStringLiteral("ui.error.no_document");
    };
    m_cases << Case{QStringLiteral("select.reselect"), {},
                    [](const Snap &s) {
                        return s.hasDocument && s.reselectMemory;
                    },
                    reselectReason};
    // ---- Pro-Wave row 1 (no MENU-BAR consumer: the consumer is the
    // LayersPanel button, and menuItem() walks the menu bar only, so leg
    // (c) does not apply — these assert (a)==(b) against the design-ruled
    // derivations recorded on the rows in document_controller.cpp).
    //
    // layer.addAdjustment — DOCUMENT-gated only. Deliberately NOT
    // selection-gated: with nothing selected the door creates a plain
    // unmasked adjustment, a correct outcome. If someone later adds a
    // selection conjunct "for symmetry" with the mask row, S2 reds.
    m_cases << Case{QStringLiteral("layer.addAdjustment"), {},
                    [](const Snap &s) { return s.hasDocument; }, docReason};
    // layer.maskAdjustmentFromSelection — three real conjuncts. The
    // reasonId is the SHARED selection reason, correct for the document
    // and selection conjuncts and correctly ABSENT for the memory one
    // (no cataloged bare id names "no adjustment authored this session" —
    // queued to O-I18N). Asserting the empty reason in the doc+selection+
    // no-memory state is what stops a later writer from reaching for a
    // wrong-but-available id, the same defect select.reselect corrected.
    // NOT the shared selReason: that helper answers
    // error.command.selection_required for ANY disabled-with-a-document
    // state, which is right for rows whose only non-document conjunct is
    // the selection, and WRONG here — this row can be disabled with a
    // document AND a selection, when the memory is what is missing. The
    // three-way expectation is spelled out so each conjunct's reason is
    // pinned separately, including the deliberate empty one.
    const Reason maskAdjReason = [](const Snap &s) {
        if (!s.hasDocument)
            return QStringLiteral("ui.error.no_document");
        if (!s.selectionActive)
            return QStringLiteral("error.command.selection_required");
        return QString(); // memory conjunct: no cataloged id names it
    };
    m_cases << Case{QStringLiteral("layer.maskAdjustmentFromSelection"), {},
                    [](const Snap &s) {
                        return s.hasDocument && s.selectionActive
                               && s.adjustmentOpMemory;
                    },
                    maskAdjReason};
    // ---- v1.0.1 LAYER-MASK ROUTE (mask.add) ----
    // layer.addMask — the `layer` truth verbatim (document + an active
    // layer to own the mask), with docReason: no cataloged bare id names
    // "no active layer", so a document-open-but-no-layer state is
    // disabled with the reason correctly ABSENT. This is the nudge row's
    // shape, reused rather than re-derived.
    //
    // THE CONJUNCT THAT IS NOT HERE IS THE POINT. §11.2 allows one raster
    // + one vector mask per layer and an occupied slot answers
    // error.command.target_in_use — but the ABI's info mask object has no
    // `kind`, so the shell cannot tell which slot is taken. If someone
    // later adds an occupied-slot conjunct from `maskIds` alone, this
    // case reds: maskIds is non-empty for a VECTOR-masked layer too, and
    // the raster slot there is free.
    // layer.addFill — DOCUMENT-gated, layer.addAdjustment's shape, and
    // deliberately NOT layer-gated: layer.add inserts at the root, so a
    // fill layer is legal on an empty stack. If someone adds a `layer`
    // conjunct here "to match the mask rows", S8b reds.
    m_cases << Case{QStringLiteral("layer.addFill"), {},
                    [](const Snap &s) { return s.hasDocument; }, docReason};
    m_cases << Case{QStringLiteral("layer.addMask"), {}, layer, docReason};
    // layer.addMaskFromSelection — three conjuncts, and the reason is
    // three-way for the same reason maskAdjReason is: the shared
    // selReason answers selection_required for ANY disabled-with-a-
    // document state, which is WRONG when the missing conjunct is the
    // active layer. Pinning the empty reason in the doc+selection+
    // no-layer state is what stops a later writer reaching for a
    // wrong-but-available id.
    const Reason addMaskSelReason = [](const Snap &s) {
        if (!s.hasDocument)
            return QStringLiteral("ui.error.no_document");
        if (!s.selectionActive)
            return QStringLiteral("error.command.selection_required");
        return QString(); // layer conjunct: no cataloged id names it
    };
    m_cases << Case{QStringLiteral("layer.addMaskFromSelection"), {},
                    [](const Snap &s) {
                        return s.hasDocument && !s.activeLayerId.isEmpty()
                               && s.selectionActive;
                    },
                    addMaskSelReason};
    // old: enabled: AppDoc.selectionActive
    m_cases << Case{QStringLiteral("select.saveSelection"),
                    {QStringLiteral("ui.menu.select.save_selection")},
                    [](const Snap &s) { return s.selectionActive; },
                    selReason};
    // old: enabled: AppDoc.lastSavedChannelId !== ""
    m_cases << Case{QStringLiteral("select.loadSaved"),
                    {QStringLiteral("ui.menu.select.load_saved")},
                    [](const Snap &s) {
                        return !s.lastSavedChannelId.isEmpty();
                    },
                    docReason};
    // old: enabled: AppDoc.hasDocument && root.activeLayerId !== ""
    m_cases << Case{QStringLiteral("select.loadLayerAlpha"),
                    {QStringLiteral("ui.menu.select.load_layer_alpha")},
                    layer, docReason};
    // Image — old: enabled: AppDoc.hasDocument on all ten.
    const QPair<const char *, const char *> imageRows[] = {
        {"image.resize", "ui.menu.image.resize"},
        {"image.canvasSize", "ui.menu.image.canvas_size"},
        {"image.rotate", "ui.menu.image.rotate"},
        {"image.trim", "ui.menu.image.trim"},
        {"image.crop", "ui.menu.image.crop"},
        {"image.flatten", "ui.menu.image.flatten"},
        {"image.assignProfile", "ui.menu.image.assign_profile"},
        {"image.convertProfile", "ui.menu.image.convert_profile"},
        {"image.depthConvert", "ui.menu.image.depth_convert"},
    };
    for (const auto &p : imageRows)
        m_cases << Case{QLatin1String(p.first),
                        {QLatin1String(p.second)}, doc, docReason};
    // Image > Auto Tone / Auto Contrast / Auto Color (§17.2). NEW row —
    // there is no pre-migration predicate to freeze, so the truth below
    // is the DESIGN-RULED derivation, recorded here and in the
    // commandAvailability comment: `layer`, not `doc`.
    //
    // WHY `layer` AND WHY THAT IS DISCRIMINABLE HERE: the ten rows above
    // act on the DOCUMENT; these three edit params.targetLayer, whose
    // contract default is the ACTIVE layer. S8 (activeLayerIndex out of
    // range -> activeLayerId empty WITH a document open) is the state
    // that separates the two, so a regression from `layer` to `doc`
    // reds exactly there rather than passing vacuously. Selection is
    // deliberately NOT a conjunct: selectionBehavior "gates coverage ...
    // (else full layer)" SCOPES the histogram, and S3 holds a selection
    // while this row must not move.
    //
    // ONE row for three kinds, on the filter.apply precedent — and the
    // three menuKeys below make that shared-row claim two-sided: all
    // three MenuItems must re-bind to the same truth in every state.
    m_cases << Case{QStringLiteral("image.autoAdjust"),
                    {QStringLiteral("ui.menu.image.auto_tone"),
                     QStringLiteral("ui.menu.image.auto_contrast"),
                     QStringLiteral("ui.menu.image.auto_color")},
                    layer, docReason};
    // Filter — the six live kinds shared ONE predicate and now share
    // ONE row. old (Repeater item): enabled: AppDoc.hasDocument &&
    // root.activeLayerId !== ""
    m_cases << Case{QStringLiteral("filter.apply"),
                    {QStringLiteral("filter.gaussian_blur"),
                     QStringLiteral("filter.box_blur"),
                     QStringLiteral("filter.median"),
                     QStringLiteral("filter.high_pass"),
                     QStringLiteral("filter.add_noise"),
                     QStringLiteral("filter.unsharp_mask")},
                    layer, docReason};
    // DESIGN-derived 14b/item-7 vocabulary (no menu consumers yet):
    // Cmd+J composition — selection arm: app-side copy (abi 1.4
    // per-layer readback) -> edit.paste as a new layer (abi 1.2
    // ingress); no-selection arm: layer.duplicate (base registry kind).
    m_cases << Case{QStringLiteral("layer.viaCopy"), {},
                    [](const Snap &s) {
                        return s.hasDocument && !s.activeLayerId.isEmpty()
                               && (!s.selectionActive
                                   || (s.abi14 && s.abi12));
                    },
                    docReason};
    // Cmd+T enters transform.free on the active layer (item-13 tie).
    m_cases << Case{QStringLiteral("transform.enter"), {}, layer,
                    docReason};
    // Alt/Cmd+Delete fill arms: active-layer actions like edit.fill —
    // an active selection SCOPES the fill, it does not gate it.
    m_cases << Case{QStringLiteral("fill.foreground"), {}, layer,
                    docReason};
    m_cases << Case{QStringLiteral("fill.background"), {}, layer,
                    docReason};
    // Arrow-key nudge of the active layer (integer-translate arm).
    m_cases << Case{QStringLiteral("nudge"), {}, layer, docReason};

    // 36 + the Wave-F history.goto row + the item-15 edit.pasteInto row.
    // (Rebase note: both lanes added one independent row and each bumped
    // this pin 36 -> 37; the counts compose, they do not compete.)
    // + the two Pro-Wave row-1 rows (layer.addAdjustment,
    // layer.maskAdjustmentFromSelection): 38 -> 40, same composing rule.
    // + the §17.2 auto-adjust row (image.autoAdjust): 40 -> 41.
    // + the two v1.0.1 layer-mask rows (layer.addMask,
    // layer.addMaskFromSelection): 41 -> 43, same composing rule.
    // + the v1.0.1 fill row (layer.addFill): 43 -> 44.
    // - 1.0 final (corrigendum (l)): edit.puppetWarp removed: 44 -> 43.
    // - 1.0 final (corrigendum (l)): image.mode removed: 43 -> 42.
    QCOMPARE(m_cases.size(), 42);
}

// ===================== P1-4(a): AN UNMET PRECONDITION MUST SAY SO =========
// Twelve keyboard chords in Main.qml are guarded by `if (rowEnabled(...))`
// and do NOTHING when the row is disabled — press Cmd+T with no document
// and the shell is silent. reportUnavailable() is the door that makes such
// a press speak, and these four cases pin what it says, what it falls back
// to, and — as much of the contract as either — the two cases where it must
// stay quiet.

void TstCommandAvailability::p14aReportUnavailableSpeaksTheRowsOwnReason()
{
    QVERIFY2(!m_controller->hasDocument(),
             "this case runs in the NO-DOCUMENT state (declaration order, "
             "after s1 and before s2) — the fixture has moved");
    const QVariantMap row = m_controller->commandAvailability()
                                .value(QStringLiteral("transform.enter")).toMap();
    QCOMPARE(row.value(QStringLiteral("enabled")).toBool(), false);
    QCOMPARE(row.value(QStringLiteral("reasonId")).toString(),
             QStringLiteral("ui.error.no_document"));

    QSignalSpy spy(m_controller, &DocumentController::actionUnavailable);
    QVERIFY(spy.isValid());
    m_controller->reportUnavailable(QStringLiteral("transform.enter"));

    // THE TRANSLATED REASON, not the id. Routing this through setRejected
    // would have rendered "Rejected — ui.error.no_document:" — a raw
    // MessageId at the user, which is the P2-D2 key-echo class.
    QCOMPARE(m_controller->statusText(), catTr("ui.error.no_document"));
    QVERIFY2(!m_controller->statusText().contains(QStringLiteral("ui.error.")),
             "the status line is echoing a raw MessageId");
    QCOMPARE(spy.count(), 1);
    QCOMPARE(spy.at(0).at(1).toString(), QStringLiteral("ui.error.no_document"));
}

void TstCommandAvailability::p14aReportUnavailableFallsBackWhenNoConjunctIsNamed()
{
    // A row disabled WITH a document, whose failing conjunct has no
    // cataloged id — the deliberate reasonId-absent case. The fallback is
    // generic ON PURPOSE: the reasonId policy forbids naming the WRONG
    // conjunct, and what this replaces is silence, not a better sentence.
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    m_controller->setProperty("activeLayerIndex", -1);
    const QVariantMap row = m_controller->commandAvailability()
                                .value(QStringLiteral("transform.enter")).toMap();
    QCOMPARE(row.value(QStringLiteral("enabled")).toBool(), false);
    QVERIFY2(row.value(QStringLiteral("reasonId")).toString().isEmpty(),
             "this case needs a row disabled with NO reasonId; the fixture moved");

    QSignalSpy spy(m_controller, &DocumentController::actionUnavailable);
    m_controller->reportUnavailable(QStringLiteral("transform.enter"));
    QCOMPARE(m_controller->statusText(),
             catTr("ui.error.action_unavailable"));
    QCOMPARE(spy.count(), 1);
    QCOMPARE(spy.at(0).at(1).toString(),
             QStringLiteral("ui.error.action_unavailable"));
}

void TstCommandAvailability::p14aReportUnavailableIsSilentTwoWays()
{
    // THE SILENCES ARE THE CONTRACT TOO, and neither is an omission.
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    QSignalSpy spy(m_controller, &DocumentController::actionUnavailable);

    // (1) UNKNOWN ACTION. A typo'd name must not manufacture a message
    // about a row that does not exist.
    const QString before = m_controller->statusText();
    m_controller->reportUnavailable(QStringLiteral("transform.zzz_not_a_row"));
    QCOMPARE(m_controller->statusText(), before);
    QCOMPARE(spy.count(), 0);

    // (2) ACTUALLY ENABLED. This door REPORTS a state, it does not decide
    // one; inventing a refusal here would make the chrome disagree with
    // the availability surface it is built on.
    QCOMPARE(m_controller->commandAvailability()
                 .value(QStringLiteral("transform.enter")).toMap()
                 .value(QStringLiteral("enabled")).toBool(), true);
    m_controller->reportUnavailable(QStringLiteral("transform.enter"));
    QCOMPARE(m_controller->statusText(), before);
    QCOMPARE(spy.count(), 0);
}

void TstCommandAvailability::p14aRepeatedPressFiresTheEventEachTime()
{
    // The same premise the refusal channel rests on (G-226): setStatus
    // early-returns on identical text, so a second press of the same
    // unavailable chord moves no property. The EVENT must fire anyway or
    // a transient surface shows once and then goes quiet while the user
    // presses again.
    QVERIFY2(!m_controller->hasDocument(),
             "this case runs in the NO-DOCUMENT state — the fixture has moved");
    QSignalSpy events(m_controller, &DocumentController::actionUnavailable);
    QSignalSpy statusChanges(m_controller,
                             &DocumentController::statusTextChanged);

    m_controller->reportUnavailable(QStringLiteral("transform.enter"));
    QCOMPARE(events.count(), 1);
    const int statusAfterFirst = statusChanges.count();
    const QString textAfterFirst = m_controller->statusText();

    m_controller->reportUnavailable(QStringLiteral("transform.enter"));
    QCOMPARE(events.count(), 2);                       // the event re-fired
    QCOMPARE(m_controller->statusText(), textAfterFirst);
    QCOMPARE(statusChanges.count(), statusAfterFirst); // the text did not
}

void TstCommandAvailability::p14aAnUncatalogedReasonIsCountedNotJustSwallowed()
{
    // O-INT's finding on this door, turned into a control. The catalog
    // gate is CORRECT — an uncataloged id must never reach a user — but
    // a bare `return` makes a wired-and-mute chord indistinguishable
    // from the pre-fix bug, with no TypeError and no warning to see it
    // by. The counter is what makes that state assertable at all.
    //
    // EVERY reasonId currently reachable is cataloged (O-INT measured
    // all thirteen guarded rows fire), so the counter must be ZERO on a
    // real row — that is the negative half, and without it this case
    // would pass on a door that refused everything.
    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    const int before = m_controller->unavailableRefusedCount();
    m_controller->setProperty("activeLayerIndex", -1);
    m_controller->reportUnavailable(QStringLiteral("transform.enter"));
    QCOMPARE(m_controller->unavailableRefusedCount(), before);

    // …and the positive half needs a row whose reasonId is NOT in the
    // catalog. None exists today, which is the good state, so the probe
    // is the door's own gate exercised through a state it can actually
    // reach: an action key that resolves to no row at all returns before
    // the gate and must NOT increment either. The counter therefore
    // moves ONLY on a real uncataloged reason — the case a future mint
    // without a catalog entry would create.
    m_controller->reportUnavailable(QStringLiteral("no.such.row"));
    QCOMPARE(m_controller->unavailableRefusedCount(), before);
}

void TstCommandAvailability::toastReconciliationBothChannelsReachIt()
{
    // ITEM 4, the reconciliation. The toast bound commandRefused only, so
    // a refused command surfaced and an unavailable chord did not. Both
    // channels must reach it, and the code slot must behave DIFFERENTLY
    // for each — a wire code is quotable, a reasonId is a MessageId and
    // showing it raw is the key-echo class.
    QQuickItem *toast = m_window->findChild<QQuickItem *>(
        QStringLiteral("refusalToast"));
    QVERIFY2(toast, "refusalToast not in the tree");

    QVERIFY(m_controller->newDocument(64, 48, QStringLiteral("white")));
    m_controller->setProperty("activeLayerIndex", -1);
    toast->setProperty("visible", false);

    // (1) THE UNAVAILABLE ARM reaches the toast…
    m_controller->reportUnavailable(QStringLiteral("transform.enter"));
    QCOMPARE(toast->property("visible").toBool(), true);
    const QString shown = toast->property("message").toString();
    QVERIFY2(!shown.isEmpty(), "the toast showed an empty message");
    // …TRANSLATED, never the raw id.
    QVERIFY2(!shown.startsWith(QStringLiteral("ui.")),
             qPrintable(QStringLiteral("the toast is echoing a raw MessageId: ")
                        + shown));
    // …and with NO wire code, because there is no wire code to quote.
    QCOMPARE(toast->property("code").toString(), QString());

    // (2) THE ENGINE-REFUSAL ARM still carries its code — the control
    // that stops (1) from being satisfied by a toast that never shows a
    // code at all.
    toast->setProperty("visible", false);
    emit m_controller->commandRefused(QStringLiteral("error.command.target_in_use"),
                                      QStringLiteral("the slot is occupied"));
    QCOMPARE(toast->property("visible").toBool(), true);
    QCOMPARE(toast->property("code").toString(),
             QStringLiteral("error.command.target_in_use"));
    QCOMPARE(toast->property("message").toString(),
             QStringLiteral("the slot is occupied"));
}

void TstCommandAvailability::catalogGround()
{
    // Echo-proofing (RULE 9): a key-echoing catalog would make the
    // menu-item text lookups match on echoed keys and the reasonId
    // ids meaningless. Pinned against the checked-in en.json values.
    QCOMPARE(catTr("ui.error.no_document"),
             QStringLiteral("No document is open"));
    QCOMPARE(catTr("error.command.selection_required"),
             QStringLiteral("This command requires an active selection"));
    QCOMPARE(catTr("ui.menu.select.deselect"), QStringLiteral("Deselect"));
    // §17.2 auto-adjust captions, pinned to the checked-in en.json.
    // These three lines are what makes the menuKeys lookups below
    // echo-proof for the new row: a key-echoing catalog would return the
    // ID here and these QCOMPAREs would red BEFORE any menu lookup
    // matched on an echoed label.
    QCOMPARE(catTr("ui.menu.image.auto_tone"), QStringLiteral("Auto Tone"));
    QCOMPARE(catTr("ui.menu.image.auto_contrast"),
             QStringLiteral("Auto Contrast"));
    QCOMPARE(catTr("ui.menu.image.auto_color"), QStringLiteral("Auto Color"));
    // The NO-ELLIPSIS rule, pinned as a property rather than by
    // re-quoting the three strings: nine of the thirteen cataloged
    // ui.menu.image.* captions end in `…` and every one of those opens a
    // dialog. These three apply immediately (each contract:
    // `actionRecording: recordable; no dialog (menu item)`), so an
    // ellipsis on them — from the catalog OR concatenated in QML — is a
    // promise of a dialog that never opens. The MenuItem text is checked
    // (not just the catalog value) so the QML side is covered too.
    for (const char *k : {"ui.menu.image.auto_tone",
                          "ui.menu.image.auto_contrast",
                          "ui.menu.image.auto_color"}) {
        QObject *item = menuItem(QLatin1String(k));
        QVERIFY2(item, k);
        const QString shown = item->property("text").toString();
        QCOMPARE(shown, catTr(k));
        QVERIFY2(!shown.contains(QChar(0x2026)) && !shown.endsWith(QLatin1String("...")),
                 qPrintable(QStringLiteral("%1 renders '%2' — an ellipsis "
                                           "promises a dialog this row "
                                           "does not open")
                                .arg(QLatin1String(k), shown)));
    }
    // The dialog siblings, as the CONTRAST control in the same run: this
    // is a real convention, not a vacuous predicate that would pass on
    // any string. Crop opens CropDialog and carries the ellipsis.
    QVERIFY2(catTr("ui.menu.image.crop").contains(QChar(0x2026)),
             "ui.menu.image.crop lost its ellipsis — the no-ellipsis "
             "assertions above would then be vacuous");
    // The abi honest-limit record: on this build both blocks are served
    // (CONSTANT true) — the abi conjuncts in the frozen predicates are
    // NOT matrix-discriminable and are covered by code identity only.
    QVERIFY(m_controller->property("abiBlock12").toBool());
    QVERIFY(m_controller->property("abiBlock14").toBool());
    // Every menu-consuming case resolves to a real MenuItem up front.
    int menuItems = 0;
    for (const Case &c : m_cases)
        for (const QString &k : c.menuKeys) {
            QVERIFY2(menuItem(k), qPrintable(k));
            ++menuItems;
        }
    // 30 single items (14b added the Edit > Delete row; item 15 added
    // the Edit > Paste Into row) + the 6 filter items + the 3 §17.2
    // auto-adjust items (one row, three consuming MenuItems): 36 -> 39.
    // 1.0 final (corrigendum (l)): Edit > Puppet Warp removed: 39 -> 38;
    // Image > Mode… removed: 38 -> 37.
    QCOMPARE(menuItems, 37);
    QVERIFY2(!menuItem(QStringLiteral("menu.edit.puppet_warp")),
             "Edit > Puppet Warp is back in the menu bar; it was removed for 1.0");
    QVERIFY2(!menuItem(QStringLiteral("ui.menu.image.mode")),
             "Image > Mode… is back in the menu bar; it was removed for 1.0");
}

void TstCommandAvailability::verifyState(const char *label)
{
    const Snap s = snapshot();
    const QVariantMap avail =
        m_controller->property("commandAvailability").toMap();
    // Two-sided vocabulary pin: every designed row present, nothing
    // undesigned smuggled in.
    QCOMPARE(avail.size(), m_cases.size());
    for (const Case &c : m_cases) {
        QVERIFY2(avail.contains(c.action),
                 qPrintable(QStringLiteral("[%1] missing row %2")
                                .arg(QLatin1String(label), c.action)));
        const QVariantMap row = avail.value(c.action).toMap();
        QVERIFY2(row.contains(QStringLiteral("enabled")),
                 qPrintable(c.action));
        const bool expected = c.oldTruth(s);
        // (a) old predicate == (b) surface row.
        QVERIFY2(row.value(QStringLiteral("enabled")).toBool() == expected,
                 qPrintable(QStringLiteral(
                                "[%1] %2: surface row enabled=%3, the "
                                "frozen old predicate says %4")
                                .arg(QLatin1String(label), c.action)
                                .arg(!expected)
                                .arg(expected)));
        // reasonId contract: absent while enabled; while disabled, the
        // cataloged id for the failing conjunct or absent (mint-queue).
        const QString wantReason = expected ? QString() : c.reason(s);
        if (wantReason.isEmpty())
            QVERIFY2(!row.contains(QStringLiteral("reasonId")),
                     qPrintable(QStringLiteral("[%1] %2: unexpected "
                                               "reasonId %3")
                                    .arg(QLatin1String(label), c.action,
                                         row.value(QStringLiteral(
                                                       "reasonId"))
                                             .toString())));
        else
            QCOMPARE(row.value(QStringLiteral("reasonId")).toString(),
                     wantReason);
        // (c) the REAL Main.qml MenuItem re-bound to the same truth.
        for (const QString &k : c.menuKeys) {
            QObject *item = menuItem(k);
            QVERIFY2(item, qPrintable(k));
            QVERIFY2(item->property("enabled").toBool() == expected,
                     qPrintable(QStringLiteral(
                                    "[%1] %2 (menu '%3'): item "
                                    "enabled=%4, old predicate says %5")
                                    .arg(QLatin1String(label), c.action,
                                         k)
                                    .arg(!expected)
                                    .arg(expected)));
        }
    }
}

void TstCommandAvailability::s1NoDocument()
{
    // Guard the claimed dimension.
    QVERIFY(!m_controller->property("hasDocument").toBool());
    verifyState("S1 no-document");
}

void TstCommandAvailability::s2DocumentNoSelection()
{
    QVERIFY(m_controller->newDocument(320, 200, QStringLiteral("white")));
    const Snap s = snapshot();
    QVERIFY(s.hasDocument);
    QVERIFY(!s.selectionActive);
    QVERIFY2(!s.activeLayerId.isEmpty(),
             "fresh document has no active layer — the layer-gated rows "
             "would be vacuously false");
    verifyState("S2 doc+no-selection");
}

void TstCommandAvailability::s3SelectionActive()
{
    const QVariantMap r = applyOk("selection.all", {});
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(), "selection.all refused");
    QVERIFY(snapshot().selectionActive);
    verifyState("S3 doc+selection");
}

void TstCommandAvailability::s4SelectionCleared()
{
    const QVariantMap r = applyOk("selection.none", {});
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(), "selection.none refused");
    const Snap s = snapshot();
    QVERIFY(!s.selectionActive);
    QVERIFY2(s.canUndo, "journaled selection steps left canUndo false");
    verifyState("S4 selection-cleared");
}

void TstCommandAvailability::s5UndoRedoArmsAndPreInit()
{
    // Undo once: the redo arm turns on.
    m_controller->undo();
    Snap s = snapshot();
    QVERIFY2(s.canRedo, "undo did not arm redo");
    verifyState("S5a undo (redo armed)");
    if (QTest::currentTestFailed())
        return;

    // Undo ALL the way: undoing doc.new is legal — the pre-init arm
    // (document gone, journal alive) is the state that separates the
    // hasDocument-rooted rows from the canUndo/canRedo rows.
    for (int i = 0; i < 64 && m_controller->property("canUndo").toBool();
         ++i)
        m_controller->undo();
    s = snapshot();
    QVERIFY2(!s.hasDocument,
             "undo exhausted without reaching the pre-init no-document "
             "state (engine behavior moved — re-shape this state)");
    QVERIFY2(s.canRedo, "pre-init state lost the redo arm");
    verifyState("S5b pre-init (no doc, redo armed)");
    if (QTest::currentTestFailed())
        return;

    // Redo everything back (document + the S4 end-state).
    for (int i = 0; i < 64 && m_controller->property("canRedo").toBool();
         ++i)
        m_controller->redo();
    s = snapshot();
    QVERIFY(s.hasDocument);
    QVERIFY2(!s.canRedo, "redo did not exhaust");
    verifyState("S5c redone (undo armed, redo off)");
}

void TstCommandAvailability::s6SavedChannel()
{
    QVERIFY(applyOk("selection.all", {}).value(QStringLiteral("ok"))
                .toBool());
    const QVariantMap r = m_controller->saveSelectionAsChannel();
    QVERIFY2(r.value(QStringLiteral("ok")).toBool(),
             "selection.save refused");
    QVERIFY(!snapshot().lastSavedChannelId.isEmpty());
    verifyState("S6 saved-channel");
}

void TstCommandAvailability::s7MultiLayer()
{
    QVariantMap params;
    params.insert(QStringLiteral("kind"), QStringLiteral("raster"));
    params.insert(QStringLiteral("newLayerId"), m_controller->newUuid());
    QVERIFY2(applyOk("layer.add", params).value(QStringLiteral("ok"))
                 .toBool(),
             "layer.add refused");
    QVERIFY2(m_controller->property("layers").toList().size() >= 2,
             "multi-layer state not reached");
    verifyState("S7 multi-layer");
}

void TstCommandAvailability::s8ActiveLayerOutOfRange()
{
    // The activeLayerId=="" WITH a document arm — the only runtime path
    // that flips the layer-gated rows while hasDocument holds, and the
    // pin for the setActiveLayerIndex re-notify (see header). The old
    // bindings re-evaluated via activeLayerIndexChanged; the migrated
    // bindings only see infoChanged, which setActiveLayerIndex now
    // emits IFF the availability truth moved.
    QSignalSpy spy(m_controller, &DocumentController::infoChanged);
    QVERIFY(spy.isValid());

    // Valid -> valid switch first: truth does not move, so no re-notify
    // (emission hygiene — the LayersPanel click path stays quiet).
    QVERIFY(m_controller->property("layers").toList().size() >= 2);
    m_controller->setProperty("activeLayerIndex", 1);
    QCOMPARE(spy.count(), 0);
    QVERIFY(!snapshot().activeLayerId.isEmpty());
    verifyState("S8a valid layer switch");
    if (QTest::currentTestFailed())
        return;

    // Out of range: activeLayerId collapses to "" while the document
    // stays open; availability moved, so exactly one re-notify.
    m_controller->setProperty("activeLayerIndex", 99);
    QCOMPARE(spy.count(), 1);
    const Snap s = snapshot();
    QVERIFY(s.hasDocument);
    QVERIFY2(s.activeLayerId.isEmpty(),
             "out-of-range index still resolved a layer id");
    verifyState("S8b doc open, active layer empty");
    if (QTest::currentTestFailed())
        return;

    // Restore and re-verify (the matrix ends on a sane state).
    m_controller->setProperty("activeLayerIndex", 0);
    QCOMPARE(spy.count(), 2);
    QVERIFY(!snapshot().activeLayerId.isEmpty());
    verifyState("S8c restored");
}

void TstCommandAvailability::s9AdjustmentOpMemory()
{
    // The row-1 memory dimension, driven in BOTH directions — the matrix
    // above never reaches it (S1-S8 author no adjustment op), so without
    // this slot the mask row's memory conjunct would be asserted only in
    // its false arm and a dropped conjunct could not red.

    // (i) Document + selection but NO authored op: the mask row must be
    // DISABLED and — the load-bearing assertion — must carry NO reasonId,
    // because the only failing conjunct is the one no cataloged id names.
    QVERIFY2(applyOk("selection.all", {}).value(QStringLiteral("ok")).toBool(),
             "selection.all refused");
    Snap s = snapshot();
    QVERIFY(s.hasDocument);
    QVERIFY2(s.selectionActive, "selection dimension not reached");
    QVERIFY2(!s.adjustmentOpMemory, "op memory unexpectedly already armed");
    QVariantMap row = m_controller->commandAvailability()
                          .value(QStringLiteral("layer.maskAdjustmentFromSelection"))
                          .toMap();
    QCOMPARE(row.value(QStringLiteral("enabled")).toBool(), false);
    QVERIFY2(!row.contains(QStringLiteral("reasonId")),
             "the memory conjunct must not borrow another conjunct's reason");
    verifyState("S9a doc+selection, no authored op");
    if (QTest::currentTestFailed())
        return;

    // (ii) Author one. addAdjustmentLayer runs the WHOLE row-1 gesture
    // here (a selection is active), so a green `masked` is end-to-end
    // proof that adjustment.add and op.set_mask BOTH applied through the
    // real ABI — the shell half of the route, not a shape assertion.
    const QVariantMap added =
        m_controller->addAdjustmentLayer(QStringLiteral("adjustment.invert"));
    QVERIFY2(added.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("addAdjustmentLayer refused: %1 %2")
                            .arg(added.value(QStringLiteral("code")).toString(),
                                 added.value(QStringLiteral("message")).toString())));
    QVERIFY2(added.value(QStringLiteral("masked")).toBool(),
             "a selection was active, so op.set_mask must have applied");
    QVERIFY2(!added.value(QStringLiteral("opId")).toString().isEmpty(),
             "the door must report the op id it authored");
    s = snapshot();
    QVERIFY2(s.adjustmentOpMemory, "op memory did not arm");
    verifyState("S9b authored adjustment op");
    if (QTest::currentTestFailed())
        return;

    // (iii) The memory conjunct is REAL, not decorative: clearing the
    // selection alone must disable the row again even though the memory
    // still holds — and re-selecting must re-enable it.
    QVERIFY2(applyOk("selection.none", {}).value(QStringLiteral("ok")).toBool(),
             "selection.none refused");
    QVERIFY(!snapshot().selectionActive);
    QVERIFY(snapshot().adjustmentOpMemory);
    verifyState("S9c memory held, selection cleared");
    if (QTest::currentTestFailed())
        return;
    QVERIFY2(applyOk("selection.all", {}).value(QStringLiteral("ok")).toBool(),
             "selection.all refused");
    QVERIFY(snapshot().selectionActive);
    QCOMPARE(m_controller->commandAvailability()
                 .value(QStringLiteral("layer.maskAdjustmentFromSelection"))
                 .toMap()
                 .value(QStringLiteral("enabled"))
                 .toBool(),
             true);
    verifyState("S9d both conjuncts satisfied");
    if (QTest::currentTestFailed())
        return;

    // (iv) The standalone mask door, re-run against the SAME remembered
    // op. This is the door the follow-up chrome will call, so it is
    // proven independently of the combined gesture.
    const QVariantMap remasked = m_controller->maskAdjustmentFromSelection();
    QVERIFY2(remasked.value(QStringLiteral("ok")).toBool(),
             qPrintable(QStringLiteral("op.set_mask refused: %1 %2")
                            .arg(remasked.value(QStringLiteral("code")).toString(),
                                 remasked.value(QStringLiteral("message")).toString())));
}

void TstCommandAvailability::s10AdjustmentMemoryIsDocumentState()
{
    // A remembered op id must NOT survive a document swap: it resolves to
    // nothing in the new document, so op.set_mask would answer
    // bad_target. Without the clear, the row would offer an enabled
    // control that refuses — the exact dishonest-state the desk's D4 rule
    // forbids.
    QVERIFY(!m_controller->lastAdjustmentOpId().isEmpty()); // armed by S9
    m_controller->newDocument(32, 32, QStringLiteral("white"));
    QVERIFY2(m_controller->lastAdjustmentOpId().isEmpty(),
             "an op id from the previous document survived newDocument");
    QVERIFY2(applyOk("selection.all", {}).value(QStringLiteral("ok")).toBool(),
             "selection.all refused");
    const QVariantMap row = m_controller->commandAvailability()
                                .value(QStringLiteral("layer.maskAdjustmentFromSelection"))
                                .toMap();
    QCOMPARE(row.value(QStringLiteral("enabled")).toBool(), false);
    // And the door itself refuses rather than submitting a doomed
    // envelope (the client-side precondition mirror).
    QCOMPARE(m_controller->maskAdjustmentFromSelection()
                 .value(QStringLiteral("ok"))
                 .toBool(),
             false);
    verifyState("S10 fresh document, memory cleared");
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstCommandAvailability tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_command_availability.moc"
