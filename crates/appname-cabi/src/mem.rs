// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Strings, errors, image buffers: the §9.3 ownership surface.
//!
//! Spec: ES §9.3 (ownership matrix; every free accepts NULL as a no-op;
//! borrowed pointers valid until the owner's free), §9.2 (`AppnameMapInfo`
//! layout + pixel encoding enums mirror ES §7.4). Handles are registry
//! cookies (see [`crate::registry`]): misuse — double-free, use-after-free,
//! a handle of the wrong class — is TYPED (`APPNAME_ERR_INVALID_ARG` or a
//! NULL return), never undefined behavior.

use std::ffi::c_char;

use crate::registry;
use crate::{guarded_status, AppnameStatus};

/// Opaque owned UTF-8 text (JSON on every current producer). Read with
/// `appname_string_data`; free with `appname_string_free`.
#[repr(C)]
pub struct AppnameString {
    _private: [u8; 0],
}

/// Opaque structured error. Read with `appname_error_status` /
/// `appname_error_json`; free with `appname_error_free`. `*err` out-params
/// are untouched when a function returns `APPNAME_OK` (ES §9.3).
#[repr(C)]
pub struct AppnameError {
    _private: [u8; 0],
}

/// Opaque pixel block from a readback symbol (`appname_doc_read_region`,
/// `appname_doc_read_selection`). Map with `appname_buffer_map`; free with
/// `appname_buffer_free`.
#[repr(C)]
pub struct AppnameImageBuffer {
    _private: [u8; 0],
}

/// Mapped view of an `AppnameImageBuffer` (ES §9.2). Pixel encodings are
/// the tile-chunk enums (ES §7.4): `pixel_format` 0=u8 1=u16 2=f16 3=f32;
/// `color_model` 0=Gray 1=GrayA 2=Rgb 3=Rgba 4=Cmyk 5=CmykA 6=Lab 7=LabA.
/// Composite readback delivers `pixel_format` 0, `color_model` 3,
/// `premultiplied` 0 (straight alpha), tight `stride` = `width * 4` bytes;
/// selection readback (ABI 1.2 block) delivers `pixel_format` 0,
/// `color_model` 0 (Gray coverage), tight `stride` = `width` bytes.
/// Consumers branch on the reported fields, never on the producing symbol.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AppnameMapInfo {
    /// First byte of row 0. Valid until `appname_buffer_unmap` /
    /// `appname_buffer_free` of the owning buffer.
    pub data: *mut u8,
    /// Total mapped bytes (= `height * stride`).
    pub len: usize,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row stride in BYTES.
    pub stride: u32,
    /// Pixel encoding (0 = u8 in v1).
    pub pixel_format: u8,
    /// Color model (3 = Rgba in v1).
    pub color_model: u8,
    /// 0 = straight alpha (v1 always), 1 = premultiplied.
    pub premultiplied: u8,
}

// ---------------------------------------------------------------------------
// Strings.
// ---------------------------------------------------------------------------

/// The string's bytes and length. The pointer is borrowed — valid until
/// `appname_string_free(s)` — UTF-8, and NUL-terminated as a convenience
/// (`out_len`, when non-NULL, receives the length EXCLUDING the
/// terminator; portable consumers should use the length, the long-term
/// §9.3 contract). Returns NULL (and zero length) for NULL, freed, or
/// non-string handles — typed, never a crash.
///
/// # Safety
/// `out_len` must be NULL or point to writable storage for one `size_t`.
#[no_mangle]
pub unsafe extern "C" fn appname_string_data(
    s: *const AppnameString,
    out_len: *mut usize,
) -> *const c_char {
    let looked = std::panic::catch_unwind(|| registry::string_bytes(s)).unwrap_or(None);
    let (ptr, len) = looked.unwrap_or((std::ptr::null(), 0));
    if !out_len.is_null() {
        // SAFETY: `out_len` is non-NULL (checked) and points to writable
        // storage for one usize per the documented contract.
        unsafe { *out_len = len };
    }
    ptr
}

/// Free a string. NULL is an OK no-op; double-free / non-string handles
/// return `APPNAME_ERR_INVALID_ARG` (typed — cookies are never reused).
///
/// # Safety
/// `s` must be NULL or a value previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn appname_string_free(s: *mut AppnameString) -> AppnameStatus {
    guarded_status(|| registry::free_string(s))
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// The stable status code stored in an error. Returns
/// `APPNAME_ERR_INVALID_ARG` for NULL/freed/non-error handles — when that
/// answer is ambiguous, `appname_error_json` returning NULL is the
/// discriminator for an invalid handle.
///
/// # Safety
/// `e` must be NULL or a value previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn appname_error_status(e: *const AppnameError) -> AppnameStatus {
    guarded_status(|| registry::error_status(e).unwrap_or(AppnameStatus::AppnameErrInvalidArg))
}

/// The error's UTF-8 JSON payload `{"code":"error.…","message":…,
/// "field"?:…}` — the engine's wire error shape; `code` is the
/// fine-grained taxonomy id (e.g. `error.schema.out_of_range`), stable
/// across locales. Borrowed until `appname_error_free(e)`; NUL-terminated;
/// `out_len` (when non-NULL) receives the length excluding the terminator.
/// Returns NULL for NULL/freed/non-error handles.
///
/// # Safety
/// `out_len` must be NULL or point to writable storage for one `size_t`.
#[no_mangle]
pub unsafe extern "C" fn appname_error_json(
    e: *const AppnameError,
    out_len: *mut usize,
) -> *const c_char {
    let looked = std::panic::catch_unwind(|| registry::error_bytes(e)).unwrap_or(None);
    let (ptr, len) = looked.unwrap_or((std::ptr::null(), 0));
    if !out_len.is_null() {
        // SAFETY: `out_len` is non-NULL (checked) and points to writable
        // storage for one usize per the documented contract.
        unsafe { *out_len = len };
    }
    ptr
}

/// Free an error. NULL is an OK no-op; double-free / non-error handles
/// return `APPNAME_ERR_INVALID_ARG`.
///
/// # Safety
/// `e` must be NULL or a value previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn appname_error_free(e: *mut AppnameError) -> AppnameStatus {
    guarded_status(|| registry::free_error(e))
}

// ---------------------------------------------------------------------------
// Image buffers.
// ---------------------------------------------------------------------------

/// Map a buffer: fill `*out` with the view described on `AppnameMapInfo`.
/// The data pointer stays valid until `appname_buffer_unmap` or
/// `appname_buffer_free` of the same buffer. Mapping an already-mapped
/// buffer refreshes the same view (idempotent). Invalid/NULL handles or a
/// NULL `out` return `APPNAME_ERR_INVALID_ARG` with `*out` untouched.
///
/// # Safety
/// `b` must be NULL or a value previously returned by this library; `out`
/// must be NULL or point to writable storage for one `AppnameMapInfo`.
#[no_mangle]
pub unsafe extern "C" fn appname_buffer_map(
    b: *mut AppnameImageBuffer,
    out: *mut AppnameMapInfo,
) -> AppnameStatus {
    guarded_status(|| {
        if out.is_null() {
            return AppnameStatus::AppnameErrInvalidArg;
        }
        let info = registry::with_buffer(b, |body| {
            body.mapped = true;
            AppnameMapInfo {
                // The Vec's heap block is registry-owned and does not move
                // while the handle lives (§9.3 borrow rule).
                data: body.data.as_mut_ptr(),
                len: body.data.len(),
                width: body.width,
                height: body.height,
                stride: body.stride,
                // ES §7.4 enums, carried per-body: composite readback is
                // u8/Rgba, selection readback u8/Gray. Both straight.
                pixel_format: body.pixel_format,
                color_model: body.color_model,
                premultiplied: 0,
            }
        });
        match info {
            Some(info) => {
                // SAFETY: `out` is non-NULL (checked) and points to writable
                // storage for one AppnameMapInfo per the documented contract.
                unsafe { *out = info };
                AppnameStatus::AppnameOk
            }
            None => AppnameStatus::AppnameErrInvalidArg,
        }
    })
}

/// Unmap a mapped buffer: the pointers from the matching `appname_buffer_map`
/// must not be used afterwards. NULL is an OK no-op; invalid handles return
/// `APPNAME_ERR_INVALID_ARG`; unmapping a never-mapped buffer is an OK
/// no-op.
///
/// # Safety
/// `b` must be NULL or a value previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn appname_buffer_unmap(b: *mut AppnameImageBuffer) -> AppnameStatus {
    guarded_status(|| {
        if b.is_null() {
            return AppnameStatus::AppnameOk;
        }
        match registry::with_buffer(b, |body| body.mapped = false) {
            Some(()) => AppnameStatus::AppnameOk,
            None => AppnameStatus::AppnameErrInvalidArg,
        }
    })
}

/// Free a buffer and its pixels (unmap first if mapped — freeing while
/// mapped is accepted but every mapped pointer dies with the buffer). NULL
/// is an OK no-op; double-free / non-buffer handles return
/// `APPNAME_ERR_INVALID_ARG`.
///
/// # Safety
/// `b` must be NULL or a value previously returned by this library.
#[no_mangle]
pub unsafe extern "C" fn appname_buffer_free(b: *mut AppnameImageBuffer) -> AppnameStatus {
    guarded_status(|| registry::free_buffer(b))
}
