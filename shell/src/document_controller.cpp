// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header. Every visible string below goes through the catalog
// (key-echo until O-I18N-A11Y catalogs land). Minted ui.* keys are listed
// in reports/O-UI-log.md for the O-I18N upstream sweep.

#include "document_controller.h"

#include "clipboard_ops.h"
#include "font_meta.h"
#include "selection_scan.h"

#include <QBuffer>
#include <QClipboard>
#include <QDir>
#include <QFile>
#include <QImageReader>
#include <QFileInfo>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QPainter>
#include <QRandomGenerator>
#include <QSet>
#include <QUrl>

#include <algorithm>
#include <cmath>
#include <utility>

namespace appname::shell {

namespace {

QString trArgs(const Catalog *catalog,
               const QString &key,
               std::initializer_list<QPair<QString, QString>> args)
{
    return catalog->tr(key, QVector<QPair<QString, QString>>(args));
}

// Repaint-scoping slice (sweep F1a): the share cap for the per-revision
// full-doc selection mask (selectionMaskFull) — 2^24 px (16.7 Mpx, 16 MB
// Grayscale8 retained worst-case; covers 4096^2 with headroom, and the
// whole sweep size axis). Past it the announcer keeps its strip scan and
// the ants take direct viewport-window reads.
constexpr qint64 kSharedMaskCapPx = qint64(1) << 24;

// The F1 revision gate's KIND side: the engine-derived SELECTION-WRITER
// set — every command kind whose impl at f4e7bdc writes the selection
// surface (RULE 8, enumerated from the mutation doors' call sites, never
// guessed from names):
//  - combine_and_install (appname-tools kind_impls.rs:5131-5135):
//    selection.set/.modify/.transform/.all/.none/.invert;
//  - SelectionLoadKind (kinds_channels.rs:358-365) selection.load;
//  - SelectionFromPathKind (kinds_vector.rs:933) selection.from_path;
//  - MaskToSelectionKind (kinds.rs:1250-1274) mask.to_selection;
//  - ChannelCalculationsKind "selection" result arm
//    (kinds_apply_image.rs:818-830) — conditional write,
//    conservative-included;
//  - the doc geometry family via map_all_surfaces over ALL raster
//    surfaces incl. the selection (kind_impls.rs:3833/:3926):
//    doc.resize/.canvas_resize/.crop/.trim/.rotate —
//    conservative-included (some arms leave the mask unchanged).
// Verified NON-writers stay out: selection.save (reads only),
// mode/depth/profile kinds ("masks/channels/selection NEVER converted",
// kinds_mode.rs:5), image.flatten, fill/edit.cut/strokes/layer.transform
// (the selection GATES those — read-only). history.v1 events carry the
// STEP's kind on apply, undo, AND redo (cabi doc.rs:249/:312/:341), so
// undo/redo of a writer kind gates identically.
//
// THE DAGGER/REPLAY MEMBER — history.goto (§5 fix round, MAJOR-2):
// history.goto is NOT a selection-writer impl; it is here DELIBERATELY,
// as an over-inclusion, and the reason is that the host cannot see what
// it did. Its position form (engine mod.rs:1050-1135) runs an internal
// undo/redo run and DISCARDS the replayed rows' kinds (only their dirty
// rects survive); its snapshot form replaces the whole document
// (mod.rs:1186), selection surface included. Either way the cabi emits
// exactly ONE history.v1 row labelled "history.goto" (doc.rs:249 =
// receipt.kind) — the kinds of the steps it replayed NEVER reach the
// shell. A goto can therefore replay or rewind ANY step, selection
// writers included, behind a label that names none of them. It must be
// treated as an unconditional potential selection writer.
// THE TRADE, taken deliberately: a jump that touched no selection costs
// one redundant shared mask read + one overlay repaint — bounded (one
// read per goto) and rare (goto is a history-UI jump, not a hot path).
// That is chosen OVER the alternative of missing a real content change,
// which shows as stale marching ants over a changed selection plus an
// unbusted shared mask cache (reviewer-reproduced, twice).
//
// COMPLETENESS — CORRECTED (§5 fix round; the claim that stood here
// before was FALSE as written and is not being silently reworded).
// It read: "a selection change with identical (active, bounds) can only
// come from a writer impl (code truth)". history.goto is the
// counterexample: undo of selection.invert over two corner dots changes
// every covered pixel while leaving `active` and the tight nonzero
// bounds identical, and it arrives labelled "history.goto" — outside
// the writer set as that set was originally derived. Arm (a) missed it
// and arm (b) is blind by construction in the identical-bounds case.
// The CORRECTED claim, which holds only GIVEN history.goto's
// unconditional inclusion above: the two-sided union is complete
// because every kind that can move the selection at all is either (i) a
// writer impl — enumerated above, re-derived over all 102 registered
// kinds — or (ii) a replay/dagger kind that hides the kinds it replays,
// of which history.goto is the only one in this tree (history.snapshot
// appends a marker and writes no surface, mod.rs:970; doc.duplicate has
// no engine arm), and BOTH classes are in the set. Whatever remains
// moves (active, bounds) and the refreshInfo compare side catches it.
// MAINTENANCE: re-derive this list when the engine grows kinds, and
// classify each new kind against BOTH classes — a new replay/dagger
// kind belongs here even though it writes nothing itself. The compare
// side keeps bounds-moving changes safe meanwhile.
bool isSelectionWriterKind(const QString &kind)
{
    static const QSet<QString> kWriters = {
        QStringLiteral("selection.set"),
        QStringLiteral("selection.modify"),
        QStringLiteral("selection.transform"),
        QStringLiteral("selection.all"),
        QStringLiteral("selection.none"),
        QStringLiteral("selection.invert"),
        QStringLiteral("selection.load"),
        QStringLiteral("selection.from_path"),
        QStringLiteral("mask.to_selection"),
        QStringLiteral("channel.calculations"),
        QStringLiteral("doc.resize"),
        QStringLiteral("doc.canvas_resize"),
        QStringLiteral("doc.crop"),
        QStringLiteral("doc.trim"),
        QStringLiteral("doc.rotate"),
        // Replay/dagger member — writes nothing itself, hides what it
        // replayed. See THE DAGGER/REPLAY MEMBER above.
        QStringLiteral("history.goto"),
    };
    return kWriters.contains(kind);
}

} // namespace

DocumentController::DocumentController(const Catalog *catalog, QObject *parent)
    : QObject(parent)
    , m_catalog(catalog)
{
    // Idle event-loop poll (plan row: timer or after each ABI call — both:
    // pollNow() also runs after every mutating call).
    m_pollTimer.setInterval(50);
    connect(&m_pollTimer, &QTimer::timeout, this, &DocumentController::pollNow);
    setStatus(m_catalog->tr(QStringLiteral("ui.status.ready")));
}

void DocumentController::setDeterministicIds(quint64 seed)
{
    m_deterministic = true;
    m_uuids = UuidSource(seed);
    m_strokeSeedCounter = seed;
}

void DocumentController::setActiveLayerIndex(int index)
{
    if (index == m_activeLayerIndex)
        return;
    // Item 14a behavior-identity: the pre-migration menu predicates
    // bound root.activeLayerId (= AppDoc.layers + AppDoc.activeLayerIndex)
    // and therefore re-evaluated on THIS signal too, not only on
    // infoChanged. commandAvailability notifies on infoChanged alone, so
    // an index move that flips the active-layer truth (in/out of the
    // valid range — the only way activeLayerId() flips here) must
    // re-notify availability consumers. Conditional: switching between
    // two valid layers changes no row, so no extra emission rides the
    // common LayersPanel click.
    const QVariantMap before = commandAvailability();
    m_activeLayerIndex = index;
    emit activeLayerIndexChanged();
    if (commandAvailability() != before)
        emit infoChanged();
}

bool DocumentController::documentHasUnsavedWork() const
{
    if (!m_hasDocument)
        return false;
    // Saved at least once: work is any departure from that baseline. Undo
    // back onto it counts as none, which is the property tst_dirty_flag's
    // discriminator pins.
    if (m_savedCursor >= 0)
        return m_journalCursor != m_savedCursor;
    // NEVER SAVED, so there is no baseline -- and the floor is doc.new
    // itself, which is not the user's work. The ENGINE says the same
    // thing in its own words at appname-cli main.rs: cursor <= 1 is
    // "nothing to undo (doc.new cannot be undone across the package
    // boundary)". A pristine untitled document therefore has nothing to
    // lose, and confirming over it is the cry-wolf failure that trains a
    // user to click through the one dialog that mattered.
    return m_journalCursor > 1;
}

bool DocumentController::documentDirty() const
{
    // THE !m_hasDocument GUARD IS NOT DEFENSIVE PADDING -- it preserves
    // the exact reachability the expression had as a local. windowTitle()
    // returns "Appname" before ever evaluating dirtiness, so the
    // computation was only ever reached WITH a document. Exposed as a
    // property it is reachable with none, and m_documentPath is empty
    // then, so the second disjunct alone would report a nonexistent
    // document as having unsaved changes -- and every door reading this
    // would confirm-on-nothing.
    if (!m_hasDocument)
        return false;
    // Unchanged semantics, moved verbatim: the journal cursor has moved
    // off the baseline captured at open/save, OR the document has no path
    // and therefore has never been saved anywhere.
    return m_journalCursor != m_savedCursor || m_documentPath.isEmpty();
}

QString DocumentController::windowTitle() const
{
    // "Appname" is the D-43 product placeholder (a mark, not a message);
    // the composition template is localizable. The dirty-marker prefix is
    // an ACCEPTED LIMITATION (review P4-D4 ruling, lane log): the correct
    // form is a cataloged composition template (ui.window.title_dirty =
    // "\u25CF {title}" — QUEUED with O-I18N so locales control glyph and
    // placement); until that lands, the hardcoded LTR prefix stands,
    // logged — NOT justified by the arrow precedent, which rides catalog
    // VALUES. dirty = journal cursor moved since the last save/open
    // baseline, or a never-saved document.
    if (!m_hasDocument)
        return QStringLiteral("Appname");
    // RC3 (operator WORD 36 (b)): {doc} names the saved document; "Untitled"
    // only until it has a path.
    const QString base = trArgs(m_catalog, QStringLiteral("ui.window.title"),
                                {{QStringLiteral("doc"),
                                  m_documentPath.isEmpty()
                                      ? m_catalog->tr(QStringLiteral("app.window.untitled"))
                                      : QFileInfo(m_documentPath).completeBaseName()}});
    const bool dirty = documentDirty();
    return dirty ? QStringLiteral("\u25CF ") + base : base;
}

bool DocumentController::closeDocument()
{
    if (!m_doc.isOpen())
        return false;
    m_pollTimer.stop();
    m_doc.close();
    m_hasDocument = false;
    m_savedCursor = -1; // no document, no saved baseline (see newDocument)
    if (!m_documentPath.isEmpty()) {
        m_documentPath.clear();
        emit documentPathChanged();
    }
    // The same per-document memories newDocument() clears (see there).
    m_pendingRecoveryLog.clear();
    m_lastSavedChannelId.clear();
    m_lastSelectionSetParams.clear();
    m_hasSelectionMemory = false;
    m_lastAdjustmentOpId.clear();
    ++m_selectionRevision;
    m_selectionMaskCache = QImage();
    m_selectionMaskCacheRevision = -1;
    m_activeLayerIndex = 0;
    emit activeLayerIndexChanged();
    refreshInfo(); // a closed handle takes refreshInfo's no-document arm
    m_rootLayerId.clear();
    refreshHistory();
    emit documentReset();
    return true;
}

bool DocumentController::newDocument(int width, int height, const QString &background)
{
    // Fresh pre-init handle (a doc handle accepts doc.new exactly once).
    const CallResult created = m_doc.create();
    if (!created.ok) {
        setStatusFromResult(QStringLiteral("doc.new"), created);
        return false;
    }
    const QByteArray envelope =
        buildDocNewEnvelope(m_uuids.next(), width, height, background);
    const CallResult applied = m_doc.apply(envelope);
    setStatusFromResult(QStringLiteral("doc.new"), applied);
    if (!applied.ok) {
        m_doc.close();
        m_hasDocument = false;
        emit infoChanged();
        return false;
    }
    // RC3 D-3: a fresh document has never been saved, so the saved-cursor
    // baseline of the PREVIOUS document must not survive into it. It used to:
    // save document A (savedCursor = N), then New -- the new document was
    // compared against A's cursor, so a pristine document read as unsaved
    // (a false prompt) or, where the cursors coincided, real unsaved work
    // read as clean and a close lost it with no prompt. -1 = never saved:
    // documentHasUnsavedWork falls back to "journal past doc.new".
    m_savedCursor = -1;
    // A fresh document has no disk identity: a stale path here would make
    // plain Save silently overwrite the previously opened file (review
    // R1-chrome, MAJOR).
    if (!m_documentPath.isEmpty()) {
        m_documentPath.clear();
        emit documentPathChanged();
    }
    // Recovery tracking is only meaningful while the RECOVERED document is
    // the current one — a stale path here would let a later same-path save
    // silently delete an unresolved crash log (review P6-D1, MAJOR).
    m_pendingRecoveryLog.clear();
    m_lastSavedChannelId.clear(); // cross-document handle (see openDocument)
    // 14b: the reselect memory is document state — a fresh document has
    // no "last selection" (cross-document coordinates would be a lie).
    m_lastSelectionSetParams.clear();
    m_hasSelectionMemory = false;
    // Row 1: the authored-op memory is document state for the same reason
    // — an op id from the previous document resolves to nothing here, and
    // op.set_mask would answer bad_target. Clearing it keeps the
    // availability row honest instead of offering a control that refuses.
    m_lastAdjustmentOpId.clear();
    // Repaint-scoping slice: cross-document mask-cache safety — the
    // per-revision key cannot distinguish documents (a same-size reopen
    // with an identically-bounded selection would serve the OLD
    // document's pixels), so every document arrival bumps + clears.
    ++m_selectionRevision;
    m_selectionMaskCache = QImage();
    m_selectionMaskCacheRevision = -1;
    m_activeLayerIndex = 0;
    emit activeLayerIndexChanged();
    refreshInfo();
    // Root-group id = the receipt-created id that is NOT a layer row
    // (doc.new creates the root group + the background layer; the info
    // walk lists layers only — order-independent extraction, verified
    // against a live receipt/info pair).
    m_rootLayerId.clear();
    QJsonObject receipt = applied.json.value(QLatin1String("receipt")).toObject();
    if (receipt.isEmpty())
        receipt = applied.json;
    for (const QJsonValue &v : receipt.value(QLatin1String("createdIds")).toArray()) {
        const QString id = v.toString();
        bool isLayer = false;
        for (const QVariant &row : std::as_const(m_layers))
            if (row.toMap().value(QStringLiteral("layerId")).toString() == id)
                isLayer = true;
        if (!isLayer) {
            m_rootLayerId = id;
            break;
        }
    }
    refreshHistory();
    emit documentReset();
    pollNow(); // consume the initial dirty event
    provisionSystemFaces(); // ABI 1.6 font feed (per-document FontDb)
    m_pollTimer.start();
    return true;
}

// The kinds whose receipts carry no dirty rects although they change what
// the canvas shows: mask creation and every Mask-panel change (D-1), and the
// structural layer kinds (1.0 final D-9). ONE list, read by applyCommand AND
// by undo()/redo() -- 1.0 final S-10 (O-SIT RC5 sitting, ★): undo/redo do
// not pass through applyCommand, so with the list inline there, an undo of a
// mask change left the canvas showing the undone pixels. history.goto is in
// the set because a panel jump can step back over any of these kinds.
bool DocumentController::needsWholeDocumentRefresh(const QString &kind)
{
    return kind == QLatin1String("mask.add")
        || kind == QLatin1String("mask.set_props")
        || kind == QLatin1String("layer.delete")
        || kind == QLatin1String("layer.merge_down")
        || kind == QLatin1String("layer.reorder")
        || kind == QLatin1String("layer.group")
        || kind == QLatin1String("layer.ungroup")
        || kind == QLatin1String("layer.duplicate")
        || kind == QLatin1String("history.goto");
}

void DocumentController::refreshWholeDocument()
{
    if (m_docWidth > 0 && m_docHeight > 0)
        emit regionsDirty({QRect(0, 0, m_docWidth, m_docHeight)});
}

void DocumentController::undo()
{
    if (!m_doc.isOpen())
        return;
    const CallResult r = m_doc.undo();
    if (r.ok) {
        const QString kind = r.json.value(QLatin1String("kind")).toString();
        const QString labelKey = QStringLiteral("history.") + kind;
        const QString label = m_catalog->tr(labelKey);
        setStatus(trArgs(m_catalog, QStringLiteral("ui.status.undone"),
                         {{QStringLiteral("label"), label}}));
        // A11Y-3 wave B (review D1): the announce id's has() gate cannot
        // see key-echo riding an ARG — an uncataloged history label would
        // be SPOKEN as the raw key ("Undone: history.vector.shape_add",
        // proven live on the tour path). The VISIBLE status line keeps
        // the key-echo (visibly-untranslated is that surface's honest
        // fallback); speech gets silence until the row lands (three
        // shell-emitted kinds queued to O-I18N).
        if (m_announcer && m_catalog->has(labelKey))
            m_announcer->announce(QStringLiteral("history"),
                                  QStringLiteral("a11y.history.undone"),
                                  {{QStringLiteral("label"), label}});
    } else {
        // The only setStatusFromResult kind (with redo()'s below) that is
        // neither a registered command kind nor a cataloged label. Every
        // other caller passes one or the other: applyCommand's kind is
        // refused by the engine before the label frame is ever built if
        // it is unregistered, and doc.new (:203/:209) — also a hand-
        // written literal — is registered AND carries a history.doc.new
        // row in all four canonical catalogs. "undo" has neither: it is
        // none of registry_gen.rs's 102 kinds, and history.undo is absent
        // from i18n/{en,es,ja,pseudo}.json. So on setStatusFromResult's
        // APPNAME_ERR_IO arm this would key-echo the raw id "history.undo"
        // into the status line, and through Main.qml's
        // Accessible.description binding into what a reader speaks off
        // the canvas.
        //
        // That arm is dead today, but NOT because this door is IO-free:
        // Engine::undo/redo DO have an IO arm — revive_history_step()
        // reports a failed history-spill revival as ErrorCode::
        // IoReadFailed, which the cabi maps to APPNAME_ERR_IO. It is dead
        // only because spilling is OPT-IN and nothing on this path opts
        // in: Engine::new leaves both history limits at usize::MAX,
        // maybe_spill returns early while they are, and no cabi/shell
        // caller ever calls set_history_limits — appname-cli's main.rs
        // does, so the same engine under a sibling front-end DOES spill.
        // The guard is that cabi-side omission, not anything in this file
        // and not a property of the undo door.
        //
        // If the cabi ever bounds history (a plausible way to cap shell
        // memory), this key-echo goes live. Do not answer that by minting
        // a history.undo row — there is no such command to name. Take the
        // kind from the receipt as the ok arm above does, or route this
        // arm to setStatusFromResult's rejected frame, which needs no
        // history.* label at all.
        setStatusFromResult(QStringLiteral("undo"), r);
    }
    pollNow();
    // 1.0 final S-10: the same whole-document refresh applyCommand gives
    // these kinds forward (see needsWholeDocumentRefresh).
    if (r.ok && needsWholeDocumentRefresh(r.json.value(QLatin1String("kind")).toString()))
        refreshWholeDocument();
}

void DocumentController::redo()
{
    if (!m_doc.isOpen())
        return;
    const CallResult r = m_doc.redo();
    if (r.ok) {
        const QString kind = r.json.value(QLatin1String("kind")).toString();
        const QString labelKey = QStringLiteral("history.") + kind;
        const QString label = m_catalog->tr(labelKey);
        setStatus(trArgs(m_catalog, QStringLiteral("ui.status.redone"),
                         {{QStringLiteral("label"), label}}));
        // Review D1: same key-echo-through-args gate as undo().
        if (m_announcer && m_catalog->has(labelKey))
            m_announcer->announce(QStringLiteral("history"),
                                  QStringLiteral("a11y.history.redone"),
                                  {{QStringLiteral("label"), label}});
    } else {
        // "redo" is unregistered and uncataloged exactly as "undo" is, and
        // rides the same cabi-side spill guard (Engine::redo carries the
        // same revive_history_step IO arm). Read the note in undo()
        // before touching this line.
        setStatusFromResult(QStringLiteral("redo"), r);
    }
    pollNow();
    // 1.0 final S-10: as undo() above.
    if (r.ok && needsWholeDocumentRefresh(r.json.value(QLatin1String("kind")).toString()))
        refreshWholeDocument();
}

QVariantMap DocumentController::historyGoto(int position)
{
    // Wave F (census class F — the panel-row jump). Params per the
    // registered HistoryGoto.v1 schema (position form; oneOf position|
    // snapshot). No a11y announcement on the jump: no cataloged id fits
    // a multi-step run (a11y.history.undone/redone name ONE step's
    // {label}, goldens pinned in tst_announcer) — honest silence per the
    // announcer policy; a a11y.history.jumped-class id is a recorded
    // mint-queue gap (lane report). The † position form journals AS the
    // equivalent undo/redo run (appends nothing), so the panel refresh
    // below re-reads cursor truth, not a new row.
    QVariantMap params;
    params.insert(QStringLiteral("position"), position);
    return applyCommand(QStringLiteral("history.goto"), {}, params, {});
}

QVariantMap DocumentController::applyCommand(const QString &kind,
                                             const QVariantMap &target,
                                             const QVariantMap &params,
                                             const QString &mergeKey)
{
    // §5 tier-2 fix round (review minor-2): count the ATTEMPT at entry,
    // before any guard or engine answer — wiring truth, not outcome.
    ++m_submissionAttempts;
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (!m_doc.isOpen()) {
        // Client-side precondition, NOT an engine code — ui.* namespace so
        // the two error surfaces can never be confused.
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    // 1.0 final F-10: armed BEFORE the engine applies, so whichever
    // refreshInfo() first carries the new order re-selects the moved layer.
    if (kind == QLatin1String("layer.reorder"))
        m_pendingActiveLayerId = target.value(QStringLiteral("layerId")).toString();
    // 1.0 final R5-4 (O-SIT RC5 sitting, O-WATCH #1186): Merge Down deletes
    // the target and keeps the RECEIVING layer -- its nearest sibling below
    // (lower index, same parent). The active layer is an index, so without
    // this the highlight and the next stroke landed on the layer above.
    if (kind == QLatin1String("layer.merge_down")) {
        const QString targetId = target.value(QStringLiteral("layerId")).toString();
        int ti = -1;
        for (int i = 0; i < m_layers.size(); ++i)
            if (m_layers.at(i).toMap().value(QStringLiteral("layerId")).toString() == targetId)
                ti = i;
        if (ti > 0) {
            const QString parent = m_layers.at(ti).toMap().value(QStringLiteral("parentId")).toString();
            for (int j = ti - 1; j >= 0; --j) {
                const QVariantMap row = m_layers.at(j).toMap();
                if (row.value(QStringLiteral("parentId")).toString() == parent) {
                    m_pendingActiveLayerId = row.value(QStringLiteral("layerId")).toString();
                    break;
                }
            }
        }
    }
    const QByteArray envelope = buildCommandEnvelope(
        m_uuids.next(), kind, QJsonObject::fromVariantMap(target),
        QJsonObject::fromVariantMap(params), mergeKey);
    const CallResult r = m_doc.apply(envelope);
    setStatusFromResult(kind, r);
    // Header §autosave: with a live binding, APPNAME_ERR_IO from apply
    // means the command HAS APPLIED and only durability failed (review
    // D2). Callers must not treat it as a rejection (retries would
    // double-apply).
    const bool appliedNotDurable = !r.ok && r.status == APPNAME_ERR_IO;
    // 14b reselect memory: an APPLIED selection.set arms/refreshes the
    // last-selection memory. Captured BEFORE pollNow so the refresh's
    // own infoChanged publishes the flipped select.reselect row —
    // computed-on-read consumers see fresh truth at that emit (the 14a
    // staleness rule; the saveSelectionAsChannel counter-example).
    if ((r.ok || appliedNotDurable) && kind == QLatin1String("selection.set")) {
        m_lastSelectionSetParams = params;
        m_hasSelectionMemory = true;
    }
    pollNow();
    // RC3 D-1 (operator WORD 35, sitting S-10): mask.add and mask.set_props
    // change what the composite SHOWS, but their receipts carry no dirty
    // rects (the engine's "no visible change => no dirtyRects" rule; undo
    // and mask painting do echo rects), so the canvas kept drawing the
    // layer in full until something else repainted it. Invalidate the
    // whole document here -- the same regionsDirty path strokes and undo
    // ride -- so mask creation and every Mask-panel change show live.
    // 1.0 final D-9 (operator WORD 38/39): the structural layer kinds are the
    // same family. layer.delete removes the subtree and reports no dirty
    // rects, so a deleted layer stayed on the canvas until an unrelated
    // receipt repainted it; merge_down, reorder, group, ungroup and
    // duplicate change the stack the same way. Same whole-document refresh.
    if ((r.ok || appliedNotDurable) && needsWholeDocumentRefresh(kind))
        refreshWholeDocument();
    // A11Y-3: announce from the APPLIED receipt (applied-not-durable
    // included: the mutation is engine truth either way); pollNow above
    // already refreshed info, so name lookups below read post-state.
    if (r.ok || appliedNotDurable)
        announceAppliedCommand(kind, target, params);
    // RC3 D-7 (operator WORD 35, sittings S-05/S-13): a layer this command
    // CREATED becomes the active layer, as addAdjustmentLayer() already does.
    // Before this, Add / Paste appended the new layer on top while the active
    // index stayed on the previous layer -- the first stroke went to the old
    // (sometimes hidden) layer and Delete removed a layer the user had not
    // picked. pollNow above refreshed m_layers, so the id resolves; the id is
    // the client-generated newLayerId the envelope already carries (§9.6).
    if ((r.ok || appliedNotDurable)
        && (kind == QLatin1String("layer.add")
            || kind == QLatin1String("edit.paste")))
        selectLayerById(params.value(QStringLiteral("newLayerId")).toString());
    // 1.0 final F-10: a refused reorder must not leave a pending re-select.
    if (!(r.ok || appliedNotDurable)
        && (kind == QLatin1String("layer.reorder") || kind == QLatin1String("layer.merge_down")))
        m_pendingActiveLayerId.clear();
    out.insert(QStringLiteral("ok"), r.ok || appliedNotDurable);
    out.insert(QStringLiteral("durable"), r.ok);
    out.insert(QStringLiteral("code"), r.code);
    out.insert(QStringLiteral("message"), r.message);
    // Receipt createdIds (echoes of client-generated uuids — §9.6): the
    // wire document is either {ok,receipt:{...}} or the bare receipt.
    QJsonObject receipt = r.json.value(QLatin1String("receipt")).toObject();
    if (receipt.isEmpty())
        receipt = r.json;
    QVariantList created;
    for (const QJsonValue &v : receipt.value(QLatin1String("createdIds")).toArray())
        created.append(v.toString());
    out.insert(QStringLiteral("createdIds"), created);
    return out;
}

QString DocumentController::layerNameById(const QString &layerId) const
{
    for (const QVariant &v : m_layers) {
        const QVariantMap row = v.toMap();
        if (row.value(QStringLiteral("layerId")).toString() == layerId)
            return row.value(QStringLiteral("name")).toString();
    }
    return QString();
}

void DocumentController::announceSelectionState()
{
    if (!m_announcer)
        return;
    if (!m_selectionActive) {
        m_announcer->announce(QStringLiteral("selection"),
                              QStringLiteral("a11y.canvas.selection_cleared"),
                              {});
        return;
    }
    // Bounds need the 1.2 mask-readback door; without it the honest
    // outcome is silence (an active-with-unknown-bounds announcement id
    // does not exist).
    if (m_docWidth <= 0 || m_docHeight <= 0)
        return;
    QRect bounds;
    QImage shared;
    if (selectionMaskFull(&shared)) {
        // Repaint-scoping slice (sweep F4): the SHARED per-revision read —
        // bounds from this image are byte-equal to the strip scan's by
        // construction (same accumulateMaskBounds nonzero accumulation
        // over the same Grayscale8 delivery), so the announcement text
        // cannot move.
        bounds = accumulateMaskBounds(shared, 0, QRect());
    } else {
        // Past the share cap / refused: the strip loop, unchanged (each
        // read <= 2^26 px by construction).
        const int maxRows = qMax(1, (1 << 26) / qMax(1, m_docWidth));
        for (int y = 0; y < m_docHeight; y += maxRows) {
            const int rows = qMin(maxRows, m_docHeight - y);
            QImage strip;
            if (!readSelectionImage(QRect(0, y, m_docWidth, rows), &strip))
                return; // pre-1.2 or typed refusal: no announcement beats a wrong one
            bounds = accumulateMaskBounds(strip, y, bounds);
        }
    }
    if (bounds.isNull())
        return; // active flag with an empty mask — announce nothing over guessing
    m_announcer->announce(
        QStringLiteral("selection"),
        QStringLiteral("a11y.canvas.selection_bounds"),
        {{QStringLiteral("width"), bounds.width()},
         {QStringLiteral("height"), bounds.height()}});
}

void DocumentController::announceAppliedCommand(const QString &kind,
                                                const QVariantMap &target,
                                                const QVariantMap &params)
{
    if (!m_announcer)
        return;
    // A11Y-3 wave B: selection-mutating kinds announce POST-state
    // (bounds on settle / cleared — §31.3; the KS-1/2/7 rows).
    // Review D2: selection.set is the ENTIRE pointer surface (marquee/
    // ellipse/lasso/polygon/wand commit at mouse release — the release
    // IS §31.3's "drag end" settle) and selection.from_path the pen's —
    // both were silently missing.
    if (kind == QLatin1String("selection.all")
        || kind == QLatin1String("selection.none")
        || kind == QLatin1String("selection.invert")
        || kind == QLatin1String("selection.modify")
        || kind == QLatin1String("selection.load")
        || kind == QLatin1String("selection.set")
        || kind == QLatin1String("selection.from_path")) {
        announceSelectionState();
        return;
    }
    // Wave A: the three cataloged a11y.layers.* rows (§31.3 examples
    // table + the 2026-08-18 O-I18N rotation's golden wordings). Every
    // name resolves from POST-refresh info truth — the receipt applied,
    // so the refreshed row IS the engine's answer. A missing row (e.g.
    // a rename raced by an engine-side removal) announces nothing
    // rather than something wrong.
    const QString layerId = target.value(QStringLiteral("layerId")).toString();
    if (kind == QLatin1String("layer.set_props")) {
        // Only the visibility OFF edge is a registered announcement
        // (a11y.layers.hidden); un-hiding and opacity edits carry no
        // registered id — the layer row's checked-state change is the
        // platform's speech (§31.3 no-double-speak).
        const QVariant vis = params.value(QStringLiteral("visible"));
        if (vis.isValid() && !vis.toBool()) {
            const QString name = layerNameById(layerId);
            if (!name.isEmpty())
                m_announcer->announce(QStringLiteral("layers"),
                                      QStringLiteral("a11y.layers.hidden"),
                                      {{QStringLiteral("name"), name}});
        }
    } else if (kind == QLatin1String("layer.rename")) {
        const QString name = layerNameById(layerId); // post-state = new name
        if (!name.isEmpty())
            m_announcer->announce(QStringLiteral("layers"),
                                  QStringLiteral("a11y.layers.renamed"),
                                  {{QStringLiteral("name"), name}});
    } else if (kind == QLatin1String("layer.reorder")) {
        const QString name = layerNameById(layerId);
        // {index} is the sibling index the engine ACCEPTED — the
        // command's applied parameter, not a client re-derivation.
        if (!name.isEmpty() && params.contains(QStringLiteral("index")))
            m_announcer->announce(
                QStringLiteral("layers"), QStringLiteral("a11y.layers.moved"),
                {{QStringLiteral("name"), name},
                 {QStringLiteral("index"),
                  params.value(QStringLiteral("index"))}});
    }
}

namespace {

// QML FileDialog hands back file:// URLs; the ABI takes filesystem paths.
QString toLocalPath(const QString &pathOrUrl)
{
    if (pathOrUrl.startsWith(QLatin1String("file:")))
        return QUrl(pathOrUrl).toLocalFile();
    return pathOrUrl;
}

} // namespace

bool DocumentController::openDocument(const QString &pathOrUrl)
{
    const QString path = toLocalPath(pathOrUrl);
    // Open into a FRESH handle and swap only on success: a corrupt or
    // wrong file must not destroy the current in-memory document (review
    // R2-chrome).
    AbiDoc fresh;
    const CallResult r = fresh.open(path);
    if (!r.ok) {
        setRejected(r.code, r.message);
        return false;
    }
    m_doc = std::move(fresh);
    m_pendingRecoveryLog.clear(); // same rule as newDocument (review P6-D1)
    bindAutosaveIfSafe(path);
    m_documentPath = path;
    emit documentPathChanged();
    m_rootLayerId.clear(); // unknown for opened docs until info v2
    // Cross-document handles must not survive a swap: the new journal's
    // cursor is unrelated to the old one, so the cursor rule alone
    // cannot catch this (request-9 interim).
    m_lastSavedChannelId.clear();
    m_lastSelectionSetParams.clear(); // 14b reselect memory: same rule
    m_hasSelectionMemory = false;
    // Row 1: the authored-op memory is document state for the same reason
    // — an op id from the previous document resolves to nothing here, and
    // op.set_mask would answer bad_target. Clearing it keeps the
    // availability row honest instead of offering a control that refuses.
    m_lastAdjustmentOpId.clear();
    // Cross-document mask-cache safety (see newDocument).
    ++m_selectionRevision;
    m_selectionMaskCache = QImage();
    m_selectionMaskCacheRevision = -1;
    m_savedCursor = -2; // set from info below (refreshInfo runs next)
    m_activeLayerIndex = 0;
    emit activeLayerIndexChanged();
    refreshInfo();
    m_savedCursor = m_journalCursor; // opened state == on-disk state
    // The title binding re-evaluated during refreshInfo's emit with the
    // transient -2 baseline — re-notify so a clean just-opened document
    // does not sit wrongly marked (review P4-D1).
    emit infoChanged();
    refreshHistory();
    emit documentReset();
    pollNow(); // consume the whole-canvas open seed event
    provisionSystemFaces(); // ABI 1.6 font feed (per-document FontDb)
    m_pollTimer.start();
    setStatus(trArgs(m_catalog, QStringLiteral("ui.status.opened"),
                     {{QStringLiteral("path"), path}}));
    return true;
}

bool DocumentController::saveDocument(const QString &pathOrUrl)
{
    const QString path = toLocalPath(pathOrUrl);
    const QString dest = path.isEmpty() ? m_documentPath : path;
    if (!m_doc.isOpen() || dest.isEmpty())
        return false;
    const CallResult r = m_doc.save(dest);
    if (!r.ok) {
        setRejected(r.code, r.message);
        return false;
    }
    if (m_documentPath != dest) {
        m_documentPath = dest;
        emit documentPathChanged();
    }
    // A save to the recovered package's own path RESOLVES the recovery
    // (contract: discard the log explicitly once resolved) — the fold-in
    // is complete, the stale crash log must not re-prompt. A save
    // elsewhere leaves the original package's log intact (still the
    // crash state's honest record) and merely stops tracking it here.
    if (!m_pendingRecoveryLog.isEmpty()) {
        if (m_pendingRecoveryLog == dest + QStringLiteral(".autosave"))
            autosaveDiscard(m_pendingRecoveryLog);
        m_pendingRecoveryLog.clear();
    }
    bindAutosaveIfSafe(dest);
    m_savedCursor = m_journalCursor; // title dirty-marker baseline
    emit infoChanged();
    setStatus(trArgs(m_catalog, QStringLiteral("ui.status.saved"),
                     {{QStringLiteral("path"), dest}}));
    // A11Y-3 wave B: KS-1 s10's announcement (§31.3 golden "Document
    // saved").
    if (m_announcer)
        m_announcer->announce(QStringLiteral("file"),
                              QStringLiteral("a11y.file.saved"), {});
    return true;
}

bool DocumentController::abiBlock12() const
{
    return abiHasBlock12();
}

bool DocumentController::abiBlock14() const
{
    return abiHasBlock14();
}

QVariantMap DocumentController::commandAvailability() const
{
    // Item 14a: the ONE availability compute (design entry, O-UI-log.md
    // items 7+14). Every truth below derives from the same info-truth
    // members the old Main.qml predicates read through Q_PROPERTYs —
    // the migration is BEHAVIOR-IDENTICAL by construction (the frozen
    // old predicates are pinned verbatim in tst_command_availability).
    //
    // reasonId policy: attached ONLY when disabled AND an existing
    // cataloged id names the failing conjunct — "ui.error.no_document"
    // for a missing document, "error.command.selection_required" for a
    // missing selection. Other disabled causes (empty active layer, an
    // ABI block absent, no saved channel, no last-selection memory)
    // have NO cataloged bare id today (ui.panel.layers.milestone_tooltip
    // is parameterized — unusable as a bare id): reasonId stays absent,
    // mint-queue candidates recorded in the lane log.
    const bool doc = m_hasDocument;
    const bool layer = doc && !activeLayerId().isEmpty();
    const bool sel = m_selectionActive;
    const QString noDocId = doc ? QString()
                                : QStringLiteral("ui.error.no_document");
    const QString selReasonId =
        !doc ? QStringLiteral("ui.error.no_document")
             : (!sel ? QStringLiteral("error.command.selection_required")
                     : QString());

    const auto row = [](bool enabled, const QString &reasonId = QString()) {
        QVariantMap r;
        r.insert(QStringLiteral("enabled"), enabled);
        if (!enabled && !reasonId.isEmpty())
            r.insert(QStringLiteral("reasonId"), reasonId);
        return r;
    };

    QVariantMap m;
    // ---- File (old predicates: hasDocument; hasDocument && abiBlock12
    // for the ABI-1.2 doors) ----
    m.insert(QStringLiteral("file.save"), row(doc, noDocId));
    m.insert(QStringLiteral("file.saveAs"), row(doc, noDocId));
    m.insert(QStringLiteral("file.import"), row(doc && abiBlock12(), noDocId));
    m.insert(QStringLiteral("file.export"), row(doc && abiBlock12(), noDocId));
    // ---- Edit ----
    m.insert(QStringLiteral("edit.undo"), row(m_canUndo));
    m.insert(QStringLiteral("edit.redo"), row(m_canRedo));
    // Wave F: the history.goto 14a row (census class F) — the
    // HistoryPanel rows' enabled truth. Document-gated: rows only exist
    // with a journal; position range/reachability stays the engine's
    // typed out_of_range at apply time (§40.8/§10.4), never
    // pre-computed.
    m.insert(QStringLiteral("history.goto"), row(doc, noDocId));
    // §26.1 Cut (ABI 1.4 flip): app-side per-layer-readback Copy + the
    // edit.cut mutation — pre-1.4 stays disabled (absence beats a
    // mutation-only Cut, review R3).
    m.insert(QStringLiteral("edit.cut"), row(layer && abiBlock14(), noDocId));
    m.insert(QStringLiteral("edit.copy"), row(doc, noDocId));
    m.insert(QStringLiteral("edit.paste"), row(doc && abiBlock12(), noDocId));
    // Item 15 — Paste Into. THE PREDICATE'S DERIVATION (engine truth,
    // read first-hand, not guessed): edit.paste_into's kind impl
    // (crates/appname-core/src/engine/kinds.rs, EditPasteIntoKind::apply)
    // gates on `cx.document().selection().active()` and refuses
    // error.command.selection_required otherwise — the contract's
    // selectionBehavior: "REQUIRED — consumed as the new raster mask's
    // coverage" (§40.5). So the row is edit.paste's predicate AND the
    // SELECTION conjunct: doc && abiBlock12() && sel. Each conjunct is
    // real: doc/abi12 because the ingress is the same 1.2 asset_create
    // door paste uses, sel because the engine will refuse without one.
    // reasonId rides the SHARED selReasonId — no_document when there is
    // no document, error.command.selection_required when there is one
    // but no selection (the engine's OWN code, so the chrome names the
    // same cause the engine would), and correctly ABSENT when only the
    // uncataloged abi-1.2 conjunct fails (selReasonId is empty there).
    // The engine's remaining arms (bad_target, duplicate_id, locked
    // parent) stay APPLY-TIME typed refusals — the item-13 wall pattern;
    // no pre-computed disable can honestly express them.
    m.insert(QStringLiteral("edit.pasteInto"),
             row(doc && abiBlock12() && sel, selReasonId));
    m.insert(QStringLiteral("edit.fill"), row(layer, noDocId));
    // 1.0 final (corrigendum (l)): no edit.puppetWarp row; Edit > Puppet
    // Warp is removed.
    // Delete (desk ruling): the edit.cut ENGINE kind WITHOUT the
    // app-side copy — mutation-only, so NO abiBlock14 conjunct. Both
    // disposition arms (selection -> §13.1 clear; none -> §32.5
    // confirm-then-whole-layer) act on the active layer. Transparency
    // lock = the HONEST LIMIT: surfaces as the typed refusal at apply
    // time, never pre-computed (locks are write-only pre-info-v2).
    m.insert(QStringLiteral("edit.delete"), row(layer, noDocId));
    // ---- Pro-Wave row 1: adjustment layers, OP-mask route ----
    // addAdjustment: DOCUMENT-gated only. adjustment.add inserts at the
    // root by default and needs no active layer (unlike the layer-editing
    // rows above) — the engine's own default is `parent = root`. It is
    // deliberately NOT selection-gated: with no selection the door
    // creates a plain unmasked adjustment, which is a correct outcome.
    m.insert(QStringLiteral("layer.addAdjustment"), row(doc, noDocId));
    // maskAdjustmentFromSelection: THREE real conjuncts, each one a cause
    // the engine or the shell would genuinely refuse on.
    //   doc  — no handle, nothing to submit through.
    //   sel  — op.set_mask{from:'selection'} answers
    //          error.command.selection_required without one; the engine's
    //          OWN gate, reproduced first-hand through the CLI script door
    //          before this row was written.
    //   memory — the shell can only address an op it AUTHORED, because the
    //          ABI publishes no op ids (header honest limit). Without the
    //          memory the submission would answer bad_target.
    // reasonId rides the SHARED selReasonId, which is correct for the
    // first two conjuncts and correctly ABSENT for the third: no cataloged
    // bare id names "no adjustment authored in this session" (queued to
    // O-I18N). Naming the wrong conjunct is the defect the select.reselect
    // row already had to correct once — the row stays silent instead.
    m.insert(QStringLiteral("layer.maskAdjustmentFromSelection"),
             row(doc && sel && !m_lastAdjustmentOpId.isEmpty(), selReasonId));
    // ---- LAYER MASKS (mask.add) — the LAYER-mask route, and it is NOT
    // the OP-mask route directly above. op.set_mask masks an adjustment's
    // OpNode; mask.add attaches a mask to the LAYER itself (§11.2,
    // appended to LayerNode::masks). The header note on
    // maskAdjustmentFromSelection already warns the two must not be
    // conflated — these rows are the other half of that sentence.
    // DEMO-SCRIPT.md:78's "NOT YET" box is their acceptance statement:
    // the shell could composite a masked layer and could not create the
    // mask.
    //
    // addMask — TWO conjuncts, and a third one DELIBERATELY ABSENT.
    //   doc   — no handle, nothing to submit through.
    //   layer — mask.add's targetResolution is target.layerId (the mask's
    //           OWNER; maskId does not exist yet and rides in params as
    //           the §9.6 client-generated newMaskId). No active layer,
    //           nothing to address.
    //   NOT an occupied-slot conjunct, and this is a measured limit
    //   rather than an omission: §11.2 allows at most one raster + one
    //   vector mask per layer and an occupied slot answers
    //   error.command.target_in_use — but the ABI's info mask object is
    //   {id,enabled,inverted,density,feather,linked} with NO `kind`
    //   (appname-cabi/src/doc.rs:465), so the shell cannot tell a raster
    //   mask from a vector one and CANNOT honestly compute "the raster
    //   slot is taken". A guess would disable a legal action on a
    //   vector-masked layer — a false surface with its polarity
    //   reversed, the shape item 19's fallback exists to avoid. The
    //   engine owns the precondition and its typed refusal surfaces on
    //   ui.status.rejected, exactly the division pasteIntoFromClipboard
    //   documents for selection_required. ABI ASK, filed not worked:
    //   publish `kind` on the info mask object.
    // reasonId: the document reason only. No cataloged bare id names "no
    // active layer" — the nudge row's precedent, and the row stays silent
    // rather than naming a conjunct that did not fail.
    // ---- FILL LAYERS (layer.add{kind:'fill'}) ----
    // DOCUMENT-gated only, and for layer.addAdjustment's exact reason:
    // layer.add inserts at the ROOT by default (§10.3) and needs no
    // active layer. Adding a layer conjunct "for symmetry" with the mask
    // rows above would disable a legal action on an empty stack.
    //
    // The engine has served fill layers since before this shell had a
    // menu for them: demo/build-demo.jsonl lines 4-5 author two gradient
    // fill layers, and reports/DEMO-SCRIPT.md section 8 demonstrates
    // them. The gap was never the engine — it was that no UI reached it.
    m.insert(QStringLiteral("layer.addFill"), row(doc, noDocId));
    m.insert(QStringLiteral("layer.addMask"), row(layer, noDocId));
    // addMaskFromSelection — the same two conjuncts plus the selection,
    // which mask.add's selectionBehavior makes REQUIRED for
    // from:'selection' and 'selection_inverted' (the engine answers
    // error.command.selection_required without one). Shares selReasonId:
    // correct for the document and selection conjuncts, and correctly
    // ABSENT for the layer conjunct — a document with a selection and no
    // active layer is disabled with no cataloged id to name why.
    m.insert(QStringLiteral("layer.addMaskFromSelection"),
             row(layer && sel, selReasonId));
    // ---- Select ----
    m.insert(QStringLiteral("select.all"), row(doc, noDocId));
    m.insert(QStringLiteral("select.deselect"), row(sel, selReasonId));
    m.insert(QStringLiteral("select.invert"), row(doc, noDocId));
    m.insert(QStringLiteral("select.modify"), row(sel, selReasonId));
    // Reselect (14b): enabled once the shell's last-selection memory is
    // armed (an applied selection.set — see reselect()). Item 25b
    // (census class H, stale-record consume-fix): the old record here
    // said "nothing to reselect yet" had no cataloged bare id and so
    // dropped the reasonId entirely. STALE — ui.error.nothing_to_reselect
    // exists (i18n/en.json:505, all four locales). The row now names its
    // FAILING CONJUNCT: no document -> the document reason; document but
    // no memory -> the memory reason. Naming the wrong conjunct is the
    // defect the old comment was avoiding, so the two are kept distinct.
    m.insert(QStringLiteral("select.reselect"),
             row(doc && m_hasSelectionMemory,
                 doc ? QStringLiteral("ui.error.nothing_to_reselect")
                     : noDocId));
    m.insert(QStringLiteral("select.saveSelection"), row(sel, selReasonId));
    m.insert(QStringLiteral("select.loadSaved"),
             row(!m_lastSavedChannelId.isEmpty(), noDocId));
    m.insert(QStringLiteral("select.loadLayerAlpha"), row(layer, noDocId));
    // ---- Image (all: hasDocument) ----
    for (const char *k :
         {"image.resize", "image.canvasSize", "image.rotate", "image.trim",
          "image.crop", "image.flatten", "image.assignProfile",
          "image.convertProfile", "image.depthConvert"})
        m.insert(QLatin1String(k), row(doc, noDocId));
    // ---- Image > Auto Tone / Auto Contrast / Auto Color (§17.2) ----
    // ONE row for the three kinds, on the filter.apply precedent: their
    // three contracts carry a BYTE-IDENTICAL precondition, so three rows
    // would be three copies of one truth with three chances to drift.
    //
    // The predicate is the ACTIVE-LAYER one (edit.fill's), NOT the
    // document-only one the ten rows above take. Derived from the
    // contracts read first-hand, not guessed:
    //   contracts/commands/image.auto_tone.yaml
    //     targetResolution: "document handle; target block ignored
    //       (§40.2 family rule) - the edited layer is params.targetLayer
    //       (default: the active layer), which MUST be a Raster layer
    //       -> error.command.target_kind_mismatch"
    //     selectionBehavior: "gates coverage - histogram statistics are
    //       computed over the selection support when active (else full
    //       layer) ..."
    // So the edit lands on a LAYER (unlike resize/canvasSize/rotate/
    // trim/crop/flatten/mode/profiles, which act on the document), and a
    // selection SCOPES the histogram rather than gating the command —
    // exactly edit.fill's rule. Selection is therefore NOT a conjunct.
    // Contrast image.crop, which IS document-scoped and stays above.
    //
    // Layer KIND (raster-vs-not) and the pixels/all LOCK classes stay
    // APPLY-TIME typed refusals (error.command.target_kind_mismatch /
    // error.command.locked) — the item-13 wall pattern: locks are
    // write-only pre-info-v2, and no pre-computed disable can honestly
    // express a cause the shell cannot read.
    //
    // reasonId: the SHARED noDocId, which names the failing conjunct
    // when there is no document and is correctly ABSENT when a document
    // is open but the active-layer index is out of range — no cataloged
    // bare id names "no active layer" (i18n/en.json carries no such id;
    // proto.status.no_layer is a different surface's). Naming the wrong
    // conjunct is the defect the select.reselect row had to correct
    // once; the row stays silent instead. Queued to O-I18N.
    m.insert(QStringLiteral("image.autoAdjust"), row(layer, noDocId));
    // ---- Filter: the six live table-driven kinds share ONE predicate
    // (and will share this row from the item-7 context menu too);
    // liquify/custom are recorded later-wave surfaces (literal
    // enabled: false in the menu), not availability rows. ----
    m.insert(QStringLiteral("filter.apply"), row(layer, noDocId));
    // ---- 14b/item-7 vocabulary (no menu consumer yet) ----
    // Cmd+J (desk ruling, composition): with a selection = app-side
    // copy (1.4 per-layer readback) -> edit.paste as a new layer (1.2
    // ingress); without = layer.duplicate (base registry kind).
    m.insert(QStringLiteral("layer.viaCopy"),
             row(layer && (!sel || (abiBlock14() && abiBlock12())), noDocId));
    // Cmd+T enters transform.free on the active layer (item-13 tie).
    m.insert(QStringLiteral("transform.enter"), row(layer, noDocId));
    // Alt/Cmd+Delete: the engine's headless-constant fill arms —
    // active-layer actions like edit.fill; an active selection SCOPES
    // the fill, it does not gate it.
    m.insert(QStringLiteral("fill.foreground"), row(layer, noDocId));
    m.insert(QStringLiteral("fill.background"), row(layer, noDocId));
    // Arrow-key nudge of the active layer (integer-translate arm).
    m.insert(QStringLiteral("nudge"), row(layer, noDocId));
    return m;
}

void DocumentController::provisionSystemFaces()
{
    m_fontsProvided = m_fontsSkipped = m_fontsRefused = 0;
    if (!m_doc.isOpen() || !abiHasBlock16())
        return;
    // v1 scope (lane log): single-face .ttf/.otf, <= 5 MB each (bounded
    // IO — giant CJK faces and .ttc collections are the recorded
    // follow-up), from the three host dirs in fixed order, sorted names
    // within each (deterministic provisioning order — the door
    // preserves call order and §44.2 step-2 is order-aware).
#ifdef Q_OS_WIN
    // W-05 (O-WIN): the Windows host dirs, same order rule (per-user
    // first, then system): per-user installs land in
    // %LOCALAPPDATA%\Microsoft\Windows\Fonts, system faces in %WINDIR%\Fonts.
    const QStringList dirs = {
        qEnvironmentVariable("LOCALAPPDATA") + QStringLiteral("/Microsoft/Windows/Fonts"),
        qEnvironmentVariable("WINDIR", QStringLiteral("C:/Windows")) + QStringLiteral("/Fonts"),
    };
#else
    const QStringList dirs = {
        QDir::homePath() + QStringLiteral("/Library/Fonts"),
        QStringLiteral("/Library/Fonts"),
        QStringLiteral("/System/Library/Fonts/Supplemental"),
    };
#endif
    for (const QString &dirPath : dirs) {
        QDir dir(dirPath);
        const QStringList files = dir.entryList(
            {QStringLiteral("*.ttf"), QStringLiteral("*.otf")},
            QDir::Files | QDir::Readable, QDir::Name);
        for (const QString &name : files) {
            QFile f(dir.filePath(name));
            if (f.size() > 5 * 1024 * 1024 || !f.open(QIODevice::ReadOnly)) {
                ++m_fontsSkipped;
                continue;
            }
            const QByteArray bytes = f.readAll();
            QString family;
            QString style;
            QString psName;
            if (!parseFontNames(bytes, &family, &style, &psName)) {
                ++m_fontsSkipped; // not a parseable single face — never guess
                continue;
            }
            const CallResult r = m_doc.provideFace(
                bytes, QStringLiteral("system"), family, style, psName,
                nullptr);
            if (r.ok)
                ++m_fontsProvided;
            else
                ++m_fontsRefused; // typed (psName collision, engine reject) — counted, quiet
        }
    }
}

QVariantMap DocumentController::fontFacesSummary()
{
    QVariantMap out;
    out.insert(QStringLiteral("provided"), m_fontsProvided);
    out.insert(QStringLiteral("skipped"), m_fontsSkipped);
    out.insert(QStringLiteral("refused"), m_fontsRefused);
    QJsonObject faces;
    if (m_doc.isOpen() && abiHasBlock16() && m_doc.fontFaces(&faces).ok) {
        out.insert(QStringLiteral("stamp"),
                   faces.value(QLatin1String("stamp")).toInt());
        out.insert(QStringLiteral("faces"),
                   faces.value(QLatin1String("faces")).toArray().size());
    }
    return out;
}

QImage DocumentController::layerThumbImage(const QString &layerId)
{
    if (!m_doc.isOpen() || !abiHasBlock14())
        return {};
    const QRect full(0, 0, docWidth(), docHeight());
    if (full.isEmpty())
        return {};
    QImage img;
    if (!m_doc.readLayer(layerId, full, &img).ok)
        return {}; // typed refusals expected (service matrix) — quiet
    return img;
}

QVariantMap DocumentController::layerContentBounds(const QString &layerId)
{
    // ONE implementation, two callers: G-3's bounds-only entry derivation
    // is the snapshot form with nowhere to put the pixels. Keeping a
    // second body here is how the two would eventually disagree about
    // what "the layer's content" means.
    return layerContentSnapshot(layerId, nullptr);
}

QVariantMap DocumentController::layerContentSnapshot(const QString &layerId,
                                                     QImage *snapshot)
{
    if (snapshot)
        *snapshot = QImage();
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    out.insert(QStringLiteral("snapshot"), false);
    if (!m_doc.isOpen() || !abiHasBlock14() || layerId.isEmpty()) {
        // Client-side precondition — the cutLayer arm's exact conjunct
        // and code (belt-and-braces; the tools gate on abiBlock14).
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    const int w = docWidth();
    const int h = docHeight();
    if (w <= 0 || h <= 0) {
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    // item 19 §D.1: the strip loop, the selection announcer's exact
    // pattern — each read is <= 2^26 px BY CONSTRUCTION, which is also
    // why accumulateAlphaBounds takes a yOffset and a running rect
    // (selection_scan.h: "accumulated strip-by-strip so every single
    // read stays under the 2^26-px readback cap"). This site simply
    // never used it that way. maxRows >= h exactly when w*h <= 2^26, so
    // for every document at or below the cap the loop runs ONCE over the
    // full doc and the result is byte-identical to the old one-shot read.
    const int maxRows = qMax(1, (1 << 26) / qMax(1, w));
    const bool fitsOneRead = static_cast<qint64>(w) * h <= (qint64(1) << 26);
    QRect bounds;
    QImage whole;
    for (int y = 0; y < h; y += maxRows) {
        const int rows = qMin(maxRows, h - y);
        QImage strip;
        const CallResult rr =
            m_doc.readLayer(layerId, QRect(0, y, w, rows), &strip);
        if (!rr.ok) {
            // The door's typed refusal IS the caller's answer — surfaced
            // on the EXISTING rejected path (mint-free), returned
            // queryable. The caller rules what the refusal costs it; this
            // function never guesses a rect.
            setRejected(rr.code, rr.message);
            out.insert(QStringLiteral("code"), rr.code);
            out.insert(QStringLiteral("message"), rr.message);
            return out;
        }
        bounds = accumulateAlphaBounds(strip, y, bounds);
        if (snapshot && fitsOneRead)
            whole = strip; // single iteration by construction (see above)
    }
    out.insert(QStringLiteral("ok"), true);
    out.insert(QStringLiteral("x"), bounds.x());
    out.insert(QStringLiteral("y"), bounds.y());
    out.insert(QStringLiteral("w"), bounds.width());
    out.insert(QStringLiteral("h"), bounds.height());
    if (snapshot && !whole.isNull()) {
        *snapshot = whole;
        out.insert(QStringLiteral("snapshot"), true);
    }
    // Above the cap `snapshot` stays false with ok:true — bounds are real,
    // the pixels are not in hand. Item 21's caller degrades to frame-only
    // AND SAYS SO; a partial-strip preview would be a capped partial lie,
    // the exact choice the ants overlay already declined.
    return out;
}

QVariantMap DocumentController::cutLayer(const QString &layerId)
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (!m_doc.isOpen() || !abiHasBlock14() || layerId.isEmpty()) {
        // Client-side precondition (menu gates on abiBlock14; this arm is
        // belt-and-braces for scripted callers).
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    // Copy half FIRST (the pixels must be read before the mutation clears
    // them), clipboard update LAST (a typed refusal must not clobber the
    // previous clipboard). §26.1 rule 1: the Copy is an app step.
    const QRect full(0, 0, docWidth(), docHeight());
    QImage content;
    const CallResult rr = m_doc.readLayer(layerId, full, &content);
    if (!rr.ok) {
        setRejected(rr.code, rr.message);
        out.insert(QStringLiteral("code"), rr.code);
        out.insert(QStringLiteral("message"), rr.message);
        return out;
    }
    if (m_selectionActive) {
        // §40.5: the active selection defines the region; fractional
        // coverage scales the copied alpha exactly as edit.cut scales the
        // removed alpha (clipboard_ops — the faithful pairing). No
        // selection = whole content, no masking.
        QImage coverage;
        const CallResult rs = m_doc.readSelection(full, &coverage);
        if (!rs.ok) {
            setRejected(rs.code, rs.message);
            out.insert(QStringLiteral("code"), rs.code);
            out.insert(QStringLiteral("message"), rs.message);
            return out;
        }
        applySelectionCoverage(&content, coverage);
    }
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerId);
    const QVariantMap r =
        applyCommand(QStringLiteral("edit.cut"), target, {}, {});
    if (r.value(QStringLiteral("ok")).toBool())
        QGuiApplication::clipboard()->setImage(content);
    return r;
}

QVariantMap DocumentController::deleteLayerContent(const QString &layerId)
{
    // 14b Delete (see header): edit.cut WITHOUT the app-side copy —
    // mutation-only, so no 1.4 gate (unlike cutLayer). Both disposition
    // arms are the ENGINE's own: selection -> §13.1 coverage-scaled
    // clear; none -> §40.5 whole-content clear. The §32.5 confirm on the
    // no-selection arm is the caller's (Main.qml / DeleteConfirmDialog).
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (!m_doc.isOpen() || layerId.isEmpty()) {
        // Client-side precondition, ui.* namespace (the applyCommand
        // precedent — chrome gates on the edit.delete row already).
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerId);
    // A typed refusal (transparency/pixels/ALL lock) surfaces on the
    // existing ui.status.rejected path inside applyCommand — the
    // item-13 wall class, no new strings.
    return applyCommand(QStringLiteral("edit.cut"), target, {}, {});
}

QVariantMap DocumentController::layerViaCopy(const QString &layerId)
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (!m_doc.isOpen() || layerId.isEmpty()) {
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    if (!m_selectionActive) {
        // No-selection arm: layer.duplicate over the active layer's
        // subtree. idMap composition = the LayersPanel duplicate
        // button's established walk (layers + maskIds from info truth;
        // op-stack entry ids are NOT in info — the recorded upstream
        // gap: layers carrying op stacks reject typed, honest).
        QSet<QString> inSubtree{layerId};
        bool grew = true;
        while (grew) {
            grew = false;
            for (const QVariant &v : std::as_const(m_layers)) {
                const QVariantMap r = v.toMap();
                const QString id =
                    r.value(QStringLiteral("layerId")).toString();
                const QString parent =
                    r.value(QStringLiteral("parentId")).toString();
                if (!inSubtree.contains(id) && inSubtree.contains(parent)) {
                    inSubtree.insert(id);
                    grew = true;
                }
            }
        }
        QVariantMap idMap;
        for (const QVariant &v : std::as_const(m_layers)) {
            const QVariantMap r = v.toMap();
            const QString id = r.value(QStringLiteral("layerId")).toString();
            if (!inSubtree.contains(id))
                continue;
            idMap.insert(id, m_uuids.next());
            const QVariantList masks =
                r.value(QStringLiteral("maskIds")).toList();
            for (const QVariant &m : masks)
                idMap.insert(m.toString(), m_uuids.next());
        }
        QVariantMap target;
        target.insert(QStringLiteral("layerId"), layerId);
        QVariantMap params;
        params.insert(QStringLiteral("idMap"), idMap);
        params.insert(QStringLiteral("insertAbove"), true);
        return applyCommand(QStringLiteral("layer.duplicate"), target,
                            params, {});
    }
    // Selection arm: the app-side copy (cutLayer's exact copy half —
    // per-layer 1.4 readback, alpha scaled by the selection's fractional
    // coverage) pasted back as a NEW layer through the journaled 1.2
    // ingress. The full-doc-rect image pastes at [0,0], so the copied
    // pixels land exactly where they were.
    if (!abiHasBlock14() || !abiHasBlock12()) {
        // Belt-and-braces: the layer.viaCopy row already carries the
        // abi conjuncts on the selection arm.
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    const QRect full(0, 0, docWidth(), docHeight());
    QImage content;
    const CallResult rr = m_doc.readLayer(layerId, full, &content);
    if (!rr.ok) {
        setRejected(rr.code, rr.message);
        out.insert(QStringLiteral("code"), rr.code);
        out.insert(QStringLiteral("message"), rr.message);
        return out;
    }
    QImage coverage;
    const CallResult rs = m_doc.readSelection(full, &coverage);
    if (!rs.ok) {
        setRejected(rs.code, rs.message);
        out.insert(QStringLiteral("code"), rs.code);
        out.insert(QStringLiteral("message"), rs.message);
        return out;
    }
    applySelectionCoverage(&content, coverage);
    // PNG bytes = the ingress transport format (the pasteFromClipboard
    // precedent: OS-bridge/app-side encoding of content on its way in;
    // the journaled asset makes replay byte-identical).
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    if (!content.save(&buffer, "PNG")) {
        setRejected(QStringLiteral("error.io.write_failed"), QStringLiteral("via-copy PNG serialization"));
        out.insert(QStringLiteral("code"),
                   QStringLiteral("error.io.write_failed"));
        return out;
    }
    QString code;
    QString message;
    const auto assetId = ingestSamplesBlob(m_doc.raw(), bytes, &code, &message);
    if (!assetId) {
        setRejected(code, message);
        out.insert(QStringLiteral("code"), code);
        out.insert(QStringLiteral("message"), message);
        return out;
    }
    QVariantMap params;
    params.insert(QStringLiteral("assetId"), *assetId);
    params.insert(QStringLiteral("at"), QVariantList{0, 0});
    params.insert(QStringLiteral("newLayerId"), m_uuids.next());
    return applyCommand(QStringLiteral("edit.paste"), {}, params, {});
}

// ---- Pro-Wave row 6: §11.2 mask properties -------------------------------

QVariantList DocumentController::masksOf(const QString &layerId) const
{
    for (const QVariant &v : m_layers) {
        const QVariantMap row = v.toMap();
        if (row.value(QStringLiteral("layerId")).toString() == layerId)
            return row.value(QStringLiteral("masks")).toList();
    }
    return {};
}

QVariantMap DocumentController::setMaskProp(const QString &layerId,
                                            const QString &maskId,
                                            const QString &prop,
                                            const QVariant &value,
                                            const QString &sessionUuid)
{
    // Name -> registered param. Anything else never reaches the wire:
    // mask.set_props is additionalProperties:false, so a typo would come
    // back as a stage-3 schema violation whose message names the schema,
    // not the caller's mistake.
    //
    // ★ THE REFUSAL CARRIES ITS CATALOGED ID — THE HOLD IS OVER.
    //
    // ★★ HISTORICAL RECORD, LEFT AS WRITTEN BECAUSE IT IS WHY THE CODE
    // WAS EMPTY (1b-C: log the correction beneath the claim, not over it).
    // This comment used to read: "`ui.error.bad_argument` — is NOT
    // registered, NOT minted and NOT in any of the four catalogs … the
    // standing rule is DO NOT INVENT IDS, so the id is HELD … until it
    // lands this door refuses with `ok:false` and an EMPTY code." That was
    // true when row 6 was written and the i18n gate proved it (RED-1/RED-2
    // on this slice's first gate run).
    //
    // ★★★ IT IS NO LONGER TRUE. A-05 MINTED `ui.error.bad_argument` on
    // 2026-09-05 and it now ships in ALL FOUR catalogs (en "That property
    // is not recognized" · es · ja · pseudo), verified here rather than
    // taken from the amendment. So the door emits the id, matching the
    // house pattern for a client-side precondition (applyCommand's
    // `ui.error.no_document`): a cataloged `code`, an EMPTY `message` —
    // the caller resolves the wording from the id, never from a string
    // built here.
    //
    // ★ WHY THIS MATTERS NOW AND NOT BEFORE: the reason "no user can
    // reach this" was that the only caller was the test net. B-12 builds
    // the panel that calls it, so this refusal becomes user-visible in
    // the same wave — an empty code would have reached a real person as
    // silence.
    static const QHash<QString, MaskProp> kProps = {
        {QStringLiteral("enabled"), MaskProp::Enabled},
        {QStringLiteral("inverted"), MaskProp::Inverted},
        {QStringLiteral("density"), MaskProp::Density},
        {QStringLiteral("feather"), MaskProp::Feather},
        {QStringLiteral("linked"), MaskProp::Linked},
    };
    const auto it = kProps.constFind(prop);
    if (it == kProps.constEnd() || layerId.isEmpty() || maskId.isEmpty()) {
        QVariantMap out;
        out.insert(QStringLiteral("ok"), false);
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.bad_argument")); // minted A-05
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    const MaskProp p = it.value();
    const bool isBool = (p == MaskProp::Enabled || p == MaskProp::Inverted
                         || p == MaskProp::Linked);
    const QJsonObject params = isBool
        ? buildMaskSetPropsParams(p, value.toBool())
        : buildMaskSetPropsParams(p, value.toDouble());
    if (params.isEmpty()) {
        QVariantMap out;
        out.insert(QStringLiteral("ok"), false);
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.bad_argument")); // minted A-05
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    // §10.3 mask.* family targetResolution: BOTH ids ride the target and
    // both must resolve at stage 4.
    QVariantMap target;
    target.insert(QStringLiteral("layerId"), layerId);
    target.insert(QStringLiteral("maskId"), maskId);
    return applyCommand(QStringLiteral("mask.set_props"), target,
                        params.toVariantMap(),
                        maskPropSessionMergeKey(maskId, sessionUuid, p));
}

QVariantMap DocumentController::reselect()
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (!m_hasSelectionMemory) {
        // Client-side precondition mirror of the select.reselect row
        // (chrome disables on it; belt-and-braces here).
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    // Replay through the SAME journaled door the capture rode. The
    // apply re-captures the identical params (idempotent memory).
    return applyCommand(QStringLiteral("selection.set"), {},
                        m_lastSelectionSetParams, {});
}

QVariantMap DocumentController::addAdjustmentLayer(const QString &opKind,
                                                   const QVariantMap &opParams)
{
    // Row 1, the OP-mask route. Both creation uuids are minted HERE and
    // the op id is REMEMBERED — that memory is the whole reason the mask
    // step below can address the op at all (the ABI publishes no op ids;
    // see the header's honest limit).
    const QString newLayerId = m_uuids.next();
    const QString newOpId = m_uuids.next();
    QVariantMap out = applyCommand(
        QStringLiteral("adjustment.add"), {},
        buildAdjustmentAddParams(opKind, newLayerId, newOpId,
                                 QJsonObject::fromVariantMap(opParams))
            .toVariantMap(),
        {});
    out.insert(QStringLiteral("masked"), false);
    if (out.value(QStringLiteral("ok")).toBool() != true)
        return out; // engine refused; nothing was authored, remember nothing
    m_lastAdjustmentOpId = newOpId;
    // SELECT WHAT WE JUST CREATED, and do it BEFORE the mask step below:
    // the row-1 gesture masks the OP, but a user's next action is
    // frequently a LAYER mask from the panel, and that reads
    // activeLayerIndex. Without this the adjustment is added and the
    // previous row stays selected — the demo flow masks the wrong layer
    // and looks right until the adjustment is toggled off.
    selectLayerById(newLayerId);
    out.insert(QStringLiteral("opId"), newOpId);
    out.insert(QStringLiteral("layerId"), newLayerId);
    // No selection = no mask. The engine would answer
    // error.command.selection_required, so asking would manufacture a
    // rejection out of a correct outcome: an unmasked adjustment layer is
    // what "add an adjustment with nothing selected" MEANS. The
    // applyCommand above already reported the add on the status bar.
    if (!m_selectionActive)
        return out;
    const QVariantMap maskResult = maskAdjustmentFromSelection();
    // The mask submission's own result becomes the answer — its status
    // line (applied or the engine's typed refusal) is already on screen,
    // and a caller that branched on `ok` must see the LAST engine verdict,
    // never the add's stale success.
    QVariantMap merged = maskResult;
    merged.insert(QStringLiteral("opId"), newOpId);
    merged.insert(QStringLiteral("layerId"), newLayerId);
    merged.insert(QStringLiteral("masked"),
                  maskResult.value(QStringLiteral("ok")).toBool() == true);
    return merged;
}

QVariantMap DocumentController::maskAdjustmentFromSelection()
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    // Client-side precondition MIRRORS of the two gates the chrome's
    // availability row reads. Both name causes the engine itself would
    // refuse on (bad_target for an unresolvable op, selection_required for
    // no selection) — belt-and-braces, the reselect() precedent.
    if (m_lastAdjustmentOpId.isEmpty()) {
        // Client-side precondition mirror of the
        // layer.maskAdjustmentFromSelection row (the chrome disables on
        // it; belt-and-braces here) — the reselect() house pattern, and
        // the generic client-precondition code it uses is carried
        // VERBATIM rather than improved on.
        //
        // BE CLEAR ABOUT WHAT THIS CODE IS: `ui.error.no_document` is the
        // house marker for "a client precondition stopped this before the
        // engine saw it", NOT an accurate name for this cause — the
        // document is usually open and the real cause is "this shell has
        // authored no adjustment op". No cataloged id names that, so the
        // availability row carries NO reasonId (rather than a wrong one)
        // and the id is queued to O-I18N in the lane log. Deviating here
        // would mean inventing a code, which is the larger defect.
        out.insert(QStringLiteral("code"),
                   QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    return applyCommand(
        QStringLiteral("op.set_mask"), {},
        buildOpSetMaskFromSelectionParams(m_lastAdjustmentOpId).toVariantMap(),
        {});
}

void DocumentController::bindAutosaveIfSafe(const QString &packagePath)
{
    // §7.8 protection with the review-D1 guard: appname_doc_autosave_begin
    // TRUNCATES the log at the path, so binding blindly would destroy a
    // crash log holding unsaved work (from a crash of this document, or of
    // whatever previously lived at this path). Query first; a recoverable
    // log (commands > 0 or torn) is left INTACT — this session then runs
    // unprotected for this path, honestly, until the W05 prompt lands
    // (dialog.recovery.* catalog coordination; CLI `recover` meanwhile).
    // A never-saved document stays unprotected until first Save (header
    // posture). Bind failures are silent-by-necessity (no cataloged
    // notice id) — queued mint, lane log.
    if (!abiHasBlock12())
        return;
    const QString logPath = packagePath + QStringLiteral(".autosave");
    QJsonObject report;
    const CallResult q = autosaveQuery(logPath, &report);
    const bool recoverable = q.ok
        && (report.value(QLatin1String("commands")).toInt() > 0
            || report.value(QLatin1String("torn")).toBool());
    // Existing-but-unreadable is NOT missing: a query failure on a file
    // that exists means real IO trouble — never truncate possibly-
    // recoverable bytes (re-review R1).
    const bool unreadableButPresent = !q.ok && QFileInfo::exists(logPath);
    if (recoverable || unreadableButPresent) {
        // Skipping the bind must also END any previous binding: engine-
        // side framing into the OLD path's sidecar while the disk base is
        // now THIS path would be unpairable on both sides (re-review R2;
        // end is idempotent, a no-op on fresh handles).
        m_doc.autosaveEnd();
        return;
    }
    m_doc.autosaveBegin(logPath);
}

bool DocumentController::exportDocument(const QString &pathOrUrl,
                                        const QString &format,
                                        int quality)
{
    if (!m_doc.isOpen() || !abiHasBlock12())
        return false;
    const QString path = toLocalPath(pathOrUrl);
    QByteArray options;
    if (quality >= 0) {
        // §8.2: quality is jpeg/webp-only; other formats reject typed —
        // the dialog only offers it there, this is defense.
        options = QByteArrayLiteral("{\"quality\":") + QByteArray::number(quality)
            + QByteArrayLiteral("}");
    }
    const CallResult r = m_doc.exportComposite(path, format, options);
    if (!r.ok) {
        setRejected(r.code, r.message);
        return false;
    }
    // A11Y-3 wave B: ui.status.exported landed with the 17-row batch —
    // the staged->emitted adoption replaces the recorded "Saved {path}"
    // stand-in.
    setStatus(trArgs(m_catalog, QStringLiteral("ui.status.exported"),
                     {{QStringLiteral("path"), path}}));
    // a11y.job.completed "{label} finished": the label is the written
    // file's basename — document content (i18n-exempt), and the truthful
    // name of what just finished. (The §9.5 async-job surface with
    // progressbar does not exist in this shell yet; the export is
    // synchronous — the completion is still the user-meaningful event.)
    if (m_announcer) {
        const QString base = path.section(QLatin1Char('/'), -1);
        m_announcer->announce(QStringLiteral("job"),
                              QStringLiteral("a11y.job.completed"),
                              {{QStringLiteral("label"), base}});
    }
    return true;
}

bool DocumentController::openImageAsDocument(const QString &pathOrUrl)
{
    // The document takes the image's STORED pixel size (no EXIF transform):
    // importImage() pastes the decoded samples at (0,0) as they are, so this
    // is the size that frames them exactly. QImageReader reads the header
    // only; the shipped app deploys the gif/jpeg/tiff/webp readers and png/bmp
    // are built into QtGui. An unreadable header is refused with the same
    // registered io code importImage() uses for a read failure.
    const QString path = toLocalPath(pathOrUrl);
    QImageReader reader(path);
    reader.setAutoTransform(false);
    const QSize size = reader.size();
    if (!size.isValid() || size.isEmpty()) {
        setRejected(QStringLiteral("error.io.read_failed"), path);
        return false;
    }
    if (!newDocument(size.width(), size.height(), QStringLiteral("transparent")))
        return false;
    return importImage(pathOrUrl);
}

bool DocumentController::importImage(const QString &pathOrUrl)
{
    if (!m_doc.isOpen() || !abiHasBlock12())
        return false;
    const QString path = toLocalPath(pathOrUrl);
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        // Shell-side read failure surfaced with the registered io code as
        // data (same rendering path as engine rejections; review D15).
        setRejected(QStringLiteral("error.io.read_failed"), path);
        return false;
    }
    const QByteArray bytes = f.readAll();
    QString code;
    QString message;
    const auto assetId = ingestSamplesBlob(m_doc.raw(), bytes, &code, &message);
    if (!assetId) {
        setRejected(code, message);
        return false;
    }
    QVariantMap params;
    params.insert(QStringLiteral("assetId"), *assetId);
    params.insert(QStringLiteral("at"), QVariantList{0, 0});
    params.insert(QStringLiteral("newLayerId"), m_uuids.next());
    const QVariantMap r =
        applyCommand(QStringLiteral("edit.paste"), {}, params, {});
    return r.value(QStringLiteral("ok")).toBool();
}

std::optional<QString> DocumentController::ingestClipboardRaster()
{
    const QImage img = QGuiApplication::clipboard()->image();
    if (img.isNull())
        return std::nullopt;
    // Ingress transport: the engine's raster arm ingests PNG bytes; the
    // OS clipboard hands raw pixels. Serializing them to PNG here is the
    // OS-bridge encoding of FOREIGN content on its way in (the journaled
    // asset then makes replay byte-identical) — not document pixel math.
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    if (!img.save(&buffer, "PNG")) {
        setRejected(QStringLiteral("error.io.write_failed"), QStringLiteral("clipboard PNG serialization"));
        return std::nullopt;
    }
    QString code;
    QString message;
    const auto assetId = ingestSamplesBlob(m_doc.raw(), bytes, &code, &message);
    if (!assetId) {
        setRejected(code, message);
        return std::nullopt;
    }
    return assetId;
}

bool DocumentController::pasteFromClipboard()
{
    if (!m_doc.isOpen() || !abiHasBlock12())
        return false;
    const auto assetId = ingestClipboardRaster();
    if (!assetId)
        return false;
    QVariantMap params;
    params.insert(QStringLiteral("assetId"), *assetId);
    params.insert(QStringLiteral("newLayerId"), m_uuids.next());
    // `at` omitted: the engine journals the resolved viewport-centred
    // placement per the contract.
    const QVariantMap r =
        applyCommand(QStringLiteral("edit.paste"), {}, params, {});
    return r.value(QStringLiteral("ok")).toBool();
}

bool DocumentController::pasteIntoFromClipboard()
{
    if (!m_doc.isOpen() || !abiHasBlock12())
        return false;
    const auto assetId = ingestClipboardRaster();
    if (!assetId)
        return false;
    QVariantMap params;
    params.insert(QStringLiteral("assetId"), *assetId);
    // §9.6 client-generated creation uuids — BOTH are required by
    // edit.paste_into's paramsSchema (the mask is a second created node,
    // and additionalProperties:false makes the set exact).
    params.insert(QStringLiteral("newLayerId"), m_uuids.next());
    params.insert(QStringLiteral("newMaskId"), m_uuids.next());
    // `at` omitted for the same reason edit.paste omits it: the engine
    // resolves and journals the placement (replay never reads shell
    // state). The SELECTION is what confines the result — the engine
    // rasterizes it into the attached mask; no shell-side placement math
    // could add containment the mask does not already give.
    //
    // NO shell-side selection gate: the engine owns the precondition
    // (error.command.selection_required, §40.5) and applyCommand
    // surfaces its typed refusal on ui.status.rejected. The chrome is
    // gated by the edit.pasteInto availability row instead.
    const QVariantMap r =
        applyCommand(QStringLiteral("edit.paste_into"), {}, params, {});
    return r.value(QStringLiteral("ok")).toBool();
}

void DocumentController::trackCreatedPath(const QString &pathId)
{
    m_lastCreatedPathId = pathId;
    emit infoChanged();
}

bool DocumentController::selectionFromLastPath()
{
    if (m_lastCreatedPathId.isEmpty())
        return false;
    QVariantMap params;
    params.insert(QStringLiteral("pathId"), m_lastCreatedPathId);
    params.insert(QStringLiteral("antialias"), true);
    const QVariantMap r = applyCommand(
        QStringLiteral("selection.from_path"), {}, params, {});
    return r.value(QStringLiteral("ok")).toBool();
}

QVariantMap DocumentController::saveSelectionAsChannel()
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (!m_selectionActive) {
        // Client-side precondition mirror of the §40.4 stage-4 gate
        // (chrome disables on selectionActive; belt-and-braces here).
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    const QString channelId = newUuid();
    QVariantMap params;
    params.insert(QStringLiteral("newChannelId"), channelId);
    const QVariantMap r =
        applyCommand(QStringLiteral("selection.save"), {}, params, {});
    if (r.value(QStringLiteral("ok")).toBool()) {
        // AFTER applyCommand's pollNow: m_journalCursor is fresh — the
        // cursor position with the save applied is the invalidation key
        // (see refreshInfo).
        m_lastSavedChannelId = channelId;
        m_lastSavedChannelCursor = m_journalCursor;
        emit infoChanged();
    }
    return r;
}

QVariantMap DocumentController::loadSavedChannelSelection()
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (m_lastSavedChannelId.isEmpty()) {
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    QVariantMap source;
    source.insert(QStringLiteral("channelId"), m_lastSavedChannelId);
    QVariantMap params;
    params.insert(QStringLiteral("source"), source);
    return applyCommand(QStringLiteral("selection.load"), {}, params, {});
}

QVariantMap DocumentController::loadLayerAlphaSelection(const QString &layerId)
{
    QVariantMap out;
    out.insert(QStringLiteral("ok"), false);
    if (layerId.isEmpty()) {
        out.insert(QStringLiteral("code"), QStringLiteral("ui.error.no_document"));
        out.insert(QStringLiteral("message"), QString());
        return out;
    }
    QVariantMap source;
    source.insert(QStringLiteral("layerAlpha"), layerId);
    QVariantMap params;
    params.insert(QStringLiteral("source"), source);
    return applyCommand(QStringLiteral("selection.load"), {}, params, {});
}

QVariantMap DocumentController::queryRecovery(const QString &pathOrUrl)
{
    QVariantMap out;
    out.insert(QStringLiteral("available"), false);
    if (!abiHasBlock12())
        return out;
    const QString logPath = toLocalPath(pathOrUrl) + QStringLiteral(".autosave");
    QJsonObject report;
    const CallResult q = autosaveQuery(logPath, &report);
    const int commands = report.value(QLatin1String("commands")).toInt();
    const bool torn = report.value(QLatin1String("torn")).toBool();
    if (!q.ok || (commands == 0 && !torn))
        return out; // absent, unreadable (preserved), or cleanly folded
    out.insert(QStringLiteral("available"), true);
    out.insert(QStringLiteral("commands"), commands);
    out.insert(QStringLiteral("torn"), torn);
    out.insert(QStringLiteral("logPath"), logPath);
    return out;
}

bool DocumentController::openRecoveredDocument(const QString &packagePathOrUrl,
                                               const QString &logPath)
{
    if (!abiHasBlock12())
        return false;
    const QString path = toLocalPath(packagePathOrUrl);
    AbiDoc fresh;
    QJsonObject report;
    const CallResult r = fresh.openRecovered(path, logPath, &report);
    if (!r.ok) {
        // replay_failed's typed detail rides the standard rejected status;
        // the log is untouched (contract) — nothing lost.
        setRejected(r.code, r.message);
        return false;
    }
    m_doc = std::move(fresh);
    m_documentPath = path;
    emit documentPathChanged();
    m_rootLayerId.clear();
    m_lastSavedChannelId.clear(); // cross-document handle (see openDocument)
    m_lastSelectionSetParams.clear(); // 14b reselect memory: same rule
    m_hasSelectionMemory = false;
    // Row 1: the authored-op memory is document state for the same reason
    // — an op id from the previous document resolves to nothing here, and
    // op.set_mask would answer bad_target. Clearing it keeps the
    // availability row honest instead of offering a control that refuses.
    m_lastAdjustmentOpId.clear();
    // Cross-document mask-cache safety (see newDocument).
    ++m_selectionRevision;
    m_selectionMaskCache = QImage();
    m_selectionMaskCacheRevision = -1;
    m_activeLayerIndex = 0;
    emit activeLayerIndexChanged();
    refreshInfo();
    // Recovered state is NOT the on-disk state: dirty until saved (the
    // -1 sentinel never equals a journal cursor).
    m_savedCursor = -1;
    m_pendingRecoveryLog = logPath; // resolved (discarded) on save-to-path
    emit infoChanged();
    refreshHistory();
    emit documentReset();
    pollNow();
    provisionSystemFaces(); // ABI 1.6 font feed (per-document FontDb)
    m_pollTimer.start();
    setStatus(trArgs(m_catalog, QStringLiteral("ui.status.opened"),
                     {{QStringLiteral("path"), path}}));
    return true;
}

bool DocumentController::discardRecoveryLog(const QString &logPath)
{
    if (!abiHasBlock12())
        return false;
    return autosaveDiscard(logPath).ok;
}

bool DocumentController::copyCompositeToClipboard(const QRect &docRect)
{
    if (!m_doc.isOpen())
        return false;
    const QRect full(0, 0, m_docWidth, m_docHeight);
    const QRect rect = docRect.isValid() && !docRect.isEmpty()
        ? docRect.intersected(full)
        : full;
    if (rect.isEmpty())
        return false;
    // Chunked assembly keeps every readback far under the 2^26 px cap.
    constexpr int kChunk = 2048;
    QImage img(rect.width(), rect.height(), QImage::Format_RGBA8888);
    img.fill(Qt::transparent);
    QPainter p(&img);
    p.setCompositionMode(QPainter::CompositionMode_Source);
    for (int y = rect.y(); y < rect.y() + rect.height(); y += kChunk) {
        for (int x = rect.x(); x < rect.x() + rect.width(); x += kChunk) {
            const QRect chunk(x, y,
                              std::min(kChunk, rect.x() + rect.width() - x),
                              std::min(kChunk, rect.y() + rect.height() - y));
            QImage part;
            if (!m_doc.readRegion(chunk, &part).ok)
                return false;
            p.drawImage(chunk.topLeft() - rect.topLeft(), part);
        }
    }
    p.end();
    QGuiApplication::clipboard()->setImage(img);
    setStatus(m_catalog->tr(QStringLiteral("ui.status.copied")));
    return true;
}

void DocumentController::strokeBegin(const QString &kind, const QColor &color, double sizePx,
                                     std::optional<double> hardness,
                                     const QString &mode, bool sampleAllLayers,
                                     const QPointF &cloneOffset)
{
    // §5 tier-2 fix round (review minor-2): a stroke session START is a
    // submission attempt — the review's demonstration (a live-wired
    // armed tool riding the brush branch) reaches here even when the
    // engine later rejects the kind and history never grows.
    ++m_submissionAttempts;
    m_strokeActive = true;
    m_strokeKind = kind;
    m_strokeMode = mode;
    m_strokeSampleAllLayers = sampleAllLayers; // row 5: builder kind-guards the emit
    m_strokeCloneOffset = cloneOffset;         // B-02: clone.stroke only, kind-guarded
    m_strokeColor = color;
    // The canvas clamps its own brushSize, but this is a QML-invokable
    // surface: sanitize so a non-finite/out-of-range size cannot reach the
    // envelope (Qt serializes non-finite doubles as JSON null — a silent
    // type change the engine would reject on type, not range; tester T4).
    m_strokeSizePx = std::isfinite(sizePx) ? std::clamp(sizePx, 0.1, 10000.0) : 16.0;
    // Item 4: same sanitation for the hardness override (contract 0..=1).
    // O-29: an ABSENT hardness stays absent — it is not a number to
    // sanitize but the caller declining to override, and the envelope must
    // omit the param so `preset.hardness` governs. A non-finite EXPLICIT
    // value also degrades to absent rather than to a made-up 0.0: the
    // shell has no honest number to substitute (it does not know the
    // preset's hardness — the engine owns that), and falling back to the
    // preset is the one choice that cannot silently contradict it.
    if (hardness && std::isfinite(*hardness))
        m_strokeHardness = std::clamp(*hardness, 0.0, 1.0);
    else
        m_strokeHardness = std::nullopt;
    m_strokeSamples.clear();
}

void DocumentController::strokeAppend(double docX, double docY, double pressure, double tSeconds)
{
    if (!m_strokeActive)
        return;
    appendNormalizedSample(m_strokeSamples, docX, docY, pressure, tSeconds);
}

void DocumentController::strokeCancel()
{
    if (!m_strokeActive)
        return;
    m_strokeActive = false;
    m_strokeSamples.clear();
    // Deliberately does NOT bump m_submissionAttempts: strokeBegin already
    // counted the door entry, and an abandoned stroke submits nothing.
    // Deliberately emits NO strokeCommitted: the false arm of that signal
    // is the typed-rejection path, which PRESERVES the echo.
}

bool DocumentController::strokeEnd()
{
    if (!m_strokeActive)
        return false;
    m_strokeActive = false;
    if (m_strokeSamples.isEmpty() || !m_doc.isOpen()) {
        emit strokeCommitted(false);
        return false;
    }

    // 1. Canonical appname.samples.v1 bytes (bit-exact contract, tested).
    //    The blob is built even though v1 ingestion may be absent: it is
    //    the persisted-format proof and the validator run.
    const auto blob = encodeSamplesBlob(m_strokeSamples);
    if (!blob) {
        setRejected(QStringLiteral("samples.invalid"), validateSamples(m_strokeSamples).detail);
        emit strokeCommitted(false);
        return false;
    }

    // 2. Ingest the canonical blob through appname_asset_create (the §9.2
    //    ingress as landed). The ingress is part of the linked ABI now: a
    //    rejection here is engine truth about these exact bytes — surface
    //    it typed and keep the echo as a pending preview. No client-uuid
    //    fallback envelope: it could only fail later with a less precise
    //    bad_target.
    QString ingestCode;
    QString ingestMessage;
    const auto assetId = ingestSamplesBlob(m_doc.raw(), *blob, &ingestCode, &ingestMessage);
    if (!assetId) {
        setRejected(ingestCode, ingestMessage);
        pollNow();
        emit strokeCommitted(false);
        return false;
    }

    StrokeParams params;
    params.kind = m_strokeKind;
    params.mode = m_strokeMode;
    params.sampleAllLayers = m_strokeSampleAllLayers;
    params.cloneOffset = m_strokeCloneOffset;
    params.targetLayerId = activeLayerId();
    params.samplesAssetId = *assetId;
    params.sizePx = m_strokeSizePx;
    // brush.stroke only on the wire, and only when SET (builder omits an
    // unset hardness so the engine's preset hardness governs — O-29).
    params.hardness = m_strokeHardness;
    params.color = m_strokeColor;
    // Masked at generation so the seed the shell holds IS the recorded
    // seed (the builder masks identically — recorded == generated by
    // construction; tester T3).
    params.seed = (m_deterministic ? m_strokeSeedCounter++
                                   : QRandomGenerator::global()->generate64())
        & 0x7fffffffffffffffULL;

    const QByteArray envelope = buildStrokeEnvelope(m_uuids.next(), params);
    const CallResult r = m_doc.apply(envelope);
    if (r.ok) {
        setStatus(trArgs(m_catalog, QStringLiteral("ui.status.applied"),
                         {{QStringLiteral("label"),
                           m_catalog->tr(QStringLiteral("history.") + m_strokeKind)}}));
    } else if (r.code == QLatin1String("error.command.not_implemented")) {
        // Typed-resilience branch: all 102 kinds are registered from
        // CP-0.1, so an unimplemented kind answers typed and the echo
        // overlay stays as a preview. Stroke kinds implement since the
        // cabi flip; this branch guards any future kind the shell grows
        // before its engine wiring ships.
        setStatus(trArgs(m_catalog, QStringLiteral("ui.status.stroke_preview_pending"),
                         {{QStringLiteral("kind"), m_strokeKind},
                          {QStringLiteral("code"), r.code}}));
    } else {
        setStatusFromResult(m_strokeKind, r);
    }
    pollNow();
    const bool committed = r.ok || r.status == APPNAME_ERR_IO; // review D2
    // ---- Q-D7 RIDER 1: THE DEFINED NO-OP GETS A VOICE (operator, 2026-08-29) ----
    //
    // ★★★ THE FAILURE THIS CLOSES IS SUCCESS-SHAPED SILENCE. heal.stroke
    // onto a fully transparent layer is a DEFINED no-op -- the contract
    // says so itself (heal.stroke.yaml:210): "healing fully transparent
    // pixels is a defined no-op on visible output", because heal never
    // modifies alpha. Measured on the demo document: the stroke is
    // ACCEPTED, a history entry APPEARS, and the composite is
    // byte-identical. No error, no refusal, no toast. Every signal a
    // presenter would check says success, which is why this cost a demo
    // step rather than an afternoon.
    //
    // ★★ WHY `actionUnavailable` AND NOT `commandRefused`, since neither
    // name fits an action that RAN. commandRefused is documented "emitted
    // ONLY on rejection" and carries the engine's `error.*` taxonomy;
    // this was not rejected, and inventing an error code for a successful
    // command is a lie of a different shape. actionUnavailable carries
    // `ui.*` ids and its own stated PURPOSE is the one that fits exactly:
    // "the user presses Cmd+T with no document and gets silence. This is
    // the door that makes that press speak." A user action produced
    // silence; this makes it speak. Its consumer sets code="" so no error
    // code is shown -- the toast reads as the NOTICE it is.
    //
    // ★ THE PREDICATE IS THE CONTRACT'S OWN CONDITION, not a proxy:
    // layerContentBounds returns ALPHA bounds, and an all-transparent
    // layer has none. Checked AFTER the commit deliberately -- heal never
    // modifies alpha, so an empty target is still empty afterwards, and
    // reading after means the notice describes what happened rather than
    // what was predicted.
    if (committed && m_strokeKind == QLatin1String("heal.stroke")) {
        const QVariantMap b = layerContentBounds(params.targetLayerId);
        if (b.value(QStringLiteral("ok")).toBool()
            && (b.value(QStringLiteral("w")).toInt() <= 0
                || b.value(QStringLiteral("h")).toInt() <= 0)) {
            emit actionUnavailable(
                QStringLiteral("heal.stroke"),
                QStringLiteral("ui.notice.heal_no_effect_empty_target"));
        }
    }
    emit strokeCommitted(committed);
    return committed;
}

void DocumentController::statusPending(const QString &messageId,
                                       const QVariantMap &args)
{
    // Catalog gate first: an uncataloged id is REFUSED, never echoed.
    // Key-echoing a raw id at the user is the P2-D2 regression class,
    // and the announcer already refuses on the speech half — the status
    // half must not be the softer door.
    if (!m_catalog || messageId.isEmpty() || !m_catalog->has(messageId))
        return;
    QVector<QPair<QString, QString>> ordered;
    ordered.reserve(args.size());
    for (auto it = args.constBegin(); it != args.constEnd(); ++it)
        ordered.append({it.key(), it.value().toString()});
    setStatus(m_catalog->tr(messageId, ordered));
}

bool DocumentController::readRegionImage(const QRect &rect, QImage *out)
{
    if (!m_doc.isOpen())
        return false;
    const auto r = m_doc.readRegion(rect, out);
    if (!r.ok && !r.code.isEmpty()) {
        // A TYPED composite refusal must not freeze the canvas silently
        // (slice-V flip verification: a document holding a layer kind
        // the engine has no composite path for yet refuses WHOLE-
        // composite readback typed — e.g. shape layers pre-render-leg).
        // The last-good cache stays up in the view; the refusal reaches
        // the user through the standard rejection surface.
        setRejected(r.code, r.message);
    }
    return r.ok;
}

bool DocumentController::readSelectionImage(const QRect &rect, QImage *out)
{
    if (!m_doc.isOpen() || !abiHasBlock12())
        return false;
    ++m_selectionMaskReads; // probe/test truth (selectionMaskReads())
    return m_doc.readSelection(rect, out).ok;
}

bool DocumentController::selectionMaskFull(QImage *out)
{
    // Repaint-scoping slice (sweep F1b/F4): ONE full-doc Grayscale8 read
    // per selection revision serves BOTH consumers (canvas ants
    // vectorizer + announcer bounds scan). Refusal arms fall back to the
    // callers' own windowed/strip reads; QImage implicit sharing makes
    // serving the cache pixel-copy-free.
    if (!m_doc.isOpen() || !abiHasBlock12() || !m_selectionActive)
        return false;
    if (m_docWidth <= 0 || m_docHeight <= 0)
        return false;
    if (static_cast<qint64>(m_docWidth) * m_docHeight > kSharedMaskCapPx)
        return false; // past the share cap: no full-doc retention
    if (m_selectionMaskCacheRevision == m_selectionRevision
        && !m_selectionMaskCache.isNull()) {
        *out = m_selectionMaskCache;
        return true;
    }
    QImage mask;
    if (!readSelectionImage(QRect(0, 0, m_docWidth, m_docHeight), &mask))
        return false;
    m_selectionMaskCache = mask;
    m_selectionMaskCacheRevision = m_selectionRevision;
    *out = m_selectionMaskCache;
    return true;
}

void DocumentController::pollNow()
{
    if (!m_doc.isOpen())
        return;
    QJsonArray events;
    if (!m_doc.pollEvents(&events).ok)
        return;
    if (events.isEmpty())
        return;

    QVariantList dirty;
    bool history = false;
    bool selectionKindHint = false;
    for (const QJsonValue &ev : events) {
        const QJsonObject obj = ev.toObject();
        const QString schema = obj.value(QLatin1String("schema")).toString();
        if (schema == QLatin1String("appname.event.dirtyRects.v1")) {
            const QJsonArray rects = obj.value(QLatin1String("rects")).toArray();
            for (const QJsonValue &rv : rects) {
                const QJsonArray r = rv.toArray();
                if (r.size() == 4)
                    dirty.append(QRect(r.at(0).toInt(), r.at(1).toInt(),
                                       r.at(2).toInt(), r.at(3).toInt()));
            }
        } else if (schema == QLatin1String("appname.event.history.v1")) {
            history = true;
            // Repaint-scoping slice (sweep F1a): the event's
            // labelMessageId carries the STEP's kind on apply, undo, and
            // redo (cabi events.rs emit_mutation; doc.rs call sites) —
            // the KIND side of the two-sided revision gate. Overflow
            // coalescing keeps only the newest history row, so a batch
            // could under-report kinds; the refreshInfo state-compare
            // side stays the safety net for that (recorded).
            if (isSelectionWriterKind(
                    obj.value(QLatin1String("labelMessageId")).toString()))
                selectionKindHint = true;
        }
        // Unknown schemas/fields ignored by contract (ES §2.8 mirror).
    }
    // History note: the §9.5 set_props interim repaint LIVED here
    // (poll-batch-shape conservative re-read) until the engine's
    // dirtyRects slice merged — visual props now dirty their composite
    // bounds engine-side (contract-mandated, undo/redo echo), which is
    // precisely the interim's pinned self-disarm condition; the code
    // was REMOVE-marked for that arrival and removed at it.
    if (!dirty.isEmpty())
        emit regionsDirty(dirty);
    if (history) {
        // Repaint-scoping slice (sweep F1a): the unconditional
        // ++m_selectionRevision that lived here (the F1 finding's cite)
        // moved INTO refreshInfo's two-sided gate — writer-kind hint OR
        // (active, bounds) state delta — so a non-selection commit no
        // longer re-triggers the ants read.
        m_selectionKindHint = selectionKindHint;
        // Conservative: any journal transition may have removed the
        // tracked path (undo of path_new) — drop the transient handle
        // EXCEPT on the transition that created it (trackCreatedPath runs
        // after applyCommand's pollNow, so the ordering is safe).
        m_lastCreatedPathId.clear();
        // The saved-channel handle is NOT blanket-cleared here: its
        // invalidation is cursor-truth in refreshInfo (a blanket clear
        // would make save -> deselect -> load impossible — caught by
        // the tour's own round-trip evidence).
        refreshInfo();
        m_selectionKindHint = false;
        refreshHistory();
    }
}

void DocumentController::setStatus(const QString &text)
{
    if (m_statusText == text)
        return;
    m_statusText = text;
    emit statusTextChanged();
}

bool DocumentController::selectLayerById(const QString &layerId)
{
    if (layerId.isEmpty())
        return false;
    for (int i = 0; i < m_layers.size(); ++i) {
        if (m_layers.at(i).toMap().value(QStringLiteral("layerId")).toString()
            == layerId) {
            setActiveLayerIndex(i);
            return true;
        }
    }
    return false;
}

void DocumentController::reportUnavailable(const QString &action)
{
    // P1-4(a): a chord whose precondition is unmet must SAY SO.
    //
    // WHY THIS IS NOT setRejected(). That helper renders
    // "Rejected — {code}: {message}", which is right for an ENGINE code
    // and would print a raw MessageId at the user here: a reasonId is a
    // cataloged id to be TRANSLATED, not a wire code to be displayed.
    // Routing this through it would have produced
    // "Rejected — ui.error.no_document:" on screen — the P2-D2 key-echo
    // regression wearing a refusal's clothes.
    const QVariantMap r =
        commandAvailability().value(action).toMap();
    // UNKNOWN ACTION: say nothing. A typo'd action name must not
    // manufacture a user-visible message about a row that does not
    // exist, and the silence is testable.
    if (r.isEmpty())
        return;
    // ACTUALLY ENABLED: say nothing. This door REPORTS a state, it does
    // not decide one — if a caller asks about an enabled action the
    // caller is wrong, and inventing a refusal would make the chrome
    // disagree with the availability surface it is built on.
    if (r.value(QStringLiteral("enabled")).toBool())
        return;
    // THE REASON. Prefer the row's own reasonId, which names the FAILING
    // CONJUNCT. Fall back to the generic id only when the row carries
    // none — the reasonId policy forbids naming the WRONG conjunct, not
    // saying something true-but-unspecific, and what the fallback
    // replaces is SILENCE.
    QString reasonId = r.value(QStringLiteral("reasonId")).toString();
    if (reasonId.isEmpty())
        reasonId = QStringLiteral("ui.error.action_unavailable");
    // Catalog gate, same policy as statusPending and the announcer: an
    // uncataloged id is refused, never echoed at a user.
    if (!m_catalog || !m_catalog->has(reasonId)) {
        // REFUSE, COUNT AND WARN — the Announcer's shape, adopted here
        // for the hazard O-INT measured on this door. Returning silently
        // is what the gate used to do, and a silent return is
        // indistinguishable from the bug this whole door exists to fix.
        ++m_unavailableRefused;
        qWarning("reportUnavailable: refused uncataloged reasonId '%s' for "
                 "action '%s' — the chord is wired and will stay MUTE",
                 qPrintable(reasonId), qPrintable(action));
        return;
    }
    setStatus(m_catalog->tr(reasonId));
    // The a11y arm and the event, for the same reason the refusal
    // channel has them: setStatus above dedupes on identical text, so
    // pressing an unavailable chord twice would otherwise go quiet after
    // the first press.
    if (m_announcer)
        m_announcer->announce(QStringLiteral("unavailable"), reasonId, {});
    emit actionUnavailable(action, reasonId);
}

void DocumentController::setRejected(const QString &code,
                                    const QString &message)
{
    setStatus(trArgs(m_catalog, QStringLiteral("ui.status.rejected"),
                     {{QStringLiteral("code"), code},
                      {QStringLiteral("message"), message}}));
    // THE A11Y ARM. Refusals were the one class this shell never spoke:
    // every announce() call site named an a11y.* id and not one of them
    // was a refusal, so a screen-reader user got silence where a sighted
    // user got a status line. No mint is needed — the Announcer gates on
    // catalog membership, not on the `a11y.` prefix, and
    // ui.status.rejected is the same cataloged string the status bar
    // renders. Its own SOURCE bucket, so a refusal is never coalesced
    // away by a tool-change or zoom announcement 500 ms later.
    if (m_announcer)
        m_announcer->announce(QStringLiteral("refusal"),
                              QStringLiteral("ui.status.rejected"),
                              {{QStringLiteral("code"), code},
                               {QStringLiteral("message"), message}});
    // THE EVENT, emitted UNCONDITIONALLY — after setStatus, which may
    // have deduped, and independent of whether it did.
    emit commandRefused(code, message);
}

void DocumentController::setStatusFromResult(const QString &kind, const CallResult &result)
{
    // Applied-but-not-durable (ERR_IO under a live autosave binding) IS
    // applied — say so; the distinct durability-warning text is a queued
    // mint (review D2).
    if (result.ok || result.status == APPNAME_ERR_IO) {
        setStatus(trArgs(m_catalog, QStringLiteral("ui.status.applied"),
                         {{QStringLiteral("label"),
                           m_catalog->tr(QStringLiteral("history.") + kind)}}));
        return;
    }
    // P1-2. ONE line, because setRejected does all three things the
    // merged-away version did inline (status text, a11y announcement,
    // and the unconditional emit) and does them for all 20 refusal
    // paths in this file rather than for this one.
    setRejected(result.code, result.message);
}

void DocumentController::refreshInfo()
{
    ++m_infoRevision; // bust thumbnail URLs exactly when info truth moves
    // Repaint-scoping slice (sweep F1a/F2): pre-refresh overlay truth —
    // the STATE-COMPARE side of the two-sided selection-revision gate
    // (with m_selectionKindHint, pollNow's polled-writer-kind side), plus
    // the guides/grid delta that scopes overlayChanged.
    const bool prevSelActive = m_selectionActive;
    const QRect prevSelBounds = m_selectionBounds;
    const QVariantList prevGuidesH = m_guidesH;
    const QVariantList prevGuidesV = m_guidesV;
    const QVariantMap prevGrid = m_gridSpec;
    const CallResult r = m_doc.infoJson();
    if (!r.ok) {
        // Pre-init handle (undoing doc.new is legal — the document is gone
        // but the journal is not): info fails typed while history stays
        // legal on a pre-init handle, so derive the undo/redo flags from
        // the journal cursor instead of leaving them stale (review R1 —
        // the Redo button is the only discoverable way back).
        m_hasDocument = false;
        m_layers.clear();
        m_rootLayerId.clear();
        m_selectionActive = false; // no document, no selection chip (R9)
        m_selectionBounds = QRect();
        // No document, no output frame. A stale rect here would survive
        // into the next document's first paint (the m_docWidth/Height
        // pair are deliberately left alone on this path, so the null is
        // what distinguishes "gone" from "0x0").
        m_imageRect = QRect();
        // Belt-and-braces (review observation c): stepping to pre-init
        // necessarily popped any save first (the normal arm cleared),
        // but a no-document state must never hold a channel handle.
        m_lastSavedChannelId.clear();
        const CallResult h = m_doc.historyJson();
        if (h.ok) {
            const int cursor = h.json.value(QLatin1String("cursor")).toInt();
            const int length = h.json.value(QLatin1String("length")).toInt();
            m_canUndo = cursor > 0;
            m_canRedo = cursor < length;
        } else {
            m_canUndo = false;
            m_canRedo = false;
        }
        emit infoChanged();
        // Two-sided gate, failure arm (pre-init: undoing doc.new): losing
        // the document IS a selection state delta when one was active
        // (and the batch's writer-kind hint gates identically).
        if (prevSelActive || m_selectionKindHint) {
            ++m_selectionRevision;
            m_selectionMaskCache = QImage();
            m_selectionMaskCacheRevision = -1;
            emit overlayChanged();
        }
        return;
    }
    const int prevWidth = m_docWidth;
    const int prevHeight = m_docHeight;
    const QRect prevImageRect = m_imageRect;
    const bool hadDocument = m_hasDocument;
    m_hasDocument = true;
    m_docWidth = r.json.value(QLatin1String("width")).toInt();
    m_docHeight = r.json.value(QLatin1String("height")).toInt();
    // ABI 1.7 additive: `imageRect` [x,y,w,h] — the OUTPUT FRAME
    // (image_rect() = crop_rect.unwrap_or(canvas_rect())), ABSOLUTE doc
    // coords, ORIGIN CARRIED. width/height above are UNCHANGED and keep
    // meaning the CANVAS; this reads a second, independent geometry.
    //
    // Bound UNCONDITIONALLY when present: with no crop active the engine
    // publishes the canvas at origin, so there is no state in which the
    // key is present and the canvas fallback would be more correct
    // (include/appname.h says so explicitly). The abiHasBlock17() arm is
    // the PRE-1.7 fallback — a library that does not emit the key at all
    // — and is written as a gate rather than a bare size check so the
    // reason is legible at the call site.
    const QJsonArray imgRect = r.json.value(QLatin1String("imageRect")).toArray();
    if (imgRect.size() == 4) {
        m_imageRect = QRect(imgRect.at(0).toInt(), imgRect.at(1).toInt(),
                            imgRect.at(2).toInt(), imgRect.at(3).toInt());
    } else {
        // Pre-1.7, or a malformed payload: the canvas IS the output frame.
        m_imageRect = QRect(0, 0, m_docWidth, m_docHeight);
    }
    // A degenerate frame would silently null the canvas cache; the canvas
    // rect is the honest floor.
    if (m_imageRect.isEmpty())
        m_imageRect = QRect(0, 0, m_docWidth, m_docHeight);
    const QJsonObject selInfo =
        r.json.value(QLatin1String("selection")).toObject();
    m_selectionActive = selInfo.value(QLatin1String("active")).toBool();
    // info v2 selection.bounds [x,y,w,h] — the engine's tight
    // nonzero-coverage bounds (cabi doc.rs; absent while inactive; §2.8:
    // absent pre-1.2 => null rect, the compare side then rides `active`
    // alone). Truncation to int is complete for the COMPARE use: only a
    // WRITER impl can move bounds at all, and same-(active,bounds)
    // content changes ride the kind side of the gate.
    m_selectionBounds = QRect();
    const QJsonArray selBounds =
        selInfo.value(QLatin1String("bounds")).toArray();
    if (selBounds.size() == 4)
        m_selectionBounds =
            QRect(selBounds.at(0).toInt(), selBounds.at(1).toInt(),
                  selBounds.at(2).toInt(), selBounds.at(3).toInt());
    const QJsonObject journal = r.json.value(QLatin1String("journal")).toObject();
    m_canUndo = journal.value(QLatin1String("canUndo")).toBool();
    m_canRedo = journal.value(QLatin1String("canRedo")).toBool();
    m_journalCursor = journal.value(QLatin1String("cursor")).toInt();
    // Saved-channel handle, engine-truth invalidation (request-9
    // interim): a cursor BELOW the save's recorded position means the
    // selection.save step is undone — the channel may not exist. Forward
    // transitions (deselect, edits, the load itself) keep the handle; a
    // stale id that somehow survives refuses TYPED at the door.
    if (!m_lastSavedChannelId.isEmpty()
        && m_journalCursor < m_lastSavedChannelCursor)
        m_lastSavedChannelId.clear();
    m_layers.clear();
    const QJsonArray layers = r.json.value(QLatin1String("layers")).toArray();
    for (const QJsonValue &lv : layers) {
        const QJsonObject lo = lv.toObject();
        QVariantMap row;
        row.insert(QStringLiteral("index"), lo.value(QLatin1String("index")).toInt());
        row.insert(QStringLiteral("layerId"), lo.value(QLatin1String("id")).toString());
        row.insert(QStringLiteral("name"), lo.value(QLatin1String("name")).toString());
        row.insert(QStringLiteral("visible"), lo.value(QLatin1String("visible")).toBool());
        row.insert(QStringLiteral("opacity"), lo.value(QLatin1String("opacity")).toDouble());
        row.insert(QStringLiteral("kind"), lo.value(QLatin1String("kind")).toString());
        // info v2 additions (§2.8: absent pre-1.2, defaults below).
        row.insert(QStringLiteral("parentId"), lo.value(QLatin1String("parentId")).toString());
        row.insert(QStringLiteral("depth"), lo.value(QLatin1String("depth")).toInt(0));
        row.insert(QStringLiteral("blend"),
                   lo.value(QLatin1String("blend")).toString(QStringLiteral("normal")));
        QVariantList maskIds;
        // Pro-Wave row 6: the info door has ALWAYS carried the full §11.2
        // mask prop set (cabi doc.rs:531-543 emits id/enabled/inverted/
        // density/feather/linked per mask); the shell was throwing five
        // sixths of it away and keeping only the id. `maskIds` keeps its
        // exact prior shape and order for every existing consumer
        // (LayersPanel badge/tag, layerViaCopy's idMap walk); `masks`
        // is the new parallel row carrying ENGINE truth for the property
        // controls, so no surface has to echo back what it just sent.
        QVariantList masks;
        for (const QJsonValue &mv : lo.value(QLatin1String("masks")).toArray()) {
            const QJsonObject mo = mv.toObject();
            maskIds.append(mo.value(QLatin1String("id")).toString());
            QVariantMap m;
            m.insert(QStringLiteral("maskId"), mo.value(QLatin1String("id")).toString());
            // Defaults mirror MaskNode's own construction defaults
            // (crates/appname-core/src/graph/mask_node.rs:48-52) so a
            // pre-1.2 info payload that omits the block degrades to the
            // engine's real starting state rather than to zeroes.
            m.insert(QStringLiteral("enabled"),
                     mo.value(QLatin1String("enabled")).toBool(true));
            m.insert(QStringLiteral("inverted"),
                     mo.value(QLatin1String("inverted")).toBool(false));
            m.insert(QStringLiteral("density"),
                     mo.value(QLatin1String("density")).toDouble(1.0));
            m.insert(QStringLiteral("feather"),
                     mo.value(QLatin1String("feather")).toDouble(0.0));
            m.insert(QStringLiteral("linked"),
                     mo.value(QLatin1String("linked")).toBool(true));
            masks.append(m);
        }
        row.insert(QStringLiteral("maskIds"), maskIds);
        row.insert(QStringLiteral("masks"), masks);
        m_layers.append(row);
    }
    // info v2 top-level truth (replaces the doc.new-receipt capture and
    // works for OPENED documents; pre-1.2 keeps the receipt value).
    const QString v2Root = r.json.value(QLatin1String("rootLayerId")).toString();
    if (!v2Root.isEmpty())
        m_rootLayerId = v2Root;
    m_guidesH.clear();
    m_guidesV.clear();
    const QJsonObject guides = r.json.value(QLatin1String("guides")).toObject();
    for (const QJsonValue &g : guides.value(QLatin1String("horizontal")).toArray())
        m_guidesH.append(g.toDouble());
    for (const QJsonValue &g : guides.value(QLatin1String("vertical")).toArray())
        m_guidesV.append(g.toDouble());
    m_gridSpec = r.json.value(QLatin1String("grid")).toObject().toVariantMap();
    m_profileTag = r.json.value(QLatin1String("profile")).toObject()
                       .value(QLatin1String("name")).toString();
    m_docDepth = r.json.value(QLatin1String("depth")).toString();
    if (m_activeLayerIndex >= m_layers.size()) {
        m_activeLayerIndex = m_layers.isEmpty() ? 0 : m_layers.size() - 1;
        emit activeLayerIndexChanged();
    }
    // 1.0 final F-10 (O-WATCH #1123): after a layer.reorder the moved layer
    // stays the active one (consumed once, by the first refresh after it).
    if (!m_pendingActiveLayerId.isEmpty()) {
        const QString id = m_pendingActiveLayerId;
        m_pendingActiveLayerId.clear();
        selectLayerById(id);
    }
    // Repaint-scoping slice (sweep F1a): the TWO-SIDED selection-revision
    // gate — a polled selection-WRITER kind (the hint; catches content
    // changes with identical bounds, undo/redo included) OR an
    // (active, bounds) delta across this refresh (catches every state
    // move, any kind). The bump busts the shared mask cache (F1b/F4).
    // COMPLETENESS (§5 fix round, CORRECTED — the earlier wording here,
    // "a content change with identical (active,bounds) can only come
    // from a writer impl", was FALSE: history.goto replays writer steps
    // behind its own label). The union is complete GIVEN history.goto's
    // unconditional membership in the writer set — see the corrected
    // argument at isSelectionWriterKind.
    // WHY BOTH SIDES STAY, with the kind side now closed over the
    // replay class: the compare side below is no longer reachable as the
    // SOLE cause of a bump by any kind this engine registers (all 102
    // re-derived: every (active,bounds) mover carries a writer-set
    // label). It is kept, deliberately, as the net for the two things
    // the kind side cannot see — a future engine kind not yet in the
    // list (the MAINTENANCE rule's whole point), and the pollNow
    // coalescing residual, where a >4096-event un-polled batch keeps
    // only the newest history row and can drop the writer label that
    // moved the bounds. Both are unreachable from a shell test today
    // (applyCommand polls every call), so this side is pinned by
    // derivation, not by a case — recorded rather than papered over.
    const bool selectionMoved = m_selectionKindHint
        || prevSelActive != m_selectionActive
        || prevSelBounds != m_selectionBounds;
    if (selectionMoved) {
        ++m_selectionRevision;
        m_selectionMaskCache = QImage();
        m_selectionMaskCacheRevision = -1;
    }
    // F2: overlay truth = selection delta | guides | grid — the canvas's
    // repaint trigger (view-spanning overlays => full update there);
    // info-only commits (rename/availability/history) repaint nothing;
    // composite dirt rides the partial regionsDirty path.
    const bool overlayMoved = selectionMoved || prevGuidesH != m_guidesH
        || prevGuidesV != m_guidesV || prevGrid != m_gridSpec;
    emit infoChanged();
    if (overlayMoved)
        emit overlayChanged();
    // Document geometry changed in place (crop/resize/rotate/canvas ops —
    // or their undo): the doc-sized canvas cache must rebuild. newDocument
    // and openDocument emit their own reset (prev size is 0 there).
    // ★★ THE OUTPUT FRAME IS PART OF THIS GATE, and omitting it makes the
    // whole imageRect consumption INERT for the exact case it exists to
    // serve. A NON-DESTRUCTIVE crop leaves width/height UNCHANGED (200x100
    // stays 200x100) and moves ONLY imageRect — so a width/height-only
    // gate never fires, rebuildCache() never runs, and the canvas keeps
    // painting the stale full-canvas cache no matter how correct the
    // paint path is. The destructive arm DOES change width/height, which
    // is precisely why a size-only test of this gate passes while the
    // non-destructive arm stays broken.
    if (hadDocument && prevWidth > 0
        && (prevWidth != m_docWidth || prevHeight != m_docHeight
            || prevImageRect != m_imageRect)) {
        emit documentReset();
    }
}

void DocumentController::refreshHistory()
{
    const CallResult r = m_doc.historyJson();
    if (!r.ok)
        return;
    m_historyEntries.clear();
    const QJsonArray entries = r.json.value(QLatin1String("entries")).toArray();
    // Wave F: each row carries the history.goto `position` that makes it
    // the newest applied step — the §40.8 undo-stack index = count of
    // non-marker entries through this journal index. † markers
    // (history.snapshot) journal but never occupy a stack slot, so they
    // repeat the preceding step's position (the state the snapshot
    // captured); the kind test mirrors the engine's is_marker_entry,
    // decided from the envelope kind. Protocol mapping over the same
    // history door the panel renders — no engine math.
    int position = 0;
    for (const QJsonValue &ev : entries) {
        const QJsonObject eo = ev.toObject();
        QVariantMap row;
        const QString kind = eo.value(QLatin1String("kind")).toString();
        if (kind != QLatin1String("history.snapshot"))
            ++position;
        row.insert(QStringLiteral("index"), eo.value(QLatin1String("index")).toInt());
        row.insert(QStringLiteral("kind"), kind);
        row.insert(QStringLiteral("active"), eo.value(QLatin1String("active")).toBool());
        row.insert(QStringLiteral("position"), position);
        // B-06 / A-18: the SUBJECT a generic kind cannot name. `filterId`
        // is absent on 101 of the 102 kinds (the engine skips it on the
        // wire), so an empty string here means "this row has no subject"
        // and the panel falls back to the kind's own label. Projected
        // rather than resolved here: the shell holds no catalog in this
        // layer, and the panel already owns label resolution.
        row.insert(QStringLiteral("filterId"),
                   eo.value(QLatin1String("filterId")).toString());
        m_historyEntries.append(row);
    }
    emit historyChanged();
}

QString DocumentController::activeLayerId() const
{
    if (m_activeLayerIndex >= 0 && m_activeLayerIndex < m_layers.size())
        return m_layers.at(m_activeLayerIndex).toMap().value(QStringLiteral("layerId")).toString();
    return QString();
}

} // namespace appname::shell
