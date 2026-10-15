// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Document lifecycle + the single mutation door + reads + event poll.
//!
//! Spec: ES §9.1 (design rules), §9.3 (ownership), §9.6 (command-as-JSON
//! boundary; WD-1 raw bytes), §10.3 (undo/redo as engine functions), §9.5
//! (event payloads); HLD single-door rule; plan §2.2 row "C ABI v1".
//!
//! Wire parity: JSON returned here has the same shapes the CLI prints
//! (`{"ok":true,"receipt":…}` / `{"ok":false,"id"?,"error":{code,message,
//! field?}}`; info/history objects) so every front end — C, CLI, and the
//! CP-0.7 Python tier — reads one wire language.

use std::ffi::c_char;

use appname_core::io::package::{load_package_with, save_package};
use serde_json::json;

/// The ABI engine assembly: the build's full registered kind set — core's
/// registered kinds (`fill`, `edit.*`, `mask.*`, the `doc.*` color family;
/// `appname_core::engine::kinds::register`, the same wrapper the CLI's
/// assembly point uses) over the tools impls (strokes + transforms,
/// integration arbitration 1). Suffix-aligned reopened history is the
/// engine default (wave-2c adoption; ES §10.4).
// Handle-free convenience: kept for callers/tests that do not need the
// text handle (e.g. the export-door unit tests). `allow(dead_code)`
// because those users are test-only now that the shipped doc paths route
// through `abi_assembly_with_text_handle`.
#[allow(dead_code)]
pub(crate) fn abi_assembly() -> appname_core::EngineAssembly {
    // [MECHANICAL CROSS-FENCE COMPANION, RATIFIED by O-SCRIPT (return
    // leg, reports/O-SCRIPT-log.md; core door slice 2, item D — the
    // single-source assembly constructor; O-INT MAJOR-1 is the drift
    // precedent this retires): the hand-copied chain collapses into THE
    // production constructor. The chain lives in
    // `appname_assembly::PRODUCTION_CHAIN`; the crate's parity test
    // enumerates this call site. ABI ≡ CLI ≡ batch ≡ conformance by
    // construction now.]
    appname_assembly::production_assembly()
}

/// The ABI assembly PLUS the concrete `TextLayoutService` handle the
/// §44.1 font door (`appname_doc_provide_face`) calls
/// `register_provided_face` on.
///
/// This is THE production constructor's handle variant — O-INT ruling B:
/// `appname_assembly::production_assembly_with_text_handle()` folds the
/// SAME `PRODUCTION_CHAIN` as `production_assembly()`
/// (`register() == register_with_handle(assembly).0`, chain-equality
/// pinned in the assembly crate), so the two provably cannot diverge and
/// this shipped surface never sits in the guard's blind spot. The
/// concrete `Arc<TextLayoutService>` is the same service the built engine
/// lays out with (an `Arc` clone, shared FontDb). The guard's positive
/// half accepts `doc.rs` routing through either constructor.
pub(crate) fn abi_assembly_with_text_handle() -> (
    appname_core::EngineAssembly,
    std::sync::Arc<appname_text::TextLayoutService>,
) {
    appname_assembly::production_assembly_with_text_handle()
}

use crate::events;
use crate::mem::{AppnameError, AppnameImageBuffer, AppnameString};
use crate::registry::{self, BufferBody, DocState};
use crate::{
    body_of, composite, guarded, guarded_status, invalid_arg, read_bytes, read_cstr, AppnameRect,
    AppnameStatus,
};

/// Opaque document handle (an engine instance: document + journal + history
/// + event queue). Never dereference; the value is a registry cookie.
#[repr(C)]
pub struct AppnameDoc {
    _private: [u8; 0],
}

// ---------------------------------------------------------------------------
// Lifecycle.
// ---------------------------------------------------------------------------

/// Create a new document handle in the pre-init state. The only command the
/// handle accepts is a `doc.new` envelope through
/// `appname_doc_apply_command` (the §9.6 single door; object UUIDs are
/// client-generated so replay is deterministic). The engine is assembled
/// with the build's full registered kind set (CP-0.5: strokes, transforms,
/// fill/edit, mask.*, and the doc color family; every not-yet-implemented
/// registered kind rejects `error.command.not_implemented` until its
/// milestone). Free with `appname_doc_close`.
///
/// `out` receives the handle and must be non-NULL. On failure `*out` is
/// untouched, `*err` (when `err` is non-NULL) receives an error the caller
/// frees with `appname_error_free`; `*err` is untouched on `APPNAME_OK`
/// (ES §9.3).
///
/// # Safety
/// `out` must point to writable storage for one pointer; `err` must be NULL
/// or point to writable storage for one pointer.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_new(
    out: *mut *mut AppnameDoc,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        let (assembly, text_handle) = abi_assembly_with_text_handle();
        let handle =
            registry::mint_doc(DocState::new(assembly.build()).with_text_handle(text_handle));
        // SAFETY: `out` is non-NULL (checked) and points to writable storage
        // for one pointer per the documented contract.
        unsafe { *out = handle };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Open a `.appname` package from `path` (NUL-terminated UTF-8). On success
/// `*out` receives a live handle (free with `appname_doc_close`),
/// re-assembled over the same standard kind set as `appname_doc_new` (so
/// strokes and journal replay work on reopened documents), and one
/// whole-canvas `dirtyRects` event is queued so a polling host paints the
/// fresh document. Reopened-document history is SUFFIX-ALIGNED (ES §10.4):
/// the persisted journal prefix cannot be walked in-process (undo/redo at
/// the open base report `error.history.nothing_to_undo`/`_redo`, "not
/// persisted…replay path"), while commands applied AFTER the open undo and
/// redo normally within the session suffix.
///
/// # Safety
/// `path` must be a valid NUL-terminated C string; `out`/`err` as in
/// `appname_doc_new`.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_open(
    path: *const c_char,
    out: *mut *mut AppnameDoc,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        // SAFETY: forwarded caller contract — `path` is a valid
        // NUL-terminated string for the duration of the call.
        let path = unsafe { read_cstr(path, "path") }?;
        // Build the assembly WITH the retained text handle; the loaded
        // engine shares that assembly's text service (an Arc clone), so
        // the handle drives the same FontDb the opened doc lays out with.
        let (assembly, text_handle) = abi_assembly_with_text_handle();
        let loaded =
            load_package_with(&assembly, std::path::Path::new(&path)).map_err(|e| body_of(&e))?;
        let mut state = DocState::new(loaded.engine).with_text_handle(text_handle);
        events::emit_open_seed(&mut state);
        let handle = registry::mint_doc(state);
        // SAFETY: `out` is non-NULL (checked) and writable per contract.
        unsafe { *out = handle };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Save the document as a `.appname` package at `path` (atomic
/// temp-plus-rename, ES §7.6). The journal travels with the package; the
/// in-process undo/redo delta history does not (ES §10.4). When an
/// autosave binding is live (`appname_doc_autosave_begin`), a successful
/// save FOLDS the log (§7.8 fold rule: the log truncates to a fresh
/// durable BASE anchored at this save; still-pending session assets are
/// re-framed so later envelopes referencing them stay recoverable).
///
/// # Safety
/// `doc` must be a handle from this library (or NULL); `path` a valid
/// NUL-terminated C string; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_save(
    doc: *mut AppnameDoc,
    path: *const c_char,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract for `path` (see appname_doc_open).
        let path = unsafe { read_cstr(path, "path") }?;
        registry::with_doc(doc, |state| {
            save_package(&state.engine, std::path::Path::new(&path)).map_err(|e| body_of(&e))?;
            // §7.8 fold rule: a successful save IS the fold.
            crate::recovery::on_saved(state)
        })?;
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Close a document handle and release its state. NULL is an OK no-op
/// (§9.3 free family). Closing an already-closed or invalid handle returns
/// `APPNAME_ERR_INVALID_ARG` — typed, never undefined behavior (handles are
/// registry cookies, not addresses). Any borrowed pointers previously
/// obtained through this handle's strings/buffers stay governed by THEIR
/// own handles; closing the document does not invalidate them.
///
/// # Safety
/// `doc` must be NULL or a value previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_close(doc: *mut AppnameDoc) -> AppnameStatus {
    guarded_status(|| registry::close_doc(doc))
}

// ---------------------------------------------------------------------------
// The single mutation door + undo/redo.
// ---------------------------------------------------------------------------

/// THE single mutation entry (D-48 determinism spine; ES §9.6): apply one
/// `appname.command.v1` envelope given as **raw bytes + length** (not
/// NUL-terminated). The bytes reach the engine's apply entry unmodified
/// (WD-1) — validation (≤ 1 MiB, UTF-8, JSON, schema, registry, params,
/// live-state; pinned stage order) happens engine-side and rejections are
/// typed, never crashes.
///
/// On success: returns `APPNAME_OK`; `*out_json` (when `out_json` is
/// non-NULL) receives `{"ok":true,"receipt":{id,kind,revision,createdIds}}`
/// — free with `appname_string_free`. Dirty-rect + history events are
/// queued for `appname_doc_poll_events`.
///
/// On rejection: returns the mapped status (`APPNAME_ERR_SCHEMA` /
/// `APPNAME_ERR_UNSUPPORTED` / `APPNAME_ERR_INVALID_ARG` / …); `*err`
/// carries the structured error; ADDITIONALLY `*out_json` receives the
/// rejection wire document `{"ok":false,"id"?,"error":{code,message,
/// field?}}` whenever the bytes reached the engine (boundary misuse such as
/// an invalid handle leaves `*out_json` untouched). The document is
/// unchanged by any rejection.
///
/// # Safety
/// `doc` a handle from this library; `json` must point to `json_len`
/// readable bytes when `json_len > 0`; `out_json`/`err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_apply_command(
    doc: *mut AppnameDoc,
    json: *const u8,
    json_len: usize,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract — `json` covers `json_len`
        // readable bytes; the slice is consumed inside this call only.
        let bytes = unsafe { read_bytes(json, json_len, "json") }?;
        registry::with_doc(doc, |state| {
            let before = events::snapshot(&state.engine);
            // WD-1: the raw bytes, byte-for-byte, into the single door.
            match state.engine.apply_bytes(bytes) {
                Ok(receipt) => {
                    // §9.5 events derive from the receipt's dirty rects
                    // (wave-2b: receipt truth, not an independent guess).
                    events::emit_mutation(state, &before, &receipt.kind, &receipt.dirty);
                    if !out_json.is_null() {
                        let wire = json!({ "ok": true, "receipt": &receipt }).to_string();
                        // SAFETY: `out_json` non-NULL (checked) and writable
                        // per contract; caller frees the minted string.
                        unsafe { *out_json = registry::mint_string(wire) };
                    }
                    // §7.8 autosave binding (ABI-growth round): append +
                    // checkpoint the applied envelope. On an IO failure the
                    // COMMAND HAS APPLIED (receipt + events already
                    // delivered); the call reports APPNAME_ERR_IO with
                    // `error.io.write_failed` so the host knows the
                    // durability guarantee — not the edit — failed.
                    crate::recovery::on_apply_committed(state)?;
                    Ok(AppnameStatus::AppnameOk)
                }
                Err(e) => {
                    if !out_json.is_null() {
                        // Rejection wire document (CLI/py parity). The id is
                        // scraped best-effort for correlation; the scrape
                        // never feeds the engine (WD-1 untouched).
                        let id = serde_json::from_slice::<serde_json::Value>(bytes)
                            .ok()
                            .and_then(|v| v.get("id").and_then(|i| i.as_str().map(String::from)));
                        let mut obj = json!({ "ok": false, "error": e });
                        if let Some(id) = id {
                            obj["id"] = json!(id);
                        }
                        // SAFETY: `out_json` non-NULL (checked) and writable
                        // per contract; caller frees the minted string.
                        unsafe { *out_json = registry::mint_string(obj.to_string()) };
                    }
                    Err(body_of(&e))
                }
            }
        })
    })
}

/// Undo the newest in-effect command (engine function, ES §10.3 — not a
/// command kind; `history.*` dagger kinds arrive at CP-0.7). On success
/// `*out_row_json` (when non-NULL) receives the reverted history row
/// `{"index","id","kind","timestamp"?,"active":false,"dirty":[…]?}` (free
/// with `appname_string_free`; `dirty` echoes the step's receipt rects) and
/// dirty/history events are queued. Nothing to undo reports
/// `error.history.nothing_to_undo` — including at a reopened document's
/// open base: the persisted prefix did not keep its delta history
/// (ES §10.4, suffix-aligned) and belongs to the replay path; commands
/// applied after the open undo normally.
///
/// # Safety
/// `doc` a handle from this library; `out_row_json`/`err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_undo(
    doc: *mut AppnameDoc,
    out_row_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        registry::with_doc(doc, |state| {
            let before = events::snapshot(&state.engine);
            let row = state.engine.undo().map_err(|e| body_of(&e))?;
            // Undo rows echo the step's receipt dirty rects (arbitration 2).
            events::emit_mutation(state, &before, &row.kind, &row.dirty);
            if !out_row_json.is_null() {
                let wire = serde_json::to_string(&row)
                    .unwrap_or_else(|_| r#"{"error":"row serialization failed"}"#.to_string());
                // SAFETY: `out_row_json` non-NULL (checked) and writable per
                // contract; caller frees the minted string.
                unsafe { *out_row_json = registry::mint_string(wire) };
            }
            Ok(AppnameStatus::AppnameOk)
        })
    })
}

/// Redo the next undone command. Mirror of `appname_doc_undo` (row has
/// `"active":true`); nothing to redo reports `error.history.nothing_to_redo`.
///
/// # Safety
/// As `appname_doc_undo`.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_redo(
    doc: *mut AppnameDoc,
    out_row_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        registry::with_doc(doc, |state| {
            let before = events::snapshot(&state.engine);
            let row = state.engine.redo().map_err(|e| body_of(&e))?;
            // Redo rows echo the step's receipt dirty rects (arbitration 2).
            events::emit_mutation(state, &before, &row.kind, &row.dirty);
            if !out_row_json.is_null() {
                let wire = serde_json::to_string(&row)
                    .unwrap_or_else(|_| r#"{"error":"row serialization failed"}"#.to_string());
                // SAFETY: `out_row_json` non-NULL (checked) and writable per
                // contract; caller frees the minted string.
                unsafe { *out_row_json = registry::mint_string(wire) };
            }
            Ok(AppnameStatus::AppnameOk)
        })
    })
}

// ---------------------------------------------------------------------------
// Binary ingress: assets (ES §9.2 asset block; §9.6 rule 2).
// ---------------------------------------------------------------------------

/// ABI 1.1 additive block (consumers running against older libraries gate
/// its use on `appname_version().abi_minor >= 1`, N.5 rule 2; the raster
/// arm below is ABI 1.2 behavior — gate raster ingress on
/// `abi_minor >= 2`).
///
/// Copy `len` bytes into the document asset store and return the assetId
/// UUID (lowercase hyphenated, as an `AppnameString` the caller frees with
/// `appname_string_free`) that command JSON references — the `"samples"`
/// field of stroke envelopes, or the `"assetId"` param of `edit.paste` /
/// `edit.paste_into` (ES §9.6 rule 2: large binary never travels in JSON).
/// The bytes are validated and copied BEFORE return; the caller keeps
/// ownership of `bytes`. Ids are content-addressed: the same bytes yield
/// the same id. Assets persist with the journal entries that reference
/// them, plus a session grace for just-registered ones (GC discipline).
///
/// Accepted blob types (the CP-0.3 samples-only restriction relaxed
/// additively at the ABI-growth round — the ABI 1.2 block — per its own
/// documented clause):
/// 1. `appname.samples.v1` (16-byte `APNSAMPL` header + 48-byte records,
///    ES §15.1) — sniffed FIRST, registered as a samples asset;
/// 2. otherwise the engine's raster-asset ingress (`edit.paste`'s carrier;
///    PNG at CP-0.5, decode-validated at ingestion — the ratified MAJOR-2
///    flow, so paste/drag-drop/File→Import replay byte-identically).
///
/// Anything else rejects `error.command.bad_target`, naming both contracts
/// — the same typed answer as the CLI's `asset add` / `import` doors.
///
/// When an autosave binding is live (`appname_doc_autosave_begin`), the
/// asset bytes are framed into the log before any referencing envelope can
/// follow (§9.6 rule 2). If that frame append fails, the call reports
/// `APPNAME_ERR_IO` (`error.io.write_failed`) AFTER the asset has already
/// registered engine-side: the ASSET IS REGISTERED — commands may
/// reference it in this session — and only the recovery-durability
/// guarantee failed (the same posture as `appname_doc_apply_command`'s
/// append-failure arm, where the COMMAND HAS APPLIED). `*out_uuid` is not
/// written on this failure, but ids are content-addressed: retrying with
/// the same bytes converges on the same id and re-frames it, so a
/// successful retry fully repairs the log.
///
/// # Safety
/// `doc` a handle from this library; `bytes` must point to `len` readable
/// bytes when `len > 0`; `out_uuid` must be non-NULL and writable; `err`
/// NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_asset_create(
    doc: *mut AppnameDoc,
    bytes: *const u8,
    len: usize,
    out_uuid: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out_uuid.is_null() {
            return Err(invalid_arg("out_uuid must not be NULL"));
        }
        // SAFETY: forwarded caller contract — `bytes` covers `len` readable
        // bytes; the slice is consumed inside this call only.
        let blob = unsafe { read_bytes(bytes, len, "bytes") }?;
        let uuid = registry::with_doc(doc, |state| {
            // Type sniff at the assembly point (CLI parity): samples blobs
            // first (opaque to core, decoded here), else the engine's
            // decode-validated raster ingress.
            let (id, kind) = match appname_tools::brush::decode_samples(blob) {
                Ok(_) => (
                    state.engine.add_samples_asset(blob.to_vec()),
                    appname_core::engine::assets::AssetKind::Samples,
                ),
                // Q-D5(a) IMPORT WIDENING, 2026-08-29. The blob used to go
                // straight to add_raster_asset, which decode-validates PNG
                // and nothing else -- so the SHELL WROTE SEVEN FORMATS AND
                // READ ONE. It now passes the allow-listed ingress decoder
                // first, which sniffs through the codec registry and hands
                // the engine the PNG carrier (PNG itself passes through
                // byte-identical, so existing asset ids are unchanged).
                //
                // ★ NO EXTENSION IS AVAILABLE HERE and that is correct
                // rather than a shortcoming: this door takes BYTES, not a
                // path, so the sniff is content-only. The extension is
                // only ever the registry's tie-break between equal-length
                // magics, never the accept key.
                Err(samples_err) => match appname_io::decode_for_ingress(blob.to_vec(), None)
                    .map_err(|e| appname_core::EngineError::bad_target(e.to_string()))
                    .and_then(|carrier| state.engine.add_raster_asset(carrier))
                {
                    Ok(id) => (id, appname_core::engine::assets::AssetKind::RasterImage),
                    Err(raster_err) => {
                        return Err(body_of(&appname_core::EngineError::bad_target(format!(
                            "not an appname.samples.v1 blob ({samples_err}); {}",
                            raster_err.message
                        ))))
                    }
                },
            };
            // §9.6 rule-2 ordering for recovery: the asset frame lands in
            // the autosave log BEFORE any envelope that references it.
            crate::recovery::on_asset_created(state, id, kind, blob)?;
            Ok(id)
        })?;
        // SAFETY: `out_uuid` non-NULL (checked) and writable per contract;
        // caller frees the minted string.
        unsafe { *out_uuid = registry::mint_string(uuid.to_string()) };
        Ok(AppnameStatus::AppnameOk)
    })
}

// ---------------------------------------------------------------------------
// Reads.
// ---------------------------------------------------------------------------

/// Document summary as UTF-8 JSON (free with `appname_string_free`) — the
/// CLI `info --json` object shape grown by the **info v2 additive block**
/// (ABI-growth round, O-UI request 3) and the **info v3 additive block**
/// (consolidated round, O-UI request 8; every earlier field is unchanged,
/// v3 fields are additions per N.5 rule 4 — §2.8 hosts ignore what they
/// don't know):
///
/// * v1: `{"docId","width","height","layers":[{"index","id","name",
///   "visible","opacity","kind"}],"selection":{"active"},"journal":
///   {"length","cursor","canUndo","canRedo"}}`.
/// * v2 per-layer additions: `parentId` (uuid; the root group for
///   top-level layers), `depth` (0 = child of root), `blend`,
///   `fillOpacity`, `locks` (array of `"transparency"|"pixels"|
///   `"position"`), `clipToBelow`, `colorLabel` (string or null),
///   `isolated` (groups), and `masks`: `[{"id","enabled","inverted",
///   "density","feather","linked"}]` in attachment order.
/// * v2 top-level additions: `rootLayerId`, `depth` (document channel
///   depth, e.g. `"u8"`), `profile` `{"id","source":"builtin"|"icc"|
///   "untagged","name"}` (name = the builtin's stable machine tag or the
///   source label — display naming is the shell's i18n concern),
///   `resolutionPpi`, `guides` `{"horizontal":[…],"vertical":[…],
///   "locked"}` (sorted, doc px, sub-pixel), `grid` (null until `grid.set`:
///   `{"spacing","subdivisions","show","color"?}`), and
///   `selection.bounds` `[x, y, w, h]` (absent while inactive — the tight
///   bounds of nonzero coverage, ES §13.1).
/// * v3 top-level addition: `paths` — the §20 doc-path arena (ALWAYS
///   present, `[]` when empty; the paths panel's truth, undo-safe: re-read
///   after any history event). Rows in deterministic uuid order:
///   `{"id","name"` (string or null)`,"bounds":[x,y,w,h]` (the path's
///   integer hull, doc px)`,"referencedBy":[…]` (layer uuids in z-order
///   whose vector content references the path — shape layers by base path
///   or boolean operand)`,"inBoolOps"` (true when at least one reference
///   is as a §20.3 boolean-op OPERAND)`}`.
///
/// `canUndo`/`canRedo` report the LIVE in-process capability
/// (suffix-aligned, ES §10.4): false at a freshly opened package's base
/// even when the journal is deep — the persisted prefix belongs to the
/// replay path — and true exactly when undo/redo would succeed within the
/// session suffix. A pre-init handle reports `error.command.bad_target`.
///
/// ABI 1.7 additive block (gate on `appname_version().abi_minor >= 7`).
/// `"imageRect": [x, y, w, h]` — the document's OUTPUT FRAME (§19.1), i.e.
/// `crop_rect.unwrap_or(canvas_rect())`. This is the frame every export and
/// composite door already frames on, so a consumer that paints and exports
/// on `imageRect` is correct with or without an active crop.
/// `"width"`/`"height"` above are UNCHANGED and keep meaning the CANVAS.
/// ONE NAME, ONE COMPUTATION: `imageRect` means `image_rect()` at every
/// door. Shape matches `selection.bounds` in the same payload.
/// ⚠ THE ORIGIN IS LOAD-BEARING: `appname_doc_read_region` frames on the
/// caller's ABSOLUTE document coordinates and does not clamp to canvas or
/// image, so a consumer given only a SIZE would ask for `(0,0,w,h)` and get
/// the right-sized region of the WRONG PIXELS under any crop whose origin is
/// not `(0,0)`. With no crop active `imageRect` is the canvas at origin, so
/// a consumer may bind it unconditionally.
///
/// ⚠ WHAT `abi_minor >= 7` DOES AND DOES NOT ASSERT. It asserts exactly two
/// things: `imageRect` is emitted, and `doc.crop` HONOURS
/// `deletePixels:false` by setting `crop_rect` rather than destroying
/// pixels. It is a COMPATIBILITY MARKER — it reports what the library you
/// are LINKED AGAINST does. It is NOT a capability catalogue: do NOT read a
/// version number to decide whether some other command KIND is served. That
/// question has its own door and a version string is existence-checking in
/// disguise. It is also NOT a composite-time guarantee — a command reaching
/// the composite door may still refuse, typed, and the typed refusal is the
/// authority.
///
/// # Safety
/// `doc` a handle from this library; `out_json` must be non-NULL and
/// writable; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_info_json(
    doc: *mut AppnameDoc,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out_json.is_null() {
            return Err(invalid_arg("out_json must not be NULL"));
        }
        let wire = registry::with_doc(doc, |state| {
            let engine = &state.engine;
            let d = engine.document().ok_or_else(|| {
                body_of(&appname_core::EngineError::bad_target(
                    "document not initialized; apply doc.new first",
                ))
            })?;
            let layers: Vec<serde_json::Value> = d
                .z_order_walk()
                .iter()
                .enumerate()
                .filter_map(|(i, id)| {
                    let l = d.layer(*id)?;
                    let parent = d.parent_of(*id);
                    // depth 0 = directly under the root group.
                    let mut depth = 0usize;
                    let mut probe = parent;
                    while let Some(p) = probe {
                        if p == d.root() {
                            break;
                        }
                        depth += 1;
                        probe = d.parent_of(p);
                    }
                    let masks: Vec<serde_json::Value> = l
                        .masks()
                        .iter()
                        .filter_map(|m| d.mask(*m))
                        .map(|m| {
                            json!({
                                "id": m.id(),
                                "enabled": m.enabled(),
                                "inverted": m.inverted(),
                                "density": m.density(),
                                "feather": m.feather(),
                                "linked": m.linked(),
                            })
                        })
                        .collect();
                    let mut row = json!({
                        // v1 block — byte-compatible.
                        "index": i,
                        "id": l.id(),
                        "name": l.name(),
                        "visible": l.visible(),
                        "opacity": l.opacity(),
                        "kind": l.kind().kind_str(),
                        // info v2 additive block.
                        "parentId": parent.map(|p| p.as_uuid().to_string()),
                        "depth": depth,
                        "blend": l.blend(),
                        "fillOpacity": l.fill_opacity(),
                        "locks": l.locks(),
                        "clipToBelow": l.clip_to_below(),
                        "colorLabel": l.color_label(),
                        "masks": masks,
                        // q-tutorial/Q-7 (MAJOR): every `op.*` kind requires
                        // `params.opId` and no read door published one, so an
                        // adjustment in a document you did not just author could
                        // not be masked, retuned, toggled, reordered or deleted.
                        // Additive, in this door's own v2 spirit; the CLI's info
                        // emitter publishes the same concept under the same name.
                        "ops": l
                            .adjustment_op()
                            .into_iter()
                            .chain(
                                l.smart_object()
                                    .into_iter()
                                    .flat_map(|so| so.smart_filters().iter().copied()),
                            )
                            .filter_map(|oid| {
                                d.op(oid).map(|o| json!({
                                    "id": oid.as_uuid(),
                                    "kind": o.op(),
                                    "enabled": o.enabled(),
                                }))
                            })
                            .collect::<Vec<_>>(),
                    });
                    if l.is_group() {
                        row["isolated"] = json!(l.isolated());
                    }
                    Some(row)
                })
                .collect();
            let cursor = engine.journal_cursor();
            let len = engine.journal().len();
            use appname_core::graph::document::DocProfileSource;
            let working = d.color().working;
            let profile = match d.profile(working).map(|p| &p.source) {
                Some(DocProfileSource::Builtin(space)) => json!({
                    "id": working,
                    "source": "builtin",
                    "name": space.tag(),
                }),
                Some(DocProfileSource::Icc(_)) => json!({
                    "id": working,
                    "source": "icc",
                    "name": "embedded-icc",
                }),
                // [O-CORE CP-1.0 door-slice-3 mechanical arm, RATIFIED by
                // O-SCRIPT (this round): the CIELAB connection space is
                // CMM-built (`Cms::builtin_lab`, §6.2/§5.3) — a builtin
                // with nothing to embed, named after its deterministic
                // seed id `profile/builtin/lab` (the `space.tag()`
                // spelling convention, `builtin:` namespace).]
                Some(DocProfileSource::BuiltinLab) => json!({
                    "id": working,
                    "source": "builtin",
                    "name": "builtin:lab",
                }),
                Some(DocProfileSource::Untagged) | None => json!({
                    "id": working,
                    "source": "untagged",
                    "name": "untagged",
                }),
            };
            // info v3: the §20 doc-path arena (O-UI request 8 — the paths
            // panel's truth; read through core's public arena surface).
            let paths: Vec<serde_json::Value> = d
                .path_ids_sorted()
                .iter()
                .filter_map(|pid| {
                    let p = d.path(*pid)?;
                    let refs = d.path_referencing_layers(*pid);
                    let in_bool_ops = refs.iter().any(|lid| {
                        d.layer(*lid)
                            .and_then(|n| n.shape_data())
                            .map(|s| s.bool_ops().iter().any(|(_, op_pid)| op_pid == pid))
                            .unwrap_or(false)
                    });
                    let b = p.bounds();
                    Some(json!({
                        "id": pid.as_uuid().to_string(),
                        "name": p.name(),
                        "bounds": [b.min.x, b.min.y, b.width(), b.height()],
                        "referencedBy": refs
                            .iter()
                            .map(|l| l.as_uuid().to_string())
                            .collect::<Vec<_>>(),
                        "inBoolOps": in_bool_ops,
                    }))
                })
                .collect();
            let guides = d.guides();
            let mut selection = json!({ "active": d.selection().active() });
            if d.selection().active() {
                let b = d.selection().bounds();
                selection["bounds"] = json!([b.min.x, b.min.y, b.width(), b.height()]);
            }
            Ok(json!({
                "docId": d.id(),
                "width": d.geometry().canvas_size.x,
                "height": d.geometry().canvas_size.y,
                // ABI 1.7 additive: the OUTPUT frame (§19.1). `image_rect()`
                // is `crop_rect.unwrap_or(canvas_rect())` — the frame every
                // export and composite door already frames on — so a consumer
                // painting on `imageSize` is correct with or without an active
                // crop, and equals the canvas when none is set. `width`/`height`
                // above keep meaning CANVAS and are deliberately unchanged:
                // ONE NAME, ONE COMPUTATION (O-UI's constraint — a shared name
                // with two computations is the MAJOR-32 shape).
                // ⚠ RECT, NOT SIZE, AND THE ORIGIN IS LOAD-BEARING. The
                // readback door (`appname_doc_read_region`) frames on the
                // caller's ABSOLUTE document coordinates — it does not clamp
                // to canvas or to image — so a consumer given only a SIZE
                // would request (0,0,w,h) and receive the correct-sized
                // region of the WRONG PIXELS whenever a crop's origin is not
                // (0,0). Publishing the whole rect keeps the arithmetic out
                // of every consumer. Shape matches `selection.bounds` above.
                "imageRect": [
                    d.geometry().image_rect().min.x,
                    d.geometry().image_rect().min.y,
                    d.geometry().image_rect().width(),
                    d.geometry().image_rect().height()
                ],
                "layers": layers,
                "selection": selection,
                "journal": {
                    "length": len,
                    "cursor": cursor,
                    "canUndo": engine.undoable_in_process(),
                    "canRedo": engine.redoable_in_process(),
                },
                // info v2 additive block.
                "rootLayerId": d.root().as_uuid().to_string(),
                "depth": d.color().depth,
                "profile": profile,
                "resolutionPpi": d.geometry().resolution_ppi,
                "guides": {
                    "horizontal": guides.h,
                    "vertical": guides.v,
                    "locked": guides.lock,
                },
                "grid": guides.grid.as_ref().map(|g| json!({
                    "spacing": g.spacing,
                    "subdivisions": g.subdivisions,
                    "show": g.show,
                    "color": g.color,
                })),
                // info v3 additive block (N.5 rule 4).
                "paths": paths,
            })
            .to_string())
        })?;
        // SAFETY: `out_json` non-NULL (checked) and writable per contract.
        unsafe { *out_json = registry::mint_string(wire) };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// History list as UTF-8 JSON (free with `appname_string_free`):
/// `{"length","cursor","entries":[{"index","id","kind","timestamp"?,
/// "active","filterId"?}]}` — journal rows + cursor (available on freshly
/// opened packages too; `active:false` rows behind the cursor are redoable).
/// Legal on a pre-init handle (`length` 0).
///
/// ABI 1.8 additive block (gate on `appname_version().abi_minor >= 8`).
/// `"filterId": "filter.<id>"` — WHICH filter a `filter.apply` row applied.
/// `filter.apply` is ONE kind over the eight filters registered in §16.1, so a
/// row bearing only `kind` projects all eight onto the single label
/// `history.filter.apply` ("Apply Filter") and a consumer cannot recover which
/// one ran. The key carries the command's own REQUIRED `filterId` param
/// verbatim — the same string the caller sent, not a re-derivation.
/// ⚠ IT IS ABSENT, NOT NULL, on every row that needs no such subject, so the
/// other 101 kinds serialize exactly as they did at 1.7. A consumer must read
/// a missing key as "this row's `kind` names it fully".
///
/// ⚠ WHAT `abi_minor >= 8` DOES AND DOES NOT ASSERT. It asserts exactly one
/// thing: `filterId` is emitted on `filter.apply` rows. It is a COMPATIBILITY
/// MARKER — it reports what the library you are LINKED AGAINST emits. It is
/// NOT a statement about which filters exist, and it does NOT promise that any
/// particular `filter.<id>` is registered or servable; that question has its
/// own door. Reading a version number to decide whether a filter is available
/// is existence-checking in disguise.
///
/// # Safety
/// As `appname_doc_info_json`.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_history_json(
    doc: *mut AppnameDoc,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out_json.is_null() {
            return Err(invalid_arg("out_json must not be NULL"));
        }
        let wire = registry::with_doc(doc, |state| {
            let entries = state.engine.journal_rows();
            Ok(json!({
                "length": entries.len(),
                "cursor": state.engine.journal_cursor(),
                "entries": entries,
            })
            .to_string())
        })?;
        // SAFETY: `out_json` non-NULL (checked) and writable per contract.
        unsafe { *out_json = registry::mint_string(wire) };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Drain queued events as one UTF-8 JSON array (free with
/// `appname_string_free`); `"[]"` when nothing happened since the last
/// poll. Payloads are ES §9.5 shapes, emission order preserved, and the
/// per-mutation relative order is the NORMATIVE `treeChanged` →
/// `dirtyRects` → `history`. `appname.event.treeChanged.v1` (ABI 1.2
/// additive block — its named trigger, "when the layer-panel surface
/// grows", was pulled by the chrome wave; gate on `abi_minor >= 2`)
/// carries `{"kind":"added"|
/// "removed"|"moved"|"props","layerIds":[…]}` — one event per kind per
/// mutation, derived from before/after tree-state truth, so undo/redo
/// emit the inverse of their step and hosts refresh exactly the listed
/// ids. Example, a stroke touching two tiles:
/// `{"schema":"appname.event.dirtyRects.v1","docId":…,"thread":"command",
/// "scope":"composite","rects":[[0,0,256,256],[256,0,256,256]]}` then
/// `{"schema":"appname.event.history.v1","docId":…,"thread":"command",
/// "undoDepth":…,"redoDepth":…,"labelMessageId":…,"coalesced":false}`.
/// A dirtyRects event carries ONE OR MORE rects — at most 8: receipt rects
/// are §3.7-coalesced (rects within the 8 px slop union to fixpoint; a set
/// exceeding 8 rects collapses to its single bounding box), so the array's
/// union always covers the whole change. Consumers MUST iterate the whole
/// `rects` array and MUST NOT assume `rects[0]` covers the change. Dirty
/// rects are the command RECEIPT's rects in composite space (a command
/// whose receipt reports no visible change emits no dirtyRects event);
/// undo/redo echo the step's rects. Hosts MUST
/// ignore unknown fields and unknown schemas (ES §2.8 mirror). If more than
/// 4096 events queue between polls, older dirty rects coalesce into one
/// covering rect, treeChanged `layerIds` union per kind (coverage never
/// shrinks; refresh the listed ids against info truth), and only the
/// newest history row survives.
///
/// # Safety
/// As `appname_doc_info_json`.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_poll_events(
    doc: *mut AppnameDoc,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out_json.is_null() {
            return Err(invalid_arg("out_json must not be NULL"));
        }
        let wire = registry::with_doc(doc, |state| Ok(events::drain_json(state)))?;
        // SAFETY: `out_json` non-NULL (checked) and writable per contract.
        unsafe { *out_json = registry::mint_string(wire) };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Read the flattened composite of `rect` (document space, top-left origin)
/// as tightly packed RGBA8: sRGB-encoded, STRAIGHT (un-premultiplied)
/// alpha, stride = `w * 4` bytes. Access the bytes via
/// `appname_buffer_map`; free with `appname_buffer_free`. Pixels outside
/// the canvas read `0,0,0,0`. Rects with zero extent, or covering more
/// than 2^26 px, are rejected (`error.api.invalid_arg`) — read tiles or
/// viewports, not gigapixel blocks. A pre-init handle reports
/// `error.command.bad_target`.
///
/// ONE compositing truth: readback composites through the §34.3
/// parity-gated render compositor over the engine's §12.1 op registry
/// (blend modes, layer/fill opacity, groups, adjustment layers, enabled
/// smart-filter chains — CLI parity) and converts with the §6.4 delivery
/// math — the SAME code path `appname_doc_export` encodes, so a readback
/// of a region is pixel-identical to the export of that region.
/// Constructs the compositor refuses (unregistered enabled op kinds and
/// the other typed §12.1 arms) refuse here identically.
///
/// # Safety
/// `doc` a handle from this library; `out` must be non-NULL and writable;
/// `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_read_region(
    doc: *mut AppnameDoc,
    rect: AppnameRect,
    out: *mut *mut AppnameImageBuffer,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        let data = registry::with_doc(doc, |state| {
            composite::composite_region_rgba8(&state.engine, rect.x, rect.y, rect.w, rect.h)
                .map_err(|e| body_of(&e))
        })?;
        let handle = registry::mint_buffer(BufferBody {
            data,
            width: rect.w,
            height: rect.h,
            stride: rect.w * 4,
            pixel_format: 0, // u8 (ES §7.4)
            color_model: 3,  // Rgba
            mapped: false,
        });
        // SAFETY: `out` non-NULL (checked) and writable per contract; the
        // caller owns the minted buffer (appname_buffer_free).
        unsafe { *out = handle };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// ABI 1.4 additive block (the consolidated round, O-UI ABI-growth
/// request 7; gate on `appname_version().abi_minor >= 4`, N.5 rule 2).
/// Read ONE layer's SELF-CONTAINED content over `rect` (document space,
/// top-left origin) as tightly packed RGBA8 under the exact
/// `appname_doc_read_region` delivery convention: sRGB-encoded, STRAIGHT
/// alpha, stride = `w * 4` bytes; map with `appname_buffer_map`, free with
/// `appname_buffer_free`; the same geometry discipline (zero-extent rects
/// and rects over 2^26 px reject `error.api.invalid_arg`). Unblocks layer
/// thumbnails, faithful Cut pairing, and the move tool's per-layer echo.
///
/// `layer_id` is the layer's UUID as a NUL-terminated string (the id
/// `appname_doc_info_json` rows report). Content, not contribution: the
/// layer's own blend mode, opacity, and fillOpacity describe its
/// interaction with the backdrop and are NOT applied — a thumbnail does
/// not fade with layer opacity; hosts echoing a drag apply §4.4 from
/// info's reported props. Pixels outside the layer's content read
/// `0,0,0,0`.
///
/// Service matrix (the render/core truth; refusals mirror the §34.3
/// compositor's typed arms and the export door's posture):
/// * raster layers serve their pixels (a lone source over transparency is
///   the §4.4 source-over identity);
/// * smart objects serve their §12.1-EVALUATED result — the value-captured
///   proxy through the ENABLED smart-filter chain via the engine's op
///   registry (§12.4: no enabled entries serves the proxy directly);
///   chain refusals (unregistered enabled kind, non-Normal fade blend,
///   feathered op mask) surface typed;
/// * an adjustment layer refuses `error.command.target_kind_mismatch` —
///   its §17.1 result exists only over its backdrop; read the document
///   composite instead;
/// * a group SERVES its content under the AS-IF-ISOLATED ELECTION (the
///   contract, stated here for API clients, not just rustdoc): a group's
///   content is its CHILDREN COMPOSITED AS-IF-ISOLATED — onto a
///   transparent backdrop, with full §4.4–§4.7 blend/opacity/visibility
///   semantics applying WITHIN the subtree (nested groups, smart layers,
///   child visibility: everything), while the group's OWN contribution
///   props (blend mode, opacity, visibility) are NOT applied — a hidden
///   or faded group still serves its content, and a pass-through group
///   serves its isolated content (content is a definition, not a
///   decomposition of the document composite). The root group's content
///   is therefore exactly the whole-document composite. The §11.4
///   boundary REFUSES: an adjustment layer anywhere in the target group's
///   scope is `error.command.not_implemented` with the isolating scope
///   named — its §17.1 result over the isolated backdrop is not its
///   document content, so it is never approximated (read the document
///   composite for adjustment reach). (This capability arrived additively
///   under this symbol at the recorded render-door handoff — no ABI
///   change, exactly as filed.);
/// * non-identity transforms, clipToBelow, attached masks, non-RGBA
///   sources, and kinds without a composite path (e.g. shape, this
///   checkpoint) refuse `error.command.not_implemented`, exactly as the
///   whole-document composite would.
///
/// An unknown/unresolvable layer id is `error.command.bad_target`; a
/// pre-init handle is `error.command.bad_target`.
///
/// # Safety
/// `doc` a handle from this library; `layer_id` a valid NUL-terminated C
/// string; `out` must be non-NULL and writable; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_read_layer(
    doc: *mut AppnameDoc,
    layer_id: *const c_char,
    rect: AppnameRect,
    out: *mut *mut AppnameImageBuffer,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        // SAFETY: forwarded caller contract — `layer_id` is a valid
        // NUL-terminated string for the duration of the call.
        let id_text = unsafe { read_cstr(layer_id, "layer_id") }?;
        let layer = uuid::Uuid::parse_str(id_text.trim())
            .map_err(|_| invalid_arg("layer_id must be a UUID string"))?;
        let data = registry::with_doc(doc, |state| {
            composite::layer_region_rgba8(&state.engine, layer, rect.x, rect.y, rect.w, rect.h)
                .map_err(|e| body_of(&e))
        })?;
        let handle = registry::mint_buffer(BufferBody {
            data,
            width: rect.w,
            height: rect.h,
            stride: rect.w * 4,
            pixel_format: 0, // u8 (ES §7.4)
            color_model: 3,  // Rgba
            mapped: false,
        });
        // SAFETY: `out` non-NULL (checked) and writable per contract; the
        // caller owns the minted buffer (appname_buffer_free).
        unsafe { *out = handle };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// ABI 1.2 additive block (ABI-growth round; gate on
/// `appname_version().abi_minor >= 2`, N.5 rule 2). Read the SELECTION
/// coverage mask over `rect` (document space, top-left origin) as tightly
/// packed Gray8: `pixel_format` 0 / `color_model` 0 on the map info,
/// stride = `w` bytes, 0 = unselected .. 255 = fully selected (the §13.1
/// f32 mask under the §8.2 8-bit delivery optimization — feathered edges
/// arrive as intermediate coverage). Same delivery convention and geometry
/// discipline as `appname_doc_read_region` (map with `appname_buffer_map`,
/// free with `appname_buffer_free`; zero-extent or > 2^26 px rects reject
/// `error.api.invalid_arg`; a pre-init handle reports
/// `error.command.bad_target`). An INACTIVE selection reads back all-zero;
/// hosts branch on info's `selection.active`/`selection.bounds` first —
/// this symbol unblocks marching ants (O-UI request 4).
///
/// # Safety
/// `doc` a handle from this library; `out` must be non-NULL and writable;
/// `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_read_selection(
    doc: *mut AppnameDoc,
    rect: AppnameRect,
    out: *mut *mut AppnameImageBuffer,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        let data = registry::with_doc(doc, |state| {
            composite::selection_region_gray8(&state.engine, rect.x, rect.y, rect.w, rect.h)
                .map_err(|e| body_of(&e))
        })?;
        let handle = registry::mint_buffer(BufferBody {
            data,
            width: rect.w,
            height: rect.h,
            stride: rect.w,
            pixel_format: 0, // u8 (ES §7.4)
            color_model: 0,  // Gray coverage
            mapped: false,
        });
        // SAFETY: `out` non-NULL (checked) and writable per contract; the
        // caller owns the minted buffer (appname_buffer_free).
        unsafe { *out = handle };
        Ok(AppnameStatus::AppnameOk)
    })
}
