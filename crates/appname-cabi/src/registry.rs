// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Handle registry: opaque cookies instead of raw addresses.
//!
//! Spec: ES §9.1 rule 1 (opaque handles, layout never exposed), §9.3
//! (ownership + NULL tolerance); task-pinned hardening: **all pointers
//! validated; double-close / use-after-close are typed errors** — which
//! plain `Box::into_raw` handles cannot deliver (a freed address is
//! undefined behavior to touch, and may alias a later allocation).
//!
//! Mechanism: each handle class (document, string, image buffer, error) owns
//! a `Mutex<HashMap<usize, Body>>`; the C-visible pointer VALUE is a
//! process-unique key minted from one shared monotone counter (keys are
//! never reused and never overlap across classes, so cross-class confusion
//! is detected too). The pointer is never dereferenced. Borrowed interior
//! pointers handed to C (`appname_string_data`, `appname_error_json`,
//! `AppnameMapInfo.data`) point at heap allocations owned by the registry
//! bodies (`CString` / `Vec` — their heap blocks do not move when the map
//! rehashes) and stay valid exactly until the owning handle's free — the
//! §9.3 borrow rule.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use appname_core::Engine;

use crate::mem::{AppnameError, AppnameImageBuffer, AppnameString};
use crate::plugins::{AppnamePluginHost, PluginHostState};
use crate::{doc::AppnameDoc, AppnameStatus};

/// First key value: keeps cookies well away from NULL and small integers so
/// accidental integer-as-pointer misuse stays detectable.
const FIRST_KEY: usize = 0x1000;

static NEXT_KEY: AtomicUsize = AtomicUsize::new(FIRST_KEY);

fn mint_key() -> usize {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Bodies.
// ---------------------------------------------------------------------------

/// One live document handle: the engine (document + journal + history) plus
/// the polled event queue (ES §9.5 payloads; see [`crate::events`]) and the
/// optional §7.8 autosave writer (ABI-growth round; see [`crate::recovery`]).
pub(crate) struct DocState {
    pub engine: Engine,
    pub events: Vec<serde_json::Value>,
    /// Live autosave binding (`appname_doc_autosave_begin` .. `_end`).
    pub autosave: Option<crate::recovery::AutosaveState>,
    /// The CONCRETE text-layout service handle (ABI 1.6 font door). The
    /// engine's `assembly.text` is `Arc<dyn TextLayout>` and the trait
    /// lacks `register_provided_face`; this is the same underlying
    /// `TextLayoutService` (an `Arc` clone, shared FontDb) retained at
    /// construction via `appname_text::register_with_handle`, so
    /// `appname_doc_provide_face` reaches the concrete method without any
    /// core trait change (TEXTVEC's ruled Arc-coercion mechanism). `None`
    /// only on the bare test constructor — the shipped new/open/recovered
    /// paths always set it, and the door answers typed when it is absent.
    pub text_handle: Option<std::sync::Arc<appname_text::TextLayoutService>>,
}

impl DocState {
    pub fn new(engine: Engine) -> DocState {
        DocState {
            engine,
            events: Vec::new(),
            autosave: None,
            text_handle: None,
        }
    }

    /// Retain the concrete text-service handle (font door). Chained onto
    /// [`DocState::new`] by the shipped construction paths.
    pub fn with_text_handle(
        mut self,
        handle: std::sync::Arc<appname_text::TextLayoutService>,
    ) -> DocState {
        self.text_handle = Some(handle);
        self
    }
}

/// Owned UTF-8 text handed to C. Stored NUL-terminated (a convenience the
/// header documents; the length reported to C excludes the terminator, and
/// long-term consumers should use the length per ES §9.3).
pub(crate) struct StringBody {
    pub text: CString,
}

/// Structured error: stable status + the UTF-8 JSON payload
/// (`{"code","message","field"?}` — the engine's wire error shape).
#[derive(Debug)]
pub(crate) struct ErrorBody {
    pub status: AppnameStatus,
    pub json: CString,
}

impl ErrorBody {
    pub fn new(status: AppnameStatus, json: String) -> ErrorBody {
        ErrorBody {
            status,
            json: to_cstring(json),
        }
    }
}

/// Pixel block from a readback symbol. Composite readback delivers
/// straight RGBA8 (`pixel_format` 0 / `color_model` 3); selection readback
/// (ABI-growth round) delivers Gray8 coverage (`pixel_format` 0 /
/// `color_model` 0) — the ES §7.4 encoding enums ride on the body so
/// `appname_buffer_map` reports the delivered encoding truthfully.
pub(crate) struct BufferBody {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Row stride in BYTES (tight: `width * bytes_per_pixel`).
    pub stride: u32,
    /// ES §7.4 pixel encoding (0 = u8 on every current producer).
    pub pixel_format: u8,
    /// ES §7.4 color model (3 = Rgba composite, 0 = Gray selection).
    pub color_model: u8,
    /// Map bookkeeping (`appname_buffer_map` / `_unmap` pairing).
    pub mapped: bool,
}

/// JSON never contains a raw NUL byte (serde escapes control characters), so
/// this conversion is effectively infallible; the fallback keeps a typed
/// path rather than a panic path.
pub(crate) fn to_cstring(s: String) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        CString::new(r#"{"code":"error.api.internal","message":"text contained NUL"}"#)
            .expect("literal has no NUL")
    })
}

// ---------------------------------------------------------------------------
// The four registries.
// ---------------------------------------------------------------------------

type Reg<T> = Mutex<HashMap<usize, T>>;

struct Registries {
    docs: Reg<Arc<Mutex<DocState>>>,
    strings: Reg<StringBody>,
    errors: Reg<ErrorBody>,
    buffers: Reg<BufferBody>,
    /// ABI 1.5: plugin-host handles (app-level; see `crate::plugins`).
    plugin_hosts: Reg<Arc<Mutex<PluginHostState>>>,
}

fn regs() -> &'static Registries {
    static REGS: OnceLock<Registries> = OnceLock::new();
    REGS.get_or_init(|| Registries {
        docs: Mutex::new(HashMap::new()),
        strings: Mutex::new(HashMap::new()),
        errors: Mutex::new(HashMap::new()),
        buffers: Mutex::new(HashMap::new()),
        plugin_hosts: Mutex::new(HashMap::new()),
    })
}

/// A registry mutex can only be poisoned by a panic inside a registry
/// operation (plain map ops); recover the map rather than wedging every
/// future call (the per-document mutex, which guards real engine state, is
/// NOT recovered this way — see [`with_doc`]).
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---- documents ------------------------------------------------------------

pub(crate) fn mint_doc(state: DocState) -> *mut AppnameDoc {
    let key = mint_key();
    lock(&regs().docs).insert(key, Arc::new(Mutex::new(state)));
    key as *mut AppnameDoc
}

/// Run `f` on the live document behind `handle`. Errors are typed:
/// unknown/closed/garbage handle -> `error.api.invalid_arg`; a document
/// poisoned by a prior panic -> `error.api.internal` (its state may be
/// mid-mutation and is no longer trusted; close it).
pub(crate) fn with_doc<R>(
    handle: *const AppnameDoc,
    f: impl FnOnce(&mut DocState) -> Result<R, ErrorBody>,
) -> Result<R, ErrorBody> {
    if handle.is_null() {
        return Err(crate::invalid_arg("doc must not be NULL"));
    }
    let arc =
        {
            let docs = lock(&regs().docs);
            match docs.get(&(handle as usize)) {
                Some(arc) => Arc::clone(arc),
                None => return Err(crate::invalid_arg(
                    "doc is not a live document handle (closed, never opened, or not a document)",
                )),
            }
            // Registry lock drops here; the per-document lock serializes ops.
        };
    let mut guard = arc.lock().map_err(|_| {
        crate::internal(
            "document state was poisoned by a prior panic; the handle is unusable — close it",
        )
    })?;
    f(&mut guard)
}

/// Close (drop) a document. Typed: NULL is an OK no-op (§9.3 free family);
/// an unknown/already-closed handle reports `APPNAME_ERR_INVALID_ARG`.
pub(crate) fn close_doc(handle: *mut AppnameDoc) -> AppnameStatus {
    if handle.is_null() {
        return AppnameStatus::AppnameOk;
    }
    match lock(&regs().docs).remove(&(handle as usize)) {
        // The Arc drops here; if another thread is mid-call it holds a
        // clone, finishes safely, and the state frees when it releases.
        Some(_) => AppnameStatus::AppnameOk,
        None => AppnameStatus::AppnameErrInvalidArg,
    }
}

// ---- plugin hosts (ABI 1.5) -----------------------------------------------

pub(crate) fn mint_plugin_host(state: PluginHostState) -> *mut AppnamePluginHost {
    let key = mint_key();
    lock(&regs().plugin_hosts).insert(key, Arc::new(Mutex::new(state)));
    key as *mut AppnamePluginHost
}

/// Run `f` on the live plugin host behind `handle` (the [`with_doc`]
/// discipline: unknown/freed/garbage handle -> `error.api.invalid_arg`;
/// poisoned -> `error.api.internal`).
pub(crate) fn with_plugin_host<R>(
    handle: *mut AppnamePluginHost,
    f: impl FnOnce(&mut PluginHostState) -> Result<R, ErrorBody>,
) -> Result<R, ErrorBody> {
    if handle.is_null() {
        return Err(crate::invalid_arg("host must not be NULL"));
    }
    let arc = {
        let hosts = lock(&regs().plugin_hosts);
        match hosts.get(&(handle as usize)) {
            Some(arc) => Arc::clone(arc),
            None => {
                return Err(crate::invalid_arg(
                    "host is not a live plugin-host handle (freed, never created, or not a plugin host)",
                ))
            }
        }
    };
    let mut guard = arc.lock().map_err(|_| {
        crate::internal(
            "plugin-host state was poisoned by a prior panic; the handle is unusable — free it",
        )
    })?;
    f(&mut guard)
}

/// Free a plugin host. NULL is an OK no-op; unknown/already-freed reports
/// `APPNAME_ERR_INVALID_ARG` (§9.3 free family).
pub(crate) fn close_plugin_host(handle: *mut AppnamePluginHost) -> AppnameStatus {
    if handle.is_null() {
        return AppnameStatus::AppnameOk;
    }
    match lock(&regs().plugin_hosts).remove(&(handle as usize)) {
        Some(_) => AppnameStatus::AppnameOk,
        None => AppnameStatus::AppnameErrInvalidArg,
    }
}

// ---- strings --------------------------------------------------------------

pub(crate) fn mint_string(text: String) -> *mut AppnameString {
    let key = mint_key();
    lock(&regs().strings).insert(
        key,
        StringBody {
            text: to_cstring(text),
        },
    );
    key as *mut AppnameString
}

/// Borrow the text: `(ptr, len)` with `ptr` NUL-terminated and `len`
/// excluding the terminator. NULL/unknown handle -> `None`.
pub(crate) fn string_bytes(
    handle: *const AppnameString,
) -> Option<(*const std::ffi::c_char, usize)> {
    if handle.is_null() {
        return None;
    }
    let strings = lock(&regs().strings);
    strings.get(&(handle as usize)).map(|b| {
        // The CString's heap block is owned by the registry and does not
        // move on map growth; it lives until `free_string` removes it.
        (b.text.as_ptr(), b.text.as_bytes().len())
    })
}

pub(crate) fn free_string(handle: *mut AppnameString) -> AppnameStatus {
    if handle.is_null() {
        return AppnameStatus::AppnameOk;
    }
    match lock(&regs().strings).remove(&(handle as usize)) {
        Some(_) => AppnameStatus::AppnameOk,
        None => AppnameStatus::AppnameErrInvalidArg,
    }
}

// ---- errors ---------------------------------------------------------------

pub(crate) fn mint_error(body: ErrorBody) -> *mut AppnameError {
    let key = mint_key();
    lock(&regs().errors).insert(key, body);
    key as *mut AppnameError
}

pub(crate) fn error_status(handle: *const AppnameError) -> Option<AppnameStatus> {
    if handle.is_null() {
        return None;
    }
    lock(&regs().errors)
        .get(&(handle as usize))
        .map(|b| b.status)
}

pub(crate) fn error_bytes(handle: *const AppnameError) -> Option<(*const std::ffi::c_char, usize)> {
    if handle.is_null() {
        return None;
    }
    lock(&regs().errors)
        .get(&(handle as usize))
        .map(|b| (b.json.as_ptr(), b.json.as_bytes().len()))
}

pub(crate) fn free_error(handle: *mut AppnameError) -> AppnameStatus {
    if handle.is_null() {
        return AppnameStatus::AppnameOk;
    }
    match lock(&regs().errors).remove(&(handle as usize)) {
        Some(_) => AppnameStatus::AppnameOk,
        None => AppnameStatus::AppnameErrInvalidArg,
    }
}

// ---- image buffers --------------------------------------------------------

pub(crate) fn mint_buffer(body: BufferBody) -> *mut AppnameImageBuffer {
    let key = mint_key();
    lock(&regs().buffers).insert(key, body);
    key as *mut AppnameImageBuffer
}

pub(crate) fn with_buffer<R>(
    handle: *const AppnameImageBuffer,
    f: impl FnOnce(&mut BufferBody) -> R,
) -> Option<R> {
    if handle.is_null() {
        return None;
    }
    lock(&regs().buffers).get_mut(&(handle as usize)).map(f)
}

pub(crate) fn free_buffer(handle: *mut AppnameImageBuffer) -> AppnameStatus {
    if handle.is_null() {
        return AppnameStatus::AppnameOk;
    }
    match lock(&regs().buffers).remove(&(handle as usize)) {
        Some(_) => AppnameStatus::AppnameOk,
        None => AppnameStatus::AppnameErrInvalidArg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_cross_class() {
        let s = mint_string("a".into());
        let e = mint_error(ErrorBody::new(
            AppnameStatus::AppnameErrInvalidArg,
            "{}".into(),
        ));
        assert_ne!(s as usize, e as usize, "one counter across classes");
        // A string cookie is not an error cookie.
        assert!(error_status(s as *const AppnameError).is_none());
        assert!(string_bytes(e as *const AppnameString).is_none());
        assert_eq!(free_string(s), AppnameStatus::AppnameOk);
        assert_eq!(free_error(e), AppnameStatus::AppnameOk);
        // Keys are never reused: freed cookies stay dead.
        assert_eq!(free_string(s), AppnameStatus::AppnameErrInvalidArg);
        assert_eq!(free_error(e), AppnameStatus::AppnameErrInvalidArg);
    }
}
