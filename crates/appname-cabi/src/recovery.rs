// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Autosave binding + crash-recovery surface (ABI 1.2 additive block —
//! O-UI ABI-growth request 5; unblocks the W05 GUI prompt).
//!
//! Spec: ES §7.8 (autosave journal: BASE anchor to a saved package,
//! COMMAND/ASSET frames, CHECKPOINT durability, torn-tail truncation,
//! fold-into-package on save), §10.6 (recovery = base state + journaled
//! commands), §9.6 rule 2 (asset frames precede referencing envelopes);
//! W05 (kill mid-edit, recover to the last journaled command). The engine
//! machinery is `appname_core::io::autosave` (the CP-0.5 "Autosave &
//! recovery" row) — this module WRAPS it behind ABI symbols, per the §3.11
//! single-door rule.
//!
//! Binding semantics (documented divergences follow the CLI `--autosave`
//! precedent, divergence-noted in the lane log):
//!
//! * **Bound to apply**: every successful `appname_doc_apply_command`
//!   appends the applied envelope and CHECKPOINTs (fsync). This is the
//!   strongest §7.8 cadence — stricter than the interval/idle cadences the
//!   spec permits interactive shells; per-command durability is the same
//!   election the headless CLI made. Undo/redo append nothing: recovery
//!   restores to the last APPLIED command (the W05 posture) and in-process
//!   cursor position is session state (§10.4).
//! * **Undo-then-apply truncation**: applying after undos rewrites the
//!   journal suffix. The log detects the truncation (journal length did
//!   not grow by one) and REWRITES itself — fresh BASE, still-relevant
//!   session asset frames, then every journal envelope past the BASE
//!   anchor — so replay over the base package reproduces the live journal
//!   exactly, never the abandoned redo tail.
//! * **Fold on save** (§7.8 fold rule): a successful `appname_doc_save`
//!   truncates the log to a fresh durable BASE anchored at that save;
//!   every engine-PENDING asset is re-framed so a later paste referencing
//!   it stays recoverable.
//! * **Pending assets frame at begin, fold, and rewrite** (fix-round
//!   MAJOR): the frame set is every ENGINE-pending asset
//!   (`assets().is_pending`), never just ones registered while the
//!   binding was live — an asset registered before begin (save →
//!   asset_create → begin → paste) must reach the log before its
//!   referencing envelope or the replay fails wholesale.
//! * The BASE anchor contract is the caller's: begin autosave when the
//!   in-memory document matches a package on disk (right after open/save);
//!   recovery pairs that package with this log. For never-saved documents
//!   the shell saves a recovery base first (`appname_doc_save` to its
//!   scratch location) — expressible with existing symbols by design.

use std::ffi::c_char;
use std::path::Path;

use appname_core::engine::assets::AssetKind;
use appname_core::io::autosave::{read_log, recover, Autosave};
use appname_core::EngineError;
use serde_json::json;
use uuid::Uuid;

use crate::doc::AppnameDoc;
use crate::events;
use crate::mem::{AppnameError, AppnameString};
use crate::registry::{self, DocState, ErrorBody};
use crate::{body_of, guarded, invalid_arg, read_cstr, AppnameStatus};

/// Live autosave binding carried on a document handle.
pub(crate) struct AutosaveState {
    log: Autosave,
    /// Journal length at the BASE anchor (== the anchored package's journal
    /// length per the begin contract); COMMAND frames cover `base_len..`.
    base_len: usize,
    /// Journal length after the last append — detects suffix truncation
    /// (undo-then-apply) which forces a log rewrite.
    last_len: usize,
    /// Asset ids framed into the log since the BASE anchor, in framing
    /// order. Seeded at begin (and refreshed at fold/rewrite) from EVERY
    /// engine-pending asset — not just ones registered through this
    /// binding — so an asset registered BEFORE `appname_doc_autosave_begin`
    /// stays recoverable when a later envelope references it (the
    /// save → asset_create → begin → paste sequence).
    session_assets: Vec<Uuid>,
}

/// Every engine-pending asset id, deterministic (sorted) order. Pending =
/// registered this session, not yet carried by a loaded package (the GC
/// grace set) — exactly the blobs a recovery replay cannot obtain from the
/// BASE package and must therefore find as log frames.
fn engine_pending_assets(state: &DocState) -> Vec<Uuid> {
    state
        .engine
        .assets()
        .ids_sorted()
        .into_iter()
        .filter(|id| state.engine.assets().is_pending(*id))
        .collect()
}

fn io_write(context: &str, e: std::io::Error) -> ErrorBody {
    body_of(&EngineError::new(
        appname_core::ErrorCode::IoWriteFailed,
        format!("{context}: {e}"),
    ))
}

/// Append every journal envelope in `base_len..` plus the asset frames
/// (assets FIRST — §9.6 rule-2 ordering), then checkpoint. The frame set
/// is refreshed to EVERY engine-pending asset (fix-round MAJOR: a
/// binding-session-only set silently dropped assets registered before
/// begin, making their referencing envelopes unreplayable).
fn append_suffix(state: &mut DocState, a: &mut AutosaveState) -> Result<(), ErrorBody> {
    a.session_assets = engine_pending_assets(state);
    for id in &a.session_assets {
        if let (Some(bytes), Some(kind)) = (
            state.engine.assets().bytes(*id).map(<[u8]>::to_vec),
            state.engine.assets().kind_of(*id),
        ) {
            a.log
                .append_asset(*id, kind, &bytes)
                .map_err(|e| io_write("appending an autosave asset frame", e))?;
        }
    }
    let tail: Vec<appname_core::Envelope> = state.engine.journal()[a.base_len..].to_vec();
    let last = tail.last().map(|e| e.id);
    for env in &tail {
        a.log
            .append_command(env)
            .map_err(|e| io_write("appending to the autosave journal", e))?;
    }
    a.log
        .checkpoint(last)
        .map_err(|e| io_write("checkpointing the autosave journal", e))?;
    a.last_len = state.engine.journal().len();
    Ok(())
}

/// Hook: one envelope committed through the single door.
pub(crate) fn on_apply_committed(state: &mut DocState) -> Result<(), ErrorBody> {
    let Some(mut a) = state.autosave.take() else {
        return Ok(());
    };
    let len = state.engine.journal().len();
    let result = if len == a.last_len + 1 {
        // The common path: append the one new envelope + checkpoint.
        let env = state
            .engine
            .journal()
            .last()
            .expect("apply just committed an envelope")
            .clone();
        a.log
            .append_command(&env)
            .and_then(|_| a.log.checkpoint(Some(env.id)))
            .map_err(|e| io_write("appending to the autosave journal", e))
            .map(|()| a.last_len = len)
    } else {
        // Suffix truncation (undo-then-apply): rewrite the log so replay
        // reproduces the live journal, never the abandoned redo tail.
        rewrite(state, &mut a)
    };
    state.autosave = Some(a);
    result
}

/// Recreate the log: fresh BASE + session asset frames + the journal past
/// the BASE anchor.
fn rewrite(state: &mut DocState, a: &mut AutosaveState) -> Result<(), ErrorBody> {
    let doc_id = state
        .engine
        .document()
        .map(|d| d.id().as_uuid())
        .unwrap_or_else(Uuid::nil);
    let path = a.log.path().to_path_buf();
    a.log =
        Autosave::create(&path, doc_id).map_err(|e| io_write("rewriting the autosave log", e))?;
    append_suffix(state, a)
}

/// Hook: an asset entered through `appname_asset_create` (frame it BEFORE
/// any referencing envelope — §9.6 rule 2).
pub(crate) fn on_asset_created(
    state: &mut DocState,
    id: Uuid,
    kind: AssetKind,
    bytes: &[u8],
) -> Result<(), ErrorBody> {
    let Some(mut a) = state.autosave.take() else {
        return Ok(());
    };
    let result = a
        .log
        .append_asset(id, kind, bytes)
        .and_then(|_| a.log.checkpoint(None))
        .map_err(|e| io_write("appending an autosave asset frame", e));
    if result.is_ok() && !a.session_assets.contains(&id) {
        a.session_assets.push(id);
    }
    state.autosave = Some(a);
    result
}

/// Hook: the package saved successfully — §7.8 fold rule.
pub(crate) fn on_saved(state: &mut DocState) -> Result<(), ErrorBody> {
    let Some(mut a) = state.autosave.take() else {
        return Ok(());
    };
    let result = (|| {
        a.log
            .rebase()
            .map_err(|e| io_write("folding the autosave log", e))?;
        a.base_len = state.engine.journal().len();
        a.last_len = a.base_len;
        // Re-frame EVERY engine-pending asset — not just binding-session
        // ones (fix-round MAJOR): a later envelope may reference an asset
        // registered before begin, and the freshly folded BASE package
        // does not carry pending blobs for a recovery replay.
        a.session_assets = engine_pending_assets(state);
        for id in a.session_assets.clone() {
            if let (Some(bytes), Some(kind)) = (
                state.engine.assets().bytes(id).map(<[u8]>::to_vec),
                state.engine.assets().kind_of(id),
            ) {
                a.log
                    .append_asset(id, kind, &bytes)
                    .map_err(|e| io_write("re-framing a pending session asset", e))?;
            }
        }
        a.log
            .checkpoint(None)
            .map_err(|e| io_write("checkpointing the folded autosave log", e))
    })();
    state.autosave = Some(a);
    result
}

// ---------------------------------------------------------------------------
// ABI symbols.
// ---------------------------------------------------------------------------

/// ABI 1.2 additive block (gate on `appname_version().abi_minor >= 2`).
/// Bind a §7.8 autosave log at `path` (created/truncated; parent
/// directories created) to this document handle: every subsequent
/// successful `appname_doc_apply_command` appends the applied envelope and
/// checkpoints (fsync — per-command durability, the strongest §7.8
/// cadence); `appname_asset_create` frames the asset bytes; a successful
/// `appname_doc_save` FOLDS the log (fresh BASE). Begin itself frames
/// every asset already pending at bind time, so assets registered BEFORE
/// this call stay recoverable when later envelopes reference them (§9.6
/// rule 2). Requires an initialized
/// document (`error.command.bad_target` otherwise). Call when the
/// in-memory document matches a package on disk — that package is the
/// recovery BASE `appname_doc_open_recovered` pairs with this log (module
/// doc; for a never-saved document, save a recovery base first). Binding
/// again replaces the previous binding (the old log file is left as-is).
/// If a later append fails, the apply call reports `APPNAME_ERR_IO`
/// (`error.io.write_failed`) — the COMMAND HAS APPLIED; only durability
/// failed. Unbind with `appname_doc_autosave_end`.
///
/// # Safety
/// `doc` a handle from this library; `path` a valid NUL-terminated C
/// string; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_autosave_begin(
    doc: *mut AppnameDoc,
    path: *const c_char,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract — `path` is a valid
        // NUL-terminated string for the duration of the call.
        let path = unsafe { read_cstr(path, "path") }?;
        registry::with_doc(doc, |state| {
            let doc_id = state
                .engine
                .document()
                .ok_or_else(|| {
                    body_of(&EngineError::bad_target(
                        "document not initialized; apply doc.new first",
                    ))
                })?
                .id()
                .as_uuid();
            let log = Autosave::create(Path::new(&path), doc_id)
                .map_err(|e| io_write("creating the autosave log", e))?;
            let len = state.engine.journal().len();
            let mut a = AutosaveState {
                log,
                base_len: len,
                last_len: len,
                session_assets: Vec::new(),
            };
            // Frame every asset already engine-pending at begin (fix-round
            // MAJOR): an asset registered BEFORE the binding existed —
            // save → asset_create → begin — must be in the log before any
            // envelope references it (§9.6 rule 2), or replay of that
            // envelope fails wholesale. The journal tail past the BASE is
            // empty here, so this appends asset frames + one checkpoint.
            append_suffix(state, &mut a)?;
            state.autosave = Some(a);
            Ok(())
        })?;
        Ok(AppnameStatus::AppnameOk)
    })
}

/// End the autosave binding: the writer is dropped; the log FILE is left
/// on disk exactly as last checkpointed (a crash between end and the next
/// begin still recovers). Ending with no live binding is an OK no-op
/// (idempotent, free-family style). To delete a log after a clean close or
/// a user's "Discard", use `appname_autosave_discard`.
///
/// # Safety
/// `doc` a handle from this library; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_autosave_end(
    doc: *mut AppnameDoc,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        registry::with_doc(doc, |state| {
            state.autosave = None;
            Ok(())
        })?;
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Scan an autosave log with the §7.8 torn-tail truncation semantics
/// (never fatal on a torn tail — it costs the tail, not the document) and
/// report what is recoverable, as UTF-8 JSON (free with
/// `appname_string_free`) shaped after the engine's `read_log`:
/// `{"docId","commands":<n>,"assets":<n>,"torn":<bool>,
/// "lastCommandId":<uuid|null>,"warnings":[…]}`. `commands` counts
/// replayable envelopes past the BASE anchor — `0` with `torn:false`
/// means a cleanly folded log (nothing to recover). Unreadable paths
/// report `error.io.read_failed`. This is the W05 prompt's evidence
/// surface: query BEFORE deciding to open-recovered or discard.
///
/// # Safety
/// `path` a valid NUL-terminated C string; `out_json` must be non-NULL and
/// writable; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_autosave_query(
    path: *const c_char,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out_json.is_null() {
            return Err(invalid_arg("out_json must not be NULL"));
        }
        // SAFETY: forwarded caller contract for `path`.
        let path = unsafe { read_cstr(path, "path") }?;
        let log = read_log(Path::new(&path)).map_err(|e| {
            body_of(&EngineError::new(
                appname_core::ErrorCode::IoReadFailed,
                format!("reading autosave log: {e}"),
            ))
        })?;
        let wire = json!({
            "docId": log.doc_id,
            "commands": log.commands.len(),
            "assets": log.assets.len(),
            "torn": log.torn,
            "lastCommandId": log.commands.last().map(|e| e.id),
            "warnings": log.warnings,
        })
        .to_string();
        // SAFETY: `out_json` non-NULL (checked) and writable per contract.
        unsafe { *out_json = registry::mint_string(wire) };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// §10.6 recovery door: load the BASE package at `package_path` through
/// the full ABI assembly, re-register the log's asset frames, replay every
/// journaled command not already in the base's journal, and hand back a
/// LIVE handle (free with `appname_doc_close`) — the same posture as
/// `appname_doc_open` (suffix-aligned history at the recovered state, one
/// whole-canvas dirtyRects seed queued). `out_report_json` (when non-NULL)
/// receives `{"replayed":<n>,"torn":<bool>,"warnings":[…]}` for the W05
/// prompt. A log whose BASE anchors a different document than the package
/// reports `error.command.bad_target`; a replay failure reports the
/// failing command's typed error (the document handle is NOT created).
/// The recovered document is in-memory only: the caller decides where to
/// save it. The log file is untouched — discard it explicitly once the
/// recovery is resolved.
///
/// # Safety
/// `package_path`/`log_path` valid NUL-terminated C strings; `out` must be
/// non-NULL and writable; `out_report_json` NULL or writable; `err` NULL
/// or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_open_recovered(
    package_path: *const c_char,
    log_path: *const c_char,
    out: *mut *mut AppnameDoc,
    out_report_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        // SAFETY: forwarded caller contract for both path parameters.
        let package = unsafe { read_cstr(package_path, "package_path") }?;
        let log = unsafe { read_cstr(log_path, "log_path") }?;
        // Font door (ABI 1.6): recover through the handle-retaining
        // assembly so the recovered engine's text service is the one the
        // retained handle drives (shared Arc / FontDb).
        let (assembly, text_handle) = crate::doc::abi_assembly_with_text_handle();
        let (engine, parsed, replayed) =
            recover(&assembly, Path::new(&package), Path::new(&log)).map_err(|e| body_of(&e))?;
        let mut state = DocState::new(engine).with_text_handle(text_handle);
        events::emit_open_seed(&mut state);
        let handle = registry::mint_doc(state);
        // SAFETY: `out` is non-NULL (checked) and writable per contract.
        unsafe { *out = handle };
        if !out_report_json.is_null() {
            let wire = json!({
                "replayed": replayed,
                "torn": parsed.torn,
                "warnings": parsed.warnings,
            })
            .to_string();
            // SAFETY: `out_report_json` non-NULL (checked) and writable per
            // contract; caller frees the minted string.
            unsafe { *out_report_json = registry::mint_string(wire) };
        }
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Discard an autosave log: delete the file at `path` (the W05 prompt's
/// "Discard" arm, and the clean-close cleanup after
/// `appname_doc_autosave_end`). A path that does not exist is an OK no-op
/// (idempotent — a double discard is not an error); any other filesystem
/// failure reports `error.io.write_failed`. Never call while the log is
/// still bound to a live handle on another thread — end the binding first.
///
/// # Safety
/// `path` a valid NUL-terminated C string; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_autosave_discard(
    path: *const c_char,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract for `path`.
        let path = unsafe { read_cstr(path, "path") }?;
        match std::fs::remove_file(Path::new(&path)) {
            Ok(()) => Ok(AppnameStatus::AppnameOk),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppnameStatus::AppnameOk),
            Err(e) => Err(io_write("discarding the autosave log", e)),
        }
    })
}
