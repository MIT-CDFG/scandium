// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! ABI conformance suite seed (ES Appendix N discipline, CP-0.3 v1;
//! grows every checkpoint per plan §3.11).
//!
//! Families seeded here, named for their eventual N.x homes:
//! * coverage (N.1): a table maps EVERY exported symbol to an exercising
//!   check; the table's key set must equal the frozen census.
//! * error paths (N.3/§9.6): table-driven rejections — null doc, bad UTF-8,
//!   oversized, malformed JSON, schema violations, unknown/unimplemented
//!   kinds, duplicate ids, live-state — each a typed status + taxonomy code
//!   + untouched document, never a crash.
//! * ownership/lifecycle (N.3): free-family NULL no-ops; double-close /
//!   use-after-close / cross-class handles are TYPED (registry cookies).
//! * determinism (WD-1/D-48): the same envelope bytes through the ABI and
//!   through `Engine::apply_bytes` directly yield byte-equal receipts and
//!   identical readback.
//! * events (§9.5): dirtyRects-then-history order, payload shape, drain.
//!
//! The suite calls the `extern "C"` functions from Rust through the rlib —
//! same symbols, same bodies as the C consumer (the C-compiler leg is
//! `tests/c_smoke.rs`).

use std::ffi::CStr;
use std::ptr;

use appname_cabi::actions::appname_doc_apply_action;
use appname_cabi::doc::{
    appname_asset_create, appname_doc_apply_command, appname_doc_close, appname_doc_history_json,
    appname_doc_info_json, appname_doc_new, appname_doc_open, appname_doc_poll_events,
    appname_doc_read_layer, appname_doc_read_region, appname_doc_read_selection, appname_doc_redo,
    appname_doc_save, appname_doc_undo, AppnameDoc,
};
use appname_cabi::export::appname_doc_export;
use appname_cabi::fonts::{appname_doc_font_faces, appname_doc_provide_face};
use appname_cabi::mem::{
    appname_buffer_free, appname_buffer_map, appname_buffer_unmap, appname_error_free,
    appname_error_json, appname_error_status, appname_string_data, appname_string_free,
    AppnameError, AppnameImageBuffer, AppnameMapInfo, AppnameString,
};
use appname_cabi::plugins::{
    appname_plugin_grant, appname_plugin_host_free, appname_plugin_host_new,
    appname_plugin_host_scan, appname_plugin_revoke, appname_plugin_set_enabled, AppnamePluginHost,
};
use appname_cabi::recovery::{
    appname_autosave_discard, appname_autosave_query, appname_doc_autosave_begin,
    appname_doc_autosave_end, appname_doc_open_recovered,
};
use appname_cabi::{appname_version, AppnameRect, AppnameStatus, V1_SYMBOLS};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers (thin: every assertion goes through the public ABI).
// ---------------------------------------------------------------------------

type Doc = *mut AppnameDoc;

const DOC_NEW: &[u8] = br#"{"schema":"appname.command.v1","id":"5a5e1f7e-57a2-4bcd-9e0c-123456789abc","kind":"doc.new","params":{"width":64,"height":48,"background":"white"}}"#;

fn new_doc() -> Doc {
    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_new(&mut doc, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameOk);
    assert!(err.is_null(), "*err untouched on OK");
    assert!(!doc.is_null());
    doc
}

fn init_doc() -> Doc {
    let doc = new_doc();
    let (st, _, e) = apply(doc, DOC_NEW);
    assert_eq!(st, AppnameStatus::AppnameOk, "doc.new must apply: {e:?}");
    doc
}

/// One-plugin scan rig for the ABI 1.5 coverage rows: temp dirs, one
/// valid manifest (`org.example.conf`, requests filter.register +
/// fs.read), strict config through `appname_plugin_host_new`.
fn plugin_host_rig() -> *mut AppnamePluginHost {
    let base = std::env::temp_dir().join(format!(
        "appname-conf-plug-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let sys = base.join("sys");
    let dir = sys.join("org.example.conf-1.0.0");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        br#"{"schema":"appname.plugin.v1","id":"org.example.conf","version":"1.0.0",
            "api":{"min":"1.0","max":"1.x"},"entry":"plugin.wasm",
            "permissions":["filter.register","fs.read"],
            "provides":[{"type":"filter","id":"org.example.conf.f"}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("plugin.wasm"), b"{}").unwrap();
    let config = json!({
        "scanDirs": [sys.display().to_string()],
        "sessionStoreDir": base.join("sessions").display().to_string(),
        "statePath": base.join("plugins-state.toml").display().to_string(),
    });
    let cconfig = std::ffi::CString::new(config.to_string()).unwrap();
    let mut host: *mut AppnamePluginHost = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_plugin_host_new(cconfig.as_ptr(), &mut host, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameOk);
    assert!(err.is_null(), "*err untouched on OK");
    host
}

/// `plugin_host_rig` + scan + enable; returns the host and the plugin id
/// as a C string.
fn enabled_plugin_rig() -> (*mut AppnamePluginHost, std::ffi::CString) {
    let host = plugin_host_rig();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_plugin_host_scan(host, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameOk
    );
    let cid = std::ffi::CString::new("org.example.conf").unwrap();
    assert_eq!(
        unsafe { appname_plugin_set_enabled(host, cid.as_ptr(), true, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameOk
    );
    (host, cid)
}

/// Read + free an error handle directly (the plugin rows call at the raw
/// pointer level rather than through `apply`'s tuple).
fn take_error_raw(e: *mut AppnameError) -> (AppnameStatus, Value) {
    assert!(!e.is_null());
    let stored = unsafe { appname_error_status(e) };
    let mut len: usize = 0;
    let p = unsafe { appname_error_json(e, &mut len) };
    assert!(!p.is_null());
    let text = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_owned();
    assert_eq!(unsafe { appname_error_free(e) }, AppnameStatus::AppnameOk);
    (stored, serde_json::from_str(&text).unwrap())
}

/// Read + free an owned ABI string.
fn take_string(s: *mut AppnameString) -> String {
    let mut len: usize = 0;
    let ptr = unsafe { appname_string_data(s, &mut len) };
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("UTF-8")
        .to_owned();
    assert_eq!(
        text.len(),
        len,
        "reported length excludes the NUL terminator"
    );
    assert_eq!(unsafe { appname_string_free(s) }, AppnameStatus::AppnameOk);
    text
}

/// Read + free an error out-param: (stored status, wire JSON).
fn take_error(e: *mut AppnameError) -> (AppnameStatus, Value) {
    assert!(!e.is_null());
    let stored = unsafe { appname_error_status(e) };
    let mut len: usize = 0;
    let ptr = unsafe { appname_error_json(e, &mut len) };
    assert!(!ptr.is_null(), "live error handle yields a payload");
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("UTF-8")
        .to_owned();
    assert_eq!(text.len(), len);
    let value: Value = serde_json::from_str(&text).expect("error payload is JSON");
    assert_eq!(unsafe { appname_error_free(e) }, AppnameStatus::AppnameOk);
    (stored, value)
}

/// Apply raw bytes: (returned status, rejection/receipt wire, error).
fn apply(doc: Doc, bytes: &[u8]) -> (AppnameStatus, Option<Value>, Option<(AppnameStatus, Value)>) {
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st =
        unsafe { appname_doc_apply_command(doc, bytes.as_ptr(), bytes.len(), &mut out, &mut err) };
    let wire = (!out.is_null())
        .then(|| serde_json::from_str(&take_string(out)).expect("wire document is JSON"));
    let e = (!err.is_null()).then(|| take_error(err));
    (st, wire, e)
}

fn info(doc: Doc) -> Value {
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_info_json(doc, &mut out, &mut err) };
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "info: {:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    serde_json::from_str(&take_string(out)).expect("info is JSON")
}

fn poll(doc: Doc) -> Vec<Value> {
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_poll_events(doc, &mut out, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameOk);
    serde_json::from_str(&take_string(out)).expect("events are a JSON array")
}

fn read_full(doc: Doc, w: u32, h: u32) -> Vec<u8> {
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_region(doc, AppnameRect { x: 0, y: 0, w, h }, &mut buf, &mut err)
    };
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "{:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    let mut map = AppnameMapInfo {
        data: ptr::null_mut(),
        len: 0,
        width: 0,
        height: 0,
        stride: 0,
        pixel_format: 9,
        color_model: 9,
        premultiplied: 9,
    };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameOk
    );
    assert_eq!((map.width, map.height, map.stride), (w, h, w * 4));
    assert_eq!(
        (map.pixel_format, map.color_model, map.premultiplied),
        (0, 3, 0)
    );
    assert_eq!(map.len, (w * h * 4) as usize);
    let data = unsafe { std::slice::from_raw_parts(map.data, map.len) }.to_vec();
    assert_eq!(
        unsafe { appname_buffer_unmap(buf) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );
    data
}

fn close(doc: Doc) {
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
}

fn cstr(p: &std::path::Path) -> std::ffi::CString {
    std::ffi::CString::new(p.to_str().unwrap().to_owned()).unwrap()
}

/// Export via the ABI: (status, error).
fn export(
    doc: Doc,
    path: &std::path::Path,
    format: &str,
    options: Option<&str>,
) -> (AppnameStatus, Option<(AppnameStatus, Value)>) {
    let cpath = cstr(path);
    let cformat = std::ffi::CString::new(format).unwrap();
    let copts = options.map(|o| std::ffi::CString::new(o).unwrap());
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_export(
            doc,
            cpath.as_ptr(),
            cformat.as_ptr(),
            copts.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            &mut err,
        )
    };
    (st, (!err.is_null()).then(|| take_error(err)))
}

/// The uuid string of the layer at info row `index`.
fn layer_id_at(doc: Doc, index: usize) -> String {
    info(doc)["layers"][index]["id"]
        .as_str()
        .expect("layer row has an id")
        .to_owned()
}

/// Per-layer readback via the ABI (ABI 1.4): raw call, returning
/// (status, bytes when OK, error when populated).
fn read_layer(
    doc: Doc,
    layer_id: &str,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> (
    AppnameStatus,
    Option<Vec<u8>>,
    Option<(AppnameStatus, Value)>,
) {
    let cid = std::ffi::CString::new(layer_id).unwrap();
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_layer(
            doc,
            cid.as_ptr(),
            AppnameRect { x, y, w, h },
            &mut buf,
            &mut err,
        )
    };
    let bytes = (st == AppnameStatus::AppnameOk).then(|| {
        let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
        assert_eq!(
            unsafe { appname_buffer_map(buf, &mut map) },
            AppnameStatus::AppnameOk
        );
        // The read_region delivery convention: RGBA8, tight stride.
        assert_eq!((map.width, map.height, map.stride), (w, h, w * 4));
        assert_eq!(
            (map.pixel_format, map.color_model, map.premultiplied),
            (0, 3, 0)
        );
        let data = unsafe { std::slice::from_raw_parts(map.data, map.len) }.to_vec();
        assert_eq!(
            unsafe { appname_buffer_free(buf) },
            AppnameStatus::AppnameOk
        );
        data
    });
    let e = (!err.is_null()).then(|| take_error(err));
    (st, bytes, e)
}

/// Per-layer readback asserting OK.
fn read_layer_ok(doc: Doc, layer_id: &str, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
    let (st, bytes, e) = read_layer(doc, layer_id, x, y, w, h);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    bytes.expect("OK readback yields bytes")
}

/// Whole-region readback asserting OK (the group battery's oracle side).
fn read_region_ok(doc: Doc, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st =
        unsafe { appname_doc_read_region(doc, AppnameRect { x, y, w, h }, &mut buf, &mut err) };
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "{:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameOk
    );
    assert_eq!((map.width, map.height, map.stride), (w, h, w * 4));
    let data = unsafe { std::slice::from_raw_parts(map.data, map.len) }.to_vec();
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );
    data
}

/// Selection readback via the ABI, asserting OK: (bytes, map info).
fn read_selection(doc: Doc, x: i32, y: i32, w: u32, h: u32) -> (Vec<u8>, AppnameMapInfo) {
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st =
        unsafe { appname_doc_read_selection(doc, AppnameRect { x, y, w, h }, &mut buf, &mut err) };
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "{:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameOk
    );
    assert_eq!((map.width, map.height), (w, h));
    assert_eq!(map.len, (w * h) as usize, "Gray8 is one byte per pixel");
    let data = unsafe { std::slice::from_raw_parts(map.data, map.len) }.to_vec();
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );
    (data, map)
}

fn autosave_begin(
    doc: Doc,
    log: &std::path::Path,
) -> (AppnameStatus, Option<(AppnameStatus, Value)>) {
    let clog = cstr(log);
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_autosave_begin(doc, clog.as_ptr(), &mut err) };
    (st, (!err.is_null()).then(|| take_error(err)))
}

/// Query an autosave log; Some(report) on OK, None on error (status asserted
/// by the caller through `autosave_query_status`).
fn autosave_query(log: &std::path::Path) -> Option<Value> {
    let (st, wire, _) = autosave_query_status(log);
    (st == AppnameStatus::AppnameOk).then(|| wire.expect("OK query yields a report"))
}

fn autosave_query_status(
    log: &std::path::Path,
) -> (AppnameStatus, Option<Value>, Option<(AppnameStatus, Value)>) {
    let clog = cstr(log);
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_autosave_query(clog.as_ptr(), &mut out, &mut err) };
    let wire = (!out.is_null())
        .then(|| serde_json::from_str(&take_string(out)).expect("query report is JSON"));
    let e = (!err.is_null()).then(|| take_error(err));
    (st, wire, e)
}

/// Open-recovered via the ABI, asserting OK: (handle, report).
fn open_recovered(package: &std::path::Path, log: &std::path::Path) -> (Doc, Value) {
    let cpkg = cstr(package);
    let clog = cstr(log);
    let mut doc: Doc = ptr::null_mut();
    let mut report: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_open_recovered(
            cpkg.as_ptr(),
            clog.as_ptr(),
            &mut doc,
            &mut report,
            &mut err,
        )
    };
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "{:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    let report = serde_json::from_str(&take_string(report)).expect("recovery report is JSON");
    (doc, report)
}

fn autosave_discard(log: &std::path::Path) -> AppnameStatus {
    let clog = cstr(log);
    unsafe { appname_autosave_discard(clog.as_ptr(), ptr::null_mut()) }
}

// ---------------------------------------------------------------------------
// Coverage: every exported symbol exercised, table-driven (N.1 seed).
// ---------------------------------------------------------------------------

#[test]
fn every_v1_symbol_has_an_exercising_check() {
    let table: Vec<(&str, fn())> = vec![
        ("appname_version", || {
            // 1.1 closed at appname_asset_create (released v0.3.0-alpha/
            // v0.5.0-beta); 1.2 closed at the ABI-growth round; 1.3
            // closed at the actions round (released v0.7.0-beta); 1.4 =
            // the consolidated round; 1.5 = the plugin-lifecycle door;
            // the font-provisioning door is the 1.6 block.
            let v = appname_version();
            // 1.7 = the §19.1 block (info emits `imageRect`; `doc.crop`
            // honours `deletePixels:false`). Kept EXACT so the next bump is
            // noticed here too — this pin caught the 1.6 -> 1.7 move.
            assert_eq!((v.abi_major, v.abi_minor), (1, 8));
        }),
        ("appname_doc_new", || close(new_doc())),
        ("appname_doc_open", || {
            // Open of a missing path is typed IO (the success leg is
            // save_open_round_trip below; both exercise the symbol).
            let mut doc: Doc = ptr::null_mut();
            let mut err: *mut AppnameError = ptr::null_mut();
            let path =
                std::ffi::CString::new("/nonexistent/appname-cabi-conformance.appname").unwrap();
            let st = unsafe { appname_doc_open(path.as_ptr(), &mut doc, &mut err) };
            assert_eq!(st, AppnameStatus::AppnameErrIo);
            let (stored, payload) = take_error(err);
            assert_eq!(stored, AppnameStatus::AppnameErrIo);
            assert_eq!(payload["code"], "error.io.read_failed");
        }),
        ("appname_doc_save", || {
            let doc = init_doc();
            let dir = scratch("save-symbol");
            let path = std::ffi::CString::new(dir.join("doc.appname").to_str().unwrap().to_owned())
                .unwrap();
            let mut err: *mut AppnameError = ptr::null_mut();
            let st = unsafe { appname_doc_save(doc, path.as_ptr(), &mut err) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            assert!(err.is_null());
            close(doc);
        }),
        ("appname_doc_close", || close(new_doc())),
        ("appname_doc_apply_command", || {
            let doc = init_doc();
            close(doc);
        }),
        ("appname_asset_create", || {
            let doc = init_doc();
            let (st, uuid, _) = asset_create(doc, &sample_blob());
            assert_eq!(st, AppnameStatus::AppnameOk);
            assert_eq!(uuid.expect("uuid string").len(), 36);
            close(doc);
        }),
        ("appname_doc_apply_action", || {
            // The success leg lives in the dedicated 1.3-block tests
            // below; the symbol row exercises the typed schema reject.
            let doc = init_doc();
            let (st, _report, e) = apply_action(doc, br#"{"schema":"appname.action.v2"}"#, None);
            assert_eq!(st, AppnameStatus::AppnameErrSchema);
            let (stored, payload) = e.expect("error out-param populated");
            assert_eq!(stored, AppnameStatus::AppnameErrSchema);
            assert_eq!(payload["code"], "error.schema.wrong_schema");
            close(doc);
        }),
        ("appname_doc_export", || {
            let doc = init_doc();
            let path = scratch("export-symbol").join("out.png");
            let (st, e) = export(doc, &path, "png", None);
            assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
            let bytes = std::fs::read(&path).expect("exported file exists");
            assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "PNG magic");
            close(doc);
        }),
        ("appname_doc_read_layer", || {
            // Success + refusal legs live in the dedicated 1.4-block tests
            // below; the symbol row exercises the background-layer truth.
            let doc = init_doc();
            let layer = layer_id_at(doc, 0);
            let px = read_layer_ok(doc, &layer, 0, 0, 8, 8);
            assert!(px.chunks(4).all(|p| p == [255, 255, 255, 255]));
            close(doc);
        }),
        ("appname_doc_read_selection", || {
            let doc = init_doc();
            // No selection yet: all-zero coverage, Gray8 encoding.
            let (data, map) = read_selection(doc, 0, 0, 8, 8);
            assert_eq!((map.pixel_format, map.color_model), (0, 0));
            assert_eq!(map.stride, 8);
            assert!(data.iter().all(|b| *b == 0));
            close(doc);
        }),
        ("appname_doc_autosave_begin", || {
            let doc = init_doc();
            let log = scratch("autosave-begin-symbol").join("doc.autosave");
            assert_eq!(autosave_begin(doc, &log).0, AppnameStatus::AppnameOk);
            assert!(log.exists(), "BASE anchor written durably");
            close(doc);
        }),
        ("appname_doc_autosave_end", || {
            let doc = init_doc();
            // Idempotent: ending with no live binding is an OK no-op.
            let mut err: *mut AppnameError = ptr::null_mut();
            assert_eq!(
                unsafe { appname_doc_autosave_end(doc, &mut err) },
                AppnameStatus::AppnameOk
            );
            assert!(err.is_null());
            close(doc);
        }),
        ("appname_autosave_query", || {
            let doc = init_doc();
            let log = scratch("autosave-query-symbol").join("doc.autosave");
            assert_eq!(autosave_begin(doc, &log).0, AppnameStatus::AppnameOk);
            let q = autosave_query(&log).expect("fresh log queries");
            assert_eq!(q["commands"], 0, "BASE-only log has nothing to replay");
            assert_eq!(q["torn"], false);
            close(doc);
        }),
        ("appname_doc_open_recovered", || {
            // Missing paths are typed IO (the success leg is the dedicated
            // W05-flow test; both exercise the symbol).
            let mut doc: Doc = ptr::null_mut();
            let mut err: *mut AppnameError = ptr::null_mut();
            let pkg = std::ffi::CString::new("/nonexistent/base.appname").unwrap();
            let log = std::ffi::CString::new("/nonexistent/doc.autosave").unwrap();
            let st = unsafe {
                appname_doc_open_recovered(
                    pkg.as_ptr(),
                    log.as_ptr(),
                    &mut doc,
                    ptr::null_mut(),
                    &mut err,
                )
            };
            assert_eq!(st, AppnameStatus::AppnameErrIo);
            assert!(doc.is_null(), "no handle on failure");
            assert_eq!(take_error(err).1["code"], "error.io.read_failed");
        }),
        ("appname_autosave_discard", || {
            // Idempotent: discarding a nonexistent log is an OK no-op.
            let path =
                std::ffi::CString::new("/nonexistent/appname-cabi-discard.autosave").unwrap();
            let mut err: *mut AppnameError = ptr::null_mut();
            assert_eq!(
                unsafe { appname_autosave_discard(path.as_ptr(), &mut err) },
                AppnameStatus::AppnameOk
            );
            assert!(err.is_null());
        }),
        ("appname_doc_undo", || {
            let doc = init_doc();
            let mut row: *mut AppnameString = ptr::null_mut();
            let st = unsafe { appname_doc_undo(doc, &mut row, ptr::null_mut()) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            let row: Value = serde_json::from_str(&take_string(row)).unwrap();
            assert_eq!(row["kind"], "doc.new");
            assert_eq!(row["active"], false);
            close(doc);
        }),
        ("appname_doc_redo", || {
            let doc = init_doc();
            assert_eq!(
                unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
                AppnameStatus::AppnameOk
            );
            let mut row: *mut AppnameString = ptr::null_mut();
            let st = unsafe { appname_doc_redo(doc, &mut row, ptr::null_mut()) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            let row: Value = serde_json::from_str(&take_string(row)).unwrap();
            assert_eq!(row["active"], true);
            close(doc);
        }),
        ("appname_doc_info_json", || {
            let doc = init_doc();
            let i = info(doc);
            assert_eq!(i["width"], 64);
            assert_eq!(i["height"], 48);
            assert_eq!(i["journal"]["canUndo"], true);
            close(doc);
        }),
        ("appname_doc_history_json", || {
            let doc = init_doc();
            let mut out: *mut AppnameString = ptr::null_mut();
            let st = unsafe { appname_doc_history_json(doc, &mut out, ptr::null_mut()) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            let h: Value = serde_json::from_str(&take_string(out)).unwrap();
            assert_eq!(h["length"], 1);
            assert_eq!(h["cursor"], 1);
            assert_eq!(h["entries"][0]["kind"], "doc.new");
            close(doc);
        }),
        ("appname_doc_poll_events", || {
            let doc = init_doc();
            assert!(!poll(doc).is_empty());
            close(doc);
        }),
        ("appname_doc_read_region", || {
            let doc = init_doc();
            let px = read_full(doc, 64, 48);
            assert!(px.chunks(4).all(|p| p == [255, 255, 255, 255]));
            close(doc);
        }),
        ("appname_string_data", || {
            let doc = init_doc();
            let mut out: *mut AppnameString = ptr::null_mut();
            unsafe { appname_doc_history_json(doc, &mut out, ptr::null_mut()) };
            let _ = take_string(out); // data + free asserted inside
            close(doc);
        }),
        ("appname_string_free", || {
            assert_eq!(
                unsafe { appname_string_free(ptr::null_mut()) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_error_status", || {
            let doc = new_doc();
            let (_, _, e) = apply(doc, b"{malformed");
            let (stored, _) = e.expect("rejection populates err");
            assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
            close(doc);
        }),
        ("appname_error_json", || {
            let doc = new_doc();
            let (_, _, e) = apply(doc, b"{malformed");
            assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");
            close(doc);
        }),
        ("appname_error_free", || {
            assert_eq!(
                unsafe { appname_error_free(ptr::null_mut()) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_buffer_map", || {
            let doc = init_doc();
            let _ = read_full(doc, 8, 8); // map asserted inside
            close(doc);
        }),
        ("appname_buffer_unmap", || {
            assert_eq!(
                unsafe { appname_buffer_unmap(ptr::null_mut()) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_buffer_free", || {
            assert_eq!(
                unsafe { appname_buffer_free(ptr::null_mut()) },
                AppnameStatus::AppnameOk
            );
        }),
        // ABI 1.5 plugin-lifecycle door. The deep behavioral rows live in
        // tests/plugin_door.rs; these exercise each symbol's contract
        // through a minimal real flow (N.1: every symbol exercised HERE).
        ("appname_plugin_host_new", || {
            let host = plugin_host_rig();
            assert!(!host.is_null());
            assert_eq!(
                unsafe { appname_plugin_host_free(host) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_plugin_host_free", || {
            // NULL no-op + garbage typed (§9.3 free family).
            assert_eq!(
                unsafe { appname_plugin_host_free(ptr::null_mut()) },
                AppnameStatus::AppnameOk
            );
            assert_eq!(
                unsafe { appname_plugin_host_free(0xdead_beefusize as *mut AppnamePluginHost) },
                AppnameStatus::AppnameErrInvalidArg
            );
        }),
        ("appname_plugin_host_scan", || {
            let host = plugin_host_rig();
            let mut out: *mut AppnameString = ptr::null_mut();
            let mut err: *mut AppnameError = ptr::null_mut();
            let st = unsafe { appname_plugin_host_scan(host, &mut out, &mut err) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            let list: Value = serde_json::from_str(&take_string(out)).unwrap();
            assert_eq!(list["schema"], "appname.pluginlist.v1");
            assert_eq!(list["plugins"].as_array().unwrap().len(), 1);
            assert_eq!(
                unsafe { appname_plugin_host_free(host) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_plugin_set_enabled", || {
            let host = plugin_host_rig();
            let mut err: *mut AppnameError = ptr::null_mut();
            assert_eq!(
                unsafe { appname_plugin_host_scan(host, ptr::null_mut(), &mut err) },
                AppnameStatus::AppnameOk
            );
            let cid = std::ffi::CString::new("org.example.conf").unwrap();
            let mut out: *mut AppnameString = ptr::null_mut();
            let st =
                unsafe { appname_plugin_set_enabled(host, cid.as_ptr(), true, &mut out, &mut err) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            let reply: Value = serde_json::from_str(&take_string(out)).unwrap();
            assert_eq!(reply["toPrompt"], json!(["fs.read"]));
            assert_eq!(
                unsafe { appname_plugin_host_free(host) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_plugin_grant", || {
            let (host, cid) = enabled_plugin_rig();
            let cg = std::ffi::CString::new("clipboard").unwrap();
            let mut err: *mut AppnameError = ptr::null_mut();
            assert_eq!(
                unsafe { appname_plugin_grant(host, cid.as_ptr(), cg.as_ptr(), &mut err) },
                AppnameStatus::AppnameOk
            );
            // Malformed grant string: typed refusal.
            let bad = std::ffi::CString::new("fs.read:not-absolute").unwrap();
            let mut err: *mut AppnameError = ptr::null_mut();
            let st = unsafe { appname_plugin_grant(host, cid.as_ptr(), bad.as_ptr(), &mut err) };
            assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
            let (stored, body) = take_error_raw(err);
            assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
            assert_eq!(body["code"], "error.api.invalid_arg");
            assert_eq!(
                unsafe { appname_plugin_host_free(host) },
                AppnameStatus::AppnameOk
            );
        }),
        ("appname_plugin_revoke", || {
            let (host, cid) = enabled_plugin_rig();
            let cg = std::ffi::CString::new("clipboard").unwrap();
            let mut err: *mut AppnameError = ptr::null_mut();
            assert_eq!(
                unsafe { appname_plugin_grant(host, cid.as_ptr(), cg.as_ptr(), &mut err) },
                AppnameStatus::AppnameOk
            );
            assert_eq!(
                unsafe { appname_plugin_revoke(host, cid.as_ptr(), cg.as_ptr(), &mut err) },
                AppnameStatus::AppnameOk
            );
            // Revoking the unrecorded: typed refusal.
            let mut err: *mut AppnameError = ptr::null_mut();
            let st = unsafe { appname_plugin_revoke(host, cid.as_ptr(), cg.as_ptr(), &mut err) };
            assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
            let _ = take_error_raw(err);
            assert_eq!(
                unsafe { appname_plugin_host_free(host) },
                AppnameStatus::AppnameOk
            );
        }),
        // ABI 1.6 font-provisioning door. The deep behavioral rows live
        // in tests/font_door.rs; these exercise each symbol's ABI
        // contract minimally (N.1: every symbol exercised HERE).
        ("appname_doc_font_faces", || {
            let doc = init_doc();
            let mut out: *mut AppnameString = ptr::null_mut();
            let mut err: *mut AppnameError = ptr::null_mut();
            let st = unsafe { appname_doc_font_faces(doc, &mut out, &mut err) };
            assert_eq!(st, AppnameStatus::AppnameOk);
            let v: Value = serde_json::from_str(&take_string(out)).unwrap();
            assert_eq!(v["schema"], "appname.fontfaces.v1");
            assert!(v["faces"].as_array().unwrap().is_empty());
            close(doc);
        }),
        ("appname_doc_provide_face", || {
            // Garbage bytes: the symbol is exercised and refuses typed
            // (validation rides the engine's OutOfRange => SCHEMA); the
            // happy path with real font bytes is font_door.rs.
            let doc = init_doc();
            let (src, fam, sty, ps) = (
                std::ffi::CString::new("user").unwrap(),
                std::ffi::CString::new("F").unwrap(),
                std::ffi::CString::new("R").unwrap(),
                std::ffi::CString::new("P").unwrap(),
            );
            let bytes = b"not a font";
            let mut err: *mut AppnameError = ptr::null_mut();
            let st = unsafe {
                appname_doc_provide_face(
                    doc,
                    bytes.as_ptr(),
                    bytes.len(),
                    src.as_ptr(),
                    fam.as_ptr(),
                    sty.as_ptr(),
                    ps.as_ptr(),
                    ptr::null_mut(),
                    &mut err,
                )
            };
            assert_eq!(st, AppnameStatus::AppnameErrSchema);
            let _ = take_error_raw(err);
            close(doc);
        }),
    ];

    let mut names: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
    names.sort();
    assert_eq!(
        names, V1_SYMBOLS,
        "coverage table must exercise exactly the frozen census"
    );
    for (name, check) in table {
        check();
        println!("exercised {name}");
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/cabi-conformance")
        .join(tag);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

// ---------------------------------------------------------------------------
// Stroke kinds through the ABI (wave-2b: assembly + receipt-dirty events).
// ---------------------------------------------------------------------------

/// Encoded three-point diagonal stroke (the appname-tools reference line).
fn sample_blob() -> Vec<u8> {
    use appname_tools::brush::{encode_samples, InputSample};
    let pts = [(20.0, 20.0, 0.0), (60.0, 40.0, 0.04), (100.0, 60.0, 0.08)];
    let samples: Vec<InputSample> = pts
        .iter()
        .map(|(x, y, t)| InputSample {
            x: *x,
            y: *y,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t: *t,
        })
        .collect();
    encode_samples(&samples).expect("reference line encodes")
}

/// Register a samples blob through the ABI: (status, uuid string, error).
fn asset_create(
    doc: Doc,
    bytes: &[u8],
) -> (
    AppnameStatus,
    Option<String>,
    Option<(AppnameStatus, Value)>,
) {
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_asset_create(doc, bytes.as_ptr(), bytes.len(), &mut out, &mut err) };
    let uuid = (!out.is_null()).then(|| take_string(out));
    let e = (!err.is_null()).then(|| take_error(err));
    (st, uuid, e)
}

/// A stroke envelope byte-identical across the ABI and in-process legs.
fn stroke_bytes(
    id: &str,
    kind: &str,
    layer: &str,
    samples: &str,
    extra: &[(&str, Value)],
) -> Vec<u8> {
    let mut params = json!({
        "brushPreset": appname_tools::kind_impls::preset_hard_round_16_id().to_string(),
    });
    if kind == "brush.stroke" {
        params["color"] = json!({"model": "rgb", "comps": [1, 0, 0, 0]});
    }
    for (k, v) in extra {
        params[*k] = v.clone();
    }
    json!({
        "schema": "appname.command.v1",
        "id": id,
        "kind": kind,
        "target": {"layerId": layer},
        "params": params,
        "samples": samples,
    })
    .to_string()
    .into_bytes()
}

/// Build the shared setup OUT-OF-PROCESS style: a package containing a
/// 128x128 white document + the sample asset (persisted via the pending
/// grace — assets have no ABI ingress symbol until the §9.2 asset block).
/// Returns (package path, in-process twin engine, layer id, asset id).
fn stroke_package(tag: &str) -> (std::path::PathBuf, appname_core::Engine, String, String) {
    let mut direct = appname_tools::kind_impls::assembly().build();
    direct
        .apply_bytes(
            json!({
                "schema": "appname.command.v1",
                "id": format!("d0c00000-0000-4000-8000-00000000{:03x}1", tag.len()),
                "kind": "doc.new",
                "params": {"width": 128, "height": 128, "background": "white"}
            })
            .to_string()
            .as_bytes(),
        )
        .expect("doc.new applies");
    let asset = direct.add_samples_asset(sample_blob());
    let layer = direct.document().unwrap().z_order_walk()[0];
    let path = scratch(tag).join("stroke.appname");
    appname_core::save_package(&direct, &path).expect("package saves");
    (path, direct, layer.as_uuid().to_string(), asset.to_string())
}

/// Wire `DirtyRect.rect` ({"min":[x,y],"max":[x,y]} — IVec2 = 2-array) to
/// an `IRect`.
fn rect_from_wire(d: &Value) -> appname_core::IRect {
    let (min, max) = (&d["rect"]["min"], &d["rect"]["max"]);
    appname_core::IRect::new(
        min[0].as_i64().unwrap() as i32,
        min[1].as_i64().unwrap() as i32,
        max[0].as_i64().unwrap() as i32,
        max[1].as_i64().unwrap() as i32,
    )
}

/// The §3.7-coalesced payload expected for receipt-side dirty entries
/// (O-WATCH #2 MINOR-1): the renderer's own `RegionSet` — 8 px
/// inflate-intersect merge to fixpoint, > 8 rects collapse to the bounding
/// box (ES §3.7 "Coalescing algorithm").
fn expected_coalesced(dirty: &[Value]) -> Vec<Value> {
    let mut set = appname_render::RegionSet::new();
    for d in dirty {
        set.add(rect_from_wire(d));
    }
    set.rects()
        .iter()
        .map(|r| json!([r.min.x, r.min.y, r.width(), r.height()]))
        .collect()
}

/// The coalescing INVARIANT (independent of the implementation): at most
/// `REGION_SET_CAP` payload rects, and every non-empty receipt rect is
/// fully contained in one of them (union coverage — §3.7 merges only by
/// union, so coverage can never shrink).
fn assert_union_coverage(receipt_dirty: &[Value], event_rects: &Value) {
    let evs: Vec<appname_core::IRect> = event_rects
        .as_array()
        .expect("rects array")
        .iter()
        .map(|r| {
            let (x, y, w, h) = (
                r[0].as_i64().unwrap() as i32,
                r[1].as_i64().unwrap() as i32,
                r[2].as_i64().unwrap() as i32,
                r[3].as_i64().unwrap() as i32,
            );
            appname_core::IRect::new(x, y, x + w, y + h)
        })
        .collect();
    assert!(
        evs.len() <= appname_render::REGION_SET_CAP,
        "payload carries {} rects; the §3.7 cap is {}",
        evs.len(),
        appname_render::REGION_SET_CAP
    );
    for d in receipt_dirty {
        let r = rect_from_wire(d);
        if r.is_empty() {
            continue;
        }
        assert!(
            evs.iter().any(|e| e.min.x <= r.min.x
                && e.min.y <= r.min.y
                && r.max.x <= e.max.x
                && r.max.y <= e.max.y),
            "receipt rect {r:?} not covered by the payload rects {evs:?}"
        );
    }
}

#[test]
fn brush_stroke_flows_through_the_abi_with_receipt_dirty_events() {
    let (path, mut direct, layer, asset) = stroke_package("stroke-abi");
    let cpath = std::ffi::CString::new(path.to_str().unwrap().to_owned()).unwrap();

    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cpath.as_ptr(), &mut doc, &mut err) },
        AppnameStatus::AppnameOk
    );
    let _ = poll(doc); // drain the open-seed dirty event

    // WD-1: identical raw bytes through both doors.
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000b001",
        "brush.stroke",
        &layer,
        &asset,
        &[("seed", json!(7))],
    );
    let (st, wire, e) = apply(doc, &env);
    assert_eq!(st, AppnameStatus::AppnameOk, "brush.stroke via ABI: {e:?}");
    let wire = wire.expect("receipt wire");
    assert_eq!(wire["ok"], true);
    assert_eq!(wire["receipt"]["kind"], "brush.stroke");
    let receipt_dirty = wire["receipt"]["dirty"]
        .as_array()
        .expect("receipt carries dirty");
    assert!(
        !receipt_dirty.is_empty(),
        "tile-writing kind reports touched tiles"
    );

    // Determinism vs the in-process assembled twin: byte-equal receipt wire.
    let twin_receipt = direct
        .apply_bytes(&env)
        .expect("twin applies the same bytes");
    let twin_wire = json!({ "ok": true, "receipt": twin_receipt }).to_string();
    assert_eq!(
        serde_json::to_string(&wire).unwrap(),
        twin_wire,
        "byte-equal receipts (WD-1)"
    );

    // §9.5 events derive from the SAME receipt rects (coalesce-aware).
    let events = poll(doc);
    assert_eq!(events.len(), 2, "stroke queues dirtyRects + history");
    assert_eq!(events[0]["schema"], "appname.event.dirtyRects.v1");
    assert_eq!(
        events[0]["rects"],
        Value::Array(expected_coalesced(receipt_dirty)),
        "event rects are the receipt's rects, §3.7-coalesced"
    );
    assert_union_coverage(receipt_dirty, &events[0]["rects"]);
    assert_eq!(events[1]["schema"], "appname.event.history.v1");
    assert_eq!(events[1]["labelMessageId"], "brush.stroke");
    assert_eq!(events[1]["undoDepth"], 2);

    // The stroke crosses (60, 40): red over white, visible in readback.
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe {
            appname_doc_read_region(
                doc,
                AppnameRect {
                    x: 60,
                    y: 40,
                    w: 1,
                    h: 1,
                },
                &mut buf,
                &mut err,
            )
        },
        AppnameStatus::AppnameOk
    );
    let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameOk
    );
    let px = unsafe { std::slice::from_raw_parts(map.data, 4) };
    assert!(
        px[0] > 229 && px[1] < 153 && px[3] == 255,
        "red-ward stroke pixel, got {px:?}"
    );
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );

    // Suffix-aligned reopened history (wave-2c flip, behavior-table row 2):
    // the post-open stroke undoes IN-PROCESS, echoes the receipt rects on
    // the row and into the event stream, and canUndo agrees.
    assert_eq!(info(doc)["journal"]["canUndo"], true);
    let mut row: *mut AppnameString = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(doc, &mut row, ptr::null_mut()) },
        AppnameStatus::AppnameOk,
        "post-open undo succeeds within the session suffix"
    );
    let row: Value = serde_json::from_str(&take_string(row)).unwrap();
    assert_eq!(row["kind"], "brush.stroke");
    assert_eq!(
        row["dirty"], wire["receipt"]["dirty"],
        "undo row echoes receipt dirty"
    );
    let events = poll(doc);
    assert_eq!(
        events[0]["rects"],
        Value::Array(expected_coalesced(receipt_dirty))
    );
    assert_union_coverage(receipt_dirty, &events[0]["rects"]);
    // The stroke pixel is white again (suffix undo restores identity).
    let mut buf2: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe {
            appname_doc_read_region(
                doc,
                AppnameRect {
                    x: 60,
                    y: 40,
                    w: 1,
                    h: 1,
                },
                &mut buf2,
                &mut err,
            )
        },
        AppnameStatus::AppnameOk
    );
    let mut map2 = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf2, &mut map2) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        unsafe { std::slice::from_raw_parts(map2.data, 4) },
        [255, 255, 255, 255]
    );
    assert_eq!(
        unsafe { appname_buffer_free(buf2) },
        AppnameStatus::AppnameOk
    );
    // Row 3: back at the open base — undo typed, the suffix stays redoable.
    assert_eq!(info(doc)["journal"]["canUndo"], false);
    assert_eq!(info(doc)["journal"]["canRedo"], true);
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.history.nothing_to_undo");
    assert!(payload["message"]
        .as_str()
        .unwrap()
        .contains("not persisted"));
    close(doc);
}

/// Encode an arbitrary polyline as an `appname.samples.v1` blob.
fn sample_blob_line(pts: &[(f64, f64)]) -> Vec<u8> {
    use appname_tools::brush::{encode_samples, InputSample};
    let samples: Vec<InputSample> = pts
        .iter()
        .enumerate()
        .map(|(i, (x, y))| InputSample {
            x: *x,
            y: *y,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t: i as f64 * 0.02,
        })
        .collect();
    encode_samples(&samples).expect("polyline encodes")
}

#[test]
fn many_tile_stroke_coalesces_to_at_most_8_covering_rects() {
    // O-WATCH #2 MINOR-1 acceptance: a stroke touching many tiles reports
    // one receipt rect per touched tile (> 8 of them), and the §9.5
    // payload carries the §3.7-coalesced set — at most REGION_SET_CAP
    // rects whose union covers every receipt rect (ES §3.7: 8 px
    // inflate-intersect merge to fixpoint; > 8 rects collapse to the
    // single bounding box). Entirely through the ABI.
    let doc = new_doc();
    let (st, _, e) = apply(
        doc,
        json!({
            "schema": "appname.command.v1",
            "id": "d0c00000-0000-4000-8000-000000000371",
            "kind": "doc.new",
            "params": {"width": 2048, "height": 2048, "background": "white"}
        })
        .to_string()
        .as_bytes(),
    );
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let _ = poll(doc); // drain the doc.new events

    let blob = sample_blob_line(&[
        (20.0, 20.0),
        (500.0, 500.0),
        (1000.0, 1000.0),
        (1500.0, 1500.0),
        (2000.0, 2000.0),
    ]);
    let (st, uuid, _) = asset_create(doc, &blob);
    assert_eq!(st, AppnameStatus::AppnameOk);
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-000000000372",
        "brush.stroke",
        &layer,
        &uuid.unwrap(),
        &[("seed", json!(1))],
    );
    let (st, wire, e) = apply(doc, &env);
    assert_eq!(st, AppnameStatus::AppnameOk, "long stroke: {e:?}");
    let wire = wire.unwrap();
    let receipt_dirty = wire["receipt"]["dirty"].as_array().unwrap().clone();
    assert!(
        receipt_dirty.len() > appname_render::REGION_SET_CAP,
        "many-tile premise: {} per-tile receipt rects (> {})",
        receipt_dirty.len(),
        appname_render::REGION_SET_CAP
    );

    let events = poll(doc);
    assert_eq!(events[0]["schema"], "appname.event.dirtyRects.v1");
    let payload = &events[0]["rects"];
    assert_union_coverage(&receipt_dirty, payload);
    assert_eq!(
        payload,
        &Value::Array(expected_coalesced(&receipt_dirty)),
        "payload equals the §3.7 coalescer's output for the receipt rects"
    );
    println!(
        "many-tile stroke: {} receipt rects -> {} payload rects (cap {})",
        receipt_dirty.len(),
        payload.as_array().unwrap().len(),
        appname_render::REGION_SET_CAP
    );
    close(doc);
}

/// Rewrite a saved package's document.json to set `locks` on one layer —
/// conformance scaffolding through public doors only (the loader reads
/// `locks`; no lock-setting command kind exists until CP-0.5
/// layer.set_props). Mirrors the appname-tools kind_impls test helper.
fn lock_layer_in_package(path: &std::path::Path, layer: &str, locks: &[&str]) {
    use std::io::{Read, Write};
    let bytes = std::fs::read(path).expect("read package");
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).expect("zip opens");
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..za.len() {
        let mut entry = za.by_index(i).expect("entry");
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("entry bytes");
        entries.push((entry.name().to_string(), data));
    }
    for (name, data) in entries.iter_mut() {
        if name == "document.json" {
            let mut v: Value = serde_json::from_slice(data).expect("document.json parses");
            for l in v["layers"].as_array_mut().expect("layers array") {
                if l["id"] == layer {
                    l["locks"] = json!(locks);
                }
            }
            let mut out = serde_json::to_vec_pretty(&v).expect("re-serialize");
            out.push(b'\n');
            *data = out;
        }
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zw.start_file(name, opts).expect("start entry");
            zw.write_all(&data).expect("write entry");
        }
        zw.finish().expect("finish zip");
    }
    std::fs::write(path, cursor.into_inner()).expect("write rewritten package");
}

#[test]
fn eraser_typed_rejects_through_the_abi_lock_block_and_erase_to_history() {
    // Leg 1: TRANSPARENCY lock blocks eraser.stroke outright (§40.5
    // exception) -> error.command.locked -> APPNAME_ERR_INVALID_ARG (the
    // ratified Locked arm, exercised end-to-end).
    let (path, _direct, layer, asset) = stroke_package("eraser-locked");
    lock_layer_in_package(&path, &layer, &["transparency"]);
    let cpath = std::ffi::CString::new(path.to_str().unwrap().to_owned()).unwrap();
    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cpath.as_ptr(), &mut doc, &mut err) },
        AppnameStatus::AppnameOk
    );
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000e001",
        "eraser.stroke",
        &layer,
        &asset,
        &[],
    );
    let (st, wire, e) = apply(doc, &env);
    assert_eq!(
        st,
        AppnameStatus::AppnameErrInvalidArg,
        "lock block is typed"
    );
    let (stored, payload) = e.expect("error populated");
    assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(payload["code"], "error.command.locked");
    assert_eq!(
        wire.expect("rejection wire")["error"]["code"],
        "error.command.locked"
    );
    assert!(
        poll(doc).len() == 1,
        "only the open seed queued; rejections emit no events"
    );
    close(doc);

    // Leg 2 (unlocked package): plain eraser succeeds; the reserved
    // eraseToHistory param is typed UNSUPPORTED ->
    // error.unsupported.erase_to_history (the other ratified arm).
    let (path, _direct, layer, asset) = stroke_package("eraser-plain");
    let cpath = std::ffi::CString::new(path.to_str().unwrap().to_owned()).unwrap();
    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cpath.as_ptr(), &mut doc, &mut err) },
        AppnameStatus::AppnameOk
    );
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000e002",
        "eraser.stroke",
        &layer,
        &asset,
        &[],
    );
    let (st, wire, _) = apply(doc, &env);
    assert_eq!(st, AppnameStatus::AppnameOk, "plain eraser applies via ABI");
    assert_eq!(wire.unwrap()["receipt"]["kind"], "eraser.stroke");

    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000e003",
        "eraser.stroke",
        &layer,
        &asset,
        &[("eraseToHistory", json!(true))],
    );
    let (st, wire, e) = apply(doc, &env);
    assert_eq!(st, AppnameStatus::AppnameErrUnsupported);
    let (stored, payload) = e.expect("error populated");
    assert_eq!(stored, AppnameStatus::AppnameErrUnsupported);
    assert_eq!(payload["code"], "error.unsupported.erase_to_history");
    assert_eq!(
        wire.expect("rejection wire")["error"]["code"],
        "error.unsupported.erase_to_history"
    );
    close(doc);
}

// ---------------------------------------------------------------------------
// CP-0.5 kind families through the ABI (wave 3): one representative per
// newly-live family, raw bytes end-to-end. Landed and armed here: mask.*
// (mask.add), doc color (assign+convert), transform.* (transform.free),
// fill. Waiting (noted, not armed): selection.set family + edit.paste*
// interactive tiers (O-TOOLS wave), remaining doc.* geometry kinds.
// ---------------------------------------------------------------------------

/// Builtin working-space profile ids (deterministic v5 constants, pinned
/// against `color_glue::builtin_profile_id` derivation — drift fails here).
const PROFILE_DISPLAY_P3: &str = "ee12374b-5f25-51fd-a1c8-f45790a61d92";
const PROFILE_SRGB: &str = "7ac4a286-7940-525f-805a-dc79e0f10fd8";

fn read_px(doc: Doc, x: i32, y: i32) -> [u8; 4] {
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_region(doc, AppnameRect { x, y, w: 1, h: 1 }, &mut buf, &mut err)
    };
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "{:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameOk
    );
    let px = unsafe { std::slice::from_raw_parts(map.data, 4) };
    let out = [px[0], px[1], px[2], px[3]];
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );
    out
}

#[test]
fn mask_family_representative_mask_add_flow() {
    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let mask_id = "aaaaaaaa-1111-4000-8000-000000000d01";
    let env = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000d01",
        "kind": "mask.add",
        "target": {"layerId": layer},
        "params": {"kind": "raster", "newMaskId": mask_id}
    })
    .to_string();
    let (st, wire, e) = apply(doc, env.as_bytes());
    assert_eq!(st, AppnameStatus::AppnameOk, "mask.add via ABI: {e:?}");
    let wire = wire.unwrap();
    assert_eq!(wire["receipt"]["kind"], "mask.add");
    assert_eq!(
        wire["receipt"]["createdIds"],
        json!([mask_id]),
        "client-generated id honored"
    );

    // Occupied slot: the ratified target_in_use arm, end-to-end.
    let env2 = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000d02",
        "kind": "mask.add",
        "target": {"layerId": layer},
        "params": {"kind": "raster", "newMaskId": "aaaaaaaa-1111-4000-8000-000000000d02"}
    })
    .to_string();
    let (st, wire2, e2) = apply(doc, env2.as_bytes());
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let (stored, payload) = e2.expect("typed err");
    assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(payload["code"], "error.command.target_in_use");
    assert_eq!(
        wire2.unwrap()["error"]["code"],
        "error.command.target_in_use"
    );

    // Undo removes the mask (suffix history on a fresh doc).
    let mut row: *mut AppnameString = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(doc, &mut row, ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    let row: Value = serde_json::from_str(&take_string(row)).unwrap();
    assert_eq!(row["kind"], "mask.add");
    close(doc);
}

#[test]
fn color_family_assign_retags_convert_transforms() {
    // Representative pair with readback proving retag-vs-transform. NOTE on
    // the oracle: readback compares raw stored bytes (the CP-0.3 interim
    // composite is not color-managed) — exactly right for asserting
    // "assign never touches pixels / convert does" (§5.3, §40.2).
    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    // Non-trivial color content (white is a gamut fixed point).
    let fill = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000c01",
        "kind": "fill",
        "target": {"layerId": layer},
        "params": {"content": "color", "color": {"model": "rgb", "comps": [0.8, 0.2, 0.1, 0.0]}}
    })
    .to_string();
    let (st, _, e) = apply(doc, fill.as_bytes());
    assert_eq!(st, AppnameStatus::AppnameOk, "fill via ABI: {e:?}");
    let _ = poll(doc);
    let px_filled = read_px(doc, 5, 5);

    // ASSIGN: retag only — zero deltas, empty receipt dirty, NO dirtyRects
    // event, readback byte-identical.
    let assign = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000c02",
        "kind": "doc.assign_profile",
        "params": {"profile": PROFILE_DISPLAY_P3}
    })
    .to_string();
    let (st, wire, e) = apply(doc, assign.as_bytes());
    assert_eq!(st, AppnameStatus::AppnameOk, "assign via ABI: {e:?}");
    assert_eq!(
        wire.unwrap()["receipt"]["dirty"],
        json!([]),
        "retag reports zero deltas"
    );
    let events = poll(doc);
    assert_eq!(events.len(), 1, "no dirtyRects for a zero-delta command");
    assert_eq!(events[0]["schema"], "appname.event.history.v1");
    assert_eq!(
        read_px(doc, 5, 5),
        px_filled,
        "assign never converts pixels"
    );

    // CONVERT: appearance-preserving pixel transform — device values
    // change, receipt dirty non-empty, dirtyRects event present.
    let convert = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000c03",
        "kind": "doc.convert_profile",
        "params": {"profile": PROFILE_SRGB}
    })
    .to_string();
    let (st, wire, e) = apply(doc, convert.as_bytes());
    assert_eq!(st, AppnameStatus::AppnameOk, "convert via ABI: {e:?}");
    let wire = wire.unwrap();
    let dirty = wire["receipt"]["dirty"].as_array().unwrap();
    assert!(!dirty.is_empty(), "convert reports full-surface deltas");
    let events = poll(doc);
    assert_eq!(events[0]["schema"], "appname.event.dirtyRects.v1");
    assert_union_coverage(dirty, &events[0]["rects"]);
    assert_ne!(
        read_px(doc, 5, 5),
        px_filled,
        "conversion changes device values"
    );
    assert_eq!(read_px(doc, 5, 5)[3], px_filled[3], "alpha untouched");

    // Typed reject from the ratified color arms: unresolvable profile.
    let bad = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000c04",
        "kind": "doc.assign_profile",
        "params": {"profile": "99999999-9999-4999-8999-999999999999"}
    })
    .to_string();
    let (st, _, e) = apply(doc, bad.as_bytes());
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.command.bad_target");
    close(doc);
}

#[test]
fn transform_family_representative_free_translate() {
    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(read_px(doc, 0, 0), [255, 255, 255, 255]);
    let env = json!({
        "schema": "appname.command.v1",
        "id": "aaaaaaaa-0000-4000-8000-000000000e01",
        "kind": "transform.free",
        "target": {"layerId": layer},
        "params": {"matrix": [1, 0, 0, 0, 1, 0, 20, 10, 1], "reference": [32, 24]}
    })
    .to_string();
    let (st, wire, e) = apply(doc, env.as_bytes());
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "transform.free via ABI: {e:?}"
    );
    let wire = wire.unwrap();
    assert_eq!(wire["receipt"]["kind"], "transform.free");
    assert!(!wire["receipt"]["dirty"].as_array().unwrap().is_empty());
    // Content moved +20/+10: origin now transparent, interior still white.
    assert_eq!(
        read_px(doc, 0, 0),
        [0, 0, 0, 0],
        "old origin now transparent"
    );
    assert_eq!(
        read_px(doc, 30, 20),
        [255, 255, 255, 255],
        "content at the offset"
    );
    close(doc);
}

#[test]
fn large_area_kinds_coalesce_within_the_cap() {
    // WATCH MINOR-9 property with REAL large-area traffic: full-canvas
    // fill and full-canvas profile conversion on a 2048x2048 document
    // (8x8 = 64 tiles) — receipts report per-tile rects (> 8), payloads
    // stay §3.7-coalesced (<= 8, union-covering).
    let doc = new_doc();
    let (st, _, e) = apply(
        doc,
        json!({
            "schema": "appname.command.v1",
            "id": "d0c00000-0000-4000-8000-000000000381",
            "kind": "doc.new",
            "params": {"width": 2048, "height": 2048, "background": "white"}
        })
        .to_string()
        .as_bytes(),
    );
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let _ = poll(doc);

    for (name, env) in [
        (
            "fill (full canvas)",
            json!({
                "schema": "appname.command.v1",
                "id": "aaaaaaaa-0000-4000-8000-000000000391",
                "kind": "fill",
                "target": {"layerId": layer},
                "params": {"content": "color", "color": {"model": "rgb", "comps": [0.1, 0.4, 0.9, 0.0]}}
            }),
        ),
        (
            "doc.convert_profile (full surface)",
            json!({
                "schema": "appname.command.v1",
                "id": "aaaaaaaa-0000-4000-8000-000000000392",
                "kind": "doc.convert_profile",
                "params": {"profile": PROFILE_DISPLAY_P3}
            }),
        ),
    ] {
        let (st, wire, e) = apply(doc, env.to_string().as_bytes());
        assert_eq!(st, AppnameStatus::AppnameOk, "{name}: {e:?}");
        let wire = wire.unwrap();
        let dirty = wire["receipt"]["dirty"].as_array().unwrap().clone();
        assert!(
            dirty.len() > appname_render::REGION_SET_CAP,
            "{name}: large-area premise — {} receipt rects",
            dirty.len()
        );
        let events = poll(doc);
        assert_eq!(events[0]["schema"], "appname.event.dirtyRects.v1");
        assert_union_coverage(&dirty, &events[0]["rects"]);
        assert_eq!(
            events[0]["rects"],
            Value::Array(expected_coalesced(&dirty)),
            "{name}: payload equals the §3.7 coalescer output"
        );
        println!(
            "{name}: {} receipt rects -> {} payload rects (cap {})",
            dirty.len(),
            events[0]["rects"].as_array().unwrap().len(),
            appname_render::REGION_SET_CAP
        );
    }
    close(doc);
}

// ---------------------------------------------------------------------------
// CP-0.5 wave-4 kind families through the ABI (close ledger): layer.*,
// merge, guides/grid, doc.duplicate's dagger design, flood fill.
// ---------------------------------------------------------------------------

fn apply_ok(doc: Doc, env: Value) -> Value {
    let (st, wire, e) = apply(doc, env.to_string().as_bytes());
    assert_eq!(st, AppnameStatus::AppnameOk, "{}: {e:?}", env["kind"]);
    wire.expect("receipt wire")
}

fn cmd(id_tail: &str, kind: &str, target_layer: Option<&str>, params: Value) -> Value {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("aaaaaaaa-0000-4000-8000-000000{id_tail}"),
        "kind": kind,
        "params": params,
    });
    if let Some(l) = target_layer {
        v["target"] = json!({"layerId": l});
    }
    v
}

#[test]
fn layer_family_add_delete_reorder_flow() {
    // doc.new's receipt exposes the root group id (createdIds[0], §40.2) —
    // the reorder parent, with zero extra surface.
    let doc = new_doc();
    let wire = apply_ok(doc, serde_json::from_slice::<Value>(DOC_NEW).unwrap());
    let root = wire["receipt"]["createdIds"][0]
        .as_str()
        .unwrap()
        .to_owned();

    let a = "aaaaaaaa-1111-4000-8000-000000000a0a";
    let b = "aaaaaaaa-1111-4000-8000-000000000b0b";
    apply_ok(
        doc,
        cmd(
            "000401",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": a, "name": "Sky"}),
        ),
    );
    let wire = apply_ok(
        doc,
        cmd(
            "000402",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": b, "name": "Sea"}),
        ),
    );
    assert_eq!(
        wire["receipt"]["createdIds"],
        json!([b]),
        "client-generated id honored"
    );
    let layers = info(doc)["layers"].clone();
    assert_eq!(layers.as_array().unwrap().len(), 3, "bg + Sky + Sea");
    assert_eq!(layers[2]["name"], "Sea", "top of root by default");

    // Reorder Sea to the bottom (index 0) of the root.
    apply_ok(
        doc,
        cmd(
            "000403",
            "layer.reorder",
            Some(b),
            json!({"parent": root, "index": 0}),
        ),
    );
    assert_eq!(
        info(doc)["layers"][0]["name"],
        "Sea",
        "index 0 = bottom of the stack"
    );

    // Delete Sky; undo restores it.
    apply_ok(doc, cmd("000404", "layer.delete", Some(a), json!({})));
    assert_eq!(info(doc)["layers"].as_array().unwrap().len(), 2);
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        info(doc)["layers"].as_array().unwrap().len(),
        3,
        "delete undone"
    );
    close(doc);
}

#[test]
fn layer_set_props_value_delta_undo() {
    let doc = init_doc();
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(info(doc)["layers"][0]["opacity"], 1.0);
    // q-tutorial/Q-7 (MAJOR): the info door publishes each layer's op-stack
    // entries, so an `op.*` command is addressable on a document the caller did
    // not author. A raster layer carries an empty list, which is what makes the
    // adjustment row below evidence rather than coincidence.
    assert_eq!(
        info(doc)["layers"][0]["ops"].as_array().map(Vec::len),
        Some(0),
        "a raster layer has no ops"
    );
    let wire = apply_ok(
        doc,
        cmd(
            "000411",
            "layer.set_props",
            Some(&layer),
            json!({"opacity": 0.8, "blend": "multiply"}),
        ),
    );
    assert_eq!(wire["receipt"]["kind"], "layer.set_props");
    let op = info(doc)["layers"][0]["opacity"].as_f64().unwrap();
    assert!(
        (op - 0.8).abs() < 1e-6,
        "opacity set (f32 storage), got {op}"
    );
    // Value-delta undo: the prior values restore exactly.
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        info(doc)["layers"][0]["opacity"],
        1.0,
        "value-delta undo restores"
    );
    close(doc);
}

#[test]
fn merge_down_with_readback() {
    let doc = init_doc(); // 64x48 white bg
    let upper = "aaaaaaaa-1111-4000-8000-000000000421";
    apply_ok(
        doc,
        cmd(
            "000421",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": upper, "name": "Paint"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000422",
            "fill",
            Some(upper),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.8, 0.1, 0.1, 0.0]}}),
        ),
    );
    let before_px = read_px(doc, 5, 5);
    assert!(
        before_px[0] > 120 && before_px[1] < 60,
        "red visible pre-merge: {before_px:?}"
    );

    let wire = apply_ok(
        doc,
        cmd("000423", "layer.merge_down", Some(upper), json!({})),
    );
    assert_eq!(wire["receipt"]["kind"], "layer.merge_down");
    assert_eq!(
        info(doc)["layers"].as_array().unwrap().len(),
        1,
        "two layers became one"
    );
    assert_eq!(
        read_px(doc, 5, 5),
        before_px,
        "merge preserves the composite (readback)"
    );
    close(doc);
}

#[test]
fn guides_family_guide_add_and_grid_set() {
    let doc = init_doc();
    let len0 = info(doc)["journal"]["length"].as_u64().unwrap();
    let wire = apply_ok(
        doc,
        cmd(
            "000431",
            "guide.add",
            None,
            json!({"orientation": "vertical", "position": 30}),
        ),
    );
    assert_eq!(wire["receipt"]["kind"], "guide.add");
    let wire = apply_ok(
        doc,
        cmd(
            "000432",
            "grid.set",
            None,
            json!({"spacing": 64, "subdivisions": 4, "show": true,
               "color": {"model": "rgb", "comps": [0.5, 0.5, 0.5, 0]}}),
        ),
    );
    assert_eq!(wire["receipt"]["kind"], "grid.set");
    assert_eq!(
        info(doc)["journal"]["length"].as_u64().unwrap(),
        len0 + 2,
        "guides/grid are journaled commands"
    );
    // Both undo cleanly (furniture is undoable state).
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    close(doc);
}

#[test]
fn doc_duplicate_dagger_property_through_the_abi() {
    // §10.3 R†: doc.duplicate's APPLY path answers typed BY DESIGN (the
    // dagger surface is Engine::duplicate — an engine door with no ABI
    // symbol yet; an additive-block candidate when the shell wants
    // in-process duplication). The dagger property observable at the ABI:
    // the apply attempt leaves NO journal record on the source, and the
    // duplicate posture (§10.6 loaded-parts: same docId + journal, empty
    // in-process history) is exactly what save→open yields.
    let doc = init_doc();
    let before = info(doc);
    let (st, _, e) = apply(
        doc,
        cmd("000441", "doc.duplicate", None, json!({}))
            .to_string()
            .as_bytes(),
    );
    assert_eq!(st, AppnameStatus::AppnameErrUnsupported);
    assert_eq!(e.unwrap().1["code"], "error.command.not_implemented");
    let after = info(doc);
    assert_eq!(
        after["journal"], before["journal"],
        "source journal untouched (†)"
    );

    // The ABI-reachable duplicate posture: save + open.
    let dir = scratch("dagger-dup");
    let path =
        std::ffi::CString::new(dir.join("dup.appname").to_str().unwrap().to_owned()).unwrap();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, path.as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    let mut dup: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(path.as_ptr(), &mut dup, &mut err) },
        AppnameStatus::AppnameOk
    );
    let dinfo = info(dup);
    assert_eq!(
        dinfo["docId"], before["docId"],
        "duplicate carries the document identity"
    );
    assert_eq!(dinfo["journal"]["length"], before["journal"]["length"]);
    assert_eq!(
        dinfo["journal"]["canUndo"], false,
        "empty in-process history (§10.6 posture)"
    );
    assert_eq!(
        info(doc)["journal"],
        before["journal"],
        "source untouched by duplication"
    );
    close(dup);
    close(doc);
}

#[test]
fn flood_fill_variant_through_the_abi() {
    // The flip landed: tools' FillKind (flood via the §22.6 shared wand
    // predicate) is the assembly's fill. Wall off the left third with a
    // brush line, flood the left compartment red, verify region exactness
    // by readback — raw bytes end-to-end.
    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let wall = sample_blob_line(&[(32.0, -8.0), (32.0, 56.0)]);
    let (st, uuid, _) = asset_create(doc, &wall);
    assert_eq!(st, AppnameStatus::AppnameOk);
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-000000000451",
        "brush.stroke",
        &layer,
        &uuid.unwrap(),
        &[
            ("color", json!({"model": "rgb", "comps": [0, 0, 1, 0]})),
            ("seed", json!(3)),
        ],
    );
    let (st, _, e) = apply(doc, &env);
    assert_eq!(st, AppnameStatus::AppnameOk, "wall stroke: {e:?}");

    let wire = apply_ok(
        doc,
        cmd(
            "000452",
            "fill",
            Some(&layer),
            json!({"content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
               "seed": [5, 30]}),
        ),
    );
    assert_eq!(wire["receipt"]["kind"], "fill");
    assert!(!wire["receipt"]["dirty"].as_array().unwrap().is_empty());
    let left = read_px(doc, 5, 30);
    assert!(
        left[0] > 229 && left[2] < 80,
        "left compartment flooded red: {left:?}"
    );
    let right = read_px(doc, 60, 30);
    assert_eq!(right, [255, 255, 255, 255], "right compartment stays white");
    let wall_px = read_px(doc, 32, 30);
    assert!(
        wall_px[2] > 229 && wall_px[0] < 80,
        "wall stays blue: {wall_px:?}"
    );
    close(doc);
}

#[test]
fn merge_visible_coalesces_within_the_cap() {
    // Coalescing spot on a whole-canvas-class op (close-ledger item 3).
    let doc = new_doc();
    let (st, _, e) = apply(
        doc,
        json!({
            "schema": "appname.command.v1",
            "id": "d0c00000-0000-4000-8000-000000000461",
            "kind": "doc.new",
            "params": {"width": 2048, "height": 2048, "background": "white"}
        })
        .to_string()
        .as_bytes(),
    );
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let upper = "aaaaaaaa-1111-4000-8000-000000000462";
    apply_ok(
        doc,
        cmd(
            "000462",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": upper, "name": "Wash"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000463",
            "fill",
            Some(upper),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.2, 0.6, 0.3, 0.0]}}),
        ),
    );
    let _ = poll(doc);

    let wire = apply_ok(doc, cmd("000464", "layer.merge_visible", None, json!({})));
    let dirty = wire["receipt"]["dirty"].as_array().unwrap().clone();
    assert!(
        dirty.len() > appname_render::REGION_SET_CAP,
        "whole-canvas premise: {} receipt rects",
        dirty.len()
    );
    let events = poll(doc);
    // merge_visible collapses the stack: treeChanged (removed) precedes
    // dirtyRects per the §9.5 normative order.
    assert_eq!(events[0]["schema"], "appname.event.treeChanged.v1");
    let ev_dirty = events
        .iter()
        .find(|e| e["schema"] == "appname.event.dirtyRects.v1")
        .expect("dirtyRects event queued");
    assert_union_coverage(&dirty, &ev_dirty["rects"]);
    assert_eq!(ev_dirty["rects"], Value::Array(expected_coalesced(&dirty)));
    println!(
        "merge_visible: {} receipt rects -> {} payload rects (cap {})",
        dirty.len(),
        ev_dirty["rects"].as_array().unwrap().len(),
        appname_render::REGION_SET_CAP
    );
    close(doc);
}

// ---------------------------------------------------------------------------
// Suffix-aligned reopened history: the O-CORE behavior table through the
// ABI (wave-2c flip; table in reports/O-CORE-log.md).
// ---------------------------------------------------------------------------

fn can_undo_redo(doc: Doc) -> (bool, bool) {
    let i = info(doc);
    (
        i["journal"]["canUndo"].as_bool().unwrap(),
        i["journal"]["canRedo"].as_bool().unwrap(),
    )
}

fn undo_status(doc: Doc) -> (AppnameStatus, Option<Value>) {
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_undo(doc, ptr::null_mut(), &mut err) };
    (st, (!err.is_null()).then(|| take_error(err).1))
}

fn redo_status(doc: Doc) -> (AppnameStatus, Option<Value>) {
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_redo(doc, ptr::null_mut(), &mut err) };
    (st, (!err.is_null()).then(|| take_error(err).1))
}

#[test]
fn reopened_history_behavior_table_rows_1_to_4() {
    let (path, _direct, layer, asset) = stroke_package("suffix-table");
    let cpath = std::ffi::CString::new(path.to_str().unwrap().to_owned()).unwrap();
    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cpath.as_ptr(), &mut doc, &mut err) },
        AppnameStatus::AppnameOk
    );

    // Row 1 — immediately after open: (false, false); both calls typed.
    assert_eq!(can_undo_redo(doc), (false, false), "row 1 capabilities");
    let (st, payload) = undo_status(doc);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let payload = payload.unwrap();
    assert_eq!(payload["code"], "error.history.nothing_to_undo");
    assert!(
        payload["message"]
            .as_str()
            .unwrap()
            .contains("not persisted"),
        "row 1 undo text"
    );
    let (st, payload) = redo_status(doc);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(payload.unwrap()["code"], "error.history.nothing_to_redo");

    // Row 2 — after >= 1 post-open command: (true, false); undo succeeds.
    let env_a = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000f0a1",
        "brush.stroke",
        &layer,
        &asset,
        &[("seed", json!(7))],
    );
    let (st, _, e) = apply(doc, &env_a);
    assert_eq!(st, AppnameStatus::AppnameOk, "stroke A: {e:?}");
    assert_eq!(can_undo_redo(doc), (true, false), "row 2 capabilities");

    // Row 4 — mid-suffix (two commands, one undone): (true, true); both
    // directions succeed within the suffix.
    let env_b = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000f0b1",
        "eraser.stroke",
        &layer,
        &asset,
        &[],
    );
    let (st, _, e) = apply(doc, &env_b);
    assert_eq!(st, AppnameStatus::AppnameOk, "stroke B: {e:?}");
    let (st, _) = undo_status(doc); // undo B -> mid-suffix
    assert_eq!(st, AppnameStatus::AppnameOk, "row 4 undo succeeds");
    assert_eq!(can_undo_redo(doc), (true, true), "row 4 capabilities");
    let (st, _) = redo_status(doc); // redo B
    assert_eq!(st, AppnameStatus::AppnameOk, "row 4 redo succeeds");

    // Back to the base: undo B, undo A — both suffix steps succeed.
    assert_eq!(undo_status(doc).0, AppnameStatus::AppnameOk);
    assert_eq!(undo_status(doc).0, AppnameStatus::AppnameOk);

    // Row 3 — back at the base: (false, true); undo typed at the boundary,
    // redo succeeds within the suffix.
    assert_eq!(can_undo_redo(doc), (false, true), "row 3 capabilities");
    let (st, payload) = undo_status(doc);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let payload = payload.unwrap();
    assert_eq!(payload["code"], "error.history.nothing_to_undo");
    assert!(
        payload["message"]
            .as_str()
            .unwrap()
            .contains("not persisted"),
        "row 3 undo text"
    );
    let (st, _) = redo_status(doc);
    assert_eq!(st, AppnameStatus::AppnameOk, "row 3 redo succeeds");
    close(doc);
}

#[test]
fn reopened_history_behavior_table_row_5_persisted_redoable_tail() {
    // A package whose checkpoint cursor sits below the journal length: the
    // persisted redoable tail belongs to the CLI replay path — both
    // capabilities false, both calls typed through the ABI.
    let mut direct = appname_tools::kind_impls::assembly().build(); // [O-CORE flag-retirement, MECHANICAL]
    direct
        .apply_bytes(
            json!({
                "schema": "appname.command.v1",
                "id": "d0c00000-0000-4000-8000-0000000000a5",
                "kind": "doc.new",
                "params": {"width": 128, "height": 128, "background": "white"}
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let asset = direct.add_samples_asset(sample_blob());
    let layer = direct.document().unwrap().z_order_walk()[0]
        .as_uuid()
        .to_string();
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000f051",
        "brush.stroke",
        &layer,
        &asset.to_string(),
        &[("seed", json!(7))],
    );
    direct.apply_bytes(&env).unwrap();
    direct.undo().unwrap(); // cursor 1 < journal 2: the tail persists
    let path = scratch("suffix-row5").join("tail.appname");
    appname_core::save_package(&direct, &path).unwrap();

    let cpath = std::ffi::CString::new(path.to_str().unwrap().to_owned()).unwrap();
    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cpath.as_ptr(), &mut doc, &mut err) },
        AppnameStatus::AppnameOk
    );
    let i = info(doc);
    assert_eq!(i["journal"]["length"], 2);
    assert_eq!(i["journal"]["cursor"], 1);
    assert_eq!(can_undo_redo(doc), (false, false), "row 5 capabilities");
    let (st, payload) = undo_status(doc);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let payload = payload.unwrap();
    assert_eq!(payload["code"], "error.history.nothing_to_undo");
    assert!(payload["message"]
        .as_str()
        .unwrap()
        .contains("not persisted"));
    let (st, payload) = redo_status(doc);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let payload = payload.unwrap();
    assert_eq!(payload["code"], "error.history.nothing_to_redo");
    assert!(
        payload["message"].as_str().unwrap().contains("replay path"),
        "row 5 redo text"
    );
    close(doc);
}

// ---------------------------------------------------------------------------
// Asset ingress (wave-2c: the §9.2 asset-block door).
// ---------------------------------------------------------------------------

#[test]
fn asset_ingress_is_typed_content_addressed_and_engine_deterministic() {
    let doc = init_doc();
    let blob = sample_blob();

    // WD-1-style determinism vs the engine door: the ABI returns exactly
    // the id `Engine::add_samples_asset` derives for the same bytes.
    let (st, uuid, _) = asset_create(doc, &blob);
    assert_eq!(st, AppnameStatus::AppnameOk);
    let uuid = uuid.expect("uuid string minted");
    let mut direct = appname_tools::kind_impls::assembly().build();
    direct
        .apply_bytes(
            json!({
                "schema": "appname.command.v1",
                "id": "d0c00000-0000-4000-8000-0000000000f1",
                "kind": "doc.new",
                "params": {"width": 8, "height": 8}
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(
        uuid,
        direct.add_samples_asset(blob.clone()).to_string(),
        "content-derived id"
    );

    // Content addressing: same bytes, same id, through the ABI too.
    let (st, again, _) = asset_create(doc, &blob);
    assert_eq!(st, AppnameStatus::AppnameOk);
    assert_eq!(again.unwrap(), uuid);

    // Reject paths, all typed (CLI `asset add` parity):
    for (name, bytes) in [
        ("garbage", vec![1u8, 2, 3, 4, 5, 6, 7, 8]),
        ("empty", Vec::new()),
        (
            "bad magic",
            b"NOTMAGIC\x01\x00\x00\x00\x01\x00\x00\x00".to_vec(),
        ),
        ("truncated header", b"APNSAMPL\x01\x00".to_vec()),
    ] {
        let (st, uuid, e) = asset_create(doc, &bytes);
        assert_eq!(st, AppnameStatus::AppnameErrInvalidArg, "case `{name}`");
        assert!(uuid.is_none(), "case `{name}`: no id minted");
        let (stored, payload) = e.unwrap_or_else(|| panic!("case `{name}`: err missing"));
        assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
        assert_eq!(
            payload["code"], "error.command.bad_target",
            "case `{name}`: {payload}"
        );
        assert!(
            payload["message"]
                .as_str()
                .unwrap()
                .contains("appname.samples.v1"),
            "case `{name}`: names the blob contract"
        );
    }

    // Boundary misuse: NULL bytes with non-zero len; NULL out_uuid.
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_asset_create(doc, ptr::null(), 12, &mut out, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(out.is_null());
    assert_eq!(take_error(err).1["code"], "error.api.invalid_arg");
    let mut err: *mut AppnameError = ptr::null_mut();
    let st =
        unsafe { appname_asset_create(doc, blob.as_ptr(), blob.len(), ptr::null_mut(), &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(take_error(err).1["code"], "error.api.invalid_arg");
    close(doc);

    // Use-after-close: typed.
    let (st, uuid, e) = asset_create(doc, &blob);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(uuid.is_none());
    assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");
}

#[test]
fn abi_only_paint_flow_ingress_stroke_readback_copy_in() {
    // The shell's exact flow, no package needed: create -> doc.new ->
    // register samples -> stroke -> painted readback. Also pins §9.3
    // copy-in semantics: the caller's buffer is dead after the call.
    let doc = init_doc(); // 64x48 white
    let mut caller_buf = sample_blob();
    let (st, uuid, _) = asset_create(doc, &caller_buf);
    assert_eq!(st, AppnameStatus::AppnameOk);
    let uuid = uuid.unwrap();
    // §9.3: bytes are copied before return — clobber the caller's copy.
    for b in caller_buf.iter_mut() {
        *b = 0xAA;
    }
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let env = stroke_bytes(
        "aaaaaaaa-0000-4000-8000-00000000c001",
        "brush.stroke",
        &layer,
        &uuid,
        &[("seed", json!(7))],
    );
    let (st, wire, e) = apply(doc, &env);
    assert_eq!(
        st,
        AppnameStatus::AppnameOk,
        "stroke on the stored copy: {e:?}"
    );
    assert_eq!(wire.unwrap()["receipt"]["kind"], "brush.stroke");
    // Painted readback at the stroke center.
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe {
            appname_doc_read_region(
                doc,
                AppnameRect {
                    x: 60,
                    y: 40,
                    w: 1,
                    h: 1,
                },
                &mut buf,
                &mut err,
            )
        },
        AppnameStatus::AppnameOk
    );
    let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameOk
    );
    let px = unsafe { std::slice::from_raw_parts(map.data, 4) };
    assert!(
        px[0] > 229 && px[1] < 153 && px[3] == 255,
        "painted, got {px:?}"
    );
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );
    close(doc);
}

#[test]
fn builtin_preset_ids_are_the_pinned_literals() {
    // The C smoke hardcodes the hard-round-16 literal; pin both built-ins
    // so preset-id drift breaks here, not in the C leg.
    assert_eq!(
        appname_tools::kind_impls::preset_hard_round_16_id().to_string(),
        "eeced2e4-ee37-587c-a8a0-8fca734089d1"
    );
    assert_eq!(
        appname_tools::kind_impls::preset_soft_round_pressure_id().to_string(),
        "7efe96cd-64b6-5e4d-835c-5dd050705583"
    );
}

// ---------------------------------------------------------------------------
// Unwind-guard census (ES §9.1 rule 2 is blanket — review minor, wave 1).
// ---------------------------------------------------------------------------

/// Mechanical enforcement of the "every extern fn is unwind-guarded" claim:
/// scan every `src/*.rs` file for `extern "C" fn <name>(` definitions,
/// chunk each definition's source (to the next definition or EOF), and
/// require a guard call — `guarded(` / `guarded_status(` / `catch_unwind(`
/// — inside the chunk. The collected name set must equal the frozen census,
/// so the assertion is exactly "every exported symbol guarded, N of N" and
/// a new unguarded extern fn (or a census drift) fails this test, not a
/// review.
#[test]
fn every_extern_fn_is_unwind_guarded_matches_census() {
    const NEEDLE: &str = "extern \"C\" fn ";
    const GUARDS: [&str; 3] = ["guarded(", "guarded_status(", "catch_unwind("];
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&src_dir).expect("read src/") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read source file");
        // Definition start offsets + names ("<name>(" required right after
        // the needle, so prose mentions in comments never match).
        let mut defs: Vec<(usize, String)> = Vec::new();
        let mut at = 0;
        while let Some(rel) = text[at..].find(NEEDLE) {
            let name_start = at + rel + NEEDLE.len();
            let name: String = text[name_start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && text[name_start + name.len()..].starts_with('(') {
                defs.push((at + rel, name));
            }
            at = name_start;
        }
        for (i, (start, name)) in defs.iter().enumerate() {
            let end = defs.get(i + 1).map_or(text.len(), |(next, _)| *next);
            let chunk = &text[*start..end];
            assert!(
                GUARDS.iter().any(|g| chunk.contains(g)),
                "{}: extern fn `{name}` has no unwind guard (ES §9.1 rule 2 is blanket)",
                path.display()
            );
            found.push(name.clone());
        }
    }
    found.sort();
    found.dedup();
    assert_eq!(
        found, V1_SYMBOLS,
        "guarded extern-fn census must equal the frozen symbol table"
    );
}

// ---------------------------------------------------------------------------
// Error paths: table-driven rejections (N.3 / §9.6 seed).
// ---------------------------------------------------------------------------

enum Setup {
    Shell,
    Initialized,
}

#[test]
fn rejection_table_yields_typed_codes_not_crashes() {
    use AppnameStatus::*;
    let oversized = vec![b'x'; 1024 * 1024 + 1];
    let cases: Vec<(&str, Setup, Vec<u8>, AppnameStatus, &str)> = vec![
        (
            "bad utf-8 bytes",
            Setup::Shell,
            vec![0xFF, 0xFE, 0x00, 0x41],
            AppnameErrInvalidArg,
            "error.api.invalid_arg",
        ),
        (
            "malformed json",
            Setup::Shell,
            b"{definitely not json".to_vec(),
            AppnameErrInvalidArg,
            "error.api.invalid_arg",
        ),
        (
            "oversized document (1 MiB + 1)",
            Setup::Shell,
            oversized,
            AppnameErrInvalidArg,
            "error.api.invalid_arg",
        ),
        (
            "wrong schema constant",
            Setup::Shell,
            br#"{"schema":"appname.command.v99","id":"aaaaaaaa-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":8,"height":8}}"#.to_vec(),
            AppnameErrSchema,
            "error.schema.wrong_schema",
        ),
        (
            "missing id",
            Setup::Shell,
            br#"{"schema":"appname.command.v1","kind":"doc.new","params":{"width":8,"height":8}}"#.to_vec(),
            AppnameErrSchema,
            "error.schema.missing_field",
        ),
        (
            "unknown kind",
            Setup::Shell,
            br#"{"schema":"appname.command.v1","id":"aaaaaaaa-0000-4000-8000-000000000002","kind":"doc.transmogrify"}"#.to_vec(),
            AppnameErrUnsupported,
            "error.command.unknown_kind",
        ),
        (
            // [Fixture move to doc.duplicate, RATIFIED by O-SCRIPT
            // CP-0.5 close (reports/O-SCRIPT-log.md): layer.add became
            // implemented at O-CORE wave 4; doc.duplicate's APPLY path is
            // not_implemented BY DESIGN (dagger surface = engine door +
            // shell verb, §10.3) — the most durable NI representative,
            // and the dagger row below pins the design.]
            "registered but unimplemented kind",
            Setup::Shell,
            br#"{"schema":"appname.command.v1","id":"aaaaaaaa-0000-4000-8000-000000000003","kind":"doc.duplicate"}"#.to_vec(),
            AppnameErrUnsupported,
            "error.command.not_implemented",
        ),
        (
            "duplicate envelope id",
            Setup::Initialized,
            DOC_NEW.to_vec(),
            AppnameErrSchema,
            "error.schema.duplicate_id",
        ),
        (
            "missing required param (width)",
            Setup::Shell,
            br#"{"schema":"appname.command.v1","id":"aaaaaaaa-0000-4000-8000-000000000004","kind":"doc.new","params":{"height":9}}"#.to_vec(),
            AppnameErrSchema,
            "error.schema.missing_field",
        ),
        (
            "out-of-range param (width 0)",
            Setup::Shell,
            br#"{"schema":"appname.command.v1","id":"aaaaaaaa-0000-4000-8000-000000000005","kind":"doc.new","params":{"width":0,"height":9}}"#.to_vec(),
            AppnameErrSchema,
            "error.schema.out_of_range",
        ),
        (
            "doc.new on an initialized document",
            Setup::Initialized,
            br#"{"schema":"appname.command.v1","id":"aaaaaaaa-0000-4000-8000-000000000006","kind":"doc.new","params":{"width":8,"height":8}}"#.to_vec(),
            AppnameErrInvalidArg,
            "error.command.bad_target",
        ),
    ];

    for (name, setup, bytes, want_status, want_code) in cases {
        let doc = match setup {
            Setup::Shell => new_doc(),
            Setup::Initialized => init_doc(),
        };
        let width_before = matches!(setup, Setup::Initialized).then(|| info(doc)["width"].clone());
        let (st, wire, err) = apply(doc, &bytes);
        assert_eq!(st, want_status, "case `{name}`: status");
        let (stored, payload) = err.unwrap_or_else(|| panic!("case `{name}`: err missing"));
        assert_eq!(stored, want_status, "case `{name}`: stored status");
        assert_eq!(
            payload["code"], want_code,
            "case `{name}`: taxonomy code, got {payload}"
        );
        let wire = wire.unwrap_or_else(|| panic!("case `{name}`: rejection wire missing"));
        assert_eq!(wire["ok"], false, "case `{name}`: wire ok flag");
        assert_eq!(wire["error"]["code"], want_code, "case `{name}`: wire code");
        // The rejection left the document exactly as it was.
        match setup {
            Setup::Shell => {
                let (st2, _, e2) = apply(doc, DOC_NEW);
                assert_eq!(
                    st2,
                    AppnameStatus::AppnameOk,
                    "case `{name}`: shell untouched {e2:?}"
                );
            }
            Setup::Initialized => {
                assert_eq!(info(doc)["width"], width_before.unwrap(), "case `{name}`");
            }
        }
        close(doc);
        println!("rejected typed: {name}");
    }
}

#[test]
fn boundary_misuse_is_typed_and_out_json_untouched() {
    // NULL doc.
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_apply_command(
            ptr::null_mut(),
            DOC_NEW.as_ptr(),
            DOC_NEW.len(),
            &mut out,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(out.is_null(), "boundary misuse leaves *out_json untouched");
    let (stored, payload) = take_error(err);
    assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(payload["code"], "error.api.invalid_arg");

    // NULL json with non-zero length.
    let doc = new_doc();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_apply_command(doc, ptr::null(), 12, &mut out, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(out.is_null());
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.api.invalid_arg");

    // Zero-length body reaches the engine as an empty document -> typed parse reject.
    let (st, wire, e) = apply(doc, b"");
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(wire.expect("engine was consulted")["ok"], false);
    assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");

    // Garbage pointer value as a handle: unknown cookie, typed.
    let garbage = 0xDEAD_BEEFusize as Doc;
    let (st, wire, e) = apply(garbage, DOC_NEW);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(wire.is_none());
    assert!(e.unwrap().1["message"]
        .as_str()
        .unwrap()
        .contains("not a live document handle"));

    // NULL out on handle-creating calls.
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe { appname_doc_new(ptr::null_mut(), &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_region(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            ptr::null_mut(),
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);
    close(doc);
}

#[test]
fn readback_error_paths_are_typed() {
    // Pre-init shell has no composite.
    let doc = new_doc();
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_region(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(buf.is_null());
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.command.bad_target");
    close(doc);

    let doc = init_doc();
    // Zero-extent rect.
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_region(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 0,
                h: 5,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.api.invalid_arg");
    assert_eq!(payload["field"], "rect");
    // Over the 2^26-px cap.
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_region(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 1 << 15,
                h: 1 << 15,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let (_, payload) = take_error(err);
    assert!(payload["message"].as_str().unwrap().contains("cap"));
    close(doc);
}

// ---------------------------------------------------------------------------
// Lifecycle misuse (N.3 seed): typed, never undefined behavior.
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_misuse_is_typed() {
    // Double-close.
    let doc = new_doc();
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
    assert_eq!(
        unsafe { appname_doc_close(doc) },
        AppnameStatus::AppnameErrInvalidArg
    );

    // Use-after-close across the surface.
    let (st, wire, e) = apply(doc, DOC_NEW);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(wire.is_none());
    assert!(e.is_some());
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_info_json(doc, &mut out, &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let _ = take_error(err);
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let _ = take_error(err);

    // String double-free and use-after-free.
    let doc = init_doc();
    let mut out: *mut AppnameString = ptr::null_mut();
    unsafe { appname_doc_history_json(doc, &mut out, ptr::null_mut()) };
    let s = out;
    let _ = take_string(s); // frees
    assert_eq!(
        unsafe { appname_string_free(s) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let mut len = 7usize;
    assert!(unsafe { appname_string_data(s, &mut len) }.is_null());
    assert_eq!(len, 0, "freed string reports zero length");

    // Buffer double-free / map-after-free.
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    unsafe {
        appname_doc_read_region(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        unsafe { appname_buffer_free(buf) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let mut map = unsafe { std::mem::zeroed::<AppnameMapInfo>() };
    assert_eq!(
        unsafe { appname_buffer_map(buf, &mut map) },
        AppnameStatus::AppnameErrInvalidArg
    );
    assert_eq!(
        unsafe { appname_buffer_unmap(buf) },
        AppnameStatus::AppnameErrInvalidArg
    );

    // Error double-free.
    let (_, _, e) = apply(doc, b"{nope");
    assert!(e.is_some()); // take_error already freed it once inside apply()
    close(doc);

    // Cross-class confusion: cookies are class-checked.
    let doc = init_doc();
    let mut out: *mut AppnameString = ptr::null_mut();
    unsafe { appname_doc_history_json(doc, &mut out, ptr::null_mut()) };
    let string_cookie = out;
    // A string handle is not a document.
    let (st, wire, _) = apply(string_cookie as Doc, DOC_NEW);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(wire.is_none());
    // A document handle is not a string / error / buffer.
    assert!(unsafe { appname_string_data(doc as *const AppnameString, ptr::null_mut()) }.is_null());
    assert_eq!(
        unsafe { appname_error_status(doc as *const AppnameError) },
        AppnameStatus::AppnameErrInvalidArg
    );
    assert!(unsafe { appname_error_json(doc as *const AppnameError, ptr::null_mut()) }.is_null());
    assert_eq!(
        unsafe { appname_buffer_free(doc as *mut AppnameImageBuffer) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let _ = take_string(string_cookie);
    close(doc);
}

// ---------------------------------------------------------------------------
// Determinism (WD-1 / D-48): ABI == direct engine, byte-equal.
// ---------------------------------------------------------------------------

#[test]
fn same_bytes_through_abi_and_engine_yield_byte_equal_receipts() {
    // Direct: the engine consumes the exact bytes.
    let mut direct = appname_core::Engine::new();
    let receipt = direct
        .apply_bytes(DOC_NEW)
        .expect("doc.new applies directly");
    let direct_wire = json!({ "ok": true, "receipt": receipt }).to_string();

    // ABI: the same bytes through the door.
    let doc = new_doc();
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_apply_command(doc, DOC_NEW.as_ptr(), DOC_NEW.len(), &mut out, &mut err)
    };
    assert_eq!(st, AppnameStatus::AppnameOk);
    let abi_wire = take_string(out);
    assert_eq!(abi_wire, direct_wire, "byte-equal receipts (WD-1)");

    // Identical document identity + created ids (deterministic §9.6 v5
    // derivation), visible through the ABI info read.
    let i = info(doc);
    assert_eq!(
        i["docId"].as_str().unwrap(),
        direct.document().unwrap().id().as_uuid().to_string()
    );

    // Identical composite: ABI readback vs the direct engine's PNG export
    // decoded through appname-core's own codec (linear-domain compare — the
    // codec's u8 sRGB round-trip is exact).
    let px = read_full(doc, 64, 48);
    let png = appname_core::export_png_bytes(&direct).unwrap();
    let (decoded, pw, ph) =
        appname_core::decode_png_to_buffer(&png, appname_core::ProfileId::nil()).unwrap();
    assert_eq!((pw, ph), (64, 48));
    for y in 0..48 {
        for x in 0..64 {
            let i = (y * 64 + x) * 4;
            let a = px[i + 3] as f32 / 255.0;
            let lin = [
                appname_core::io::srgb::srgb_decode(px[i] as f32 / 255.0) * a,
                appname_core::io::srgb::srgb_decode(px[i + 1] as f32 / 255.0) * a,
                appname_core::io::srgb::srgb_decode(px[i + 2] as f32 / 255.0) * a,
                a,
            ];
            let want = decoded.read_pixel_f32(appname_core::IVec2::new(x as i32, y as i32));
            assert_eq!(lin, want, "pixel ({x},{y})");
        }
    }
    close(doc);

    // Two ABI documents fed the same bytes are byte-identical in readback.
    let a = init_doc();
    let b = init_doc();
    assert_eq!(read_full(a, 64, 48), read_full(b, 64, 48));
    close(a);
    close(b);
}

#[test]
fn undo_redo_round_trip_through_abi_matches_engine_semantics() {
    let doc = init_doc();
    assert_eq!(info(doc)["journal"]["canUndo"], true);

    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameOk
    );
    assert!(err.is_null());
    // Back to the pre-init shell (§40.2 handle inverse): info is bad_target.
    let mut out: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_info_json(doc, &mut out, &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.command.bad_target");

    // Nothing further to undo: typed.
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.history.nothing_to_undo");

    // Redo restores; readback returns.
    assert_eq!(
        unsafe { appname_doc_redo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(info(doc)["width"], 64);
    let px = read_full(doc, 1, 1);
    assert_eq!(px, [255, 255, 255, 255]);

    // Nothing further to redo: typed.
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_redo(doc, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.history.nothing_to_redo");
    close(doc);
}

// ---------------------------------------------------------------------------
// Events (§9.5 seed).
// ---------------------------------------------------------------------------

#[test]
fn events_follow_spec_shapes_and_order() {
    let doc = init_doc();
    let events = poll(doc);
    // §9.5 NORMATIVE order treeChanged → dirtyRects → history (the
    // treeChanged join is the ABI-growth round's additive event).
    assert_eq!(
        events.len(),
        3,
        "doc.new queues treeChanged + dirtyRects + history"
    );
    let tree = &events[0];
    assert_eq!(tree["schema"], "appname.event.treeChanged.v1");
    assert_eq!(tree["thread"], "command");
    assert_eq!(tree["kind"], "added");
    assert_eq!(
        tree["layerIds"].as_array().unwrap().len(),
        1,
        "doc.new added the background layer"
    );
    let dirty = &events[1];
    assert_eq!(dirty["schema"], "appname.event.dirtyRects.v1");
    assert_eq!(dirty["thread"], "command");
    assert_eq!(dirty["scope"], "composite");
    assert_eq!(dirty["rects"], json!([[0, 0, 64, 48]]));
    let hist = &events[2];
    assert_eq!(hist["schema"], "appname.event.history.v1");
    assert_eq!(hist["undoDepth"], 1);
    assert_eq!(hist["redoDepth"], 0);
    assert_eq!(hist["labelMessageId"], "doc.new");
    assert_eq!(hist["coalesced"], false);
    assert_eq!(dirty["docId"], hist["docId"], "one document, one id");
    assert_eq!(tree["docId"], hist["docId"], "one document, one id");

    // Drained.
    assert!(poll(doc).is_empty());

    // Undo queues another ordered triple with journal-truth depths — and
    // the treeChanged is the INVERSE of the step's (added -> removed).
    unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) };
    let events = poll(doc);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["schema"], "appname.event.treeChanged.v1");
    assert_eq!(events[0]["kind"], "removed");
    assert_eq!(events[1]["schema"], "appname.event.dirtyRects.v1");
    assert_eq!(events[2]["schema"], "appname.event.history.v1");
    assert_eq!(events[2]["undoDepth"], 0);
    assert_eq!(events[2]["redoDepth"], 1);
    assert_eq!(events[2]["labelMessageId"], "doc.new");
    close(doc);
}

// ---------------------------------------------------------------------------
// Package round trip through the ABI.
// ---------------------------------------------------------------------------

#[test]
fn save_open_round_trip_preserves_identity_and_journal() {
    let dir = scratch("round-trip");
    let path = dir.join("doc.appname");
    let cpath = std::ffi::CString::new(path.to_str().unwrap().to_owned()).unwrap();

    let doc = init_doc();
    let before = info(doc);
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cpath.as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    close(doc);

    let mut reopened: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cpath.as_ptr(), &mut reopened, &mut err) },
        AppnameStatus::AppnameOk
    );
    let after = info(reopened);
    assert_eq!(after["docId"], before["docId"]);
    assert_eq!(after["width"], before["width"]);
    assert_eq!(after["height"], before["height"]);
    assert_eq!(after["journal"]["length"], before["journal"]["length"]);
    // Behavior-table row 1 (immediately after open): the persisted prefix
    // is the replay path's — both capabilities false, both calls typed.
    assert_eq!(after["journal"]["canUndo"], false);
    assert_eq!(after["journal"]["canRedo"], false);
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_undo(reopened, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    let (_, payload) = take_error(err);
    assert_eq!(payload["code"], "error.history.nothing_to_undo");
    assert!(payload["message"]
        .as_str()
        .unwrap()
        .contains("not persisted"));
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_redo(reopened, ptr::null_mut(), &mut err) },
        AppnameStatus::AppnameErrInvalidArg
    );
    assert_eq!(take_error(err).1["code"], "error.history.nothing_to_redo");
    // ...while the journal itself traveled (history read works).
    let mut out: *mut AppnameString = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_history_json(reopened, &mut out, ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    let h: Value = serde_json::from_str(&take_string(out)).unwrap();
    assert_eq!(h["entries"][0]["kind"], "doc.new");

    // The open seeded one whole-canvas dirty event for the polling host.
    let events = poll(reopened);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["schema"], "appname.event.dirtyRects.v1");
    assert_eq!(events[0]["rects"], json!([[0, 0, 64, 48]]));

    // Readback parity across the package boundary.
    let px = read_full(reopened, 64, 48);
    assert!(px.chunks(4).all(|p| p == [255, 255, 255, 255]));
    close(reopened);

    // Save misuse: unwritable path is typed IO.
    let doc = init_doc();
    let bad = std::ffi::CString::new("/nonexistent-dir/x/y/z.appname").unwrap();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, bad.as_ptr(), &mut err) },
        AppnameStatus::AppnameErrIo
    );
    let (stored, payload) = take_error(err);
    assert_eq!(stored, AppnameStatus::AppnameErrIo);
    assert_eq!(payload["code"], "error.io.write_failed");
    close(doc);
}

// ---------------------------------------------------------------------------
// ABI-growth round (the 1.2 additive block, grown for O-UI's chrome wave):
// raster ingress, export door, info v2, treeChanged, selection readback,
// recovery.
// ---------------------------------------------------------------------------

/// Tiny deterministic PNG through core's own encoder: `w x h`, solid
/// linear-premultiplied color.
fn png_bytes(w: u32, h: u32, px: [f32; 4]) -> Vec<u8> {
    let mut plane = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        plane.extend_from_slice(&px);
    }
    appname_core::encode_composite_png(w, h, &plane).expect("test PNG encodes")
}

#[test]
fn raster_asset_ingress_relaxed_and_paste_flow() {
    let doc = init_doc(); // 64x48 white
    let _ = poll(doc);

    // A PNG blob now ingests through the SAME symbol (the documented
    // additive relaxation): content-addressed to the engine's own id.
    let red = png_bytes(4, 4, [1.0, 0.0, 0.0, 1.0]);
    let (st, uuid, e) = asset_create(doc, &red);
    assert_eq!(st, AppnameStatus::AppnameOk, "PNG ingress: {e:?}");
    let uuid = uuid.expect("uuid minted");
    assert_eq!(
        uuid,
        appname_core::asset_id_for(&red).to_string(),
        "content-derived id matches the engine derivation"
    );

    // Samples blobs keep their arm (sniffed FIRST — no regression).
    let (st, s_uuid, _) = asset_create(doc, &sample_blob());
    assert_eq!(st, AppnameStatus::AppnameOk);
    assert_ne!(s_uuid.unwrap(), uuid);

    // The MAJOR-2 flow the shell unblocks on: edit.paste referencing the
    // ingested raster — journaled, tree-evented, readback-true.
    let new_layer = "aaaaaaaa-1111-4000-8000-00000000a51e";
    let wire = apply_ok(
        doc,
        cmd(
            "000a51",
            "edit.paste",
            None,
            json!({"assetId": uuid, "at": [10, 20], "newLayerId": new_layer}),
        ),
    );
    assert_eq!(wire["receipt"]["createdIds"], json!([new_layer]));
    let events = poll(doc);
    assert_eq!(
        events[0]["schema"], "appname.event.treeChanged.v1",
        "paste adds a layer: treeChanged first (§9.5 order)"
    );
    assert_eq!(events[0]["kind"], "added");
    assert_eq!(events[0]["layerIds"], json!([new_layer]));
    let px = read_px(doc, 11, 21);
    assert!(px[0] > 229 && px[1] < 26, "pasted red pixel, got {px:?}");
    assert_eq!(
        read_px(doc, 0, 0),
        [255, 255, 255, 255],
        "outside untouched"
    );

    // Garbage rejects typed, naming BOTH accepted contracts.
    let (st, none, e) = asset_create(doc, b"neither samples nor png");
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(none.is_none());
    let (_, payload) = e.unwrap();
    assert_eq!(payload["code"], "error.command.bad_target");
    let msg = payload["message"].as_str().unwrap();
    assert!(
        msg.contains("appname.samples.v1") && msg.contains("raster"),
        "reject names both contracts: {msg}"
    );
    close(doc);
}

#[test]
fn export_door_engine_bytes_and_typed_rejects() {
    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    apply_ok(
        doc,
        cmd(
            "000e51",
            "fill",
            Some(&layer),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.8, 0.2, 0.1, 0.0]}}),
        ),
    );
    let dir = scratch("export-door");

    // PNG: engine bytes — decode through core's own boundary and compare
    // against the ABI composite readback (engine truth by construction).
    let png_path = dir.join("out.png");
    let (st, e) = export(doc, &png_path, "png", None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let bytes = std::fs::read(&png_path).unwrap();
    let (decoded, w, h) =
        appname_core::decode_png_to_buffer(&bytes, appname_core::ProfileId::nil()).unwrap();
    assert_eq!((w, h), (64, 48));
    let abi_px = read_px(doc, 5, 5);
    let file_px = decoded.read_pixel_f32(appname_core::IVec2::new(5, 5));
    let a = file_px[3];
    let unpremul = |c: f32| if a > 0.0 { c / a } else { 0.0 };
    for (i, comp) in [file_px[0], file_px[1], file_px[2]].into_iter().enumerate() {
        let srgb = appname_core::io::srgb::srgb_encode(unpremul(comp).clamp(0.0, 1.0));
        let byte = (srgb * 255.0).round() as u8;
        assert_eq!(byte, abi_px[i], "component {i}: file == ABI readback");
    }

    // JPEG with the quality dial; deterministic; SOI magic.
    let jpg_path = dir.join("out.jpg");
    let (st, e) = export(doc, &jpg_path, "jpeg", Some("{\"quality\": 90}"));
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let jpg = std::fs::read(&jpg_path).unwrap();
    assert!(jpg.starts_with(&[0xFF, 0xD8]), "JPEG SOI");

    // Typed rejects: the FIXED §8.1 table end-to-end.
    for (format, options, want_status, want_code) in [
        (
            "tga",
            None,
            AppnameStatus::AppnameErrUnsupported,
            "error.codec.unrecognized",
        ),
        (
            "psd",
            None,
            AppnameStatus::AppnameErrUnsupported,
            "error.codec.write_unsupported",
        ),
        (
            "png",
            Some("{\"quality\": 90}"),
            AppnameStatus::AppnameErrInvalidArg,
            "error.codec.bad_option",
        ),
        (
            "jpeg",
            Some("{\"quality\": 101}"),
            AppnameStatus::AppnameErrInvalidArg,
            "error.codec.bad_option",
        ),
    ] {
        let (st, e) = export(doc, &dir.join("never-written"), format, options);
        assert_eq!(st, want_status, "{format} {options:?}");
        let (stored, payload) = e.expect("typed error populated");
        assert_eq!(stored, want_status);
        assert_eq!(payload["code"], want_code, "{format}: {payload}");
        assert!(!dir.join("never-written").exists(), "no partial writes");
    }

    // Unwritable path: typed IO write failure.
    let (st, e) = export(
        doc,
        std::path::Path::new("/nonexistent-dir/x/out.png"),
        "png",
        None,
    );
    assert_eq!(st, AppnameStatus::AppnameErrIo);
    assert_eq!(e.unwrap().1["code"], "error.io.write_failed");

    // Pre-init handle: typed bad_target.
    let shell = new_doc();
    let (st, e) = export(shell, &dir.join("shell.png"), "png", None);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.command.bad_target");
    close(shell);
    close(doc);
}

#[test]
fn info_v2_additive_fields_truth() {
    // Build a document exercising every v2 field.
    let doc = new_doc();
    let wire = apply_ok(doc, serde_json::from_slice::<Value>(DOC_NEW).unwrap());
    let root = wire["receipt"]["createdIds"][0]
        .as_str()
        .unwrap()
        .to_owned();

    let paint = "aaaaaaaa-1111-4000-8000-000000000f21";
    let mask = "aaaaaaaa-1111-4000-8000-000000000f22";
    apply_ok(
        doc,
        cmd(
            "000f21",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": paint, "name": "Paint"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000f22",
            "layer.set_props",
            Some(paint),
            json!({"blend": "multiply", "fillOpacity": 0.5, "locks": {"position": true}}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000f23",
            "mask.add",
            Some(paint),
            json!({"kind": "raster", "newMaskId": mask}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000f24",
            "guide.add",
            None,
            json!({"orientation": "vertical", "position": 30}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000f25",
            "grid.set",
            None,
            json!({"spacing": 64, "subdivisions": 4, "show": true}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000f26",
            "selection.set",
            None,
            json!({"source": "rect", "rect": {"x": 8, "y": 4, "w": 12, "h": 6}}),
        ),
    );

    let i = info(doc);

    // v1 fields byte-compatible (spot rows).
    assert_eq!(i["width"], 64);
    assert_eq!(i["layers"][1]["name"], "Paint");
    assert_eq!(i["layers"][1]["kind"], "raster");

    // v2 per-layer block.
    let row = &i["layers"][1];
    assert_eq!(
        row["parentId"].as_str().unwrap(),
        root,
        "top-level parent is the root group"
    );
    assert_eq!(row["depth"], 0);
    assert_eq!(row["blend"], "multiply");
    assert_eq!(row["fillOpacity"], 0.5);
    assert_eq!(row["locks"], json!(["position"]));
    assert_eq!(row["clipToBelow"], false);
    assert_eq!(row["colorLabel"], Value::Null);
    assert_eq!(row["masks"][0]["id"], mask);
    assert_eq!(row["masks"][0]["enabled"], true);
    assert_eq!(row["masks"][0]["inverted"], false);
    assert_eq!(row["masks"][0]["density"], 1.0);
    assert_eq!(row["masks"][0]["linked"], true);
    // The untouched background layer: default props, no masks.
    let bg = &i["layers"][0];
    assert_eq!(bg["blend"], "normal");
    assert_eq!(bg["fillOpacity"], 1.0);
    assert_eq!(bg["locks"], json!([]));
    assert_eq!(bg["masks"], json!([]));

    // v2 top-level block.
    assert_eq!(i["rootLayerId"].as_str().unwrap(), root);
    assert_eq!(i["depth"], "u8");
    assert_eq!(i["profile"]["source"], "builtin");
    assert_eq!(i["profile"]["name"], "builtin:srgb");
    assert!(i["profile"]["id"].is_string());
    assert_eq!(i["resolutionPpi"], 300.0, "the doc.new contract default");
    assert_eq!(i["guides"]["vertical"], json!([30.0]));
    assert_eq!(i["guides"]["horizontal"], json!([]));
    assert_eq!(i["guides"]["locked"], false);
    assert_eq!(i["grid"]["spacing"], 64.0);
    assert_eq!(i["grid"]["subdivisions"], 4);
    assert_eq!(i["grid"]["show"], true);
    assert_eq!(i["selection"]["active"], true);
    assert_eq!(i["selection"]["bounds"], json!([8, 4, 12, 6]));

    // Inactive selection: no bounds field (absent, not null-ish).
    apply_ok(doc, cmd("000f27", "selection.none", None, json!({})));
    let i = info(doc);
    assert_eq!(i["selection"]["active"], false);
    assert!(
        i["selection"].get("bounds").is_none(),
        "bounds absent while inactive"
    );
    close(doc);
}

#[test]
fn tree_changed_kinds_added_removed_moved_props() {
    let doc = new_doc();
    let wire = apply_ok(doc, serde_json::from_slice::<Value>(DOC_NEW).unwrap());
    let root = wire["receipt"]["createdIds"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let _ = poll(doc);

    // added.
    let a = "aaaaaaaa-1111-4000-8000-000000000c01".to_string();
    apply_ok(
        doc,
        cmd(
            "000c01",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": a, "name": "Sky"}),
        ),
    );
    let events = poll(doc);
    assert_eq!(events[0]["schema"], "appname.event.treeChanged.v1");
    assert_eq!(events[0]["kind"], "added");
    assert_eq!(events[0]["layerIds"], json!([a]));

    // props (rename).
    apply_ok(
        doc,
        cmd("000c02", "layer.rename", Some(&a), json!({"name": "Sea"})),
    );
    let events = poll(doc);
    assert_eq!(events[0]["kind"], "props");
    assert_eq!(events[0]["layerIds"], json!([a]));

    // moved (reorder to the bottom of the root).
    apply_ok(
        doc,
        cmd(
            "000c03",
            "layer.reorder",
            Some(&a),
            json!({"parent": root, "index": 0}),
        ),
    );
    let events = poll(doc);
    let moved = events
        .iter()
        .find(|e| e["schema"] == "appname.event.treeChanged.v1" && e["kind"] == "moved")
        .expect("reorder emits moved");
    assert!(moved["layerIds"].as_array().unwrap().contains(&json!(a)));

    // removed + the undo INVERSE (added again).
    apply_ok(doc, cmd("000c04", "layer.delete", Some(&a), json!({})));
    let events = poll(doc);
    assert_eq!(events[0]["kind"], "removed");
    assert_eq!(events[0]["layerIds"], json!([a]));
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    let events = poll(doc);
    assert_eq!(events[0]["kind"], "added", "undo emits the inverse kind");
    assert_eq!(events[0]["layerIds"], json!([a]));

    // Pixel-only commands emit NO treeChanged.
    apply_ok(
        doc,
        cmd(
            "000c05",
            "fill",
            Some(&a),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.1, 0.2, 0.3, 0.0]}}),
        ),
    );
    let events = poll(doc);
    assert!(
        events
            .iter()
            .all(|e| e["schema"] != "appname.event.treeChanged.v1"),
        "fill changes pixels, not the tree"
    );
    close(doc);
}

#[test]
fn selection_readback_gray8_truth_and_typed_rejects() {
    let doc = init_doc(); // 64x48 white

    // Marching-ants truth: a rect selection reads back exact coverage.
    apply_ok(
        doc,
        cmd(
            "000d51",
            "selection.set",
            None,
            json!({"source": "rect", "rect": {"x": 8, "y": 4, "w": 12, "h": 6}}),
        ),
    );
    let (data, map) = read_selection(doc, 0, 0, 64, 48);
    assert_eq!(
        (map.pixel_format, map.color_model, map.premultiplied),
        (0, 0, 0)
    );
    assert_eq!(map.stride, 64, "tight Gray8 stride = width");
    for y in 0..48u32 {
        for x in 0..64u32 {
            let inside = (8..20).contains(&x) && (4..10).contains(&y);
            let v = data[(y * 64 + x) as usize];
            assert_eq!(v, if inside { 255 } else { 0 }, "coverage at ({x},{y})");
        }
    }
    // The bounds in info agree with the readback (one truth).
    assert_eq!(info(doc)["selection"]["bounds"], json!([8, 4, 12, 6]));

    // Offset window: coverage is document-space, not window-space.
    let (data, _) = read_selection(doc, 7, 3, 3, 3);
    assert_eq!(data[0], 0, "(7,3) outside");
    assert_eq!(data[4], 255, "(8,4) inside");

    // selection.none -> all zero (the honest inactive answer).
    apply_ok(doc, cmd("000d52", "selection.none", None, json!({})));
    let (data, _) = read_selection(doc, 0, 0, 64, 48);
    assert!(data.iter().all(|b| *b == 0));

    // Typed rejects mirror read_region's geometry discipline.
    let mut buf: *mut AppnameImageBuffer = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_selection(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 0,
                h: 4,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(take_error(err).1["code"], "error.api.invalid_arg");
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_selection(
            doc,
            AppnameRect {
                x: 0,
                y: 0,
                w: 1 << 15,
                h: 1 << 15,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(take_error(err).1["message"]
        .as_str()
        .unwrap()
        .contains("cap"));
    close(doc);

    let shell = new_doc();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_read_selection(
            shell,
            AppnameRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            &mut buf,
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(take_error(err).1["code"], "error.command.bad_target");
    close(shell);
}

/// The W05 shell flow entirely through the ABI: base save -> autosave
/// binding -> edits -> (simulated kill: close without save) -> query ->
/// open-recovered -> verify -> discard.
#[test]
fn recovery_surface_w05_flow() {
    let dir = scratch("recovery-w05");
    let base = dir.join("base.appname");
    let log = dir.join("doc.autosave");

    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cstr(&base).as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    let (st, e) = autosave_begin(doc, &log);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");

    apply_ok(
        doc,
        cmd(
            "000f51",
            "fill",
            Some(&layer),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.9, 0.1, 0.1, 0.0]}}),
        ),
    );
    let live_px = read_px(doc, 5, 5);
    // Simulated hard kill: the handle dies without a save; the log's
    // per-command checkpoint already made the fill durable.
    close(doc);

    let q = autosave_query(&log).expect("log queries");
    assert_eq!(q["commands"], 1);
    assert_eq!(q["assets"], 0);
    assert_eq!(q["torn"], false);
    assert!(q["lastCommandId"].is_string());

    let (recovered, report) = open_recovered(&base, &log);
    assert_eq!(report["replayed"], 1);
    assert_eq!(report["torn"], false);
    assert_eq!(
        read_px(recovered, 5, 5),
        live_px,
        "recovered == live at the kill point"
    );
    assert_eq!(info(recovered)["journal"]["length"], 2, "doc.new + fill");
    // The open seeded a whole-canvas repaint for the polling host.
    let events = poll(recovered);
    assert_eq!(events[0]["schema"], "appname.event.dirtyRects.v1");
    close(recovered);

    // Discard resolves the prompt; idempotent; a discarded log is typed IO.
    assert_eq!(autosave_discard(&log), AppnameStatus::AppnameOk);
    assert_eq!(autosave_discard(&log), AppnameStatus::AppnameOk);
    let (st, _, e) = autosave_query_status(&log);
    assert_eq!(st, AppnameStatus::AppnameErrIo);
    assert_eq!(e.unwrap().1["code"], "error.io.read_failed");
}

/// Undo-then-apply truncates the journal suffix: the log rewrites so
/// recovery reproduces the LIVE journal, never the abandoned redo tail.
#[test]
fn recovery_log_rewrites_after_undo_then_apply() {
    let dir = scratch("recovery-rewrite");
    let base = dir.join("base.appname");
    let log = dir.join("doc.autosave");

    let doc = init_doc();
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cstr(&base).as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(autosave_begin(doc, &log).0, AppnameStatus::AppnameOk);

    let fill = |tail: &str, comps: [f64; 3]| {
        cmd(
            tail,
            "fill",
            Some(&layer),
            json!({"content": "color",
                    "color": {"model": "rgb", "comps": [comps[0], comps[1], comps[2], 0.0]}}),
        )
    };
    let a = apply_ok(doc, fill("000f61", [0.9, 0.1, 0.1]));
    let _b = apply_ok(doc, fill("000f62", [0.1, 0.9, 0.1]));
    let b_id = _b["receipt"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    let c = apply_ok(doc, fill("000f63", [0.1, 0.1, 0.9]));
    let live_px = read_px(doc, 5, 5);
    close(doc);

    // The rewritten log replays A then C — never B.
    let q = autosave_query(&log).expect("log queries");
    assert_eq!(q["commands"], 2, "A + C after the rewrite");

    let (recovered, report) = open_recovered(&base, &log);
    assert_eq!(report["replayed"], 2);
    assert_eq!(
        read_px(recovered, 5, 5),
        live_px,
        "recovered shows C's fill"
    );
    let mut out: *mut AppnameString = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_history_json(recovered, &mut out, ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    let h: Value = serde_json::from_str(&take_string(out)).unwrap();
    assert_eq!(h["length"], 3, "doc.new + A + C");
    let ids: Vec<&str> = h["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&a["receipt"]["id"].as_str().unwrap()));
    assert!(ids.contains(&c["receipt"]["id"].as_str().unwrap()));
    assert!(!ids.contains(&b_id.as_str()), "the undone B never recovers");
    close(recovered);
    let _ = autosave_discard(&log);
}

/// Asset frames + the §7.8 fold rule: a paste referencing a session asset
/// recovers across a save fold (§9.6 rule-2 ordering held by the binding).
#[test]
fn recovery_asset_frames_and_save_fold() {
    let dir = scratch("recovery-assets");
    let base = dir.join("base.appname");
    let log = dir.join("doc.autosave");

    let doc = init_doc();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cstr(&base).as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(autosave_begin(doc, &log).0, AppnameStatus::AppnameOk);

    // Register the asset, then FOLD (save) while it is still pending —
    // the fold must re-frame it for the paste that follows.
    let green = png_bytes(3, 3, [0.0, 1.0, 0.0, 1.0]);
    let (st, uuid, e) = asset_create(doc, &green);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let uuid = uuid.unwrap();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cstr(&base).as_ptr(), &mut err) },
        AppnameStatus::AppnameOk,
        "fold-on-save with a pending asset"
    );
    let new_layer = "aaaaaaaa-1111-4000-8000-000000000f71";
    apply_ok(
        doc,
        cmd(
            "000f71",
            "edit.paste",
            None,
            json!({"assetId": uuid, "at": [30, 30], "newLayerId": new_layer}),
        ),
    );
    let live_px = read_px(doc, 31, 31);
    close(doc);

    let q = autosave_query(&log).expect("log queries");
    assert_eq!(q["commands"], 1, "only the post-fold paste replays");
    assert_eq!(
        q["assets"], 1,
        "the pending asset was re-framed at the fold"
    );

    let (recovered, report) = open_recovered(&base, &log);
    assert_eq!(report["replayed"], 1);
    assert_eq!(
        read_px(recovered, 31, 31),
        live_px,
        "paste replays via the asset frame"
    );
    close(recovered);
    let _ = autosave_discard(&log);
}

/// Fix-round MAJOR (compositing truth): `appname_doc_read_region` and
/// `appname_doc_export` serve PIXEL-IDENTICAL results on the reviewer's
/// failing scenario — a non-normal blend (multiply) layer at opacity < 1,
/// which the retired CP-0.3 interim readback oracle composited wrong
/// (it ignored blend mode and layer opacity). Compared byte-exactly over
/// the full canvas through core's own PNG decode boundary.
#[test]
fn read_region_equals_export_pixels_on_multiply_blend_and_opacity() {
    let doc = init_doc(); // 64x48 white background
    let upper = "aaaaaaaa-1111-4000-8000-000000000fb1";
    apply_ok(
        doc,
        cmd(
            "000fb1",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": upper, "name": "Tint"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000fb2",
            "fill",
            Some(upper),
            json!({"content": "color",
                    "color": {"model": "rgb", "comps": [0.2, 0.6, 0.9, 0.0]}}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000fb3",
            "layer.set_props",
            Some(upper),
            json!({"blend": "multiply", "opacity": 0.6}),
        ),
    );

    let readback = read_full(doc, 64, 48);
    // The scenario bites: with blend/opacity IGNORED (the retired oracle),
    // the opaque upper fill would read back as its own plain encoding.
    let oracle_would_say = (appname_core::io::srgb::srgb_encode(0.2) * 255.0).round() as u8;
    assert_ne!(
        readback[(5 * 64 + 5) * 4],
        oracle_would_say,
        "multiply + opacity must be live in readback"
    );

    let path = scratch("readback-export-parity").join("parity.png");
    let (st, e) = export(doc, &path, "png", None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let bytes = std::fs::read(&path).unwrap();
    let (decoded, w, h) =
        appname_core::decode_png_to_buffer(&bytes, appname_core::ProfileId::nil()).unwrap();
    assert_eq!((w, h), (64, 48));

    for y in 0..48i32 {
        for x in 0..64i32 {
            let i = ((y * 64 + x) * 4) as usize;
            let file_px = decoded.read_pixel_f32(appname_core::IVec2::new(x, y));
            let a = file_px[3];
            let unpremul = |c: f32| if a > 0.0 { c / a } else { 0.0 };
            for (comp_i, comp) in [file_px[0], file_px[1], file_px[2]].into_iter().enumerate() {
                let srgb = appname_core::io::srgb::srgb_encode(unpremul(comp).clamp(0.0, 1.0));
                assert_eq!(
                    (srgb * 255.0).round() as u8,
                    readback[i + comp_i],
                    "({x},{y}) component {comp_i}: export file == readback"
                );
            }
            assert_eq!(
                (a.clamp(0.0, 1.0) * 255.0).round() as u8,
                readback[i + 3],
                "({x},{y}) alpha: export file == readback"
            );
        }
    }
    close(doc);
}

/// Fix-round MAJOR (recovery): assets registered BEFORE
/// `appname_doc_autosave_begin` are framed AT begin — the reviewer's exact
/// crash sequence, save → asset_create → begin → paste(assetId) → crash,
/// must recover the paste, not fail wholesale with a handle-less
/// open_recovered.
#[test]
fn recovery_frames_assets_registered_before_begin() {
    let dir = scratch("recovery-pre-begin-assets");
    let base = dir.join("base.appname");
    let log = dir.join("doc.autosave");

    let doc = init_doc(); // 64x48 white
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cstr(&base).as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );

    // The asset registers while NO binding is live...
    let blue = png_bytes(3, 3, [0.0, 0.0, 1.0, 1.0]);
    let (st, uuid, e) = asset_create(doc, &blue);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let uuid = uuid.unwrap();

    // ...then the binding begins: the already-pending asset frames NOW,
    // before any envelope can reference it (§9.6 rule 2).
    assert_eq!(autosave_begin(doc, &log).0, AppnameStatus::AppnameOk);
    let q = autosave_query(&log).expect("fresh log queries");
    assert_eq!(q["assets"], 1, "begin framed the pre-begin pending asset");
    assert_eq!(q["commands"], 0);

    // ...a paste references it, and the handle dies without a save.
    let new_layer = "aaaaaaaa-1111-4000-8000-000000000f81";
    apply_ok(
        doc,
        cmd(
            "000f81",
            "edit.paste",
            None,
            json!({"assetId": uuid, "at": [40, 10], "newLayerId": new_layer}),
        ),
    );
    let live_px = read_px(doc, 41, 11);
    close(doc);

    let q = autosave_query(&log).expect("log queries");
    assert_eq!(q["commands"], 1);
    assert_eq!(q["assets"], 1);

    let (recovered, report) = open_recovered(&base, &log);
    assert_eq!(report["replayed"], 1);
    assert_eq!(report["torn"], false);
    assert_eq!(
        read_px(recovered, 41, 11),
        live_px,
        "paste replays via the begin-framed asset"
    );
    close(recovered);
    let _ = autosave_discard(&log);
}

// ---------------------------------------------------------------------------
// ABI 1.3 block: the action-invoke door (§26.1 playback; N.1 coverage rows
// + N.3 typed error paths + §9.5 event parity).
// ---------------------------------------------------------------------------

/// Call the door; returns (status, report JSON when written, error when
/// populated).
fn apply_action(
    doc: Doc,
    action_json: &[u8],
    options: Option<&str>,
) -> (AppnameStatus, Option<Value>, Option<(AppnameStatus, Value)>) {
    let copts = options.map(|o| std::ffi::CString::new(o).unwrap());
    let mut report: *mut AppnameString = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    let st = unsafe {
        appname_doc_apply_action(
            doc,
            action_json.as_ptr(),
            action_json.len(),
            copts.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            &mut report,
            &mut err,
        )
    };
    let report_value = (!report.is_null()).then(|| {
        let text = take_string(report);
        serde_json::from_str(&text).expect("report is JSON")
    });
    (st, report_value, (!err.is_null()).then(|| take_error(err)))
}

#[test]
fn apply_action_replays_steps_with_ordinary_events() {
    let doc = init_doc();
    // Drain doc.new's events so the poll below isolates the action's.
    let _ = poll(doc);

    let action = json!({
        "schema": "appname.action.v1",
        "name": "conformance add",
        "steps": [
            { "command": { "kind": "layer.add",
                           "params": { "kind": "raster", "name": "FromAction",
                                       "newLayerId": "7a13e000-0000-4000-8000-000000000001" } } },
            { "stop": { "message": "author text", "allowContinue": true } },
            { "command": { "kind": "layer.add",
                           "params": { "kind": "raster", "name": "Disabled",
                                       "newLayerId": "7a13e000-0000-4000-8000-000000000002" } },
              "enabled": false }
        ]
    })
    .to_string();
    let (st, report, err) = apply_action(doc, action.as_bytes(), None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{err:?}");
    let report = report.expect("report written");
    assert_eq!(report["status"], "ok");
    assert_eq!(report["steps"][0]["status"], "ok");
    assert_eq!(report["steps"][0]["kind"], "layer.add");
    assert_eq!(report["steps"][1]["status"], "stop");
    assert_eq!(report["steps"][1]["message"], "author text");
    assert_eq!(report["steps"][2]["status"], "skipped");

    // Info truth: the enabled step's layer exists, the disabled one's
    // does not.
    let info = info(doc);
    let names: Vec<&str> = info["layers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["name"].as_str())
        .collect();
    assert!(names.contains(&"FromAction"), "{names:?}");
    assert!(!names.contains(&"Disabled"), "{names:?}");

    // §9.5 parity: the replayed step enqueued the ordinary event triple
    // order (treeChanged for the add, then history; a fresh raster layer
    // has no visible pixels so dirtyRects may be absent — assert the
    // schemas present are in the normative relative order).
    let events = poll(doc);
    assert!(
        !events.is_empty(),
        "replayed steps emit events like hand-applied commands"
    );
    let schemas: Vec<&str> = events.iter().filter_map(|e| e["schema"].as_str()).collect();
    assert!(
        schemas.contains(&"appname.event.treeChanged.v1"),
        "{schemas:?}"
    );
    assert!(schemas.contains(&"appname.event.history.v1"), "{schemas:?}");
    close(doc);
}

#[test]
fn apply_action_step_failure_embeds_the_report() {
    let doc = init_doc();
    let action = json!({
        "schema": "appname.action.v1",
        "name": "grain fail",
        "steps": [
            { "command": { "kind": "layer.add",
                           "params": { "kind": "raster", "name": "Applied",
                                       "newLayerId": "7a13e000-0000-4000-8000-000000000003" } } },
            { "command": { "kind": "layer.set_props",
                           "target": { "layerId": "7a13e000-dead-4000-8000-000000000004" },
                           "params": { "opacity": 0.4 } },
              "recordedTarget": { "layerName": "Grain", "treeIndex": 30 } }
        ]
    })
    .to_string();
    let (st, report, err) = apply_action(doc, action.as_bytes(), None);
    // BadTarget maps INVALID_ARG (§9.6 worked-example family).
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(
        report.is_none(),
        "error leaves the report out-param untouched"
    );
    let (stored, payload) = err.expect("error populated");
    assert_eq!(stored, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(payload["messageId"], "error.batch.step_failed");
    assert_eq!(payload["code"], "error.command.bad_target");
    assert_eq!(payload["step"], 1);
    assert!(
        payload["message"].as_str().unwrap().contains("Grain"),
        "detail names the unresolved target: {payload}"
    );
    // The embedded report shows step 0 applied, step 1 errored — actions
    // are not transactions.
    assert_eq!(payload["report"]["steps"][0]["status"], "ok");
    assert_eq!(payload["report"]["steps"][1]["status"], "error");
    let info = info(doc);
    assert!(
        info["layers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["name"] == "Applied"),
        "steps before the failure HAVE applied"
    );
    close(doc);
}

#[test]
fn apply_action_halt_options_and_typed_rejects() {
    let doc = init_doc();

    // Halt via allowContinue:false is a SUCCESSFUL call.
    let halt = json!({
        "schema": "appname.action.v1",
        "name": "halt",
        "steps": [ { "stop": { "message": "stop here", "allowContinue": false } } ]
    })
    .to_string();
    let (st, report, err) = apply_action(doc, halt.as_bytes(), None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{err:?}");
    let report = report.expect("report written");
    assert_eq!(report["status"], "halted");
    assert_eq!(report["haltedAt"], 0);

    // options_json: vars substitution reaches the applied command.
    let param = json!({
        "schema": "appname.action.v1",
        "name": "param",
        "steps": [
            { "command": { "kind": "layer.add",
                           "params": { "kind": "raster", "name": "Recorded",
                                       "newLayerId": "7a13e000-0000-4000-8000-000000000005" } },
              "parameterized": ["/params/name"] }
        ]
    })
    .to_string();
    let (st, _, err) = apply_action(
        doc,
        param.as_bytes(),
        Some(r#"{"vars":{"/params/name":"FromVars"}}"#),
    );
    assert_eq!(st, AppnameStatus::AppnameOk, "{err:?}");
    let info = info(doc);
    let names: Vec<&str> = info["layers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["name"].as_str())
        .collect();
    assert!(names.contains(&"FromVars"), "{names:?}");
    assert!(!names.contains(&"Recorded"), "{names:?}");

    // Typed rejects: unknown options key; malformed options; malformed
    // action (missing name = missing_field); NULL doc handle.
    let ok_action = json!({
        "schema": "appname.action.v1", "name": "x", "steps": []
    })
    .to_string();
    let (st, _, e) = apply_action(doc, ok_action.as_bytes(), Some(r#"{"nope":1}"#));
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");
    let (st, _, e) = apply_action(doc, ok_action.as_bytes(), Some("not json"));
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");
    let (st, _, e) = apply_action(doc, br#"{"schema":"appname.action.v1","steps":[]}"#, None);
    assert_eq!(st, AppnameStatus::AppnameErrSchema);
    assert_eq!(e.unwrap().1["code"], "error.schema.missing_field");
    let (st, _, e) = apply_action(ptr::null_mut(), ok_action.as_bytes(), None);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(e.is_some(), "NULL handle is typed, never UB");
    close(doc);
}

/// MINOR-5 pin (reviewer-verified by hand, now conformance): autosave
/// parity holds THROUGH THE ACTION DOOR. §26.1 playback rides the
/// ordinary per-command pipeline, so a bound autosave log records each
/// replayed step exactly like hand-applied commands: a 2-step action
/// yields `commands: 2`, and a hard close + open-recovered restores
/// both.
#[test]
fn apply_action_autosave_parity_two_steps_recorded_and_recovered() {
    let dir = scratch("action-autosave-parity");
    let base = dir.join("base.appname");
    let log = dir.join("doc.autosave");

    let doc = init_doc(); // 64x48 white
    let layer = info(doc)["layers"][0]["id"].as_str().unwrap().to_owned();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cstr(&base).as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(autosave_begin(doc, &log).0, AppnameStatus::AppnameOk);

    // Two command steps through the action door (id-less per §26.1 —
    // replay mints envelope ids through the ordinary pipeline).
    let fill = |comps: [f64; 3]| {
        json!({ "command": { "kind": "fill",
                             "target": { "layerId": layer.clone() },
                             "params": { "content": "color",
                                         "color": { "model": "rgb",
                                                    "comps": [comps[0], comps[1], comps[2], 0.0] } } } })
    };
    let action = json!({
        "schema": "appname.action.v1",
        "name": "two fills",
        "steps": [ fill([0.9, 0.1, 0.1]), fill([0.1, 0.8, 0.2]) ]
    })
    .to_string();
    let (st, report, e) = apply_action(doc, action.as_bytes(), None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let report = report.expect("report written");
    assert_eq!(report["status"], "ok");
    assert_eq!(report["steps"][0]["status"], "ok");
    assert_eq!(report["steps"][1]["status"], "ok");
    let live_px = read_px(doc, 5, 5);

    // Simulated hard close: the handle dies without a save.
    close(doc);

    // Parity, half 1: the log holds BOTH replayed steps as commands.
    let q = autosave_query(&log).expect("log queries");
    assert_eq!(q["commands"], 2, "one log command per replayed step: {q}");
    assert_eq!(q["torn"], false);

    // Parity, half 2: recovery restores both.
    let (recovered, rep) = open_recovered(&base, &log);
    assert_eq!(rep["replayed"], 2);
    assert_eq!(rep["torn"], false);
    assert_eq!(
        read_px(recovered, 5, 5),
        live_px,
        "recovered == live at the kill point (second fill visible)"
    );
    assert_eq!(
        info(recovered)["journal"]["length"],
        3,
        "doc.new + both action steps"
    );
    close(recovered);
    let _ = autosave_discard(&log);
}

// ---------------------------------------------------------------------------
// ABI 1.4 block: the per-layer readback door (O-UI request 7) + info v3
// paths (request 8) + the §12.1 op-registry fix on the composite calls
// (N.1 coverage rows above; the version pin rides the coverage table).
// ---------------------------------------------------------------------------

/// A doc with a red-filled raster layer above the white background:
/// (doc, background id, ink id).
fn doc_with_ink_layer() -> (Doc, String, String) {
    let doc = init_doc();
    let bg = layer_id_at(doc, 0);
    let ink = "aaaaaaaa-1111-4000-8000-00000000c001";
    apply_ok(
        doc,
        cmd(
            "000701",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": ink, "name": "Ink"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000702",
            "fill",
            Some(ink),
            json!({"content": "color",
                   "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 0.0]}}),
        ),
    );
    (doc, bg, ink.to_owned())
}

#[test]
fn read_layer_serves_content_not_contribution() {
    // The lane election pinned: the layer's OWN blend/opacity/fillOpacity
    // are NOT applied (thumbnails do not fade; Cut carries content; hosts
    // echoing a drag apply §4.4 themselves) — while the DOCUMENT composite
    // of course changes.
    let (doc, bg, ink) = doc_with_ink_layer();
    let full = read_layer_ok(doc, &ink, 0, 0, 64, 48);
    assert!(
        full.chunks(4).all(|p| p == [255, 0, 0, 255]),
        "the fill is the layer's content"
    );
    assert!(
        read_layer_ok(doc, &bg, 0, 0, 64, 48)
            .chunks(4)
            .all(|p| p == [255, 255, 255, 255]),
        "background content is untouched by the layer above"
    );
    // Delivery convention parity: outside-canvas pixels read 0,0,0,0
    // (read_region convention).
    let off = read_layer_ok(doc, &ink, -2, -2, 4, 4);
    assert_eq!(
        &off[0..4],
        [0, 0, 0, 0],
        "outside content reads transparent"
    );

    apply_ok(
        doc,
        cmd(
            "000703",
            "layer.set_props",
            Some(&ink),
            json!({"opacity": 0.5, "blend": "multiply"}),
        ),
    );
    let after_props = read_layer_ok(doc, &ink, 0, 0, 64, 48);
    assert_eq!(
        full, after_props,
        "content readback ignores the layer's own compositing props"
    );
    assert_ne!(
        read_px(doc, 5, 5),
        [255, 0, 0, 255],
        "the document composite DOES change with the props"
    );
    close(doc);
}

#[test]
fn read_layer_refusal_matrix_is_typed() {
    let (doc, _bg, ink) = doc_with_ink_layer();

    // Unknown layer id: bad_target.
    let (st, _, e) = read_layer(doc, "aaaaaaaa-9999-4000-8000-00000000dead", 0, 0, 4, 4);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.command.bad_target");

    // Not a uuid at all: boundary invalid_arg.
    let (st, _, e) = read_layer(doc, "not-a-uuid", 0, 0, 4, 4);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");

    // Shared geometry discipline: zero extent + the pixel cap.
    let (st, _, e) = read_layer(doc, &ink, 0, 0, 0, 4);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.api.invalid_arg");
    let (st, _, e) = read_layer(doc, &ink, 0, 0, 1 << 15, 1 << 15);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let msg = e.unwrap().1;
    assert!(msg["message"].as_str().unwrap().contains("cap"), "{msg}");

    // Group target: SERVES since the return leg (the recorded render-door
    // gap closed additively under the same symbol — the dedicated group
    // battery below carries the semantics; here just the flip itself: the
    // one-member group's content is its child's content).
    let group = "aaaaaaaa-1111-4000-8000-00000000c0f0";
    apply_ok(
        doc,
        cmd(
            "000711",
            "layer.group",
            None,
            json!({"layerIds": [ink], "newGroupId": group, "name": "Folder"}),
        ),
    );
    assert_eq!(
        read_layer_ok(doc, group, 0, 0, 4, 4),
        read_layer_ok(doc, &ink, 0, 0, 4, 4),
        "the former typed-NI branch now serves (red fill through the group)"
    );

    // Adjustment target: its §17.1 result exists only over a backdrop —
    // target_kind_mismatch (the ratified INVALID_ARG family).
    let adj_layer = "aaaaaaaa-1111-4000-8000-00000000c0a1";
    let adj_op = "aaaaaaaa-1111-4000-8000-00000000c0a2";
    apply_ok(
        doc,
        cmd(
            "000712",
            "adjustment.add",
            None,
            json!({"opKind": "adjustment.invert", "newLayerId": adj_layer,
                   "newOpId": adj_op, "params": {}}),
        ),
    );
    let (st, _, e) = read_layer(doc, adj_layer, 0, 0, 4, 4);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.command.target_kind_mismatch");
    close(doc);

    // Pre-init handle: bad_target.
    let shell = new_doc();
    let (st, _, e) = read_layer(shell, "aaaaaaaa-9999-4000-8000-00000000dead", 0, 0, 4, 4);
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(e.unwrap().1["code"], "error.command.bad_target");
    close(shell);
}

#[test]
fn read_layer_smart_object_serves_the_evaluated_chain() {
    // §12.4 identity: a chain-less smart object serves its proxy; §12.1:
    // an ENABLED chain serves the EVALUATED result (never the raw proxy);
    // op.toggle off restores the §12.4 identity bit-exactly.
    let doc = init_doc();
    let bg = layer_id_at(doc, 0);
    let smart = "aaaaaaaa-1111-4000-8000-00000000d001";
    apply_ok(
        doc,
        cmd(
            "000721",
            "smartobject.convert",
            None,
            // newAssetId omitted: asset ids are content-addressed (§7.4,
            // QA-F7 accept-if-equal ruling) — the engine derives it.
            json!({"layerIds": [bg], "newLayerId": smart}),
        ),
    );
    let proxy_px = read_layer_ok(doc, smart, 0, 0, 64, 48);
    assert!(
        proxy_px.chunks(4).all(|p| p == [255, 255, 255, 255]),
        "§12.4: no enabled entries serves the value-captured proxy"
    );

    let op = "aaaaaaaa-1111-4000-8000-00000000d003";
    // filter.custom as an exact inversion: out = clamp(-src + 1) per
    // color channel, alpha preserved (§16.3 custom-convolve math) —
    // deterministic and parameter-exact.
    let invert_kernel = json!([
        [0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0],
        [0, 0, -1, 0, 0],
        [0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0]
    ]);
    apply_ok(
        doc,
        cmd(
            "000722",
            "filter.smart_add",
            Some(smart),
            json!({"filterId": "filter.custom",
                   "params": {"kernel": invert_kernel, "divisor": 1, "offset": 1},
                   "newOpId": op}),
        ),
    );
    let evaluated = read_layer_ok(doc, smart, 0, 0, 64, 48);
    assert!(
        evaluated.chunks(4).all(|p| p == [0, 0, 0, 255]),
        "§12.1: the ENABLED chain's evaluated result (inverted white), never the raw proxy"
    );
    // The document composite agrees (the §12.1 registry rides the region
    // readback too — the consolidated round's op-registry fix).
    assert_eq!(read_px(doc, 5, 5), [0, 0, 0, 255]);

    apply_ok(
        doc,
        cmd(
            "000723",
            "op.toggle",
            Some(smart),
            json!({"opId": op, "enabled": false}),
        ),
    );
    assert_eq!(
        read_layer_ok(doc, smart, 0, 0, 64, 48),
        proxy_px,
        "§12.4 defining invariant: off == never-applied, bit-identical"
    );
    close(doc);
}

// ---------------------------------------------------------------------------
// THE GROUP ARM battery (return leg): `appname_doc_read_layer` on group
// targets over `appname_render::composite_subtree_with` — the recorded
// handoff closed additively under the same symbol (census unchanged). The
// AS-IF-ISOLATED ELECTION is the contract under test: a group's content is
// its children composited as-if-isolated onto transparency; the group's own
// contribution props are never applied; the §11.4 boundary refuses.
// ---------------------------------------------------------------------------

/// Build the nested children set under `doc`'s root: opaque blue fill `B1`,
/// then isolated subgroup `H` [ red fill `R1` @ opacity 0.5 ] — the nested
/// oracle shape (an in-subtree §4.7 row-1 isolation with a faded child, so
/// within-subtree contribution semantics are actually exercised).
fn build_nested_children(doc: Doc) -> (String, String) {
    let b1 = "aaaaaaaa-1111-4000-8000-00000000e001";
    let r1 = "aaaaaaaa-1111-4000-8000-00000000e002";
    let h = "aaaaaaaa-1111-4000-8000-00000000e003";
    apply_ok(
        doc,
        cmd(
            "000751",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": b1, "name": "Blue"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000752",
            "fill",
            Some(b1),
            json!({"content": "color",
                   "color": {"model": "rgb", "comps": [0.0, 0.0, 1.0, 0.0]}}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000753",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": r1, "name": "Red"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000754",
            "fill",
            Some(r1),
            json!({"content": "color",
                   "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 0.0]}}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000755",
            "layer.set_props",
            Some(r1),
            json!({"opacity": 0.5}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000756",
            "layer.group",
            None,
            json!({"layerIds": [r1], "newGroupId": h, "name": "H"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000757",
            "layer.set_props",
            Some(h),
            json!({"isolated": true}),
        ),
    );
    (b1.to_owned(), h.to_owned())
}

#[test]
fn read_layer_group_serves_as_if_isolated_content_nested_oracle() {
    // The oracle: the group readback must equal the WHOLE-DOCUMENT
    // composite of a reference document holding exactly the group's
    // children over a transparent background — "children composited
    // as-if-isolated" made literal, nested §4.7 isolation and child
    // opacity included, byte-for-byte through two independent ABI doors.
    let doc = init_doc(); // 64x48 white
    let (b1, h) = build_nested_children(doc);
    let g = "aaaaaaaa-1111-4000-8000-00000000e004";
    apply_ok(
        doc,
        cmd(
            "000758",
            "layer.group",
            None,
            json!({"layerIds": [b1, h], "newGroupId": g, "name": "G"}),
        ),
    );
    let got = read_layer_ok(doc, g, 0, 0, 64, 48);
    close(doc);

    let oracle = new_doc();
    apply_ok(
        oracle,
        cmd(
            "000750",
            "doc.new",
            None,
            json!({"width": 64, "height": 48, "background": "transparent"}),
        ),
    );
    build_nested_children(oracle);
    let want = read_region_ok(oracle, 0, 0, 64, 48);
    close(oracle);

    assert_eq!(
        got, want,
        "group content == the reference composite of its children onto transparency"
    );
    // Coarse pixel truth: red@0.5 (through the isolated subgroup) over
    // opaque blue = a purple — both channels present, green untouched,
    // opaque (the nested blend really happened inside the subtree).
    let p = &got[0..4];
    assert!(p[0] > 0 && p[2] > 0 && p[1] == 0 && p[3] == 255, "{p:?}");
}

#[test]
fn read_layer_hidden_and_faded_group_serves_content() {
    // Content, not contribution — the group edition of the pinned lane
    // election: the target group's OWN visible/opacity/blend never change
    // the served bytes, while the document composite of course changes.
    let doc = init_doc();
    let (b1, h) = build_nested_children(doc);
    let g = "aaaaaaaa-1111-4000-8000-00000000e004";
    apply_ok(
        doc,
        cmd(
            "000758",
            "layer.group",
            None,
            json!({"layerIds": [b1, h], "newGroupId": g, "name": "G"}),
        ),
    );
    let before = read_layer_ok(doc, g, 0, 0, 64, 48);
    apply_ok(
        doc,
        cmd(
            "000759",
            "layer.set_props",
            Some(g),
            json!({"visible": false, "opacity": 0.25, "blend": "multiply"}),
        ),
    );
    assert_eq!(
        read_layer_ok(doc, g, 0, 0, 64, 48),
        before,
        "hidden + faded + non-Normal group still serves its content unchanged"
    );
    assert_eq!(
        read_px(doc, 5, 5),
        [255, 255, 255, 255],
        "while the DOCUMENT composite dropped the hidden group (white bg)"
    );
    close(doc);
}

#[test]
fn read_layer_group_scoped_adjustment_refuses_typed_with_scope_named() {
    // The §11.4 boundary: an adjustment's §17.1 result over the door's
    // transparency backdrop is not its document content — refused typed,
    // never approximated, with the isolating scope NAMED in the message
    // (the render door's election, surfaced through the cabi's standing
    // typed arm unchanged).
    let doc = init_doc();
    let ink = "aaaaaaaa-1111-4000-8000-00000000e011";
    apply_ok(
        doc,
        cmd(
            "000761",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": ink, "name": "Ink"}),
        ),
    );
    let adj_layer = "aaaaaaaa-1111-4000-8000-00000000e012";
    let adj_op = "aaaaaaaa-1111-4000-8000-00000000e013";
    apply_ok(
        doc,
        cmd(
            "000762",
            "adjustment.add",
            None,
            json!({"opKind": "adjustment.invert", "newLayerId": adj_layer,
                   "newOpId": adj_op, "params": {}}),
        ),
    );
    let g = "aaaaaaaa-1111-4000-8000-00000000e014";
    apply_ok(
        doc,
        cmd(
            "000763",
            "layer.group",
            None,
            json!({"layerIds": [ink, adj_layer], "newGroupId": g, "name": "G"}),
        ),
    );
    // Adjustment directly in the target scope: the door's own subtree
    // scope, named as such.
    let (st, _, e) = read_layer(doc, g, 0, 0, 8, 8);
    assert_eq!(st, AppnameStatus::AppnameErrUnsupported);
    let msg = e.unwrap().1;
    assert_eq!(msg["code"], "error.command.not_implemented");
    assert!(
        msg["message"]
            .as_str()
            .unwrap()
            .contains("subtree-composite door's target scope"),
        "the refusing scope is named: {msg}"
    );
    // Adjustment nested under an ISOLATING subgroup inside the target:
    // the §4.7 row that isolates is named instead.
    let h = "aaaaaaaa-1111-4000-8000-00000000e015";
    apply_ok(
        doc,
        cmd(
            "000764",
            "layer.group",
            None,
            json!({"layerIds": [adj_layer], "newGroupId": h, "name": "H"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000765",
            "layer.set_props",
            Some(h),
            json!({"isolated": true}),
        ),
    );
    let (st, _, e) = read_layer(doc, g, 0, 0, 8, 8);
    assert_eq!(st, AppnameStatus::AppnameErrUnsupported);
    let msg = e.unwrap().1;
    assert_eq!(msg["code"], "error.command.not_implemented");
    assert!(
        msg["message"].as_str().unwrap().contains("§4.7 row 1"),
        "the isolating §4.7 row is named: {msg}"
    );
    close(doc);
}

#[test]
fn read_layer_root_as_group_equals_whole_document_readback() {
    // The identity contract: the root group's content IS the document
    // composite (`composite_subtree_with(root)` delegates to the document
    // arm by construction) — including ROOT-SCOPE ADJUSTMENT SERVING via
    // the §11.4 segment pipeline, which subtree-target scopes refuse.
    let doc = new_doc();
    let wire = apply_ok(doc, serde_json::from_slice::<Value>(DOC_NEW).unwrap());
    let root = wire["receipt"]["createdIds"][0]
        .as_str()
        .expect("doc.new reports the root group id (§40.2)")
        .to_owned();
    let ink = "aaaaaaaa-1111-4000-8000-00000000e021";
    apply_ok(
        doc,
        cmd(
            "000771",
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": ink, "name": "Ink"}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000772",
            "fill",
            Some(ink),
            json!({"content": "color",
                   "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 0.0]}}),
        ),
    );
    apply_ok(
        doc,
        cmd(
            "000773",
            "adjustment.add",
            None,
            json!({"opKind": "adjustment.invert",
                   "newLayerId": "aaaaaaaa-1111-4000-8000-00000000e022",
                   "newOpId": "aaaaaaaa-1111-4000-8000-00000000e023",
                   "params": {}}),
        ),
    );
    let via_layer = read_layer_ok(doc, &root, 0, 0, 64, 48);
    let via_region = read_region_ok(doc, 0, 0, 64, 48);
    assert_eq!(
        via_layer, via_region,
        "root-as-group == whole-document readback, root-scope adjustment included"
    );
    assert_eq!(
        &via_layer[0..4],
        [0, 255, 255, 255],
        "the adjustment really evaluated (red fill inverted to cyan)"
    );
    close(doc);
}

#[test]
fn adjustment_documents_serve_through_region_readback_and_export() {
    // The consolidated round's op-registry fix, pinned red-to-green: the
    // composite calls previously carried a BARE registry, so this document
    // REFUSED through the ABI while the CLI served it. Now: one truth.
    let doc = init_doc();
    apply_ok(
        doc,
        cmd(
            "000731",
            "adjustment.add",
            None,
            json!({"opKind": "adjustment.invert",
                   "newLayerId": "aaaaaaaa-1111-4000-8000-00000000e001",
                   "newOpId": "aaaaaaaa-1111-4000-8000-00000000e002",
                   "params": {}}),
        ),
    );
    // Readback serves the §17.1 result over the backdrop: inverted white.
    let px = read_full(doc, 64, 48);
    assert!(
        px.chunks(4).all(|p| p == [0, 0, 0, 255]),
        "adjustment layers serve through the ABI region readback"
    );
    // Export serves through BOTH arms: png (core's parametric-aware
    // reference path) and jpeg (the general compositor path with the
    // engine's op registry).
    let dir = scratch("adjustment-export");
    let (st, e) = export(doc, &dir.join("adj.png"), "png", None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    let (st, e) = export(doc, &dir.join("adj.jpg"), "jpeg", None);
    assert_eq!(st, AppnameStatus::AppnameOk, "{e:?}");
    // Readback == export truth through core's own decode boundary.
    let png = std::fs::read(dir.join("adj.png")).expect("exported png");
    let (decoded, pw, ph) =
        appname_core::decode_png_to_buffer(&png, appname_core::ProfileId::nil()).expect("decodes");
    assert_eq!((pw, ph), (64, 48));
    assert_eq!(
        decoded.read_pixel_f32(appname_core::IVec2::new(5, 5)),
        [0.0, 0.0, 0.0, 1.0],
        "export carries the adjustment"
    );
    close(doc);
}

#[test]
fn info_v3_paths_array_truth() {
    // Fresh documents: the v3 key is ALWAYS present, empty.
    let doc = init_doc();
    let i = info(doc);
    assert_eq!(i["paths"], json!([]), "paths is present-and-empty: {i}");
    close(doc);

    // Populated arena. SERVICE-FLIP (return leg): the real VectorGeometry
    // service rides `production_assembly()`, so every truth the live doors
    // can create is now exercised THROUGH the live doors — named +
    // unreferenced rows via `vector.path_new`, the base-reference +
    // null-name + derived-bounds truths via `vector.shape_add` (both legs
    // below). The package craft (lock-injection precedent: save via the
    // ABI, rewrite with the writer's own §20 wire forms + REAL validated
    // appname.vector.v1 sidecars, reopen via the ABI) is KEPT ONLY for
    // the recorded gap: no command creates §20.3 bool_ops rows yet
    // (`vector.path_bool` shape-mode APPEND is the escalated-NI contract
    // gap; doc-path mode bakes) — the crafted fixture narrows to exactly
    // the operand/`inBoolOps` truths, nothing else.
    let doc = init_doc();
    let dir = scratch("info-v3-paths");
    let pkg = dir.join("paths.appname");
    let cpath = cstr(&pkg);
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_save(doc, cpath.as_ptr(), &mut err) },
        AppnameStatus::AppnameOk
    );
    close(doc);

    let p1 = "bbbbbbbb-2222-4000-8000-000000000001"; // named, base path
    let p2 = "bbbbbbbb-2222-4000-8000-000000000002"; // unnamed, bool operand
    let shape = "bbbbbbbb-2222-4000-8000-00000000000f";
    inject_paths_and_shape(&pkg, p1, p2, shape);

    let mut doc: Doc = ptr::null_mut();
    let mut err: *mut AppnameError = ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_open(cstr(&pkg).as_ptr(), &mut doc, &mut err) },
        AppnameStatus::AppnameOk,
        "{:?}",
        (!err.is_null()).then(|| take_error(err))
    );
    let i = info(doc);
    let rows = i["paths"].as_array().expect("paths array");
    assert_eq!(rows.len(), 2, "the crafted bool_ops fixture rows: {i}");
    // Deterministic uuid order.
    assert_eq!(rows[0]["id"], p1);
    assert_eq!(rows[1]["id"], p2);
    // Names: string or null.
    assert_eq!(rows[0]["name"], "Outline");
    assert_eq!(rows[1]["name"], Value::Null);
    // Bounds: the validated sidecar's integer hull as [x, y, w, h].
    assert_eq!(rows[0]["bounds"], json!([2, 3, 20, 10]));
    // Shape-layer references: base path AND bool operand both list the
    // shape layer.
    assert_eq!(rows[0]["referencedBy"], json!([shape]));
    assert_eq!(rows[1]["referencedBy"], json!([shape]));
    // boolOps presence — THE crafted-only truth (the recorded gap): only
    // the operand reference sets it.
    assert_eq!(rows[0]["inBoolOps"], false);
    assert_eq!(rows[1]["inBoolOps"], true);
    // The shape layer itself is a v1/v2 layer row (kind "shape").
    let layers = i["layers"].as_array().unwrap();
    assert_eq!(layers.last().unwrap()["kind"], "shape");

    // LIVE leg 1 (Door-1 rebase): vector.path_new through the single
    // mutation entry grows the panel truth — the named + unreferenced row
    // served by the REAL registered service.
    let p4 = "bbbbbbbb-2222-4000-8000-000000000004";
    apply_ok(
        doc,
        cmd(
            "000741",
            "vector.path_new",
            None,
            json!({"path": rect_vector_path(30.0, 30.0, 10.0, 8.0),
                   "name": "Live", "newPathId": p4}),
        ),
    );
    let rows_after = info(doc)["paths"].clone();
    let rows_after = rows_after.as_array().unwrap();
    assert_eq!(rows_after.len(), 3, "door-created path joins the array");
    let live = rows_after
        .iter()
        .find(|r| r["id"] == p4)
        .expect("live row present");
    assert_eq!(live["name"], "Live");
    assert_eq!(
        live["bounds"],
        json!([30, 30, 10, 8]),
        "bounds = the validator's integer hull"
    );
    assert_eq!(live["referencedBy"], json!([]));
    assert_eq!(live["inBoolOps"], false);

    // LIVE leg 2 (service flip, return leg): vector.shape_add derives the
    // shape's base path through the REAL service — base-reference,
    // null-name, and derived-bounds truths now come off the live door
    // (the crafted fixture no longer carries them).
    let shape2 = "bbbbbbbb-2222-4000-8000-000000000010";
    let wire = apply_ok(
        doc,
        cmd(
            "000742",
            "vector.shape_add",
            None,
            json!({"path": rect_vector_path(40.0, 10.0, 12.0, 6.0),
                   "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0.0, 0.0, 0.0, 1.0]}}, "stroke": {"widthPx": 1.0},
                   "newLayerId": shape2}),
        ),
    );
    let derived = wire["receipt"]["createdIds"][1]
        .as_str()
        .expect("shape_add reports layer + its derived base path")
        .to_owned();
    let rows_after = info(doc)["paths"].clone();
    let rows_after = rows_after.as_array().unwrap();
    assert_eq!(rows_after.len(), 4, "the derived base path joins the array");
    let row = rows_after
        .iter()
        .find(|r| r["id"] == derived.as_str())
        .expect("derived-path row present");
    assert_eq!(row["name"], Value::Null, "derived base paths are unnamed");
    assert_eq!(
        row["bounds"],
        json!([40, 10, 12, 6]),
        "bounds = the live validator's integer hull"
    );
    assert_eq!(
        row["referencedBy"],
        json!([shape2]),
        "the base reference through the live door"
    );
    assert_eq!(row["inBoolOps"], false);

    // Undo shrinks the panel truth (re-read is undo-safe) — both live
    // legs unwind through the ordinary history door.
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        unsafe { appname_doc_undo(doc, ptr::null_mut(), ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    assert_eq!(
        info(doc)["paths"].as_array().unwrap().len(),
        2,
        "undo x2 shrinks the panel truth back to the crafted rows"
    );
    close(doc);
}

/// Rewrite a saved package to carry a §20 paths inventory (2 paths), their
/// vector sidecars, and a shape layer referencing p1 (base) + p2 (boolean
/// operand) — public-doors conformance scaffolding (the lock-injection
/// precedent; the writer's own wire forms, package.rs). NARROWED at the
/// service flip: this craft exists ONLY for the §20.3 bool_ops truths (no
/// command creates them yet — the recorded gap); every other row truth
/// rides the live vector doors in the test above.
fn inject_paths_and_shape(path: &std::path::Path, p1: &str, p2: &str, shape: &str) {
    use std::io::{Read, Write};
    let bytes = std::fs::read(path).expect("read package");
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).expect("zip opens");
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..za.len() {
        let mut entry = za.by_index(i).expect("entry");
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("entry bytes");
        entries.push((entry.name().to_string(), data));
    }
    for (name, data) in entries.iter_mut() {
        if name == "document.json" {
            let mut v: Value = serde_json::from_slice(data).expect("document.json parses");
            let root = v["root"].as_str().expect("root id").to_owned();
            v["paths"] = json!([
                {"id": p1, "name": "Outline", "pathRef": p1,
                 "bounds": {"x": 2, "y": 3, "w": 20, "h": 10}},
                {"id": p2, "pathRef": p2,
                 "bounds": {"x": 0, "y": 0, "w": 8, "h": 8}},
            ]);
            let layers = v["layers"].as_array_mut().expect("layers array");
            layers.push(json!({
                "id": shape,
                "name": "Vector",
                "kind": "shape",
                "pathRef": p1,
                "boolOps": [{"op": "subtract", "pathId": p2}],
            }));
            for l in v["layers"].as_array_mut().expect("layers array") {
                if l["id"] == root.as_str() {
                    l["children"]
                        .as_array_mut()
                        .expect("root children")
                        .push(json!(shape));
                }
            }
            let mut out = serde_json::to_vec_pretty(&v).expect("re-serialize");
            out.push(b'\n');
            *data = out;
        }
    }
    for (pid, rect) in [(p1, (2.0, 3.0, 20.0, 10.0)), (p2, (0.0, 0.0, 8.0, 8.0))] {
        let (x, y, w, h) = rect;
        entries.push((format!("vector/{pid}.json"), rect_sidecar(x, y, w, h)));
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zw.start_file(name, opts).expect("start entry");
            zw.write_all(&data).expect("write entry");
        }
        zw.finish().expect("finish zip");
    }
    std::fs::write(path, cursor.into_inner()).expect("write rewritten package");
}

/// One cubic segment for a straight edge (collinear 1/3-2/3 controls —
/// the §20.1 straight-segment convention).
fn straight_seg(ax: f64, ay: f64, bx: f64, by: f64) -> Value {
    json!({
        "p0": [ax, ay],
        "c1": [ax + (bx - ax) / 3.0, ay + (by - ay) / 3.0],
        "c2": [ax + 2.0 * (bx - ax) / 3.0, ay + 2.0 * (by - ay) / 3.0],
        "p1": [bx, by],
    })
}

/// A closed axis-aligned rectangle as a vectorPath param object
/// (`fillRule` + contiguous cubic subpath — the real §20.1 wire form the
/// live VectorGeometry validator accepts; integer hull = the rect).
fn rect_vector_path(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({
        "fillRule": "nonzero",
        "subpaths": [{"closed": true, "segments": [
            straight_seg(x, y, x + w, y),
            straight_seg(x + w, y, x + w, y + h),
            straight_seg(x + w, y + h, x, y + h),
            straight_seg(x, y + h, x, y),
        ]}],
    })
}

/// The same rectangle as `appname.vector.v1` SIDECAR bytes (the param
/// object plus the schema stamp — the vector/{id}.json wire form).
fn rect_sidecar(x: f64, y: f64, w: f64, h: f64) -> Vec<u8> {
    let mut v = rect_vector_path(x, y, w, h);
    v["schema"] = json!("appname.vector.v1");
    v.to_string().into_bytes()
}

// ---------------------------------------------------------------------
// COMPOSITE-LEGS SLICE (O-INT-brokered): the parametric shape/text serve
// matrix through the SHIPPED cabi doors — O-UI's sharpened probe (readback
// ALWAYS refused; export split on raster count and format). Post-fix every
// cell SERVES uniformly through render; the raster-count fast-path branch
// only survives for NON-parametric documents.
// ---------------------------------------------------------------------

/// Append a second raster layer (flips `rasters <= 1` false — the old
/// fast-path escape hatch that used to make a 2+-raster parametric export
/// REFUSE at the render catch-all).
fn add_second_raster(doc: Doc, tail: &str, lid: &str) {
    apply_ok(
        doc,
        cmd(
            tail,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": lid}),
        ),
    );
}

fn tmp_export_path(ext: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "appname-conf-legs-{}.{ext}",
        uuid::Uuid::new_v4().simple()
    ))
}

/// O-UI's DOCUMENT-STATE matrix, post-fix. For a shape doc and a text doc,
/// at 1-raster AND 2-raster counts: the cabi READBACK door (§9.2), the
/// PNG export door, and a NON-PNG (bmp) export door ALL serve — no
/// serve↔refuse flip on raster count or format (the three-way disagreement
/// the diagnosis recorded is collapsed). Readback (`read_full`) asserts OK
/// internally; export is asserted OK here.
#[test]
fn parametric_shape_text_serve_across_the_full_document_state_matrix() {
    // params: (label, apply the parametric layer)
    let shape = |doc: Doc| {
        apply_ok(
            doc,
            cmd(
                "00a101",
                "vector.shape_add",
                None,
                json!({
                    "path": rect_vector_path(8.0, 8.0, 24.0, 16.0),
                    "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0.0, 0.0, 0.0, 1.0]}},
                    "stroke": {"widthPx": 0.0},
                    "newLayerId": "aaaaaaaa-0000-4000-8000-0000000005a1",
                }),
            ),
        );
    };
    let text = |doc: Doc| {
        apply_ok(
            doc,
            cmd(
                "00a201",
                "text.add",
                None,
                json!({
                    "layout": "point",
                    "origin": [10.0, 20.0],
                    "text": "Ag",
                    "newLayerId": "aaaaaaaa-0000-4000-8000-0000000007a1",
                }),
            ),
        );
    };

    for (kind, build) in [
        ("shape", &shape as &dyn Fn(Doc)),
        ("text", &text as &dyn Fn(Doc)),
    ] {
        for two_raster in [false, true] {
            let doc = init_doc(); // 64x48, one white raster
            build(doc);
            if two_raster {
                add_second_raster(doc, "00b001", "aaaaaaaa-0000-4000-8000-0000000000f1");
            }
            let n = if two_raster { "2-raster" } else { "1-raster" };

            // (a) readback door — read_full asserts OK (was: ALWAYS refused).
            let _ = read_full(doc, 64, 48);

            // (b) PNG export — was: served @ ≤1 raster, REFUSED @ 2+.
            let png = tmp_export_path("png");
            let (st, err) = export(doc, &png, "png", None);
            assert_eq!(
                st,
                AppnameStatus::AppnameOk,
                "{kind}/{n} PNG export: {err:?}"
            );
            assert!(std::fs::metadata(&png)
                .map(|m| m.len() > 0)
                .unwrap_or(false));
            let _ = std::fs::remove_file(&png);

            // (c) non-PNG (bmp) export — was: ALWAYS refused (general path).
            let bmp = tmp_export_path("bmp");
            let (st, err) = export(doc, &bmp, "bmp", None);
            assert_eq!(
                st,
                AppnameStatus::AppnameOk,
                "{kind}/{n} bmp export: {err:?}"
            );
            let _ = std::fs::remove_file(&bmp);

            close(doc);
        }
    }
}
