// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! `appname` — the §25.1 TRUSTED Python tier over the C ABI (CP-0.7 v2).
//!
//! Spec: ES §25.1 (tiers; Python = trusted), §25.4 (the `appname` module
//! surface — this checkpoint implements the headless subset, growth list
//! in the README), §9 (the C ABI it rides), §26 (action invoke +
//! recorder); DD ④ (Apache-2.0 SDK split); plan §2.4 row "Python tier
//! v1" (lane O-SCRIPT).
//!
//! # Trusted by design (ES §25.1 — say so)
//!
//! This tier is TRUSTED BY DESIGN: a Python script running here has the
//! full power of the process (Python itself gives it the filesystem, the
//! network, and `ctypes`). There is NO sandbox at this tier and none is
//! claimed; the sandbox story is the WASM/Lua tier (ES §25.3, CP-1.0).
//! Install-time consent and the capability table in §25.3 govern what a
//! DISTRIBUTED plugin may do; a user running their own script answers to
//! nobody.
//!
//! # One door
//!
//! Every document mutation goes through [`Document::apply`] with an
//! `appname.command.v1` envelope — the same single entry as the CLI and
//! the shell (D-48; preamble rule 5). There are deliberately NO
//! convenience mutators that bypass it. Reads (`info_json`,
//! `history_json`, `read_region`, …) and lifecycle calls are the C ABI's
//! read surfaces 1:1.
//!
//! # Implementation notes
//!
//! * This crate consumes ONLY `appname-cabi`'s `extern "C"` surface (the
//!   same symbols `include/appname.h` declares, linked as a Rust rlib) —
//!   the §25.1 "over the C ABI" discipline, machine-checkable: the only
//!   non-`pyo3` imports below are `appname_cabi::…` symbol names.
//! * FFI-boundary `unsafe` is confined to the cabi call sites, each with
//!   a SAFETY note (the DD ① posture this crate shares with
//!   `appname-cabi`; flagged in the lane log for review).
//! * `AppnameDoc` handles are process-unique cookies; [`Document`] is
//!   `unsendable` (single-thread use per handle) and closes its handle on
//!   drop (double-close is typed and ignored).

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde_json::{json, Value};

use appname_cabi::actions::appname_doc_apply_action;
use appname_cabi::doc::{
    appname_asset_create, appname_doc_apply_command, appname_doc_close, appname_doc_history_json,
    appname_doc_info_json, appname_doc_new, appname_doc_open, appname_doc_poll_events,
    appname_doc_read_layer, appname_doc_read_region, appname_doc_read_selection, appname_doc_redo,
    appname_doc_save, appname_doc_undo, AppnameDoc,
};
use appname_cabi::export::appname_doc_export;
use appname_cabi::mem::{
    appname_buffer_free, appname_buffer_map, appname_buffer_unmap, appname_error_free,
    appname_error_json, appname_string_data, appname_string_free, AppnameError as CabiError,
    AppnameImageBuffer, AppnameMapInfo, AppnameString,
};
use appname_cabi::recovery::{
    appname_autosave_discard, appname_autosave_query, appname_doc_autosave_begin,
    appname_doc_autosave_end, appname_doc_open_recovered,
};
use appname_cabi::{appname_version, AppnameRect, AppnameStatus};

create_exception!(
    appname,
    EngineError,
    PyException,
    "Structured Appname engine error; args[0] is the error JSON ({code, message, ...})."
);

// ---------------------------------------------------------------------------
// FFI plumbing (the only unsafe in this crate).
// ---------------------------------------------------------------------------

/// Consume a minted `AppnameString`: copy out the text, free the handle.
fn take_string(s: *mut AppnameString) -> String {
    if s.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // SAFETY: `s` is a live handle minted by the cabi (caller contract of
    // every out-param below); data/len are valid until the free.
    let p = unsafe { appname_string_data(s, &mut len) };
    let text = if p.is_null() {
        String::new()
    } else {
        // SAFETY: the cabi guarantees `len` readable bytes of UTF-8 at `p`
        // until `appname_string_free`.
        let bytes = unsafe { std::slice::from_raw_parts(p as *const u8, len) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    // SAFETY: `s` is live (minted above) and freed exactly once here.
    unsafe { appname_string_free(s) };
    text
}

/// Consume a populated `AppnameError` into a Python `EngineError`.
fn take_error(e: *mut CabiError) -> PyErr {
    if e.is_null() {
        return EngineError::new_err(
            r#"{"code":"error.api.internal","message":"error out-param not populated"}"#
                .to_string(),
        );
    }
    let mut len = 0usize;
    // SAFETY: `e` is a live error handle; json/len valid until the free.
    let p = unsafe { appname_error_json(e, &mut len) };
    let text = if p.is_null() {
        r#"{"code":"error.api.internal","message":"error payload unavailable"}"#.to_string()
    } else {
        // SAFETY: the cabi guarantees `len` readable bytes at `p` until
        // `appname_error_free`.
        let bytes = unsafe { std::slice::from_raw_parts(p as *const u8, len) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    // SAFETY: `e` is live and freed exactly once here.
    unsafe { appname_error_free(e) };
    EngineError::new_err(text)
}

/// Call an out-string cabi function: OK → the minted string's text.
fn call_string(
    f: impl FnOnce(*mut *mut AppnameString, *mut *mut CabiError) -> AppnameStatus,
) -> PyResult<String> {
    let mut out: *mut AppnameString = std::ptr::null_mut();
    let mut err: *mut CabiError = std::ptr::null_mut();
    let st = f(&mut out, &mut err);
    if st == AppnameStatus::AppnameOk {
        Ok(take_string(out))
    } else {
        Err(take_error(err))
    }
}

fn cstring(text: &str, what: &str) -> PyResult<std::ffi::CString> {
    std::ffi::CString::new(text).map_err(|_| {
        EngineError::new_err(format!(
            r#"{{"code":"error.api.invalid_arg","message":"{what} contains an interior NUL"}}"#
        ))
    })
}

// ---------------------------------------------------------------------------
// Document
// ---------------------------------------------------------------------------

/// A document handle over the C ABI. The ONLY mutation path is
/// [`Document::apply`] (plus undo/redo, which are engine functions, ES
/// §10.3).
#[pyclass(unsendable)]
pub struct Document {
    handle: *mut AppnameDoc,
}

impl Drop for Document {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: the handle came from the cabi; double-close after an
            // explicit `close()` is a TYPED error we deliberately ignore.
            unsafe { appname_doc_close(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

impl Document {
    fn info_value(&self) -> PyResult<Value> {
        let text = self.info_json_impl()?;
        serde_json::from_str(&text).map_err(|e| {
            EngineError::new_err(format!(
                r#"{{"code":"error.api.internal","message":"info JSON unparseable: {e}"}}"#
            ))
        })
    }

    fn info_json_impl(&self) -> PyResult<String> {
        // SAFETY: live handle; out-params are local writable slots.
        call_string(|out, err| unsafe { appname_doc_info_json(self.handle, out, err) })
    }

    fn apply_impl(&mut self, envelope_json: &str) -> PyResult<String> {
        let bytes = envelope_json.as_bytes();
        let mut out: *mut AppnameString = std::ptr::null_mut();
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: live handle; `bytes` valid for the call; out-params are
        // local writable slots. WD-1: these exact bytes reach the engine.
        let _st = unsafe {
            appname_doc_apply_command(self.handle, bytes.as_ptr(), bytes.len(), &mut out, &mut err)
        };
        if !out.is_null() {
            // Receipt (ok) or rejection wire document — either way the
            // CLI-parity JSON is the return value, never an exception
            // (§25.4 apply semantics).
            if !err.is_null() {
                // SAFETY: populated error handle, freed exactly once.
                unsafe { appname_error_free(err) };
            }
            return Ok(take_string(out));
        }
        // Boundary misuse (closed handle, NUL in input, …): raise.
        Err(take_error(err))
    }

    fn asset_create_impl(&mut self, data: &[u8]) -> PyResult<String> {
        // SAFETY: live handle; `data` valid for the call.
        call_string(|out, err| unsafe {
            appname_asset_create(self.handle, data.as_ptr(), data.len(), out, err)
        })
    }

    fn read_buffer(
        &self,
        rect: AppnameRect,
        f: impl FnOnce(
            *mut AppnameDoc,
            AppnameRect,
            *mut *mut AppnameImageBuffer,
            *mut *mut CabiError,
        ) -> AppnameStatus,
    ) -> PyResult<(Vec<u8>, u32, u32, u32)> {
        let mut buf: *mut AppnameImageBuffer = std::ptr::null_mut();
        let mut err: *mut CabiError = std::ptr::null_mut();
        let st = f(self.handle, rect, &mut buf, &mut err);
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        let mut map = AppnameMapInfo {
            data: std::ptr::null_mut(),
            len: 0,
            width: 0,
            height: 0,
            stride: 0,
            pixel_format: 0,
            color_model: 0,
            premultiplied: 0,
        };
        // SAFETY: `buf` is the live buffer minted above; `map` is a local
        // writable struct.
        let st = unsafe { appname_buffer_map(buf, &mut map) };
        if st != AppnameStatus::AppnameOk || map.data.is_null() {
            // SAFETY: live buffer handle, freed exactly once.
            unsafe { appname_buffer_free(buf) };
            return Err(EngineError::new_err(
                r#"{"code":"error.api.internal","message":"buffer map failed"}"#.to_string(),
            ));
        }
        // SAFETY: the map contract guarantees `len` readable bytes at
        // `data` until unmap/free; we copy before both.
        let bytes = unsafe { std::slice::from_raw_parts(map.data, map.len) }.to_vec();
        let (w, h, stride) = (map.width, map.height, map.stride);
        // SAFETY: live buffer, unmapped then freed exactly once.
        unsafe {
            appname_buffer_unmap(buf);
            appname_buffer_free(buf);
        }
        Ok((bytes, w, h, stride))
    }
}

#[pymethods]
impl Document {
    /// Create a document by applying `doc.new` through the single entry.
    /// `background` is `"transparent"` (default), `"white"`, `"black"`,
    /// or `"background_color"` (the doc.new contract's enum).
    #[staticmethod]
    #[pyo3(signature = (w, h, background=None))]
    fn new(w: u32, h: u32, background: Option<String>) -> PyResult<Self> {
        let mut out: *mut AppnameDoc = std::ptr::null_mut();
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: out-params are local writable slots.
        let st = unsafe { appname_doc_new(&mut out, &mut err) };
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        let mut doc = Document { handle: out };
        let envelope = json!({
            "schema": "appname.command.v1",
            "id": uuid::Uuid::new_v4().to_string(),
            "kind": "doc.new",
            "params": {
                "width": w,
                "height": h,
                "background": background.unwrap_or_else(|| "transparent".to_string()),
            },
        })
        .to_string();
        let wire = doc.apply_impl(&envelope)?;
        let parsed: Value = serde_json::from_str(&wire).unwrap_or(Value::Null);
        if parsed.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(EngineError::new_err(wire));
        }
        Ok(doc)
    }

    /// Open a `.appname` package.
    #[staticmethod]
    fn open(path: &str) -> PyResult<Self> {
        let cpath = cstring(path, "path")?;
        let mut out: *mut AppnameDoc = std::ptr::null_mut();
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: `cpath` is a valid NUL-terminated string for the call;
        // out-params are local writable slots.
        let st = unsafe { appname_doc_open(cpath.as_ptr(), &mut out, &mut err) };
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        Ok(Document { handle: out })
    }

    /// Save as a `.appname` package (atomic temp+rename, engine-side).
    fn save(&mut self, path: &str) -> PyResult<()> {
        let cpath = cstring(path, "path")?;
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: live handle; valid C string; writable err slot.
        let st = unsafe { appname_doc_save(self.handle, cpath.as_ptr(), &mut err) };
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        Ok(())
    }

    /// Close the handle now (drop also closes; explicit close makes
    /// use-after-close a TYPED `EngineError` instead of a surprise).
    fn close(&mut self) -> PyResult<()> {
        if !self.handle.is_null() {
            // SAFETY: live handle, closed exactly once here; the field is
            // nulled so drop is a no-op.
            unsafe { appname_doc_close(self.handle) };
            self.handle = std::ptr::null_mut();
        }
        Ok(())
    }

    /// THE single mutation entry: apply one `appname.command.v1` envelope
    /// (JSON text). Returns the CLI-parity wire JSON —
    /// `{"ok":true,"receipt":…}` or `{"ok":false,"id"?,"error":…}` — and
    /// never raises for a command REJECTION (§25.4). Boundary misuse
    /// (closed handle, interior NUL) raises `EngineError`.
    fn apply(&mut self, envelope_json: &str) -> PyResult<String> {
        self.apply_impl(envelope_json)
    }

    /// Undo the newest in-effect command; returns its history-row JSON.
    fn undo(&mut self) -> PyResult<String> {
        // SAFETY: live handle; out-params are local writable slots.
        call_string(|out, err| unsafe { appname_doc_undo(self.handle, out, err) })
    }

    /// Redo the next undone command; returns its history-row JSON.
    fn redo(&mut self) -> PyResult<String> {
        // SAFETY: live handle; out-params are local writable slots.
        call_string(|out, err| unsafe { appname_doc_redo(self.handle, out, err) })
    }

    /// Document summary JSON (the ABI's info v3 shape: layers with
    /// parent/depth/blend/locks/masks, selection bounds, journal state,
    /// profile, guides/grid, and the v3 `paths` array — the §20 doc-path
    /// arena).
    fn info_json(&self) -> PyResult<String> {
        self.info_json_impl()
    }

    /// History rows JSON: `{"length","cursor","entries":[…]}`.
    fn history_json(&self) -> PyResult<String> {
        // SAFETY: live handle; out-params are local writable slots.
        call_string(|out, err| unsafe { appname_doc_history_json(self.handle, out, err) })
    }

    /// Drain queued §9.5 events as one JSON array (`"[]"` when idle).
    fn poll_events(&mut self) -> PyResult<String> {
        // SAFETY: live handle; out-params are local writable slots.
        call_string(|out, err| unsafe { appname_doc_poll_events(self.handle, out, err) })
    }

    /// Read the flattened composite of a document-space rect as tightly
    /// packed straight-alpha sRGB RGBA8: `(bytes, w, h)`. (Copies — the
    /// §25.4 zero-copy memoryview optimization is listed growth.)
    fn read_region<'py>(
        &self,
        py: Python<'py>,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> PyResult<(Bound<'py, PyBytes>, u32, u32)> {
        let (bytes, bw, bh, _stride) =
            self.read_buffer(AppnameRect { x, y, w, h }, |d, r, out, err| {
                // SAFETY: live handle; out-params are writable slots.
                unsafe { appname_doc_read_region(d, r, out, err) }
            })?;
        Ok((PyBytes::new(py, &bytes), bw, bh))
    }

    /// Read ONE layer's self-contained content over a document-space rect
    /// as tightly packed straight-alpha sRGB RGBA8: `(bytes, w, h)` (ABI
    /// 1.4 block — thumbnails, Cut pairing, move echo). Content, not
    /// contribution: the layer's own blend/opacity/fillOpacity are NOT
    /// applied. Raster layers serve their pixels; smart objects serve the
    /// §12.1-evaluated chain result; GROUPS serve their children
    /// composited as-if-isolated onto transparency (the header's stated
    /// election — the group's own props never applied; the root group ==
    /// the whole-document composite; a group-scoped adjustment refuses
    /// typed); adjustment layers refuse typed — see the header's service
    /// matrix.
    fn read_layer<'py>(
        &self,
        py: Python<'py>,
        layer_id: &str,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> PyResult<(Bound<'py, PyBytes>, u32, u32)> {
        let cid = cstring(layer_id, "layer_id")?;
        let (bytes, bw, bh, _stride) =
            self.read_buffer(AppnameRect { x, y, w, h }, |d, r, out, err| {
                // SAFETY: live handle; valid C string for the call
                // duration; out-params are writable slots.
                unsafe { appname_doc_read_layer(d, cid.as_ptr(), r, out, err) }
            })?;
        Ok((PyBytes::new(py, &bytes), bw, bh))
    }

    /// Read selection coverage of a rect as Gray8: `(bytes, w, h)`;
    /// all-zero when no selection is active (ABI 1.2 block).
    fn read_selection<'py>(
        &self,
        py: Python<'py>,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    ) -> PyResult<(Bound<'py, PyBytes>, u32, u32)> {
        let (bytes, bw, bh, _stride) =
            self.read_buffer(AppnameRect { x, y, w, h }, |d, r, out, err| {
                // SAFETY: live handle; out-params are writable slots.
                unsafe { appname_doc_read_selection(d, r, out, err) }
            })?;
        Ok((PyBytes::new(py, &bytes), bw, bh))
    }

    /// Whole-canvas composite convenience: `(bytes, width, height)`.
    fn composite_rgba8<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyBytes>, u32, u32)> {
        let info = self.info_value()?;
        let w = info["width"].as_u64().unwrap_or(0) as u32;
        let h = info["height"].as_u64().unwrap_or(0) as u32;
        self.read_region(py, 0, 0, w, h)
    }

    /// Engine-side export through the ABI's export door (ABI 1.2 block):
    /// `format` is an §8.1 codec id (`"png"`, `"jpeg"`, `"tiff"`,
    /// `"webp"`, `"gif"`, `"bmp"`, `"ora"`); `options_json` e.g.
    /// `'{"quality": 82}'` for jpeg/webp.
    #[pyo3(signature = (path, format, options_json=None))]
    fn export(&mut self, path: &str, format: &str, options_json: Option<String>) -> PyResult<()> {
        let cpath = cstring(path, "path")?;
        let cformat = cstring(format, "format")?;
        let copts = match &options_json {
            Some(o) => Some(cstring(o, "options_json")?),
            None => None,
        };
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: live handle; valid C strings (options possibly NULL per
        // contract); writable err slot.
        let st = unsafe {
            appname_doc_export(
                self.handle,
                cpath.as_ptr(),
                cformat.as_ptr(),
                copts.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                &mut err,
            )
        };
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        Ok(())
    }

    /// Register a binary blob as a document asset (samples or PNG raster,
    /// type-sniffed engine-side); returns the content-addressed asset id
    /// the referencing envelope's `samples`/`assetId` field uses.
    fn asset_create(&mut self, data: &[u8]) -> PyResult<String> {
        self.asset_create_impl(data)
    }

    /// Replay an `appname.action.v1` document through the ABI's
    /// action-invoke door (ABI 1.3 block; §26.1 semantics — see the
    /// header). Returns the replay report JSON
    /// (`{"status":"ok"|"halted","steps":[…]}`). A STEP FAILURE raises
    /// `EngineError` whose payload embeds the partial report
    /// (`{"code","message","messageId":"error.batch.step_failed","step",
    /// "report"}`) — steps before the failure HAVE applied. `options_json`
    /// is `'{"vars": {"<pointer>": value}, "activeLayerId": "uuid"}'`.
    #[pyo3(signature = (action_json, options_json=None))]
    fn apply_action(
        &mut self,
        action_json: &str,
        options_json: Option<String>,
    ) -> PyResult<String> {
        let bytes = action_json.as_bytes();
        let copts = match &options_json {
            Some(o) => Some(cstring(o, "options_json")?),
            None => None,
        };
        // SAFETY: live handle; `bytes` valid for the call; options NULL or
        // a valid C string; out-params are local writable slots.
        call_string(|out, err| unsafe {
            appname_doc_apply_action(
                self.handle,
                bytes.as_ptr(),
                bytes.len(),
                copts.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                out,
                err,
            )
        })
    }

    /// Bind a §7.8 autosave log (ABI 1.2 block): every subsequent
    /// successful mutation appends + checkpoints (per-command fsync).
    fn autosave_begin(&mut self, path: &str) -> PyResult<()> {
        let cpath = cstring(path, "path")?;
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: live handle; valid C string; writable err slot.
        let st = unsafe { appname_doc_autosave_begin(self.handle, cpath.as_ptr(), &mut err) };
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        Ok(())
    }

    /// End the autosave binding (the log FILE is left in place).
    fn autosave_end(&mut self) -> PyResult<()> {
        let mut err: *mut CabiError = std::ptr::null_mut();
        // SAFETY: live handle; writable err slot.
        let st = unsafe { appname_doc_autosave_end(self.handle, &mut err) };
        if st != AppnameStatus::AppnameOk {
            return Err(take_error(err));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Module functions
// ---------------------------------------------------------------------------

/// `(abi_major, abi_minor, engine_major, engine_minor)` — callable before
/// anything else; gate additive-block use on `abi_minor` (N.5 rule 2).
#[pyfunction]
fn version() -> (u32, u32, u32, u32) {
    let v = appname_version();
    (v.abi_major, v.abi_minor, v.engine_major, v.engine_minor)
}

/// Scan an autosave log (§7.8 torn-tail semantics) and report what is
/// recoverable, as JSON.
#[pyfunction]
fn autosave_query(path: &str) -> PyResult<String> {
    let cpath = cstring(path, "path")?;
    // SAFETY: valid C string; out-params are local writable slots.
    call_string(|out, err| unsafe { appname_autosave_query(cpath.as_ptr(), out, err) })
}

/// Recover: load the package, replay the log's journal suffix. Returns
/// `(Document, report_json)`.
#[pyfunction]
fn open_recovered(package_path: &str, log_path: &str) -> PyResult<(Document, String)> {
    let cpkg = cstring(package_path, "package_path")?;
    let clog = cstring(log_path, "log_path")?;
    let mut out: *mut AppnameDoc = std::ptr::null_mut();
    let mut report: *mut AppnameString = std::ptr::null_mut();
    let mut err: *mut CabiError = std::ptr::null_mut();
    // SAFETY: valid C strings; out-params are local writable slots.
    let st = unsafe {
        appname_doc_open_recovered(
            cpkg.as_ptr(),
            clog.as_ptr(),
            &mut out,
            &mut report,
            &mut err,
        )
    };
    if st != AppnameStatus::AppnameOk {
        return Err(take_error(err));
    }
    Ok((Document { handle: out }, take_string(report)))
}

/// Delete an autosave log (the W05 "Discard" arm). Missing file = no-op.
#[pyfunction]
fn autosave_discard(path: &str) -> PyResult<()> {
    let cpath = cstring(path, "path")?;
    // SAFETY: valid C string; NULL err per contract (status alone).
    let st = unsafe { appname_autosave_discard(cpath.as_ptr(), std::ptr::null_mut()) };
    if st != AppnameStatus::AppnameOk {
        return Err(EngineError::new_err(
            r#"{"code":"error.io.write_failed","message":"autosave discard failed"}"#.to_string(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ActionRecorder — the CALLER-SIDE tap (§26.1).
// ---------------------------------------------------------------------------

/// Records the command stream THIS script emits into an
/// `appname.action.v1` document (§26.1: the recorder taps the command
/// stream; in a headless script, the stream is what you apply). Only
/// SUCCESSFUL applies are recorded (§26.1 rule 2's committed-parameters
/// discipline falls out: a rejected envelope never becomes a step).
///
/// Rule-3 capture rides along: when a recorded envelope targets a layer,
/// the layer's current name + info-row index are captured into the
/// step's `recordedTarget` so replay can re-resolve by name, then index.
/// Assets registered through [`ActionRecorder::add_asset`] travel with
/// the action (content-addressed; replay re-ingests them).
///
/// The ENGINE-SIDE journal tap (recording edits made by any front end,
/// not just this script) lives in `appname-script`; an ABI read surface
/// for it is a typed gap recorded in the lane log.
#[pyclass(unsendable)]
struct ActionRecorder {
    name: String,
    steps: Vec<Value>,
    assets: Vec<Value>,
}

#[pymethods]
impl ActionRecorder {
    #[new]
    fn new(name: String) -> Self {
        ActionRecorder {
            name,
            steps: Vec::new(),
            assets: Vec::new(),
        }
    }

    /// Apply an envelope through `doc` AND record it on success. Returns
    /// the same wire JSON as [`Document::apply`].
    fn apply(&mut self, mut doc: PyRefMut<'_, Document>, envelope_json: &str) -> PyResult<String> {
        let wire = doc.apply_impl(envelope_json)?;
        let ok = serde_json::from_str::<Value>(&wire)
            .ok()
            .and_then(|v| v.get("ok").and_then(Value::as_bool))
            .unwrap_or(false);
        if ok {
            if let Ok(env) = serde_json::from_str::<Value>(envelope_json) {
                let mut step = serde_json::Map::new();
                let recorded_target = env
                    .get("target")
                    .and_then(|t| t.get("layerId"))
                    .and_then(Value::as_str)
                    .and_then(|id| capture_target(&doc, id));
                step.insert("command".into(), env);
                if let Some(rt) = recorded_target {
                    step.insert("recordedTarget".into(), rt);
                }
                self.steps.push(Value::Object(step));
            }
        }
        Ok(wire)
    }

    /// Register an asset through `doc` AND capture its bytes for the
    /// action's asset table. Returns the content-addressed id. The kind
    /// tag mirrors the engine's ingress sniff (PNG magic = raster; the
    /// samples blob format otherwise — PNG is the only raster ingress at
    /// this checkpoint).
    fn add_asset(&mut self, mut doc: PyRefMut<'_, Document>, data: &[u8]) -> PyResult<String> {
        let id = doc.asset_create_impl(data)?;
        let kind = if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            "rasterImage"
        } else {
            "samples"
        };
        if !self.assets.iter().any(|a| a["id"] == json!(id)) {
            self.assets.push(json!({
                "id": id,
                "kind": kind,
                "base64": base64_encode(data),
            }));
        }
        Ok(id)
    }

    /// Insert a `stop` step (§26.1: author text, shown verbatim at
    /// replay).
    #[pyo3(signature = (message, allow_continue=true))]
    fn add_stop(&mut self, message: String, allow_continue: bool) {
        self.steps.push(json!({
            "stop": { "message": message, "allowContinue": allow_continue }
        }));
    }

    /// The recorded `appname.action.v1` document as JSON text.
    fn finish(&self) -> String {
        let mut obj = serde_json::Map::new();
        obj.insert("schema".into(), json!("appname.action.v1"));
        obj.insert("id".into(), json!(uuid::Uuid::new_v4().to_string()));
        obj.insert("name".into(), json!(self.name));
        obj.insert("steps".into(), json!(self.steps));
        if !self.assets.is_empty() {
            obj.insert("assets".into(), json!(self.assets));
        }
        Value::Object(obj).to_string()
    }

    /// Steps recorded so far.
    fn __len__(&self) -> usize {
        self.steps.len()
    }
}

/// Rule-3 capture from the live info rows (name + index vocabulary is
/// the info rows', identical to the engine-side recorder's).
fn capture_target(doc: &Document, layer_id: &str) -> Option<Value> {
    let info = doc.info_value().ok()?;
    let row = info["layers"]
        .as_array()?
        .iter()
        .find(|l| l["id"].as_str() == Some(layer_id))?;
    Some(json!({
        "layerName": row["name"],
        "treeIndex": row["index"],
    }))
}

/// Minimal RFC 4648 base64 (encode only — asset carriage). Hand-rolled to
/// keep this Apache-2.0 crate free of new external dependencies.
fn base64_encode(bytes: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[pymodule]
fn appname(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Document>()?;
    m.add_class::<ActionRecorder>()?;
    m.add("EngineError", m.py().get_type::<EngineError>())?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(autosave_query, m)?)?;
    m.add_function(wrap_pyfunction!(open_recovered, m)?)?;
    m.add_function(wrap_pyfunction!(autosave_discard, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
