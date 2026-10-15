// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! The action-invoke door — ABI 1.3 additive block.
//!
//! Spec: ES §26.1 (action format + replay semantics — implemented ONCE in
//! `appname-script` and consumed here; a per-host reimplementation of the
//! playback rules would fork normative semantics, the same reasoning that
//! rejected shell-side export encoding), §26.2 (step-report shape,
//! `error.batch.step_failed`), §25.1 (the ABI as the one extension
//! substrate — the Python tier's action invoke rides this symbol), ES
//! §9.1/Appendix N (additive-block discipline); plan §2.4 rows
//! "Actions/macros v1" / "Python tier v1" (lane O-SCRIPT).
//!
//! # Why this symbol exists (Appendix-N justification, task rule 4)
//!
//! Replaying an action is NOT expressible by composing existing symbols
//! without duplicating §26.1's normative playback rules (rule-3 target
//! rewriting, created-id remap, stop/disabled semantics, parameterized
//! substitution, asset re-entry ordering) in every host. The door keeps
//! ONE playback implementation engine-side; hosts submit the action
//! document and read a report. Everything the door does per step goes
//! through the SAME per-command pipeline as `appname_doc_apply_command`
//! (event snapshot → apply → §9.5 event emission → §7.8 autosave append),
//! so a replayed step is indistinguishable from a hand-applied one.

use std::ffi::c_char;

use serde_json::{json, Value};
use uuid::Uuid;

use appname_core::engine::assets::AssetKind;
use appname_core::engine::envelope::Envelope;
use appname_core::engine::ApplyReceipt;
use appname_core::{Engine, EngineError, ErrorCode};
use appname_script::replay::{replay, ApplyDoor, ReplayOptions, ReplayOutcome};
use appname_script::Action;

use crate::doc::AppnameDoc;
use crate::mem::{AppnameError, AppnameString};
use crate::registry::{self, DocState, ErrorBody};
use crate::{
    body_of, events, guarded, invalid_arg, read_bytes, read_cstr, status_of, AppnameStatus,
};

/// The C ABI's [`ApplyDoor`]: every replayed step rides the full
/// `appname_doc_apply_command` pipeline (module docs).
struct CabiDoor<'a> {
    state: &'a mut DocState,
}

impl ApplyDoor for CabiDoor<'_> {
    fn engine(&self) -> &Engine {
        &self.state.engine
    }

    fn apply(&mut self, env: Envelope) -> Result<ApplyReceipt, EngineError> {
        let before = events::snapshot(&self.state.engine);
        let receipt = self.state.engine.apply_envelope(env)?;
        events::emit_mutation(self.state, &before, &receipt.kind, &receipt.dirty);
        // §7.8 autosave binding: same posture as the command door — the
        // step HAS applied; a log-write failure reports as the step's
        // failure (durability, not the edit, failed). on_apply_committed
        // only ever produces io_write bodies, so the code is
        // `error.io.write_failed` by construction.
        crate::recovery::on_apply_committed(self.state).map_err(|body| {
            EngineError::new(ErrorCode::IoWriteFailed, autosave_failure_message(&body))
        })?;
        Ok(receipt)
    }

    fn ingest_asset(&mut self, kind: AssetKind, bytes: &[u8]) -> Result<Uuid, EngineError> {
        let id = match kind {
            AssetKind::Samples => self.state.engine.add_samples_asset(bytes.to_vec()),
            AssetKind::RasterImage => self.state.engine.add_raster_asset(bytes.to_vec())?,
            // MECHANICAL companion arms (core door slice 2, Doors 10 +
            // D-ML-1 — RATIFIED by O-SCRIPT, return leg,
            // reports/O-SCRIPT-log.md): §23.5 embedded faces +
            // appname.mask.v1 coverage planes.
            AssetKind::FontFace => self.state.engine.add_font_asset(bytes.to_vec())?,
            AssetKind::MaskPlane => self.state.engine.add_mask_asset(bytes.to_vec())?,
        };
        // §9.6 rule-2 ordering: the asset frame lands in the autosave log
        // BEFORE any referencing envelope (same hook as
        // `appname_asset_create`).
        crate::recovery::on_asset_created(self.state, id, kind, bytes).map_err(|body| {
            EngineError::new(ErrorCode::IoWriteFailed, autosave_failure_message(&body))
        })?;
        Ok(id)
    }
}

/// Extract the human message from an autosave-failure [`ErrorBody`]
/// (always an `error.io.write_failed` payload — see call sites).
fn autosave_failure_message(body: &ErrorBody) -> String {
    body.json
        .to_str()
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.get("message").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "autosave journal write failed".to_string())
}

/// Parsed `options_json`.
fn parse_options(text: Option<String>) -> Result<ReplayOptions, ErrorBody> {
    let Some(text) = text else {
        return Ok(ReplayOptions::default());
    };
    if text.trim().is_empty() {
        return Ok(ReplayOptions::default());
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| invalid_arg(&format!("options_json is not valid JSON: {e}")))?;
    let Value::Object(obj) = value else {
        return Err(invalid_arg("options_json must be a JSON object"));
    };
    let mut opts = ReplayOptions::default();
    for (key, val) in obj {
        match key.as_str() {
            "vars" => match val {
                Value::Object(map) => opts.vars = map,
                other => {
                    return Err(invalid_arg(&format!(
                        "options_json.vars must be an object of pointer -> value, got {other}"
                    )))
                }
            },
            "activeLayerId" => match &val {
                Value::Null => {}
                Value::String(s) => {
                    opts.active_layer = Some(Uuid::parse_str(s).map_err(|_| {
                        invalid_arg(&format!("options_json.activeLayerId is not a UUID: {s:?}"))
                    })?);
                }
                other => {
                    return Err(invalid_arg(&format!(
                        "options_json.activeLayerId must be a UUID string, got {other}"
                    )))
                }
            },
            // Unknown keys reject (export-door precedent: future dials
            // land typed, never silently ignored).
            other => {
                return Err(invalid_arg(&format!(
                    "unknown options_json key {other:?} (supported: vars, activeLayerId)"
                )))
            }
        }
    }
    Ok(opts)
}

/// ABI 1.3 additive block (gate on `appname_version().abi_minor >= 3`).
/// Replay an `appname.action.v1` document (as raw UTF-8 JSON bytes,
/// `contracts/schemas/action.v1.json` shape) against this document
/// through the ordinary per-command pipeline — §26.1 semantics: disabled
/// steps skip; a `stop` step is reported (its author text verbatim) and,
/// when `allowContinue` is false, HALTS the replay; recorded targets
/// re-resolve by captured name, then tree index (rule 3); recorded
/// `new*Id` params re-mint with intra-action remapping; embedded assets
/// re-ingest content-addressed; every step applies under a fresh command
/// id. Each applied step enqueues the ordinary §9.5 events and, when an
/// autosave binding is live, appends + checkpoints like any command.
///
/// `options_json` is NULL/empty for defaults, else a JSON object:
/// `{"vars": {"<RFC 6901 pointer>": value, …}, "activeLayerId": "uuid"}`
/// — `vars` feeds `parameterized` step pointers (an unsupplied pointer
/// keeps its recorded value); `activeLayerId` resolves symbolic
/// `layerId: null` targets (absent = such a step is a step error).
/// Unknown keys reject `error.api.invalid_arg`.
///
/// On success (`APPNAME_OK`) `*out_report` (when non-NULL) receives the
/// replay report as UTF-8 JSON (free with `appname_string_free`):
/// `{"status":"ok"|"halted","steps":[{"index","kind"?,"status":"ok"|
/// "skipped"|"stop"|"error","message"?,"detail"?},…],"haltedAt"?}` — a
/// HALT is a successful call (the action asked to stop). A STEP FAILURE
/// returns the failing step's mapped status and the error out-param
/// carries `{"code","message","messageId":"error.batch.step_failed",
/// "step","report"}` with the full report embedded (steps before the
/// failure HAVE applied — actions are not transactions; §26.2 posture).
/// A malformed action document rejects `APPNAME_ERR_SCHEMA` /
/// `APPNAME_ERR_INVALID_ARG` with nothing applied. A pre-init handle
/// fails its first command step with `error.command.bad_target` (or
/// applies cleanly when the action's first command is `doc.new`).
///
/// # Safety
/// `doc` a handle from this library; `json` must point to `json_len`
/// readable bytes when `json_len > 0`; `options_json` NULL or a valid
/// NUL-terminated C string; `out_report` NULL or writable; `err` NULL or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_apply_action(
    doc: *mut AppnameDoc,
    json: *const u8,
    json_len: usize,
    options_json: *const c_char,
    out_report: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract — `json` covers `json_len`
        // readable bytes; the slice is consumed inside this call only.
        let bytes = unsafe { read_bytes(json, json_len, "json") }?;
        let text = std::str::from_utf8(bytes)
            .map_err(|e| invalid_arg(&format!("action document is not UTF-8: {e}")))?;
        let options = if options_json.is_null() {
            None
        } else {
            // SAFETY: non-NULL checked; NUL termination + validity for the
            // call duration is the documented caller contract.
            Some(unsafe { read_cstr(options_json, "options_json") }?)
        };
        let action = Action::from_json(text).map_err(|e| body_of(&e))?;
        let opts = parse_options(options)?;

        let report_value = registry::with_doc(doc, |state| {
            let mut door = CabiDoor { state };
            let report = replay(&action, &mut door, &opts);
            match report.outcome {
                ReplayOutcome::Failed { at, ref error } => {
                    // Rich §26.2 error payload with the report embedded
                    // (hosts ignore unknown fields, §2.8 mirror).
                    let payload = json!({
                        "code": error.code.as_str(),
                        "message": error.message,
                        "messageId": "error.batch.step_failed",
                        "step": at,
                        "report": report.to_value(),
                    });
                    Err(ErrorBody::new(status_of(error.code), payload.to_string()))
                }
                _ => Ok(report.to_value()),
            }
        })?;
        if !out_report.is_null() {
            // SAFETY: `out_report` non-NULL (checked) and writable per
            // contract; caller frees the minted string.
            unsafe { *out_report = registry::mint_string(report_value.to_string()) };
        }
        Ok(AppnameStatus::AppnameOk)
    })
}
