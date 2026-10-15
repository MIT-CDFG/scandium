// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! `appname-cabi` — the Appname stable C ABI, v1 (ABI 1.6).
//!
//! Spec: ES Part 9 (C ABI design rules; §9.1 opaque handles / no-unwind /
//! ownership pairs / status returns, §9.3 ownership, §9.5 event payloads,
//! §9.6 command-as-JSON boundary), ES §25.1 (the ABI as the one extension
//! substrate), ES Appendix N (conformance discipline); HLD C-ABI single door
//! (no privileged back door); DD ① (FFI-boundary `unsafe` confined to this
//! crate, every block justified), DD ④ (Apache-2.0 SDK split); plan §2.2 row
//! "C ABI v1" (lane O-SCRIPT).
//!
//! # Shape of v1
//!
//! * **Handles are opaque cookies, not addresses.** `AppnameDoc*`,
//!   `AppnameString*`, `AppnameImageBuffer*`, `AppnameError*` values are
//!   process-unique tokens resolved through an internal registry; they are
//!   never dereferenced. Consequence (task-pinned hardening over ES §9.3,
//!   which only requires NULL tolerance): *every* pointer is validated, and
//!   lifecycle misuse — double-close, use-after-close, cross-class handle
//!   confusion, garbage pointers — returns a **typed error instead of
//!   undefined behavior**. Keys are minted from a monotone counter and never
//!   reused.
//! * **The single mutation door** (D-48 / WD-1): [`appname_doc_apply_command`]
//!   takes the command document as **raw bytes + length** and hands them to
//!   `Engine::apply_bytes` byte-for-byte — no re-serialization, no
//!   normalization, no pre-parse. Undo/redo are engine functions (ES §10.3),
//!   not commands, and are exposed as calls.
//! * **No unwind crosses the boundary** (ES §9.1 rule 2): every `extern "C"`
//!   body runs under `catch_unwind`; a panic converts to
//!   `APPNAME_ERR_INTERNAL` with a structured JSON payload, and the poisoned
//!   document state stays typed-unusable afterwards.
//! * **Composite readback is the render truth** (one compositing path):
//!   `appname_render::composite_document_with` over the engine's §12.1 op
//!   registry (blend modes, layer/fill opacity, groups, adjustment layers,
//!   smart-filter chains — the §34.3 parity-gated compositor), delivered
//!   as sRGB-encoded straight-alpha RGBA8 via the §6.4 conversion the
//!   export door shares. Per-layer content readback
//!   (`appname_doc_read_layer`, ABI 1.4) serves a single layer's
//!   self-contained content under the same delivery convention. See
//!   [`composite`].
//! * **Events are polled, not called back** (v1): each successful mutation
//!   enqueues `appname.event.treeChanged.v1` (when the layer tree's shape
//!   or props changed — the 1.2 block), `appname.event.dirtyRects.v1`, and
//!   `appname.event.history.v1` payloads (ES §9.5 shapes and the normative
//!   relative order); [`appname_doc_poll_events`] drains the queue.
//!   Callback subscription (`appname_event_subscribe`) is a later additive
//!   block.
//!
//! Additions to this surface land only as new symbols with conformance
//! tests (ES §9.1 / Appendix N; lane rule: never a side door).

#![deny(unsafe_op_in_unsafe_fn)]

pub mod actions;
pub mod composite;
pub mod doc;
pub mod events;
pub mod export;
pub mod fonts;
pub mod mem;
pub mod plugins;
pub mod recovery;
pub mod registry;

use std::ffi::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};

use appname_core::{EngineError, ErrorCode};

use crate::registry::ErrorBody;

// ---------------------------------------------------------------------------
// Status and version (ES §9.2 values; N.2 `abi_sym_enum_values`: literal
// numeric values are frozen — new members only append).
// ---------------------------------------------------------------------------

/// Stable status code returned by every fallible ABI function. The numeric
/// values are frozen (ES §9.2 / Appendix H); rich detail rides in the
/// `AppnameError` out-param as UTF-8 JSON (`appname_error_json`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppnameStatus {
    /// Success. `*err` is untouched (ES §9.3).
    AppnameOk = 0,
    /// Boundary misuse (NULL/invalid/closed handle, bad pointer, non-UTF-8
    /// path), live-state rejections (`error.command.bad_target`,
    /// `error.command.locked`, `error.history.nothing_to_undo`/`_redo`,
    /// `error.color.profile_space_mismatch`), unparseable/oversized command
    /// documents (`error.api.invalid_arg`), and rejected readback geometry.
    AppnameErrInvalidArg = 1,
    /// Package/codec IO failure (`error.io.read_failed` / `error.io.write_failed`).
    AppnameErrIo = 2,
    /// Unknown command kind (`error.command.unknown_kind`), a registered
    /// kind not implemented at this checkpoint
    /// (`error.command.not_implemented`), a reserved capability
    /// (`error.unsupported.erase_to_history`, §40.5), or a package written
    /// by a newer major (`error.unsupported.newer_document`, §51.4 clean
    /// refusal — the file is untouched by the attempt).
    AppnameErrUnsupported = 3,
    /// A panic was caught at the FFI boundary (ES §9.1 rule 2), or internal
    /// state was poisoned by a prior panic. Wire code `error.api.internal`.
    AppnameErrInternal = 4,
    /// The command document failed envelope/param schema validation
    /// (`error.schema.*`, ES §9.6 rule 1 — including the §40.6 nested-op
    /// arm `error.schema.unknown_op_kind`).
    AppnameErrSchema = 5,
    /// Reserved: plugin capability denials (ES §25.3; not emitted by v1).
    AppnameErrPermission = 6,
    /// Reserved: cancelled jobs (ES §9.5; no jobs in v1).
    AppnameErrCancelled = 7,
}

/// ABI + engine version, queryable before any other call (ES §9.1 rule 6,
/// N.5 rule 5). `abi_major` bumps on breaking change, `abi_minor` on
/// additive blocks; consumers gate additive-block usage on `abi_minor`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AppnameVersion {
    /// Breaking ABI changes bump this. This header is series 1.
    pub abi_major: u32,
    /// Additive changes bump this. This header describes ABI 1.8. The
    /// blocks (checkpoint tags ARE releases for Appendix-N purposes —
    /// `v0.3.0-alpha` and `v0.5.0-beta` both shipped `abi_minor` 1 with a
    /// 21-symbol census, so 1.1 is CLOSED at exactly that surface; the
    /// 1.2 block shipped in the CP-0.7 ABI-growth round):
    ///
    /// * **1.1** — exactly `appname_asset_create` (its 21st-symbol landing
    ///   bumped the minor per N.5). Gate on `abi_minor >= 1` (N.5 rule 2).
    /// * **1.2** — the ABI-growth round: `appname_doc_export`,
    ///   `appname_doc_read_selection`, the recovery surface
    ///   (`appname_doc_autosave_begin`/`_end`, `appname_autosave_query`,
    ///   `appname_doc_open_recovered`, `appname_autosave_discard`), plus
    ///   the additive behaviors that shipped with them: the relaxed
    ///   `appname_asset_create` raster arm, the info v2 additive fields,
    ///   and the `treeChanged` event. Gate ALL of it on `abi_minor >= 2`.
    /// * **1.3** — the actions round: exactly
    ///   `appname_doc_apply_action` (§26.1 playback over the ordinary
    ///   per-command pipeline; the Python tier's action invoke). Released
    ///   at `v0.7.0-beta`; gate on `abi_minor >= 3`.
    /// * **1.4** — the consolidated round (O-UI ABI-growth requests 7–8):
    ///   `appname_doc_read_layer` (per-layer content readback over the
    ///   read_region delivery convention), plus the additive behaviors
    ///   shipping with it: the **info v3** `paths` array (N.5 rule 4
    ///   additive JSON — no symbol) and §12.1 parametric serving through
    ///   `appname_doc_read_region`/`appname_doc_export` (the engine's op
    ///   registry now rides the composite calls, CLI parity). Gate on
    ///   `abi_minor >= 4`. OPEN until the next checkpoint tag releases it
    ///   (checkpoint tags are releases — O-INT ruling on record).
    pub abi_minor: u32,
    /// Engine crate major version (informational).
    pub engine_major: u32,
    /// Engine crate minor version (informational).
    pub engine_minor: u32,
}

/// Integer pixel rectangle, document space, top-left origin (ES §9.2).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AppnameRect {
    /// Left edge, document pixels (may be negative).
    pub x: i32,
    /// Top edge, document pixels (may be negative).
    pub y: i32,
    /// Width in pixels. Zero is rejected (`APPNAME_ERR_INVALID_ARG`).
    pub w: u32,
    /// Height in pixels. Zero is rejected (`APPNAME_ERR_INVALID_ARG`).
    pub h: u32,
}

/// The frozen ABI 1.0 block (sorted): the 20 symbols of the original v1
/// surface. Not part of the C surface (skipped by cbindgen).
pub const ABI_1_0_SYMBOLS: &[&str] = &[
    "appname_buffer_free",
    "appname_buffer_map",
    "appname_buffer_unmap",
    "appname_doc_apply_command",
    "appname_doc_close",
    "appname_doc_history_json",
    "appname_doc_info_json",
    "appname_doc_new",
    "appname_doc_open",
    "appname_doc_poll_events",
    "appname_doc_read_region",
    "appname_doc_redo",
    "appname_doc_save",
    "appname_doc_undo",
    "appname_error_free",
    "appname_error_json",
    "appname_error_status",
    "appname_string_data",
    "appname_string_free",
    "appname_version",
];

/// The frozen ABI 1.1 additive block: exactly one symbol (§9.2 asset
/// ingress). CLOSED at this surface — the `v0.3.0-alpha` and
/// `v0.5.0-beta` checkpoint releases both shipped `abi_minor` 1 with the
/// 21-symbol census (checkpoint tags are releases for Appendix-N
/// purposes).
pub const ABI_1_1_SYMBOLS: &[&str] = &["appname_asset_create"];

/// The frozen ABI 1.2 additive block (sorted): the ABI-growth round's 7
/// symbols — export door, selection readback, recovery surface. Gate on
/// `abi_minor >= 2` (N.5 rule 2).
pub const ABI_1_2_SYMBOLS: &[&str] = &[
    "appname_autosave_discard",
    "appname_autosave_query",
    "appname_doc_autosave_begin",
    "appname_doc_autosave_end",
    "appname_doc_export",
    "appname_doc_open_recovered",
    "appname_doc_read_selection",
];

/// The frozen ABI 1.3 additive block: exactly one symbol — the
/// action-invoke door (§26.1 playback; see `actions`). CLOSED at this
/// surface: `v0.7.0-beta` shipped `abi_minor` 3 with the 29-symbol census
/// (checkpoint tags are releases for Appendix-N purposes). Gate on
/// `abi_minor >= 3` (N.5 rule 2).
pub const ABI_1_3_SYMBOLS: &[&str] = &["appname_doc_apply_action"];

/// The frozen ABI 1.4 additive block: exactly one symbol — the per-layer content
/// readback door (O-UI ABI-growth request 7; see `doc`). The info v3
/// `paths` array (request 8) rides the same gate as additive JSON (N.5
/// rule 4 — fields added, none removed or retyped, no symbol). Gate on
/// `abi_minor >= 4` (N.5 rule 2). OPEN until the next checkpoint tag
/// releases it.
pub const ABI_1_4_SYMBOLS: &[&str] = &["appname_doc_read_layer"];

/// The ABI 1.5 additive block (sorted): the plugin-lifecycle door — see
/// `plugins` for the Appendix-N justification and for why there is no
/// invoke symbol in this block. Gate on `abi_minor >= 5` (N.5 rule 2).
/// OPEN until the next checkpoint tag releases it.
pub const ABI_1_5_SYMBOLS: &[&str] = &[
    "appname_plugin_grant",
    "appname_plugin_host_free",
    "appname_plugin_host_new",
    "appname_plugin_host_scan",
    "appname_plugin_revoke",
    "appname_plugin_set_enabled",
];

/// The ABI 1.6 additive block (sorted): the font-provisioning door — the
/// §44.1 host-provided-face surface (`appname_doc_provide_face`) plus the
/// Door-10 enumeration + fonts-stamp readback (`appname_doc_font_faces`).
/// Gate on `abi_minor >= 6` (N.5 rule 2). OPEN until the next checkpoint
/// tag releases it.
pub const ABI_1_6_SYMBOLS: &[&str] = &["appname_doc_font_faces", "appname_doc_provide_face"];

/// The frozen exported-symbol census (sorted; ES Appendix N.2 seed — the
/// conformance suite asserts the built cdylib, the committed header, and
/// its own coverage table against this list). Not part of the C surface
/// (skipped by cbindgen: not representable in C). ABI 1.0 = 20 symbols;
/// the 1.1 additive block (`appname_asset_create`, released at
/// `v0.3.0-alpha`/`v0.5.0-beta`) adds 1; the 1.2 additive block (the
/// ABI-growth round — export door, selection readback, recovery surface)
/// adds 7; the 1.3 additive block (the action-invoke door, released at
/// `v0.7.0-beta`) adds 1; the 1.4 additive block (the per-layer readback
/// door) adds 1; the 1.5 additive block (the plugin-lifecycle door) adds
/// 6; the 1.6 additive block (the font-provisioning door) adds 2 — 38
/// symbols. The unit test
/// `census_is_the_partition_of_the_release_blocks` pins this list as
/// exactly the union of the seven block tables.
pub const V1_SYMBOLS: &[&str] = &[
    "appname_asset_create",
    "appname_autosave_discard",
    "appname_autosave_query",
    "appname_buffer_free",
    "appname_buffer_map",
    "appname_buffer_unmap",
    "appname_doc_apply_action",
    "appname_doc_apply_command",
    "appname_doc_autosave_begin",
    "appname_doc_autosave_end",
    "appname_doc_close",
    "appname_doc_export",
    "appname_doc_font_faces",
    "appname_doc_history_json",
    "appname_doc_info_json",
    "appname_doc_new",
    "appname_doc_open",
    "appname_doc_open_recovered",
    "appname_doc_poll_events",
    "appname_doc_provide_face",
    "appname_doc_read_layer",
    "appname_doc_read_region",
    "appname_doc_read_selection",
    "appname_doc_redo",
    "appname_doc_save",
    "appname_doc_undo",
    "appname_error_free",
    "appname_error_json",
    "appname_error_status",
    "appname_plugin_grant",
    "appname_plugin_host_free",
    "appname_plugin_host_new",
    "appname_plugin_host_scan",
    "appname_plugin_revoke",
    "appname_plugin_set_enabled",
    "appname_string_data",
    "appname_string_free",
    "appname_version",
];

/// The current ABI + engine version. Callable before any other function and
/// from any thread; never fails — on an internal panic (unreachable in
/// practice) it returns an all-zero version rather than unwinding (ES §9.1
/// rule 2 is blanket).
#[no_mangle]
pub extern "C" fn appname_version() -> AppnameVersion {
    catch_unwind(|| {
        // Engine version: this crate versions in lockstep with the workspace.
        let mut parts = env!("CARGO_PKG_VERSION").split('.');
        let mut next = || {
            parts
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(0)
        };
        AppnameVersion {
            abi_major: 1,
            abi_minor: 8,
            engine_major: next(),
            engine_minor: next(),
        }
    })
    .unwrap_or(AppnameVersion {
        abi_major: 0,
        abi_minor: 0,
        engine_major: 0,
        engine_minor: 0,
    })
}

// ---------------------------------------------------------------------------
// Error mapping (engine taxonomy -> stable status; the fine-grained wire
// code always rides unmodified in the JSON payload).
// ---------------------------------------------------------------------------

/// `EngineError` code -> `AppnameStatus` (ES §9.6 pins the schema /
/// unknown-kind / bad-target families; the rest follow the same families).
pub(crate) fn status_of(code: ErrorCode) -> AppnameStatus {
    match code {
        // [O-CORE CP-0.7 fix-round mechanical arm (M9: the §40.6
        // `error.schema.unknown_op_kind` taxonomy addition), RATIFIED by
        // O-SCRIPT consolidated round (reports/O-SCRIPT-log.md): the §40.x
        // wire table pins `error.schema.unknown_op_kind` →
        // `APPNAME_ERR_SCHEMA` EXPLICITLY ("nested opKind/filterId not
        // registered", §12.1/§16.1/§17.1) — a schema-family wire code
        // buckets with the schema family.]
        // [O-TOOLS CP-1.0 retouch-row CROSS-FENCE mechanical arm, FLAGGED
        // for O-SCRIPT ratification (same lineage as every arm above):
        // §45.1 pins the family VERBATIM — "a payload carrying both fails
        // stage-3 validation (`APPNAME_ERR_SCHEMA`,
        // `error.schema.exclusive_params`)" — a schema-family wire code
        // bucketing with the schema family, the unknown_op_kind precedent.]
        ErrorCode::WrongSchema
        | ErrorCode::MissingField
        | ErrorCode::OutOfRange
        | ErrorCode::DuplicateId
        | ErrorCode::ExclusiveParams
        | ErrorCode::UnknownOpKind => AppnameStatus::AppnameErrSchema,
        // [O-CORE wave-2 mechanical arms, RATIFIED by O-SCRIPT wave-2b
        // (reports/O-SCRIPT-log.md)]: the stroke-kind taxonomy extension
        // added two codes; the eraser contract itself names
        // APPNAME_ERR_UNSUPPORTED for erase_to_history, and Locked buckets
        // with the §9.6 stage-5 target-validation family like BadTarget
        // (the worked example maps `error.command.locked` to
        // APPNAME_ERR_INVALID_ARG explicitly).
        // [O-CORE CP-1.0 door-slice mechanical arm, RATIFIED by O-SCRIPT
        // consolidated round (reports/O-SCRIPT-log.md): UnsupportedNewerDocument
        // is the QA-F9 closure — §51.4 names the `APPNAME_ERR_UNSUPPORTED`
        // family for `error.unsupported.newer_document` explicitly (Flow D
        // clean refusal; Appendix H), and the F8 file-untouched posture is
        // the engine's, unchanged by this mapping.]
        // [O-CORE CP-1.0 door-slice-3 mechanical arm, RATIFIED by O-SCRIPT
        // (this round, reports/O-SCRIPT-log.md): §14.3 spot-channel params
        // are reserved Post-1.0 and the contract names
        // `APPNAME_ERR_UNSUPPORTED` for `error.unsupported.spot_channels`
        // verbatim — the erase_to_history / newer_document
        // `error.unsupported.*` precedent, unchanged.]
        ErrorCode::UnknownKind
        | ErrorCode::NotImplemented
        | ErrorCode::EraseToHistoryUnsupported
        | ErrorCode::SpotChannelsUnsupported
        | ErrorCode::UnsupportedNewerDocument => AppnameStatus::AppnameErrUnsupported,
        // [O-CORE CP-0.5 mechanical arms (waves 1-2: target_kind_mismatch +
        // selection_required with fill/edit.*; target_in_use with mask.*;
        // profile_class/_malformed/_too_large with the color family),
        // RATIFIED by O-SCRIPT wave 3 (reports/O-SCRIPT-log.md): all six
        // are stage-5/6 target- and live-state-validation rejects — the
        // §9.6 worked-example family that maps to APPNAME_ERR_INVALID_ARG —
        // and the three error.color.* codes bucket with the
        // profile_space_mismatch precedent ratified at wave 1.]
        // [O-CORE door-slice-2 mechanical arms, RATIFIED by O-SCRIPT
        // (return leg, reports/O-SCRIPT-log.md; item E, the retouch
        // taxonomy): the §45.8 rows name
        // APPNAME_ERR_INVALID_ARG explicitly for no_source_found (§45.8
        // failure step 3), pattern_missing (§45.8 edge table), and
        // sample_region_empty (§45.3 rule 2); sample_region_missing's
        // live-state missing-channel case is the same worked-example
        // family (its params-absent sibling is generic schema traffic and
        // never reaches this code); insufficient_samples is a WARN-surface
        // id per §45.3 rule 1 (the command SUCCEEDS — the variant exists
        // for taxonomy registration; the receipt-warn seam is a separate
        // routed filing) — bucketed with its family for totality.]
        // [O-CORE CP-1.0 door-slice-3 mechanical arm, RATIFIED by O-SCRIPT
        // (this round, reports/O-SCRIPT-log.md): SizeMismatch is the §40.4
        // cross-document size gate ("MUST be open with identical pixel
        // dimensions" — channel.duplicate toDocument, channel.apply_image/
        // calculations source docId), wire `error.command.size_mismatch`:
        // a stage-5/6 target/live-state validation reject, the §9.6
        // worked-example family → APPNAME_ERR_INVALID_ARG, exactly the
        // BadTarget/TargetKindMismatch precedent.]
        ErrorCode::BadTarget
        | ErrorCode::Locked
        | ErrorCode::SizeMismatch
        | ErrorCode::TargetKindMismatch
        | ErrorCode::SelectionRequired
        | ErrorCode::TargetInUse
        | ErrorCode::RetouchNoSourceFound
        | ErrorCode::RetouchPatternMissing
        | ErrorCode::RetouchInsufficientSamples
        | ErrorCode::RetouchSampleRegionEmpty
        | ErrorCode::RetouchSampleRegionMissing
        | ErrorCode::ProfileSpaceMismatch
        | ErrorCode::ProfileClass
        | ErrorCode::ProfileMalformed
        | ErrorCode::ProfileTooLarge
        | ErrorCode::NothingToUndo
        | ErrorCode::NothingToRedo
        | ErrorCode::InvalidArg => AppnameStatus::AppnameErrInvalidArg,
        ErrorCode::IoReadFailed | ErrorCode::IoWriteFailed => AppnameStatus::AppnameErrIo,
    }
}

pub(crate) fn body_of(e: &EngineError) -> ErrorBody {
    ErrorBody::new(
        status_of(e.code),
        serde_json::to_string(e).unwrap_or_else(|_| {
            // Unreachable in practice (EngineError always serializes); keep a
            // structured fallback rather than a panic path.
            r#"{"code":"error.api.internal","message":"error serialization failed"}"#.to_string()
        }),
    )
}

/// Boundary-misuse error body (`error.api.invalid_arg`, ES §40.1 family).
pub(crate) fn invalid_arg(message: &str) -> ErrorBody {
    ErrorBody::new(
        AppnameStatus::AppnameErrInvalidArg,
        serde_json::to_string(&serde_json::json!({
            "code": "error.api.invalid_arg",
            "message": message,
        }))
        .expect("literal object serializes"),
    )
}

/// Panic / poisoned-state error body. Wire code `error.api.internal` —
/// minted by this crate (not in the v5.5 message-id set; on the upstream
/// mint list, lane-log divergence entry, same OQ-1 route as
/// `error.command.not_implemented`).
pub(crate) fn internal(message: &str) -> ErrorBody {
    ErrorBody::new(
        AppnameStatus::AppnameErrInternal,
        serde_json::to_string(&serde_json::json!({
            "code": "error.api.internal",
            "message": message,
        }))
        .expect("literal object serializes"),
    )
}

// ---------------------------------------------------------------------------
// FFI guard: no unwind crosses the boundary (ES §9.1 rule 2).
// ---------------------------------------------------------------------------

/// Run `f`; report failure through `err` (when non-NULL) and the returned
/// status. Panics become `APPNAME_ERR_INTERNAL` (never an unwind across the
/// boundary).
pub(crate) fn guarded(
    err: *mut *mut mem::AppnameError,
    f: impl FnOnce() -> Result<AppnameStatus, ErrorBody>,
) -> AppnameStatus {
    let outcome = catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let what = panic_text(&panic);
        Err(internal(&format!(
            "panic caught at the FFI boundary: {what}"
        )))
    });
    match outcome {
        Ok(status) => status,
        Err(body) => {
            let status = body.status;
            if !err.is_null() {
                let handle = registry::mint_error(body);
                // SAFETY: `err` is non-NULL and, per every function's
                // documented contract, points to writable storage for one
                // `AppnameError*`; we store the freshly minted handle the
                // caller must release with `appname_error_free`.
                unsafe { *err = handle };
            }
            status
        }
    }
}

/// Guard for functions with no `err` channel: the status alone reports.
pub(crate) fn guarded_status(f: impl FnOnce() -> AppnameStatus) -> AppnameStatus {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(AppnameStatus::AppnameErrInternal)
}

pub(crate) fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> &str {
    if let Some(s) = panic.downcast_ref::<&str>() {
        s
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s
    } else {
        "opaque panic payload"
    }
}

// ---------------------------------------------------------------------------
// C-string / byte-slice ingestion helpers (the only raw-pointer reads).
// ---------------------------------------------------------------------------

/// Read a NUL-terminated C string as UTF-8.
///
/// # Safety
/// If non-NULL, `s` must point to a NUL-terminated string valid for the
/// duration of the call (the caller-side contract of every `const char*`
/// parameter in the header).
pub(crate) unsafe fn read_cstr(s: *const c_char, what: &str) -> Result<String, ErrorBody> {
    if s.is_null() {
        return Err(invalid_arg(&format!("{what} must not be NULL")));
    }
    // SAFETY: non-NULL checked above; NUL termination + validity for the
    // call duration is the documented caller contract for this parameter.
    let c = unsafe { std::ffi::CStr::from_ptr(s) };
    c.to_str()
        .map(str::to_owned)
        .map_err(|_| invalid_arg(&format!("{what} is not valid UTF-8")))
}

/// View `(ptr, len)` as a byte slice without copying or altering it (WD-1:
/// these exact bytes reach the engine).
///
/// # Safety
/// If `len > 0`, `ptr` must point to `len` readable bytes valid for the
/// duration of the call.
pub(crate) unsafe fn read_bytes<'a>(
    ptr: *const u8,
    len: usize,
    what: &str,
) -> Result<&'a [u8], ErrorBody> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(invalid_arg(&format!(
            "{what} must not be NULL when its length is non-zero"
        )));
    }
    // SAFETY: non-NULL checked above; the caller contract guarantees `len`
    // readable bytes for the duration of the call. The lifetime is confined
    // to the extern-fn body that immediately consumes the slice.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_are_the_spec_constants() {
        // N.2 `abi_sym_enum_values` seed: literal values frozen.
        assert_eq!(AppnameStatus::AppnameOk as i32, 0);
        assert_eq!(AppnameStatus::AppnameErrInvalidArg as i32, 1);
        assert_eq!(AppnameStatus::AppnameErrIo as i32, 2);
        assert_eq!(AppnameStatus::AppnameErrUnsupported as i32, 3);
        assert_eq!(AppnameStatus::AppnameErrInternal as i32, 4);
        assert_eq!(AppnameStatus::AppnameErrSchema as i32, 5);
        assert_eq!(AppnameStatus::AppnameErrPermission as i32, 6);
        assert_eq!(AppnameStatus::AppnameErrCancelled as i32, 7);
    }

    #[test]
    fn version_reports_abi_1_7() {
        // 1.1 closed at appname_asset_create (released `v0.3.0-alpha`/
        // `v0.5.0-beta`); 1.2 closed at the ABI-growth round's 7 symbols;
        // 1.3 closed at `appname_doc_apply_action` (released
        // `v0.7.0-beta`); 1.4 = `appname_doc_read_layer` (consolidated
        // round); the plugin-lifecycle door is the 1.5 additive block —
        // N.5 requires the minor bump alongside it.
        let v = appname_version();
        // 1.7 is the §19.1 block: `doc.crop` honours `deletePixels:false`
        // by setting `crop_rect`, and info publishes `imageRect`. It is a
        // COMPATIBILITY marker for what this library honours — NOT a
        // capability catalogue for which kinds exist.
        assert_eq!((v.abi_major, v.abi_minor), (1, 8));
        // E-11 (operator WORD 16 (3), 2026-09-18) moved the workspace to
        // 1.0.2 and this assertion still read the literal `(0, 1)`, so the
        // bump turned the gate's `test` step red on main: "workspace 0.1.x
        // line", left (1, 0), right (0, 1). The literal was never the
        // subject. What this line is for is that `appname_version()` REPORTS
        // the crate's own version rather than a hand-kept constant, so it
        // derives both sides from CARGO_PKG_VERSION and cannot go stale at
        // the next bump. The ABI pair above stays a literal on purpose: that
        // one IS a compatibility marker and must only move deliberately.
        let mut want = env!("CARGO_PKG_VERSION").split('.');
        let want_major: u32 = want.next().unwrap().parse().unwrap();
        let want_minor: u32 = want.next().unwrap().parse().unwrap();
        assert_eq!(
            (v.engine_major, v.engine_minor),
            (want_major, want_minor),
            "appname_version() must report this crate's own version"
        );
    }

    #[test]
    fn census_is_the_partition_of_the_release_blocks() {
        // Appendix-N block accounting: 20 (1.0) + 1 (1.1) + 7 (1.2) +
        // 1 (1.3) + 1 (1.4) + 6 (1.5) + 2 (1.6) = 38, disjoint, and their
        // sorted union IS the frozen census.
        assert_eq!(ABI_1_0_SYMBOLS.len(), 20);
        assert_eq!(ABI_1_1_SYMBOLS.len(), 1);
        assert_eq!(ABI_1_2_SYMBOLS.len(), 7);
        assert_eq!(ABI_1_3_SYMBOLS.len(), 1);
        assert_eq!(ABI_1_4_SYMBOLS.len(), 1);
        assert_eq!(ABI_1_5_SYMBOLS.len(), 6);
        assert_eq!(ABI_1_6_SYMBOLS.len(), 2);
        let mut union: Vec<&str> = ABI_1_0_SYMBOLS
            .iter()
            .chain(ABI_1_1_SYMBOLS)
            .chain(ABI_1_2_SYMBOLS)
            .chain(ABI_1_3_SYMBOLS)
            .chain(ABI_1_4_SYMBOLS)
            .chain(ABI_1_5_SYMBOLS)
            .chain(ABI_1_6_SYMBOLS)
            .copied()
            .collect();
        union.sort();
        union.dedup();
        assert_eq!(union.len(), 38, "blocks are disjoint");
        assert_eq!(
            union, V1_SYMBOLS,
            "census == union of the 1.0/1.1/1.2/1.3/1.4/1.5/1.6 release blocks"
        );
    }

    #[test]
    fn panic_in_guarded_body_becomes_typed_internal() {
        // ES §9.1 rule 2: no unwind crosses the boundary. Reached here by
        // direct injection (the shipped surface has no deliberate panic
        // path to trigger from outside — that is the point).
        let mut err: *mut mem::AppnameError = std::ptr::null_mut();
        let status = guarded(&mut err, || panic!("boom at the boundary"));
        assert_eq!(status, AppnameStatus::AppnameErrInternal);
        assert!(!err.is_null(), "panic populates the error out-param");
        let json = registry::error_bytes(err).expect("live error");
        // SAFETY (test): pointer/len come straight from the registry.
        let text = unsafe {
            std::str::from_utf8(std::slice::from_raw_parts(json.0 as *const u8, json.1)).unwrap()
        };
        assert!(text.contains("error.api.internal"), "{text}");
        assert!(text.contains("boom at the boundary"), "{text}");
        assert_eq!(registry::free_error(err), AppnameStatus::AppnameOk);

        // err == NULL: the status alone reports; nothing is minted.
        let status = guarded(std::ptr::null_mut(), || panic!("no err channel"));
        assert_eq!(status, AppnameStatus::AppnameErrInternal);
    }

    #[test]
    fn status_mapping_covers_the_taxonomy() {
        use AppnameStatus::*;
        let mut seen = [false; 31];
        for (code, status) in [
            (ErrorCode::WrongSchema, AppnameErrSchema),
            (ErrorCode::MissingField, AppnameErrSchema),
            (ErrorCode::OutOfRange, AppnameErrSchema),
            (ErrorCode::DuplicateId, AppnameErrSchema),
            // Consolidated-round ratification of the O-CORE mechanical arms
            // (lane log): unknown_op_kind is pinned SCHEMA by the §40.x wire
            // table; newer_document is pinned UNSUPPORTED by §51.4.
            (ErrorCode::UnknownOpKind, AppnameErrSchema),
            (ErrorCode::UnsupportedNewerDocument, AppnameErrUnsupported),
            (ErrorCode::UnknownKind, AppnameErrUnsupported),
            (ErrorCode::NotImplemented, AppnameErrUnsupported),
            // Wave-2b ratification of the O-CORE mechanical arms (lane log):
            // the eraser contract names UNSUPPORTED for erase_to_history;
            // Locked buckets with §9.6 stage-5 target validation.
            (ErrorCode::EraseToHistoryUnsupported, AppnameErrUnsupported),
            (ErrorCode::Locked, AppnameErrInvalidArg),
            // Wave-3 ratification of the CP-0.5 mechanical arms (lane log):
            // stage-5/6 target/live-state family, INVALID_ARG per the §9.6
            // worked example; error.color.* per the wave-1 precedent.
            (ErrorCode::TargetKindMismatch, AppnameErrInvalidArg),
            (ErrorCode::SelectionRequired, AppnameErrInvalidArg),
            (ErrorCode::TargetInUse, AppnameErrInvalidArg),
            (ErrorCode::ProfileClass, AppnameErrInvalidArg),
            (ErrorCode::ProfileMalformed, AppnameErrInvalidArg),
            (ErrorCode::ProfileTooLarge, AppnameErrInvalidArg),
            (ErrorCode::BadTarget, AppnameErrInvalidArg),
            (ErrorCode::ProfileSpaceMismatch, AppnameErrInvalidArg),
            (ErrorCode::NothingToUndo, AppnameErrInvalidArg),
            (ErrorCode::NothingToRedo, AppnameErrInvalidArg),
            (ErrorCode::IoReadFailed, AppnameErrIo),
            (ErrorCode::IoWriteFailed, AppnameErrIo),
            (ErrorCode::InvalidArg, AppnameErrInvalidArg),
            // Door-slice-2 mechanical arms (item E, RATIFIED by O-SCRIPT,
            // return leg): §45.8 names APPNAME_ERR_INVALID_ARG for the
            // retouch family explicitly (no_source_found step 3;
            // pattern_missing edge table; sample_region_empty §45.3 rule
            // 2; sample_region_missing's live-state channel case);
            // insufficient_samples is a WARN-surface id (§45.3 rule 1 —
            // the command succeeds) bucketed with its family.
            (ErrorCode::RetouchNoSourceFound, AppnameErrInvalidArg),
            (ErrorCode::RetouchPatternMissing, AppnameErrInvalidArg),
            (ErrorCode::RetouchInsufficientSamples, AppnameErrInvalidArg),
            (ErrorCode::RetouchSampleRegionEmpty, AppnameErrInvalidArg),
            (ErrorCode::RetouchSampleRegionMissing, AppnameErrInvalidArg),
            // Door-slice-3 mechanical arms (RATIFIED by O-SCRIPT, this
            // round): size_mismatch is the §40.4 cross-document size gate —
            // stage-5/6 target/live-state family, INVALID_ARG per the §9.6
            // worked example; the §14.3 spot-channel contract names
            // APPNAME_ERR_UNSUPPORTED verbatim.
            (ErrorCode::SizeMismatch, AppnameErrInvalidArg),
            (ErrorCode::SpotChannelsUnsupported, AppnameErrUnsupported),
            // Retouch-row CROSS-FENCE mechanical arm (O-TOOLS, FLAGGED
            // for O-SCRIPT ratification): §45.1 pins the family verbatim
            // ("fails stage-3 validation (APPNAME_ERR_SCHEMA,
            // error.schema.exclusive_params)") — schema family, the
            // unknown_op_kind precedent.
            (ErrorCode::ExclusiveParams, AppnameErrSchema),
        ] {
            assert_eq!(status_of(code), status, "{}", code.as_str());
            seen[ordinal(code)] = true;
        }
        assert!(
            seen.iter().all(|s| *s),
            "every taxonomy code appears in the table exactly once"
        );

        // Self-enforcing coverage (O-INT MINOR-3, consolidated fix
        // round): the match below carries NO wildcard arm, so a 31st
        // `ErrorCode` variant refuses to compile HERE — and the
        // all-ordinals-seen assert above then forces its row into the
        // table. The taxonomy is 31 codes (the CP-1.0 retouch row added
        // exclusive_params — the tripwire fired exactly as designed).
        fn ordinal(code: ErrorCode) -> usize {
            use ErrorCode::*;
            match code {
                WrongSchema => 0,
                MissingField => 1,
                OutOfRange => 2,
                DuplicateId => 3,
                UnknownKind => 4,
                NotImplemented => 5,
                BadTarget => 6,
                Locked => 7,
                TargetKindMismatch => 8,
                SelectionRequired => 9,
                TargetInUse => 10,
                UnknownOpKind => 11,
                EraseToHistoryUnsupported => 12,
                UnsupportedNewerDocument => 13,
                RetouchNoSourceFound => 23,
                RetouchPatternMissing => 24,
                RetouchInsufficientSamples => 25,
                RetouchSampleRegionEmpty => 26,
                RetouchSampleRegionMissing => 27,
                SizeMismatch => 28,
                SpotChannelsUnsupported => 29,
                ExclusiveParams => 30,
                ProfileSpaceMismatch => 14,
                ProfileClass => 15,
                ProfileMalformed => 16,
                ProfileTooLarge => 17,
                NothingToUndo => 18,
                NothingToRedo => 19,
                IoReadFailed => 20,
                IoWriteFailed => 21,
                InvalidArg => 22,
            }
        }
    }
}
