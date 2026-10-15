// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `libraw-sys` — Appname's OWN LibRaw facade crate (ES §1.4 crate map:
//! `sys/libraw-sys`; the deps/libraw.yaml `bindingCensus` election).
//!
//! Spec: ES §1.4 (FFI islands: `sys/*`, DD-① sanctioned `unsafe`
//! location, strict leaf — depends on no `appname-*` crate); ES §8.3 +
//! Part 43 §43.4 rule 1 (stage 1 = LibRaw behind the fuzzed, sanitized
//! wrapper); ES §48.2 rules 1–2 (LGPL-2.1-elected of the dual grant,
//! dynamic-link-only); deps record `deps/libraw.yaml` (approved-
//! unconsumed → THIS crate is the sanctioned consumption, per its
//! `preAuthorizedFor`).
//!
//! **Posture (the record, restated where it binds code):**
//!
//! * **Runtime dynamic loading ONLY.** No LibRaw source is vendored or
//!   compiled; no build-time link directive exists (build.rs is a
//!   documented no-op). The module is a REPLACEABLE shared library
//!   (LGPL-2.1 §6): [`RawLib::load`] takes an explicit path — the
//!   packaging channel (undecided; the record's `packagingChannel` gap)
//!   supplies it in shipped builds, `APPNAME_LIBRAW_DYLIB` supplies it
//!   for development, and absence is a CLEAN typed condition upstream
//!   (§48.4), never a load failure of the application binary. This
//!   crate performs NO default-name system search: an arbitrary system
//!   libraw would silently break the Part 43 rule-1 doubly-pinned
//!   goldens (the record's posture treats the [NORMATIVE] determinism
//!   pin as controlling; §6 user substitution remains possible via the
//!   explicit path and voids determinism claims, which is exactly what
//!   §6 protects).
//! * **Documented C API only.** Every extern signature below is
//!   hand-declared from the PUBLISHED LibRaw C-API documentation
//!   (www.libraw.org/docs/API-C.html, read first-hand 2026-08-17 for
//!   this slice), not from LibRaw headers or sources — the record's
//!   Part 47 posture ("no Appname implementer reads LibRaw source")
//!   is honored strictly. Consequence, RECORDED as a finding in
//!   `reports/O-IO-log.md`: the documented C API exposes NO accessor
//!   for the raw CFA pixel plane, per-channel black levels, the
//!   orientation flag, or the EXIF block (`libraw_data_t` internals
//!   are not layout-documented), so stage-1 MOSAIC EGRESS is not
//!   implementable at this boundary today. The wrapper in `appname-io`
//!   refuses that hop typed; resolution options are escalated to
//!   O-INT/O-BUILD, not improvised here.
//! * **Version pin.** The record pins LibRaw 0.22.1
//!   ([`PINNED_VERSION`]). Loading does not hard-refuse other versions
//!   (§6 substitution must remain possible); callers surface
//!   [`RawLib::version`] and the Part 43 determinism claims key on the
//!   pin.
//!
//! `unsafe` lives in this crate only (DD ①): the safe API is value- and
//! handle-based; no raw pointer crosses the crate boundary.

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::fmt;
use std::path::Path;

#[cfg(feature = "conformance-stub")]
pub mod stub;

/// The record's pinned LibRaw version (deps/libraw.yaml
/// `candidateVersion`; spec-side §48.1 pin row).
pub const PINNED_VERSION: &str = "0.22.1";

/// The documented `LIBRAW_CHECK_VERSION` encoding
/// (`major << 16 | minor << 8 | patch`) for comparing
/// [`RawLib::version_number`] against a `(major, minor, patch)` triple.
pub const fn make_version(major: i32, minor: i32, patch: i32) -> i32 {
    (major << 16) | (minor << 8) | patch
}

// ---------------------------------------------------------------------------
// Platform dynamic loader (hand-declared; no libc crate — zero deps)
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
mod dl {
    use std::ffi::{c_char, c_int, c_void};

    // POSIX dlfcn interface. On macOS these live in libSystem (linked
    // implicitly); on Linux glibc ≥ 2.34 they are in libc proper and the
    // explicit `dl` link is a no-op alias kept for older toolchains.
    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        pub fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        pub fn dlclose(handle: *mut c_void) -> c_int;
        pub fn dlerror() -> *mut c_char;
    }

    /// `RTLD_NOW`: resolve all symbols at load so a partial/mismatched
    /// module fails at [`super::RawLib::load`] (typed) rather than at
    /// first call (crash). Value 2 on every supported POSIX platform.
    pub const RTLD_NOW: c_int = 2;
}

// W-04 (O-WIN, Windows desk): Windows has no dlfcn. The same four entry
// points, same contract, over kernel32 — hand-declared, so the crate stays
// zero-dependency. `dlopen` takes the explicit path only: the UTF-8 C string
// is widened to UTF-16 and loaded with LOAD_WITH_ALTERED_SEARCH_PATH, so the
// module's own dependencies resolve beside it, never by a default-name
// search. Windows resolves every import at load, which is RTLD_NOW's
// guarantee; the flag is accepted and ignored.
#[cfg(windows)]
mod dl {
    use std::ffi::{c_char, c_int, c_void, CStr};

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> c_int;
    }

    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

    pub const RTLD_NOW: c_int = 2;

    pub unsafe fn dlopen(filename: *const c_char, _flag: c_int) -> *mut c_void {
        // SAFETY: the caller passes a valid NUL-terminated string.
        let Ok(path) = (unsafe { CStr::from_ptr(filename) }).to_str() else {
            return std::ptr::null_mut();
        };
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated UTF-16 and outlives the call.
        unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        }
    }

    pub unsafe fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
        // SAFETY: the caller passes a live module handle and a NUL-terminated name.
        unsafe { GetProcAddress(handle, symbol) }
    }

    pub unsafe fn dlclose(handle: *mut c_void) -> c_int {
        // dlclose's convention: 0 on success.
        // SAFETY: the caller passes a live module handle, closed once.
        if unsafe { FreeLibrary(handle) } != 0 {
            0
        } else {
            -1
        }
    }
}

#[cfg(windows)]
fn last_dl_error() -> String {
    // GetLastError, rendered by the OS's own message table.
    std::io::Error::last_os_error().to_string()
}

#[cfg(not(windows))]
fn last_dl_error() -> String {
    // SAFETY: dlerror returns either null or a NUL-terminated string
    // owned by the loader; it is copied out immediately.
    unsafe {
        let p = dl::dlerror();
        if p.is_null() {
            "unknown dynamic-loader error".to_owned()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

// ---------------------------------------------------------------------------
// The documented C-API subset (deps/libraw.yaml: "hand-declared extern
// bindings for exactly the consumed API subset")
// ---------------------------------------------------------------------------

/// Opaque `libraw_data_t*`. Its layout is deliberately UNKNOWN to this
/// crate (documented-API posture, module doc).
type Handle = *mut c_void;

/// The resolved function table. One field per consumed, DOCUMENTED
/// C-API function; signatures verbatim from API-C.html (module doc).
#[allow(clippy::type_complexity)]
struct Api {
    /// `libraw_data_t *libraw_init(unsigned int flags);`
    init: unsafe extern "C" fn(c_uint) -> Handle,
    /// `void libraw_close(libraw_data_t *);`
    close: unsafe extern "C" fn(Handle),
    /// `int libraw_open_buffer(libraw_data_t*, void *buffer, size_t bufsize)`
    open_buffer: unsafe extern "C" fn(Handle, *const c_void, usize) -> c_int,
    /// `int libraw_unpack(libraw_data_t*);`
    unpack: unsafe extern "C" fn(Handle) -> c_int,
    /// `const char *libraw_strerror(int errorcode);`
    strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    /// `const char* libraw_version()`
    version: unsafe extern "C" fn() -> *const c_char,
    /// `int libraw_versionNumber()`
    version_number: unsafe extern "C" fn() -> c_int,
    /// `int libraw_get_iheight(libraw_data_t *lr);`
    get_iheight: unsafe extern "C" fn(Handle) -> c_int,
    /// `int libraw_get_iwidth(libraw_data_t *lr);`
    get_iwidth: unsafe extern "C" fn(Handle) -> c_int,
    /// `int libraw_get_raw_height(libraw_data_t *lr);`
    get_raw_height: unsafe extern "C" fn(Handle) -> c_int,
    /// `int libraw_get_raw_width(libraw_data_t *lr);`
    get_raw_width: unsafe extern "C" fn(Handle) -> c_int,
    /// `float libraw_get_cam_mul(libraw_data_t *lr, int index);`
    get_cam_mul: unsafe extern "C" fn(Handle, c_int) -> f32,
    /// `float libraw_get_rgb_cam(libraw_data_t *lr, int index1, int index2);`
    get_rgb_cam: unsafe extern "C" fn(Handle, c_int, c_int) -> f32,
    /// `int libraw_get_color_maximum(libraw_data_t *lr);`
    get_color_maximum: unsafe extern "C" fn(Handle) -> c_int,
    /// `int libraw_COLOR(libraw_data_t*, int row, int col);`
    color: unsafe extern "C" fn(Handle, c_int, c_int) -> c_int,
}

/// Why [`RawLib::load`] failed — both arms carry the loader's own text.
#[derive(Debug)]
pub enum LoadError {
    /// `dlopen` failed (path missing, wrong architecture, …).
    Open { path: String, detail: String },
    /// The module loaded but a documented symbol is absent — a
    /// mismatched or non-LibRaw library; refused whole (never a
    /// partial binding).
    SymbolMissing {
        symbol: &'static str,
        detail: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Open { path, detail } => {
                write!(f, "cannot load libraw module {path:?}: {detail}")
            }
            LoadError::SymbolMissing { symbol, detail } => {
                write!(
                    f,
                    "loaded module lacks documented symbol {symbol}: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// `libraw_open_buffer` rejected the bytes (LibRaw could not identify
/// the camera format — the §43.5 `error.codec.raw.unsupported_camera`
/// arm upstream).
#[derive(Debug)]
pub struct OpenError {
    /// The raw (UNDOCUMENTED numeric) LibRaw error code — carried for
    /// diagnostics only; callers must never branch on its value.
    pub code: i32,
    /// `libraw_strerror` text (developer text, never parsed — §1.6).
    pub message: String,
}

/// `libraw_unpack` failed after successful identification (truncated or
/// corrupt sensor data — the §43.5 `Corrupt`/`Truncated` arm upstream).
#[derive(Debug)]
pub struct UnpackError {
    /// Raw LibRaw code (diagnostics only, as [`OpenError::code`]).
    pub code: i32,
    /// `libraw_strerror` text.
    pub message: String,
}

/// A loaded LibRaw module (function table + owned dylib handle).
///
/// Loading is explicit-path only (module doc). One `RawLib` serves any
/// number of sequential [`RawShot`]s; per-shot LibRaw state is isolated
/// per handle, so distinct shots on distinct threads are fine while a
/// single shot is single-threaded (`RawShot` is `!Send`).
pub struct RawLib {
    api: Api,
    /// The `dlopen` handle (None for the conformance stub).
    lib: Option<Handle>,
}

// SAFETY: `Api` is a table of immutable `extern "C"` function pointers;
// `lib` is only ever used by `Drop::drop` (dlclose) and dlclose on a
// still-valid handle is thread-safe. All mutable LibRaw state lives in
// per-shot handles owned by `RawShot`, which is `!Send`/`!Sync`.
unsafe impl Send for RawLib {}
// SAFETY: as above — shared references only read function pointers.
unsafe impl Sync for RawLib {}

impl Drop for RawLib {
    fn drop(&mut self) {
        if let Some(h) = self.lib.take() {
            // SAFETY: `h` came from dlopen in `load` and is closed once.
            unsafe {
                dl::dlclose(h);
            }
        }
    }
}

impl fmt::Debug for RawLib {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawLib")
            .field("loaded", &self.lib.is_some())
            .finish()
    }
}

macro_rules! resolve {
    ($lib:expr, $name:literal, $ty:ty) => {{
        let sym = CString::new($name).expect("static symbol name");
        // SAFETY: `$lib` is a live dlopen handle; dlsym only reads it.
        let p = unsafe { dl::dlsym($lib, sym.as_ptr()) };
        if p.is_null() {
            // SAFETY (drop path): closing the handle we just opened.
            unsafe {
                dl::dlclose($lib);
            }
            return Err(LoadError::SymbolMissing {
                symbol: $name,
                detail: last_dl_error(),
            });
        }
        // SAFETY: transmuting a non-null dlsym result to the DOCUMENTED
        // signature of this exact symbol (module doc: signatures are
        // hand-declared from the published C-API documentation).
        unsafe { std::mem::transmute::<*mut c_void, $ty>(p) }
    }};
}

impl RawLib {
    /// Load a LibRaw module from an EXPLICIT path (never a default-name
    /// system search — module doc). All documented symbols are resolved
    /// eagerly; any gap refuses the whole module.
    pub fn load(path: &Path) -> Result<RawLib, LoadError> {
        let path_str = path.display().to_string();
        let c_path = CString::new(path_str.clone()).map_err(|_| LoadError::Open {
            path: path_str.clone(),
            detail: "path contains an interior NUL byte".to_owned(),
        })?;
        // SAFETY: dlopen with a valid NUL-terminated path; RTLD_NOW so
        // unresolved internals fail here, typed, not later.
        let lib = unsafe { dl::dlopen(c_path.as_ptr(), dl::RTLD_NOW) };
        if lib.is_null() {
            return Err(LoadError::Open {
                path: path_str,
                detail: last_dl_error(),
            });
        }
        let api = Api {
            init: resolve!(lib, "libraw_init", unsafe extern "C" fn(c_uint) -> Handle),
            close: resolve!(lib, "libraw_close", unsafe extern "C" fn(Handle)),
            open_buffer: resolve!(
                lib,
                "libraw_open_buffer",
                unsafe extern "C" fn(Handle, *const c_void, usize) -> c_int
            ),
            unpack: resolve!(lib, "libraw_unpack", unsafe extern "C" fn(Handle) -> c_int),
            strerror: resolve!(
                lib,
                "libraw_strerror",
                unsafe extern "C" fn(c_int) -> *const c_char
            ),
            version: resolve!(
                lib,
                "libraw_version",
                unsafe extern "C" fn() -> *const c_char
            ),
            version_number: resolve!(lib, "libraw_versionNumber", unsafe extern "C" fn() -> c_int),
            get_iheight: resolve!(
                lib,
                "libraw_get_iheight",
                unsafe extern "C" fn(Handle) -> c_int
            ),
            get_iwidth: resolve!(
                lib,
                "libraw_get_iwidth",
                unsafe extern "C" fn(Handle) -> c_int
            ),
            get_raw_height: resolve!(
                lib,
                "libraw_get_raw_height",
                unsafe extern "C" fn(Handle) -> c_int
            ),
            get_raw_width: resolve!(
                lib,
                "libraw_get_raw_width",
                unsafe extern "C" fn(Handle) -> c_int
            ),
            get_cam_mul: resolve!(
                lib,
                "libraw_get_cam_mul",
                unsafe extern "C" fn(Handle, c_int) -> f32
            ),
            get_rgb_cam: resolve!(
                lib,
                "libraw_get_rgb_cam",
                unsafe extern "C" fn(Handle, c_int, c_int) -> f32
            ),
            get_color_maximum: resolve!(
                lib,
                "libraw_get_color_maximum",
                unsafe extern "C" fn(Handle) -> c_int
            ),
            color: resolve!(
                lib,
                "libraw_COLOR",
                unsafe extern "C" fn(Handle, c_int, c_int) -> c_int
            ),
        };
        Ok(RawLib {
            api,
            lib: Some(lib),
        })
    }

    fn strerror_text(&self, code: c_int) -> String {
        // SAFETY: documented call; result is null or a static
        // NUL-terminated string, copied out immediately.
        unsafe {
            let p = (self.api.strerror)(code);
            if p.is_null() {
                format!("libraw error {code}")
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    /// `libraw_version()` text of the LOADED module (compare against
    /// [`PINNED_VERSION`] for determinism claims; mismatch is not a
    /// refusal — module doc).
    pub fn version(&self) -> String {
        // SAFETY: documented zero-argument call; copy-out as above.
        unsafe {
            let p = (self.api.version)();
            if p.is_null() {
                "unknown".to_owned()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    /// `libraw_versionNumber()` (see [`make_version`]).
    pub fn version_number(&self) -> i32 {
        // SAFETY: documented zero-argument call.
        unsafe { (self.api.version_number)() }
    }

    /// Stage-1 open: `libraw_init` + `libraw_open_buffer`. A non-zero
    /// open code means LibRaw could not identify the format.
    ///
    /// The returned [`RawShot`] BORROWS `bytes` (review D1, ruled fix):
    /// real LibRaw wraps the caller's memory in a datastream it may
    /// read again at `unpack`, so the buffer must outlive the shot —
    /// the bound is TYPE-ENFORCED by the shared `'l` lifetime, not
    /// conventional. The escaping pattern no longer compiles:
    ///
    /// ```compile_fail
    /// # fn demo(lib: &libraw_sys::RawLib) {
    /// let shot = {
    ///     let bytes = vec![0u8; 16];
    ///     lib.open_buffer(&bytes) // ERROR: `bytes` dropped while borrowed
    /// };
    /// let _ = shot;
    /// # }
    /// ```
    pub fn open_buffer<'l>(&'l self, bytes: &'l [u8]) -> Result<RawShot<'l>, OpenError> {
        // SAFETY: documented call; 0 flags is the documented default.
        let h = unsafe { (self.api.init)(0) };
        if h.is_null() {
            return Err(OpenError {
                code: 0,
                message: "libraw_init returned NULL".to_owned(),
            });
        }
        let shot = RawShot {
            lib: self,
            handle: h,
            buffer: bytes,
        };
        // SAFETY: handle is live; buffer pointer + length describe the
        // caller's slice, and the slice's liveness for the SHOT'S WHOLE
        // LIFETIME (LibRaw's datastream may re-read it at unpack) is
        // type-enforced: `RawShot<'l>` holds `buffer: &'l [u8]`, so the
        // borrow checker rejects any caller that drops the bytes while
        // the shot is alive (D1 ruled fix; compile_fail doc-test above).
        let code =
            unsafe { (self.api.open_buffer)(shot.handle, bytes.as_ptr().cast(), bytes.len()) };
        if code != 0 {
            return Err(OpenError {
                code,
                message: self.strerror_text(code),
            });
        }
        Ok(shot)
    }
}

/// One opened RAW shot (an isolated `libraw_data_t` instance). Closed
/// on drop. Deliberately `!Send`: a single LibRaw handle is
/// single-threaded per the documented threading model.
///
/// Holds the source buffer borrow for its whole lifetime (D1 ruled
/// fix): LibRaw's datastream may re-read the caller's memory at
/// [`RawShot::unpack`], so the type carries the bound the C library
/// assumes.
pub struct RawShot<'l> {
    lib: &'l RawLib,
    handle: Handle,
    /// Keeps the `libraw_open_buffer` source memory alive for the
    /// handle's lifetime (type-enforced; see [`RawLib::open_buffer`]).
    #[allow(dead_code)]
    buffer: &'l [u8],
}

impl fmt::Debug for RawShot<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawShot").finish_non_exhaustive()
    }
}

impl Drop for RawShot<'_> {
    fn drop(&mut self) {
        // SAFETY: handle came from libraw_init and is closed exactly once.
        unsafe { (self.lib.api.close)(self.handle) }
    }
}

impl RawShot<'_> {
    /// `libraw_unpack` — decode the sensor data. Failure after a
    /// successful open means truncated/corrupt data (§43.5 mapping is
    /// the caller's).
    pub fn unpack(&mut self) -> Result<(), UnpackError> {
        // SAFETY: documented call on a live, opened handle.
        let code = unsafe { (self.lib.api.unpack)(self.handle) };
        if code != 0 {
            return Err(UnpackError {
                code,
                message: self.lib.strerror_text(code),
            });
        }
        Ok(())
    }

    /// Active-area width (`libraw_get_iwidth`).
    pub fn width(&self) -> i32 {
        // SAFETY: documented accessor on a live handle.
        unsafe { (self.lib.api.get_iwidth)(self.handle) }
    }

    /// Active-area height (`libraw_get_iheight`).
    pub fn height(&self) -> i32 {
        // SAFETY: documented accessor on a live handle.
        unsafe { (self.lib.api.get_iheight)(self.handle) }
    }

    /// Full sensor width (`libraw_get_raw_width`).
    pub fn raw_width(&self) -> i32 {
        // SAFETY: documented accessor on a live handle.
        unsafe { (self.lib.api.get_raw_width)(self.handle) }
    }

    /// Full sensor height (`libraw_get_raw_height`).
    pub fn raw_height(&self) -> i32 {
        // SAFETY: documented accessor on a live handle.
        unsafe { (self.lib.api.get_raw_height)(self.handle) }
    }

    /// As-shot white-balance multipliers `(r, g1, b, g2)`
    /// (`libraw_get_cam_mul(0..4)`).
    pub fn cam_mul(&self) -> [f32; 4] {
        // SAFETY: documented accessor, documented index range 0..4.
        unsafe {
            [
                (self.lib.api.get_cam_mul)(self.handle, 0),
                (self.lib.api.get_cam_mul)(self.handle, 1),
                (self.lib.api.get_cam_mul)(self.handle, 2),
                (self.lib.api.get_cam_mul)(self.handle, 3),
            ]
        }
    }

    /// The camera→sRGB(D65) 3×3 (`libraw_get_rgb_cam`, rows×cols of the
    /// documented 3×4's left 3×3).
    pub fn rgb_cam(&self) -> [[f32; 3]; 3] {
        let mut m = [[0.0f32; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                // SAFETY: documented accessor, indices within the
                // documented 3×4 bounds.
                *v = unsafe { (self.lib.api.get_rgb_cam)(self.handle, i as c_int, j as c_int) };
            }
        }
        m
    }

    /// Saturation/white level (`libraw_get_color_maximum`).
    pub fn color_maximum(&self) -> i32 {
        // SAFETY: documented accessor on a live handle.
        unsafe { (self.lib.api.get_color_maximum)(self.handle) }
    }

    /// CFA color code of the photosite at `(row, col)` (`libraw_COLOR`;
    /// documented codes 0 = R, 1 = G, 2 = B, 3 = G2).
    pub fn cfa_color(&self, row: i32, col: i32) -> i32 {
        // SAFETY: documented accessor; row/col are plain ints (LibRaw
        // computes the pattern arithmetically, no bounds concern).
        unsafe { (self.lib.api.color)(self.handle, row, col) }
    }
}

#[cfg(all(test, feature = "conformance-stub"))]
mod tests {
    use super::stub::{stub_bytes, StubSpec};
    use super::*;

    #[test]
    fn load_from_missing_path_is_typed_open_error() {
        let err = RawLib::load(Path::new("/nonexistent/libraw.dylib")).unwrap_err();
        match err {
            LoadError::Open { path, detail } => {
                assert!(path.contains("nonexistent"));
                assert!(!detail.is_empty());
            }
            other => panic!("expected Open error, got {other:?}"),
        }
    }

    #[test]
    fn stub_happy_path_serves_documented_surface() {
        let lib = super::stub::stub_lib();
        let spec = StubSpec::bayer_rggb(8, 6);
        let bytes = stub_bytes(&spec);
        let mut shot = lib.open_buffer(&bytes).expect("open");
        shot.unpack().expect("unpack");
        assert_eq!((shot.width(), shot.height()), (8, 6));
        assert_eq!(shot.cam_mul(), spec.cam_mul);
        assert_eq!(shot.color_maximum(), spec.color_maximum);
        // RGGB tile via libraw_COLOR codes (G2 = 3 at (1,1)).
        assert_eq!(shot.cfa_color(0, 0), 0);
        assert_eq!(shot.cfa_color(0, 1), 1);
        assert_eq!(shot.cfa_color(1, 0), 3);
        assert_eq!(shot.cfa_color(1, 1), 2);
    }

    #[test]
    fn stub_rejects_unidentified_bytes_at_open() {
        let lib = super::stub::stub_lib();
        let err = lib.open_buffer(b"not a raw file at all").unwrap_err();
        assert!(err.code != 0);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn stub_unpack_failure_is_distinct_from_open() {
        let lib = super::stub::stub_lib();
        let spec = StubSpec {
            unpack_result: -7,
            ..StubSpec::bayer_rggb(4, 4)
        };
        // The D1 bound at work: the buffer must be a named binding —
        // a temporary here no longer compiles (see open_buffer's
        // compile_fail doc-test).
        let bytes = stub_bytes(&spec);
        let mut shot = lib.open_buffer(&bytes).expect("open succeeds");
        let err = shot.unpack().unwrap_err();
        assert_eq!(err.code, -7);
    }

    #[test]
    fn stub_version_reports_pin() {
        let lib = super::stub::stub_lib();
        assert_eq!(lib.version(), PINNED_VERSION);
        assert_eq!(lib.version_number(), make_version(0, 22, 1));
    }
}
