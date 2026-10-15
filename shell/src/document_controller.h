// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.2 "Qt/QML shell v1" row; ES §9.5 (event polling: dirtyRects
// -> repaint only dirty regions, history -> refresh panels), §9.6 (every
// mutation an envelope through apply), §10.3/§10.4 (undo/redo surface).
// Pure C-ABI client: this controller builds envelopes, applies them, polls
// events, and republishes engine state to QML. No document math lives here
// — pixels arrive exclusively via readback (canvas_view.cpp).

#pragma once

#include "abi_client.h"
#include "announcer.h"
#include "envelope_builder.h"
#include "i18n_catalog.h"
#include "sample_blob.h"

#include <QObject>
#include <QPointF>
#include <QTimer>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include <optional>
#include <type_traits>

namespace appname::shell {

class DocumentController : public QObject {
    Q_OBJECT
    QML_NAMED_ELEMENT(DocumentController)
    QML_UNCREATABLE("Constructed by the shell and exposed as AppDoc")
    Q_PROPERTY(bool hasDocument READ hasDocument NOTIFY infoChanged)
    Q_PROPERTY(int docWidth READ docWidth NOTIFY infoChanged)
    Q_PROPERTY(int docHeight READ docHeight NOTIFY infoChanged)
    Q_PROPERTY(bool canUndo READ canUndo NOTIFY infoChanged)
    Q_PROPERTY(bool canRedo READ canRedo NOTIFY infoChanged)
    // Standing priority (4)-3: THE DIRTY STATE, EXPOSED. The computation
    // has been correct since 7e8bcfa9 (2026-08-13, "dirty-title
    // indicator") but lived as a LOCAL inside windowTitle(), so QML could
    // read only its rendered glyph and no action could be conditioned on
    // it -- the flag-with-no-readers shape chain row 9 donated as a
    // requirement: "the dirty flag ships WITH its readers in one change,
    // or it is decoration" (operator, ratified 2026-08-31).
    //
    // WHY NOTIFY infoChanged IS CORRECT AND NOT MERELY CONVENIENT: both
    // baseline writes emit it immediately -- openDocument at the
    // "opened state == on-disk state" line and saveDocument at the
    // "title dirty-marker baseline" line -- and the open path carries an
    // extra re-notify (review P4-D1) so the transient -2 baseline cannot
    // leave a just-opened document marked. Measured, not assumed.
    Q_PROPERTY(bool documentDirty READ documentDirty NOTIFY infoChanged)

    // ★ AND THE DOOR'S QUESTION IS NOT THE TITLE'S. Collapsing the two
    // shipped a cry-wolf confirm and tst_gesture_chrome's
    // newDocumentPresetCreatesThatDocument caught it: a freshly launched,
    // untouched, never-saved document is documentDirty (it is not on disk
    // anywhere, and the title's dot is right to say so) but there is
    // NOTHING TO LOSE, so File > New must not stop to ask.
    //
    // "documentDirty" answers: is this different from what is on disk?
    // "documentHasUnsavedWork" answers: would continuing DESTROY anything?
    // The second is what a confirm-on-replace may be conditioned on, and
    // it is the same distinction this change already honoured by refusing
    // to guard File > Import -- applied there by reasoning, and here only
    // after an existing test refused to let it through.
    Q_PROPERTY(bool documentHasUnsavedWork READ documentHasUnsavedWork
                   NOTIFY infoChanged)
    Q_PROPERTY(QVariantList layers READ layers NOTIFY infoChanged)
    Q_PROPERTY(QVariantList historyEntries READ historyEntries NOTIFY historyChanged)
    Q_PROPERTY(QString statusText READ statusText NOTIFY statusTextChanged)
    Q_PROPERTY(int activeLayerIndex READ activeLayerIndex WRITE setActiveLayerIndex
                   NOTIFY activeLayerIndexChanged)
    Q_PROPERTY(QString windowTitle READ windowTitle NOTIFY infoChanged)

public:
    explicit DocumentController(const Catalog *catalog, QObject *parent = nullptr);

    // Deterministic ids + stroke seeds for --screenshot-tour.
    void setDeterministicIds(quint64 seed);

    // A11Y-3: the §31.3 polite live region (null-safe — headless/unit
    // contexts run without one). Receipt-driven announcements only:
    // announce AFTER the engine receipt confirms the mutation applied,
    // never optimistically (the R10 truth discipline extended to speech).
    void setAnnouncer(Announcer *announcer) { m_announcer = announcer; }

    bool hasDocument() const { return m_hasDocument; }
    int docWidth() const { return m_docWidth; }
    int docHeight() const { return m_docHeight; }
    bool canUndo() const { return m_canUndo; }
    bool canRedo() const { return m_canRedo; }
    QVariantList layers() const { return m_layers; }
    QVariantList historyEntries() const { return m_historyEntries; }
    QString statusText() const { return m_statusText; }
    int activeLayerIndex() const { return m_activeLayerIndex; }
    void setActiveLayerIndex(int index);
    QString windowTitle() const;
    bool documentDirty() const;
    bool documentHasUnsavedWork() const;

    Q_INVOKABLE bool newDocument(int width, int height, const QString &background);
    // RC3 D-3 (operator WORD 35, sitting S-20): File > Close (Cmd+W). Back to
    // the launch state -- no document -- with the same per-document memories
    // cleared that newDocument() clears. The unsaved-changes guard is the
    // caller's (Main.qml confirmDiscard), as for New and Open.
    Q_INVOKABLE bool closeDocument();
    Q_INVOKABLE void undo();
    Q_INVOKABLE void redo();

    // Wave F (census class F): HistoryPanel row jump — the history.goto
    // position form (§40.8: `position` is an absolute UNDO-STACK index,
    // 0..=undoDepth; † markers never occupy a slot). Thin protocol door
    // over applyCommand: envelope through the apply door, receipt-driven
    // refresh via the standard post-call poll; range/reachability truth
    // is the ENGINE's (§40.8/§10.4 typed refusals surface on the
    // existing ui.status.rejected path, never pre-computed). Returns
    // applyCommand's result map.
    Q_INVOKABLE QVariantMap historyGoto(int position);

    // Generic envelope submission for the CP-0.5 chrome (dialogs, panel
    // actions, canvas tools). Returns {ok, code, message, createdIds:
    // [uuid,...]} — typed rejections surface in the status bar AND the
    // return value so callers can branch. Class-M callers pass mergeKey
    // ("{kind}:{targetId}:{sessionUuid}#{propPath}"); everyone else omits
    // it. Refreshes info/history via the standard post-call poll.
    Q_INVOKABLE QVariantMap applyCommand(const QString &kind,
                                         const QVariantMap &target,
                                         const QVariantMap &params,
                                         const QString &mergeKey = QString());

    // Client-generated ids (§9.6: object identity is client-supplied so
    // replay is deterministic). Deterministic under the tour seed.
    Q_INVOKABLE QString newUuid() { return m_uuids.next(); }

    // .appname package IO through the ABI door (doc_open/doc_save).
    Q_INVOKABLE bool openDocument(const QString &path);
    Q_INVOKABLE bool saveDocument(const QString &path); // empty = current path
    Q_PROPERTY(QString documentPath READ documentPath NOTIFY documentPathChanged)
    QString documentPath() const { return m_documentPath; }

    // Copy the flattened composite (or a document-space rect of it) to the
    // OS clipboard — pure readback presentation, no pixel math.
    Q_INVOKABLE bool copyCompositeToClipboard(const QRect &docRect = QRect());

    // ABI 1.2 surfaces (all gated on abiBlock12; QML disables otherwise).
    Q_PROPERTY(bool abiBlock12 READ abiBlock12 CONSTANT)
    bool abiBlock12() const;

    // ABI 1.4 surfaces (per-layer readback; QML disables otherwise).
    Q_PROPERTY(bool abiBlock14 READ abiBlock14 CONSTANT)
    bool abiBlock14() const;
    // Monotonic info refresh counter — thumbnail URLs embed it so Qt's
    // per-URL image cache is busted exactly when info truth moves.
    Q_PROPERTY(int infoRevision READ infoRevision NOTIFY infoChanged)
    int infoRevision() const { return m_infoRevision; }
    // Per-layer content readback for the thumbnail provider. QUIET on
    // typed refusals — the 1.4 service matrix makes them EXPECTED for
    // adjustment layers and kinds without a composite path; the panel
    // shows kind glyphs instead, never a faked preview.
    QImage layerThumbImage(const QString &layerId);
    // G-3 (CRITICAL): the perspective session's ENTRY-TIME source-bounds
    // derivation — one full-doc 1.4 readLayer (the thumbnail/cut door:
    // content-not-contribution, straight alpha) + an alpha-presence scan
    // (selection_scan). The engine solves transform.perspective's
    // homography from the LAYER'S SOURCE BOUNDS onto the submitted quad
    // (contract + kind impl, verified first-hand), so the session MUST
    // frame the quad at those bounds; per-layer bounds are not readable
    // through the flat info surface (ABI-growth request 3 — the filed
    // ask; this readback is the interim). Returns:
    //   ok:true  + x/y/w/h — the alpha-tight bounds (w==0 => no opaque
    //             pixels; the caller rules the empty arm);
    //   ok:false + code/message — the door's TYPED refusal (masked
    //             layers etc. — the cabi node-refusal matrix), already
    //             surfaced on the status line via ui.status.rejected
    //             (the cutLayer precedent). NEVER a doc-bounds guess.
    // HONEST LIMIT (recorded): buffer bounds grow eagerly per content
    // write (pixel-exact for content-only histories) but re-tighten
    // TILE-granularly after erasing edits — the engine's source rect
    // can then exceed this alpha-tight scan by sub-tile slack; the
    // mapping stays bounded (never the G-3 flood class) and the slack
    // dies when the real bounds ABI arm lands.
    //
    // v1.0.1 item 19 (§D.1): the read is now STRIP-CHUNKED (the selection
    // announcer's maxRows pattern) so every single readLayer stays under
    // the 2^26-px ABI cap. Before this the full-doc read was UNCHUNKED and
    // simply FAILED above 8192x8192 — G-3 then walled a perfectly good
    // layer for a reason that had nothing to do with the layer. Below the
    // cap the loop runs exactly once and the derived rect is byte-identical
    // to the old one-shot scan.
    Q_INVOKABLE QVariantMap layerContentBounds(const QString &layerId);
    // v1.0.1 item 21: the SAME derivation, additionally handing back the
    // pixels it already read — item 19's entry derivation reads the whole
    // layer and then throws the image away, and the live transform preview
    // needs exactly that image. One readback serves both; `layerContentBounds`
    // is this function with a null `snapshot`, so the two cannot drift.
    //
    // Adds one key to the returned map:
    //   snapshot:true  — *snapshot holds the layer's full-doc content
    //                    (RGBA8888, doc-origin framed, straight alpha).
    //   snapshot:false — bounds are still true, but the pixels are NOT in
    //                    hand: the layer exceeds 2^26 px, so no single
    //                    readback can deliver it and no QImage can hold it
    //                    as one strip. The caller must SAY SO rather than
    //                    preview a lie; the general fix is the viewport-
    //                    windowed snapshot (the ants' m_antsWindow shape).
    // `snapshot` is untouched on the ok:false (typed refusal) path.
    QVariantMap layerContentSnapshot(const QString &layerId, QImage *snapshot);
    // §26.1 Cut: app-side Copy (per-layer readback, alpha scaled by the
    // selection's fractional coverage when one is active) THEN the
    // edit.cut mutation. The clipboard updates ONLY when the mutation
    // applies, so a typed refusal (e.g. locked) cannot clobber the
    // previous clipboard contents. Returns applyCommand's result map.
    Q_INVOKABLE QVariantMap cutLayer(const QString &layerId);

    // v1.0.1 item 14b — Delete (desk ruling, items-7+14 design entry):
    // the edit.cut ENGINE kind WITHOUT the app-side copy. Selection
    // active -> the §13.1 fractional-coverage clear of the selected
    // region; none -> the §40.5 whole-content clear (the §32.5 confirm
    // is the CALLER's job — DeleteConfirmDialog/Main.qml own it; this is
    // the bare door). No abi-1.4 conjunct (mutation-only — nothing is
    // read back). Transparency lock = the engine's typed refusal
    // (error.command.locked) surfaced on the existing ui.status.rejected
    // path — the recorded honest limit (locks are write-only
    // pre-info-v2, no pre-computed disable).
    Q_INVOKABLE QVariantMap deleteLayerContent(const QString &layerId);

    // v1.0.1 item 14b — Cmd+J "layer via copy" (desk-ruled composition):
    // with a selection = app-side copy (1.4 per-layer readback, alpha
    // scaled by fractional coverage — cutLayer's exact copy half) ->
    // edit.paste as a NEW layer at [0,0] (full-doc-rect image, so the
    // copy lands aligned); without = layer.duplicate on the active
    // layer's subtree (idMap over layers + masks — the LayersPanel
    // duplicate button's established composition; op-stack entry ids
    // are NOT in info, so layers carrying op stacks reject typed — the
    // recorded upstream gap, honest).
    Q_INVOKABLE QVariantMap layerViaCopy(const QString &layerId);

    // ---- Pro-Wave row 6: §11.2 mask properties -----------------------
    // ENGINE truth for one layer's masks, straight off the info door:
    // a list of {maskId, enabled, inverted, density, feather, linked}.
    // Read this rather than remembering what was last sent — the whole
    // point of the row is that a property control shows what the
    // document actually holds (undo/redo and history.goto move these
    // behind the panel's back).
    Q_INVOKABLE QVariantList masksOf(const QString &layerId) const;

    // Set exactly ONE §11.2 mask property, as one §10.5 property-drag
    // session. `prop` is the registered param name — "enabled",
    // "inverted", "density", "feather", "linked" — and an unknown name
    // is refused CLIENT-side (ui.error.* namespace) rather than sent, so
    // `additionalProperties:false` can never be tripped from here.
    //
    // `sessionUuid` empty = no session (no mergeKey on the wire, one
    // history step per call: the keyboard/wheel-step case). Non-empty =
    // a drag session; pass the SAME uuid for every intermediate value
    // and a FRESH one at the next pointer-down. The key is scoped per
    // (maskId, prop) per the contract, so a feather drag and a density
    // drag never coalesce into each other.
    //
    // Refusals ride the EXISTING ui.status.rejected path through
    // applyCommand — the engine's own code and message, never a
    // shell-invented one, and never a fake success.
    Q_INVOKABLE QVariantMap setMaskProp(const QString &layerId,
                                        const QString &maskId,
                                        const QString &prop,
                                        const QVariant &value,
                                        const QString &sessionUuid = QString());

    // v1.0.1 item 14b — Cmd+Shift+D reselect: the SHELL's last-selection
    // memory. CAPTURE: the params of the last APPLIED selection.set
    // envelope (the journaled pointer-surface door — marquee/ellipse/
    // lasso/polygon/wand all commit through it), captured inside
    // applyCommand before its refresh poll so the same infoChanged that
    // publishes the new selection publishes the armed row. REPLAY: the
    // captured params re-submitted through the SAME door — journaled,
    // undoable, announced via the existing selection receipt path.
    // Cleared on document swap (the lastSavedChannelId precedent).
    // Recorded semantics: a multi-gesture composite selection replays
    // its LAST gesture (op included, §13.2 combine against the current
    // state); selection.all/load/from_path do not arm the memory (the
    // design names the selection.set door) — refinement candidates.
    Q_PROPERTY(bool hasSelectionMemory READ hasSelectionMemory NOTIFY infoChanged)
    bool hasSelectionMemory() const { return m_hasSelectionMemory; }
    Q_INVOKABLE QVariantMap reselect();

    // ABI 1.6 font feed (the request-10 arc): provision host System
    // faces into THIS document's FontDb — single-face .ttf/.otf from
    // the three host dirs, sorted enumeration (provisioning order is
    // semantic: the door preserves call order), metadata derived by the
    // pure font_meta parser, source class "system" (§3.10: feeding
    // System faces is NOT vendoring; Bundled would need its own deps
    // record first). Idempotent engine-side; typed refusals (psName
    // collisions, engine-rejected faces) are counted quietly. Runs at
    // each document-open lifecycle site.
    void provisionSystemFaces();
    // {"stamp": N, "faces": M, "provided": P, "skipped": S,
    //  "refused": R} — tour evidence + future chrome picker seed.
    Q_INVOKABLE QVariantMap fontFacesSummary();
    // Engine-side export (7 built-in formats; quality only for jpeg/webp,
    // -1 = format default).
    Q_INVOKABLE bool exportDocument(const QString &pathOrUrl,
                                    const QString &format,
                                    int quality = -1);
    // File→Import / drag-drop: file bytes → relaxed asset_create raster
    // arm → journaled edit.paste (the ratified MAJOR-2 flow; replay
    // reproduces imports byte-identically).
    Q_INVOKABLE bool importImage(const QString &pathOrUrl);
    // RC3 D-4 (operator WORD 35, sitting S-04): File > Open of an IMAGE.
    // Creates a transparent document of the image's pixel size and runs the
    // existing importImage() path into it (history: New Document, Paste).
    // .ora is out of scope. False (with the status line set) on failure.
    Q_INVOKABLE bool openImageAsDocument(const QString &pathOrUrl);
    // OS-clipboard paste: clipboard raster serialized to PNG bytes (the
    // ingress transport format — OS-bridge serialization, not document
    // math) → same asset+paste flow.
    Q_INVOKABLE bool pasteFromClipboard();
    // v1.0.1 item 15 (the menu-mint follow-up remainder) — PASTE INTO:
    // pasteFromClipboard's ingress verbatim (clipboard raster → PNG
    // bytes → appname_asset_create) submitted through the SEPARATE
    // engine kind `edit.paste_into` (contracts/commands/
    // edit.paste_into.yaml; crates/appname-core/src/engine/kinds.rs
    // EditPasteIntoKind, registered kinds.rs:74).
    //
    // WHAT THE ENGINE DOES (probed first-hand, not assumed): it does
    // NOT clip pixels. It inserts the pasted layer exactly as
    // edit.paste does and ATTACHES a raster mask whose coverage is a
    // bit-exact clone of the CURRENT SELECTION (the selection itself is
    // untouched). Containment is then a §11.2 composite truth —
    // mask_eval samples coverage at DOCUMENT-space p and absent reads
    // 0, so the pasted layer contributes nothing outside the selection.
    // Hence the extra param vs paste: `newMaskId` (§9.6 client-
    // generated creation uuid, REQUIRED alongside newLayerId).
    //
    // PRECONDITION: an ACTIVE SELECTION IS REQUIRED — the engine's own
    // gate (`!cx.document().selection().active()` →
    // error.command.selection_required, §40.5: the selection BECOMES
    // the mask). NOT re-derived here: the edit.pasteInto availability
    // row gates the chrome, and a submission that reaches the engine
    // without one gets the engine's TYPED refusal on the existing
    // ui.status.rejected path (applyCommand's setStatusFromResult) —
    // never a shell-invented verdict, never a fake success.
    // `at` is OMITTED for the same reason paste omits it: the engine
    // resolves and journals the placement, so replay never depends on
    // shell state.
    Q_INVOKABLE bool pasteIntoFromClipboard();

    // ---- Pro-Wave row 1: an adjustment layer masked by the selection ----
    //
    // ROUTE (probed first-hand through the CLI's script door before a line
    // of this was written — adjustment.add + selection.set + op.set_mask
    // all applied, and both refusal arms reproduced): the OP-mask route.
    // op.set_mask addresses the adjustment's OpNode and copies the active
    // selection's coverage into that op's mask (§12.1/§12.2, §40.6).
    // resolve_op matches `n.adjustment_op() == Some(oid)`, so an
    // adjustment layer's single op is a valid target — read in
    // kinds_ops.rs, not inferred from the smart-filter case.
    //
    // NOT THE LAYER-MASK ROUTE. adjustment.add.yaml's own selectionBehavior
    // note describes a following `mask.add{from:'selection'}` — that is the
    // LAYER-mask variant (FI-13, spec-first, another desk). This door does
    // not emit mask.add, and the two must not be conflated: one masks the
    // OpNode, the other the layer.
    //
    // WHY THE opId IS REACHABLE AT ALL (this corrects the filed blocker):
    // §9.6 makes creation uuids CLIENT-generated and adjustment.add's
    // schema puts `newOpId` in `required`. The shell AUTHORS the op's id,
    // so it can address it afterwards; the ABI never needed to publish
    // one. HONEST LIMIT, and the residual O-INT ask: the ABI publishes no
    // op ids anywhere (`appname_doc_info_json`'s layer object carries
    // index/id/name/visible/opacity/kind/parentId/depth/blend/
    // fillOpacity/locks/clipToBelow/colorLabel/masks[/isolated] — no
    // `ops` key), so ops the shell did NOT create in this session — every
    // op in a REOPENED document — stay unaddressable. m_lastAdjustmentOpId
    // is therefore session memory, exactly like m_hasSelectionMemory, and
    // the availability row below tells the truth about it rather than
    // offering a control that would refuse.
    //
    // addAdjustmentLayer: mints both creation uuids, submits
    // adjustment.add, and — when a selection is active — follows with
    // op.set_mask{from:'selection'} so the adjustment lands already
    // masked (the row-1 gesture). With no selection it creates a plain,
    // UNMASKED adjustment and says so through the normal applied status:
    // that is a correct outcome, not a silent half-failure. Returns the
    // LAST submission's result map, with "masked" naming which happened.
    // v1.0.1 item 4 class (a): opParams rides through to
    // buildAdjustmentAddParams, which has ALWAYS accepted it (its 4th
    // argument, defaulted empty) — this door was the only thing dropping
    // it. Defaulted here so every existing caller is unchanged.
    Q_INVOKABLE QVariantMap addAdjustmentLayer(const QString &opKind,
                                               const QVariantMap &opParams = {});
    // The mask step on its own, against the session's last-created
    // adjustment op. Separated so the follow-up chrome (a dedicated
    // control, once a caption id exists — see the lane log's O-I18N ask)
    // re-uses this door instead of re-deriving the route.
    Q_INVOKABLE QVariantMap maskAdjustmentFromSelection();
    // P1-4(a). Reads the 14a availability row for `action` and surfaces
    // its REASON on all three surfaces at once — status line, polite
    // announcement, and the actionUnavailable event so a transient
    // surface re-fires on a repeated press. Does nothing for an unknown
    // action or one that is actually ENABLED: this door reports a state,
    // it does not decide one.
    Q_INVOKABLE void reportUnavailable(const QString &action);
    // Select a layer BY ID. A creation site knows the uuid it minted, and
    // the doc-info refresh does not: it only CLAMPS the active index when
    // it falls out of range, so a layer inserted ABOVE the selection
    // leaves the old row selected. That is invisible on screen and lethal
    // in a flow — add an adjustment, add a mask, and the mask lands on
    // whatever was selected before. It looks correct until the adjustment
    // is toggled off. Returns false if the id does not resolve.
    Q_INVOKABLE bool selectLayerById(const QString &layerId);
    // How many times reportUnavailable REFUSED an uncataloged reasonId.
    // The Announcer's refusedCount, adopted for the same hazard O-INT
    // measured on this door: the catalog gate is correct — an
    // uncataloged id must never be echoed at a user — but a silent
    // return means A CHORD CAN BE PERFECTLY WIRED AND STILL SAY
    // NOTHING, invisibly from QML. No TypeError, no warning, the press
    // just goes quiet: the ORIGINAL BUG, reachable through the fix for
    // the original bug. Counting it makes the failure assertable.
    int unavailableRefusedCount() const { return m_unavailableRefused; }
    // Session memory: the opId of the newest adjustment THIS shell
    // created. Empty until one is created; drops back to empty when the
    // document handle changes (a reopened document's ops are not ours).
    QString lastAdjustmentOpId() const { return m_lastAdjustmentOpId; }

    // W05 recovery prompt (P6; ABI 1.2 surface + dialog.recovery.*
    // catalogs). queryRecovery: sidecar evidence BEFORE opening —
    // {available, commands, torn, logPath}. openRecoveredDocument: the
    // Restore arm (fresh handle, adopted on success; the log stays on
    // disk until resolved per contract — a later save to the SAME path
    // discards it and binds fresh). discardRecoveryLog: the confirmed
    // Discard arm (caller then opens normally). "Not now" needs no verb:
    // plain openDocument already preserves recoverable logs and runs
    // unprotected (bindAutosaveIfSafe skip arm).
    // Transient created-path tracking (slice V): the ONLY path handle the
    // shell holds is the id it just created — conservatively dropped on
    // ANY history transition (undo could remove it; no paths readback
    // until ABI-growth request 8). The canvas records it; QML consumes.
    Q_PROPERTY(QString lastCreatedPathId READ lastCreatedPathId NOTIFY infoChanged)
    QString lastCreatedPathId() const { return m_lastCreatedPathId; }
    void trackCreatedPath(const QString &pathId);
    Q_INVOKABLE bool selectionFromLastPath();

    // Transient saved-channel tracking (doors-flip wave; ABI-growth
    // request 9 files the real channels enumeration). Lifecycle is
    // CURSOR-TRUTH, deliberately DIVERGING from trackCreatedPath's
    // blanket clear-on-any-transition (which would make save ->
    // deselect -> load impossible — review-caught): the journal cursor
    // recorded when the save applied is the invalidation key; a later
    // cursor BELOW it means the save step is undone (refreshInfo
    // clears); forward transitions keep the handle; doc swaps clear
    // explicitly; a stale id refuses TYPED at the door. Menu chrome
    // holds for the mint rotation; these invokables are its mechanics.
    Q_PROPERTY(QString lastSavedChannelId READ lastSavedChannelId NOTIFY infoChanged)
    QString lastSavedChannelId() const { return m_lastSavedChannelId; }
    // selection.save create-arm (§40.4): client-generated newChannelId,
    // engine defaults the name ("Selection N" localized engine-side).
    Q_INVOKABLE QVariantMap saveSelectionAsChannel();
    // selection.load source:{channelId} on the tracked id.
    Q_INVOKABLE QVariantMap loadSavedChannelSelection();
    // selection.load source:{layerAlpha} — a layer id the shell ALWAYS
    // has from info truth; no channel enumeration needed.
    Q_INVOKABLE QVariantMap loadLayerAlphaSelection(const QString &layerId);

    Q_INVOKABLE QVariantMap queryRecovery(const QString &pathOrUrl);
    Q_INVOKABLE bool openRecoveredDocument(const QString &packagePathOrUrl,
                                           const QString &logPath);
    Q_INVOKABLE bool discardRecoveryLog(const QString &logPath);

    Q_PROPERTY(bool selectionActive READ selectionActive NOTIFY infoChanged)
    bool selectionActive() const { return m_selectionActive; }

    // v1.0.1 item 14a: THE single command-availability truth (the
    // items-7+14 design entry, reports/O-UI-log.md). Rows keyed by a
    // SHELL-INTERNAL action vocabulary (NOT wire kinds); each row =
    // {enabled: bool, reasonId?: string} — reasonId only when disabled
    // AND an EXISTING cataloged id names the cause (ui.error.no_document
    // / error.command.selection_required; other causes carry no id —
    // mint-queue, lane log). Three consumers: the menus (migrated this
    // slice), the item-7 context menu, the item-14b shortcuts.
    // COMPUTED ON READ from info truth — the accessor IS the design's
    // "one function". A cache refreshed only in refreshInfo() would be
    // STALE at the direct emit-infoChanged sites (saveSelectionAsChannel
    // sets the channel handle AFTER its applyCommand poll, then emits;
    // saveDocument/trackCreatedPath likewise) — computed-on-read makes
    // every infoChanged re-bind read fresh truth by construction.
    // HONEST LIMIT (recorded in the design): layer LOCKS are write-only
    // pre-info-v2 — lock-class refusals (e.g. transparency-lock on
    // Delete) canNOT be pre-computed into `enabled`; they surface as the
    // engine's typed refusal at apply time (the item-13 wall pattern).
    // Pre-emptive disable upgrades when lock readback lands (carry).
    Q_PROPERTY(QVariantMap commandAvailability READ commandAvailability
                   NOTIFY infoChanged)
    QVariantMap commandAvailability() const;

    // v1.0.1 item 5: THE single truth-point for the crop dialog's
    // non-destructive arm. DESK RULING: served from abi 1.7 (crop_rect
    // merged @ e1de6c7; the cabi minor bump is a follow-on slice) —
    // bound to the version marker per the house never-probe-by-refusal
    // rule: false today, flips automatically when the bump lands, and
    // the dialog's disabled toggle + note re-enable through the
    // property with zero further lane work.
    Q_PROPERTY(bool cropNonDestructiveLive READ cropNonDestructiveLive
                   CONSTANT)
    bool cropNonDestructiveLive() const { return abiHasBlock17(); }

    // ABI 1.7 additive: THE OUTPUT FRAME, `image_rect()` =
    // `crop_rect.unwrap_or(canvas_rect())`, in ABSOLUTE document
    // coordinates. This is the frame every export and composite door
    // already frames on, so a surface that paints on it is correct with
    // and without an active crop.
    //
    // ★★ THE ORIGIN IS THE LOAD-BEARING HALF, not the size.
    // `appname_doc_read_region` frames on the caller's ABSOLUTE document
    // coordinates and does NOT clamp to canvas or image (include/
    // appname.h), so a consumer given only a SIZE asks for (0,0,w,h) and
    // gets the right-SIZED region of the WRONG PIXELS under any crop
    // whose origin is not (0,0). The two arms differ exactly there:
    //     no crop          width/height 200 100   imageRect [ 0, 0,200,100]
    //     non-destructive  width/height 200 100   imageRect [40,20,120, 60]
    //     destructive      width/height 120  60   imageRect [ 0, 0,120, 60]
    //
    // ★ `docWidth`/`docHeight` are UNCHANGED and keep meaning the CANVAS.
    // This is not a rename: the canvas-size surfaces (CanvasSizeDialog,
    // ResizeDialog) and the selection surfaces (marching ants, wand seed
    // — the engine keeps the selection mask canvas-sized) legitimately
    // still want the canvas.
    Q_PROPERTY(QRect imageRect READ imageRect NOTIFY infoChanged)
    QRect imageRect() const { return m_imageRect; }

    // The document's root-group id, captured from the doc.new receipt
    // (layer.reorder requires an explicit parent uuid and the flat info
    // surface cannot express the tree). Empty for OPENED documents until
    // info v2 lands (ABI-growth request 3) — consumers disable
    // reorder-into-root UI when empty, never guess.
    Q_PROPERTY(QString rootLayerId READ rootLayerId NOTIFY infoChanged)
    QString rootLayerId() const { return m_rootLayerId; }

    // info v2 mirrors (empty/null pre-1.2; QML gates on abiBlock12).
    Q_PROPERTY(QVariantList guidesH READ guidesH NOTIFY infoChanged)
    Q_PROPERTY(QVariantList guidesV READ guidesV NOTIFY infoChanged)
    Q_PROPERTY(QVariantMap gridSpec READ gridSpec NOTIFY infoChanged)
    Q_PROPERTY(QString profileTag READ profileTag NOTIFY infoChanged)
    Q_PROPERTY(QString docDepth READ docDepth NOTIFY infoChanged)
    QVariantList guidesH() const { return m_guidesH; }
    QVariantList guidesV() const { return m_guidesV; }
    QVariantMap gridSpec() const { return m_gridSpec; }
    QString profileTag() const { return m_profileTag; }
    QString docDepth() const { return m_docDepth; }

    // §5 tier-2 fix round (review minor-2): SUBMISSION-ATTEMPT truth for
    // the armed-inert pins. History length proves engine OUTCOME, not
    // shell wiring — an engine-REJECTED submission (unknown kind) leaves
    // history unchanged, so a live-wired tool can masquerade as inert to
    // a history-length probe (the review's dodge demonstration). This
    // counter bumps at ENTRY of the two shell-side submission doors a
    // canvas gesture can reach — applyCommand and the stroke session's
    // strokeBegin — BEFORE the engine answers: an armed-tool gesture must
    // leave it unmoved; ANY wiring, accepted or rejected, moves it.
    int submissionAttempts() const { return m_submissionAttempts; }

    // Stroke session (canvas forwards normalized pointer input; the canvas
    // keeps the visual echo, this side owns samples + envelope + commit).
    // `hardness` (item 4 / O-29): the OPTIONAL 0..=1 override that rides
    // brush.stroke envelopes only. std::nullopt = the caller states no
    // hardness, so the param is omitted and the engine's preset governs
    // (contract: "absent = preset hardness"). Callers must pass nullopt
    // rather than the preset's current value: the two are identical on
    // today's hardcoded preset and diverge the moment a picker lands.
    //
    // §5 review minor-3: `std::optional<double>` converts implicitly from
    // `double`, so the members' by-type guarantee stopped at this call
    // boundary — `strokeBegin(k, c, s, m_brushHardness)` would compile and
    // silently mean STATED, which is the one mistake this slice exists to
    // make impossible. The deleted overload below closes it: a bare
    // double/int is an exact match for it and fails to COMPILE, so a
    // caller must write `std::nullopt` or an explicit optional and thereby
    // say which one it means. The guarantee is now enforced, not merely
    // documented.
    Q_INVOKABLE void strokeBegin(const QString &kind, // "brush.stroke"|"eraser.stroke"
                                 const QColor &color,
                                 double sizePx,
                                 std::optional<double> hardness,
                                 // v1.0.1: required by dodge_burn.stroke and
                                 // blur_sharpen.stroke, absent for every other
                                 // kind. Defaulted so existing callers are
                                 // unchanged.
                                 const QString &mode = QString(),
                                 // Tier-1 row 5: the §45.2/§13.3 sample-scope
                                 // INTENT. Safe to pass for any kind — the
                                 // builder kind-guards the emit (heal/clone
                                 // only), so a stray true can never reach a
                                 // schema that lacks the key. Defaulted so
                                 // existing callers are unchanged.
                                 bool sampleAllLayers = false,
                                 // B-02: the ARMED clone source offset
                                 // (anchor − strokeStart, §21.1). Safe to
                                 // pass for any kind — the builder emits it
                                 // for clone.stroke ONLY, so a stray value
                                 // cannot reach a schema without the key.
                                 const QPointF &cloneOffset = QPointF());
    // NOT Q_INVOKABLE (moc never sees it) — a compile-time tripwire only.
    void strokeBegin(const QString &kind, const QColor &color, double sizePx,
                     double hardness) = delete;
    Q_INVOKABLE void strokeAppend(double docX, double docY, double pressure, double tSeconds);
    // Returns true when the engine committed the stroke (canvas then drops
    // its echo — the dirty-rect repaint shows engine truth); false keeps
    // the echo as a pending preview (typed rejection surfaced in status).
    Q_INVOKABLE bool strokeEnd();
    // G-1: ABANDON the session without submitting. §39.2 [NORMATIVE] makes
    // Esc resolve the innermost in-progress context, and a live stroke is
    // one. Safe to do purely shell-side because strokeBegin never reaches
    // the engine — it only arms this object and clears the sample buffer,
    // so there is no engine session to unwind and nothing to undo.
    // NOT strokeEnd(): that submits. NOT strokeCommitted(false) either —
    // that means "typed rejection, KEEP the echo as a provisional preview",
    // which is the opposite of abandoning.
    Q_INVOKABLE void strokeCancel();

    // ---- item 25b HONESTY SWEEP (honest-state rule D4) ----
    // Put a PENDING note on the status line: the shell's own statement
    // that a control is armed but its wiring (shell-side or engine-side)
    // has not arrived. setStatus is private, so this is the QML door on
    // to the existing ui.status.* path — the exact sibling shape of
    // strokeEnd's ui.status.stroke_preview_pending emission, and NOT a
    // second status channel.
    //
    // The messageId MUST resolve through the catalog: an uncataloged id
    // is REFUSED (status untouched), never key-echoed into the status
    // bar. That is the announcer's own policy (announcer.h) applied to
    // the status half, so a typo fails silent-and-testable in both
    // halves rather than printing a raw id at a user.
    //
    // Vocabulary-free on this side: which id, and the {tool} display
    // name, are resolved by the caller (Main.qml, beside toolNameKeys).
    Q_INVOKABLE void statusPending(const QString &messageId,
                                   const QVariantMap &args = QVariantMap());

    // Reads for the canvas (document-space rect -> straight-alpha RGBA8888).
    bool readRegionImage(const QRect &rect, QImage *out);
    // ABI 1.2: selection coverage mask (Grayscale8). False pre-1.2.
    bool readSelectionImage(const QRect &rect, QImage *out);
    // Repaint-scoping slice (sweep F1b/F4): the SHARED per-revision
    // full-doc selection mask — ONE readSelectionImage per selection
    // revision serves BOTH consumers (the canvas ants vectorizer and the
    // announcer bounds scan; bounds from this image are byte-equal to
    // the strip scan's — same Grayscale8 data, same nonzero
    // accumulation). False when no document / no active selection /
    // pre-1.2, or past the share cap (memory bound) — callers fall back
    // to their own windowed/strip reads. QImage is implicitly shared:
    // serving the cache copies no pixels.
    bool selectionMaskFull(QImage *out);
    // Bumped whenever engine state may have changed the selection (any
    // history transition / reset) — the canvas re-reads the mask lazily.
    int selectionRevision() const { return m_selectionRevision; }
    // Selection-mask ABI reads through readSelectionImage since
    // construction — probe/test truth for the repaint-scoping slice (the
    // submissionAttempts precedent): the F1/F4 discriminators assert read
    // COUNTS per commit class (a non-selection commit performs zero;
    // ants + announcer share one per selection revision).
    int selectionMaskReads() const { return m_selectionMaskReads; }
    // WHAT THE LAST STROKE SESSION WAS OPENED WITH (same probe/test-truth
    // family as submissionAttempts and selectionMaskReads above).
    //
    // ★★ THE MODE ACCESSOR EXISTS BECAUSE THE JOURNAL CANNOT CARRY THE
    // ANSWER. historyEntries() exposes index/kind/active/position and no
    // params, so the table-walking test could assert that
    // "dodge_burn.sponge" reached the engine as "dodge_burn.stroke" and
    // was STRUCTURALLY UNABLE to check which mode it carried -- and for
    // sponge the mode IS the operator's ruling: dodge_burn.stroke's enum
    // holds `saturate` and `desaturate`, two OPPOSITE operations, and
    // Q-D3(b) picked one. A test that pins the kind and not the mode
    // would go green on the tool doing precisely the wrong thing.
    // Not cleared at strokeEnd: these describe the last session, not a
    // live one.
    QString lastStrokeKind() const { return m_strokeKind; }
    QString lastStrokeMode() const { return m_strokeMode; }

    void pollNow();

signals:
    // ---- THE REFUSAL EVENT CHANNEL (P1-2, operator ruling) ----
    // MERGE NOTE: O-INT and this desk built this signal independently, in
    // this file, with byte-identical signatures — and reached the same
    // reasoning down to the same line of code. This is the union: their
    // provenance and taxonomy sentence, this desk's chokepoint. Their
    // emit was dropped, not lost — theirs fired from ONE site inside
    // setStatusFromResult; this one fires from setRejected, which every
    // refusal path in this file routes through. Keeping both would have
    // emitted TWICE per refusal and double-fired any bound toast, with
    // no compile error to show for it.
    //
    // Fires ONCE PER REFUSAL, which is the whole reason it exists and is
    // not a property. `statusText` carries the same words, but setStatus
    // early-returns when the new string EQUALS the old one — so a user
    // who presses a refused action twice gets one statusTextChanged and
    // then silence, which reads as "it worked the second time". A
    // surface bound to the text cannot represent a repeat, and the
    // refusal a user repeats is the one they least understood.
    //
    // Carries the WIRE code and the engine's message, never a rendered
    // sentence: consumers render, and the truncation requirement in
    // reports/ROW1-SHELL-BRIEF.md:291 is theirs to honour — the refusal
    // string is label-first, mechanism-after-the-em-dash, and any
    // surface that shortens it displays a sentence that is FALSE. Handing
    // out the parts rather than the prose is what lets a toast put the
    // mechanism clause first instead of eliding the true half away.
    //
    // `code` is the engine's stable error.* taxonomy id; `message` is its
    // mechanism text. Emitted ONLY on rejection.
    void commandRefused(const QString &code, const QString &message);
    // ---- P1-4(a): AN UNMET PRECONDITION MUST SAY SO ----
    // A SEPARATE signal from commandRefused, deliberately. applyCommand's
    // own comment draws the line this keeps: a client-side precondition
    // carries a `ui.*` id and an engine rejection carries `error.*`, kept
    // apart "so the two error surfaces can never be confused". Emitting
    // both through one signal would hand a consumer a `code` whose
    // namespace it cannot predict.
    //
    // Twelve keyboard chords in Main.qml are guarded by
    // `if (rowEnabled(...))` and do NOTHING when the row is disabled —
    // the user presses Cmd+T with no document and gets silence. This is
    // the door that makes that press speak.
    void actionUnavailable(const QString &action, const QString &reasonId);
    void infoChanged();
    void historyChanged();
    void statusTextChanged();
    void activeLayerIndexChanged();
    void documentPathChanged();
    // Document-space dirty rects (QVariantList of QRect) — canvas repaints
    // ONLY these regions (ES §9.5 dirtyRects contract).
    void regionsDirty(const QVariantList &rects);
    // Document replaced (new size/state) — canvas rebuilds its cache.
    void documentReset();
    // Overlay truth (selection ants / guides / grid) moved — the canvas
    // refreshes its ants vector and repaints (full item: overlays span
    // the view). Deliberately NARROWER than infoChanged (repaint-scoping
    // slice, sweep F2): info-only commits (rename, availability,
    // history) trigger NO canvas repaint; composite dirt rides the
    // partial regionsDirty path.
    void overlayChanged();
    // Stroke commit outcome for the canvas echo lifecycle.
    void strokeCommitted(bool ok);

private:
    // 1.0 final S-10: the kinds whose receipts carry no dirty rects but
    // change the canvas; one list for applyCommand and undo()/redo().
    static bool needsWholeDocumentRefresh(const QString &kind);
    void refreshWholeDocument();
    void setStatus(const QString &text);
    // THE ONE REFUSAL DOOR. Every `ui.status.rejected` write in this
    // file goes through here so the status text, the a11y announcement
    // and the commandRefused event can never disagree about whether a
    // refusal happened — three surfaces, one call site.
    void setRejected(const QString &code, const QString &message);
    void bindAutosaveIfSafe(const QString &packagePath);
    void setStatusFromResult(const QString &kind, const CallResult &result);
    void refreshInfo();
    void refreshHistory();
    QString activeLayerId() const;
    // A11Y-3 wave A: the three already-cataloged layer announcements
    // (a11y.layers.hidden/.renamed/.moved), fired from the applied
    // receipt with POST-refresh info truth. The remaining a11y.* wirings
    // adopt at wave B when their catalog rows land (queue in the lane
    // log; uncataloged ids are refused by the announcer anyway).
    void announceAppliedCommand(const QString &kind, const QVariantMap &target,
                                const QVariantMap &params);
    QString layerNameById(const QString &layerId) const;
    // A11Y-3 wave B: announce the POST-state selection — bounds via the
    // strip-chunked mask scan (each read ≤ 2^26 px), or cleared. Called
    // for the selection-mutating kinds' applied receipts only.
    void announceSelectionState();
    // Item 15: the ONE clipboard→asset ingress, shared verbatim by
    // pasteFromClipboard and pasteIntoFromClipboard (the two differ ONLY
    // in the kind they submit and its params). Reads the OS clipboard
    // raster, serializes it to PNG (the ingress transport format — OS-
    // bridge encoding of foreign content, not document math), and hands
    // the bytes to appname_asset_create. Returns the assetId, or
    // std::nullopt with the failure ALREADY surfaced on
    // ui.status.rejected; a null clipboard image returns nullopt
    // silently (nothing to paste is not a rejection — the pre-item-15
    // pasteFromClipboard behavior, preserved verbatim).
    std::optional<QString> ingestClipboardRaster();

    const Catalog *m_catalog; // not owned
    Announcer *m_announcer = nullptr; // not owned; null in headless/unit runs
    AbiDoc m_doc;
    UuidSource m_uuids;
    QTimer m_pollTimer;

    bool m_deterministic = false;
    quint64 m_strokeSeedCounter = 0;

    bool m_hasDocument = false;
    int m_docWidth = 0;
    int m_docHeight = 0;
    // The ABI 1.7 output frame in ABSOLUTE doc coords. Null with no
    // document; falls back to the canvas rect pre-1.7 (see refreshInfo).
    QRect m_imageRect;
    bool m_canUndo = false;
    bool m_canRedo = false;
    bool m_selectionActive = false;
    // info v2 selection.bounds mirror (tight nonzero-coverage rect;
    // null while inactive/pre-1.2) — one side of the F1 revision gate.
    QRect m_selectionBounds;
    // F1b/F4 shared mask cache (see selectionMaskFull).
    QImage m_selectionMaskCache;
    int m_selectionMaskCacheRevision = -1;
    // pollNow -> refreshInfo hand-off: the polled batch carried a
    // history event whose kind is in the engine-derived selection-writer
    // set (the F1 gate's kind side).
    bool m_selectionKindHint = false;
    QString m_documentPath;
    QString m_rootLayerId;
    // 1.0 final F-10: the layer a layer.reorder moved, re-selected by the
    // first refreshInfo() that carries the new order (the active layer is
    // an index, so without this it lands on the neighbour).
    QString m_pendingActiveLayerId;
    int m_selectionRevision = 0;
    int m_journalCursor = 0;
    int m_savedCursor = -1; // never-saved (doc.new) stays dirty until Save
    QString m_pendingRecoveryLog; // unresolved crash log (recovered, unsaved)
    QString m_lastCreatedPathId;
    QString m_lastSavedChannelId;
    // 14b reselect memory: the last APPLIED selection.set params (see
    // the reselect() header comment for capture/replay/clear rules).
    QVariantMap m_lastSelectionSetParams;
    bool m_hasSelectionMemory = false;
    // Row 1 session memory (see addAdjustmentLayer): the op id the shell
    // itself authored. Not readable back from the ABI — see the honest
    // limit recorded there.
    int m_unavailableRefused = 0;
    QString m_lastAdjustmentOpId;
    // Journal cursor observed right after the tracked selection.save
    // applied — the ENGINE-TRUTH invalidation key: a later cursor BELOW
    // this means the save step itself is undone (channel gone); forward
    // transitions keep the handle (request-9 interim).
    int m_lastSavedChannelCursor = 0;
    QVariantList m_guidesH;
    QVariantList m_guidesV;
    QVariantMap m_gridSpec;
    QString m_profileTag;
    QString m_docDepth;
    QVariantList m_layers;
    QVariantList m_historyEntries;
    QString m_statusText;
    int m_activeLayerIndex = 0;
    int m_infoRevision = 0;
    int m_fontsProvided = 0;
    int m_fontsSkipped = 0;
    int m_fontsRefused = 0;

    // Submission-attempt counter (see submissionAttempts()).
    int m_submissionAttempts = 0;
    // Selection-mask read counter (see selectionMaskReads()).
    int m_selectionMaskReads = 0;

    // In-flight stroke session.
    bool m_strokeActive = false;
    QString m_strokeKind;
    QString m_strokeMode; // empty = the kind takes no mode
    // Tier-1 row 5: sample-scope intent for this session; the envelope
    // builder kind-guards whether it reaches the wire at all.
    bool m_strokeSampleAllLayers = false;
    QPointF m_strokeCloneOffset; // B-02: armed clone source offset
    QColor m_strokeColor;
    double m_strokeSizePx = 16.0;
    // item 4 / O-29: sanitized at strokeBegin; unset = omit the param and
    // let the engine's preset hardness govern.
    std::optional<double> m_strokeHardness;
    QVector<InputSample> m_strokeSamples;
};

// ---------------------------------------------------------------------
// MINOR-3 WITNESS (MAJOR-25(b)). The deleted `double` overload above makes
// `strokeBegin(k, c, s, m_brushHardness)` ill-formed, so a caller must say
// `std::nullopt` or an explicit optional and thereby state which it means.
// That guarantee was PROVEN ONCE BY HAND — by making the mistake and
// reading "call to deleted member function" — and a hand-proof does not
// re-run. If someone removes the `= delete` line, nothing today notices.
// These asserts are the witness: they re-prove it on every compile.
//
// ⚠ TWO ARMS, AND THE SECOND IS NOT DECORATION. A detection trait that
// names the method wrongly reports "not callable" for the WRONG REASON and
// arm 1 passes vacuously. Arm 2 requires the trait to answer YES for the
// supported call, so a trait that can only ever say no fails the build.
// A guard that cannot say yes has not been shown to be able to say no.
namespace minor3_witness {

template <class C, class = void>
struct bare_double_callable : std::false_type {};
template <class C>
struct bare_double_callable<
    C,
    std::void_t<decltype(std::declval<C &>().strokeBegin(
        std::declval<QString>(), std::declval<QColor>(), 1.0, 1.0))>>
    : std::true_type {};

template <class C, class = void>
struct optional_callable : std::false_type {};
template <class C>
struct optional_callable<
    C,
    std::void_t<decltype(std::declval<C &>().strokeBegin(
        std::declval<QString>(), std::declval<QColor>(), 1.0,
        std::optional<double>{}))>> : std::true_type {};

static_assert(!bare_double_callable<DocumentController>::value,
              "MINOR-3: strokeBegin must REJECT a bare double for `hardness` "
              "— the deleted overload is the guarantee; if this fires, the "
              "`= delete` was removed and a bare double silently means STATED");
static_assert(optional_callable<DocumentController>::value,
              "MINOR-3 witness liveness: the trait must be able to answer YES "
              "for the supported std::optional call — if this fires the "
              "detection is broken and the assert above proves nothing");

} // namespace minor3_witness

} // namespace appname::shell
