// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Conformance suite for the ABI 1.5 plugin-lifecycle door (Appendix-N
//! discipline: every additive block lands WITH its conformance rows).
//!
//! Rows: handle lifecycle misuse (typed, never UB), strict config
//! refusal, §25.6 scan semantics through the ABI (failed rows never
//! abort; shadowing; ABI-range refusal against THIS build's version),
//! enable → silent grants + toPrompt, grant/revoke persistence
//! round-trip through `appname.pluginstate.v1`, malformed grant
//! refusal, and version gating (`abi_minor >= 5`).

use std::ffi::{CStr, CString};
use std::path::PathBuf;

use serde_json::Value;

use appname_cabi::mem::{AppnameError, AppnameString};
use appname_cabi::plugins::{
    appname_plugin_grant, appname_plugin_host_free, appname_plugin_host_new,
    appname_plugin_host_scan, appname_plugin_revoke, appname_plugin_set_enabled, AppnamePluginHost,
};
use appname_cabi::{appname_version, AppnameStatus, ABI_1_5_SYMBOLS};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "appname-cabi-plug-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn take_string(s: *mut AppnameString) -> String {
    let mut len: usize = 0;
    let ptr = unsafe { appname_cabi::mem::appname_string_data(s, &mut len) };
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
    assert_eq!(
        unsafe { appname_cabi::mem::appname_string_free(s) },
        AppnameStatus::AppnameOk
    );
    text
}

fn take_error(e: *mut AppnameError) -> (AppnameStatus, Value) {
    assert!(!e.is_null());
    let stored = unsafe { appname_cabi::mem::appname_error_status(e) };
    let mut len: usize = 0;
    let ptr = unsafe { appname_cabi::mem::appname_error_json(e, &mut len) };
    assert!(!ptr.is_null());
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
    assert_eq!(
        unsafe { appname_cabi::mem::appname_error_free(e) },
        AppnameStatus::AppnameOk
    );
    (stored, serde_json::from_str(&text).unwrap())
}

/// Write one valid plugin dir (stub-script entry — execution is not this
/// door's surface; scan/enable/grants are).
fn install_plugin(root: &std::path::Path, id: &str, perms: &str) -> PathBuf {
    let dir = root.join(format!("{id}-1.0.0"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        format!(
            r#"{{"schema":"appname.plugin.v1","id":"{id}","version":"1.0.0",
                 "api":{{"min":"1.0","max":"1.x"}},"entry":"plugin.wasm",
                 "permissions":{perms},
                 "provides":[{{"type":"filter","id":"{id}.f"}}]}}"#
        ),
    )
    .unwrap();
    std::fs::write(dir.join("plugin.wasm"), b"{}").unwrap();
    dir
}

struct Rig {
    host: *mut AppnamePluginHost,
    state_path: PathBuf,
    _dirs: (PathBuf, PathBuf),
}

fn rig_with(system: &std::path::Path, user: &std::path::Path) -> Rig {
    let state_dir = tmp("state");
    let state_path = state_dir.join("plugins-state.toml");
    let config = serde_json::json!({
        "scanDirs": [system.display().to_string(), user.display().to_string()],
        "sessionStoreDir": tmp("sessions").display().to_string(),
        "statePath": state_path.display().to_string(),
    });
    let cconfig = CString::new(config.to_string()).unwrap();
    let mut host: *mut AppnamePluginHost = std::ptr::null_mut();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_plugin_host_new(cconfig.as_ptr(), &mut host, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameOk);
    assert!(err.is_null(), "*err untouched on OK");
    assert!(!host.is_null());
    Rig {
        host,
        state_path,
        _dirs: (system.to_path_buf(), user.to_path_buf()),
    }
}

fn scan_json(rig: &Rig) -> Value {
    let mut out: *mut AppnameString = std::ptr::null_mut();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_plugin_host_scan(rig.host, &mut out, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameOk, "{:?}", take_error(err));
    serde_json::from_str(&take_string(out)).unwrap()
}

fn set_enabled(rig: &Rig, id: &str, enabled: bool) -> Result<Value, (AppnameStatus, Value)> {
    let cid = CString::new(id).unwrap();
    let mut out: *mut AppnameString = std::ptr::null_mut();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st =
        unsafe { appname_plugin_set_enabled(rig.host, cid.as_ptr(), enabled, &mut out, &mut err) };
    if st == AppnameStatus::AppnameOk {
        Ok(serde_json::from_str(&take_string(out)).unwrap())
    } else {
        Err(take_error(err))
    }
}

fn grant(rig: &Rig, id: &str, g: &str) -> Result<(), (AppnameStatus, Value)> {
    let cid = CString::new(id).unwrap();
    let cg = CString::new(g).unwrap();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_plugin_grant(rig.host, cid.as_ptr(), cg.as_ptr(), &mut err) };
    if st == AppnameStatus::AppnameOk {
        Ok(())
    } else {
        Err(take_error(err))
    }
}

fn revoke(rig: &Rig, id: &str, g: &str) -> Result<(), (AppnameStatus, Value)> {
    let cid = CString::new(id).unwrap();
    let cg = CString::new(g).unwrap();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_plugin_revoke(rig.host, cid.as_ptr(), cg.as_ptr(), &mut err) };
    if st == AppnameStatus::AppnameOk {
        Ok(())
    } else {
        Err(take_error(err))
    }
}

fn free(rig: Rig) {
    assert_eq!(
        unsafe { appname_plugin_host_free(rig.host) },
        AppnameStatus::AppnameOk
    );
}

// ---------------------------------------------------------------------------

#[test]
fn version_gates_the_block() {
    let v = appname_version();
    assert!(
        v.abi_minor >= 5,
        "the 1.5 block ships with abi_minor >= 5 (N.5 rule 2)"
    );
    assert_eq!(ABI_1_5_SYMBOLS.len(), 6);
}

#[test]
fn config_is_strict() {
    let bad = |cfg: &str| -> (AppnameStatus, Value) {
        let c = CString::new(cfg).unwrap();
        let mut host: *mut AppnamePluginHost = std::ptr::null_mut();
        let mut err: *mut AppnameError = std::ptr::null_mut();
        let st = unsafe { appname_plugin_host_new(c.as_ptr(), &mut host, &mut err) };
        assert_ne!(st, AppnameStatus::AppnameOk, "config {cfg:?} must refuse");
        assert!(host.is_null() || st != AppnameStatus::AppnameOk);
        take_error(err)
    };
    // Not JSON / not an object / unknown key / missing keys / bad types.
    for cfg in [
        "not json",
        "[1,2]",
        r#"{"scanDirs":[],"sessionStoreDir":"/tmp/s","statePath":"/tmp/p","surprise":1}"#,
        r#"{"sessionStoreDir":"/tmp/s","statePath":"/tmp/p"}"#,
        r#"{"scanDirs":"not-an-array","sessionStoreDir":"/tmp/s","statePath":"/tmp/p"}"#,
        r#"{"scanDirs":[],"sessionStoreDir":"","statePath":"/tmp/p"}"#,
    ] {
        let (st, body) = bad(cfg);
        assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
        assert_eq!(body["code"], "error.api.invalid_arg", "{body}");
    }
}

#[test]
fn handle_misuse_is_typed_never_ub() {
    // NULL host on free = OK no-op (§9.3 free family).
    assert_eq!(
        unsafe { appname_plugin_host_free(std::ptr::null_mut()) },
        AppnameStatus::AppnameOk
    );
    // Scan on NULL / garbage / freed handles: typed.
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st =
        unsafe { appname_plugin_host_scan(std::ptr::null_mut(), std::ptr::null_mut(), &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);

    let garbage = 0xdeadbeefusize as *mut AppnamePluginHost;
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_plugin_host_scan(garbage, std::ptr::null_mut(), &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);
    assert_eq!(
        unsafe { appname_plugin_host_free(garbage) },
        AppnameStatus::AppnameErrInvalidArg,
        "double/garbage free is typed"
    );

    let rig = rig_with(&tmp("sys-mis"), &tmp("user-mis"));
    let host = rig.host;
    free(rig);
    // Use-after-free: typed.
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_plugin_host_scan(host, std::ptr::null_mut(), &mut err) };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);
    // Double free: typed.
    assert_eq!(
        unsafe { appname_plugin_host_free(host) },
        AppnameStatus::AppnameErrInvalidArg
    );
}

#[test]
fn scan_lists_shadowing_failures_and_abi_refusals() {
    let sys = tmp("sys-scan");
    let user = tmp("user-scan");
    install_plugin(&sys, "org.example.dup", r#"["filter.register"]"#);
    install_plugin(&user, "org.example.dup", r#"["filter.register"]"#);
    install_plugin(&sys, "org.example.ok", r#"["filter.register","fs.read"]"#);
    // Hostile: manifest missing.
    std::fs::create_dir_all(sys.join("junk-0")).unwrap();
    // ABI out of range for this build.
    let future = sys.join("org.example.future-1.0.0");
    std::fs::create_dir_all(&future).unwrap();
    std::fs::write(
        future.join("manifest.json"),
        br#"{"schema":"appname.plugin.v1","id":"org.example.future","version":"1.0.0",
            "api":{"min":"99.0","max":"99.x"},"entry":"plugin.wasm",
            "permissions":["filter.register"],
            "provides":[{"type":"filter","id":"org.example.future.f"}]}"#,
    )
    .unwrap();
    std::fs::write(future.join("plugin.wasm"), b"{}").unwrap();

    let rig = rig_with(&sys, &user);
    let list = scan_json(&rig);
    assert_eq!(list["schema"], "appname.pluginlist.v1");
    let rows = list["plugins"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    let by_status =
        |status: &str| -> usize { rows.iter().filter(|r| r["status"] == status).count() };
    assert_eq!(by_status("shadowed"), 1, "user dir shadows system dir");
    assert_eq!(by_status("failed"), 2, "manifest-missing + ABI range");
    assert!(rows.iter().any(|r| r["status"] == "failed"
        && r["detail"]["class"] == "abiOutOfRange"
        && r["detail"]["message"]
            .as_str()
            .unwrap()
            .contains("outside this build")));
    assert!(rows
        .iter()
        .all(|r| r["signature"] == "unsigned" || r["status"] == "failed"));
    free(rig);
}

#[test]
fn enable_grant_revoke_persist_through_pluginstate_v1() {
    let sys = tmp("sys-en");
    let user = tmp("user-en");
    install_plugin(
        &sys,
        "org.example.gr",
        r#"["filter.register","fs.read","net"]"#,
    );
    let rig = rig_with(&sys, &user);
    let _ = scan_json(&rig);

    // Enable: silent registration grant recorded; fs.read + net to prompt.
    let reply = set_enabled(&rig, "org.example.gr", true).unwrap();
    assert_eq!(
        reply["toPrompt"],
        serde_json::json!(["fs.read", "net"]),
        "silent class auto-granted, explicit class listed for prompting"
    );

    // Grant a scoped read + revoke it again; malformed grants refuse.
    let dir = tmp("granted");
    grant(
        &rig,
        "org.example.gr",
        &format!("fs.read:{}", dir.display()),
    )
    .unwrap();
    let (st, body) = grant(&rig, "org.example.gr", "fs.read:relative/path").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(body["code"], "error.api.invalid_arg");
    let (st, _) = grant(&rig, "org.example.gr", "sudo").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);

    // The state file on disk is the §25.6 shape and carries the grants.
    let text = std::fs::read_to_string(&rig.state_path).unwrap();
    assert!(text.contains("schema = \"appname.pluginstate.v1\""));
    assert!(text.contains("\"filter.register\""));
    #[cfg(not(windows))]
    assert!(text.contains(&format!("\"fs.read:{}\"", dir.display())));
    // W-22: the state file is TOML; a Windows path's backslashes are
    // escaped (`\\`) inside the basic string.
    #[cfg(windows)]
    assert!(text.contains(&format!(
        "\"fs.read:{}\"",
        dir.display().to_string().replace('\\', "\\\\")
    )));

    // Revoke round-trip; revoking the unrecorded refuses typed.
    revoke(
        &rig,
        "org.example.gr",
        &format!("fs.read:{}", dir.display()),
    )
    .unwrap();
    let (st, _) = revoke(&rig, "org.example.gr", "fs.read:/never/granted").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let text = std::fs::read_to_string(&rig.state_path).unwrap();
    assert!(!text.contains("fs.read:"), "revoked grant left the file");

    // A re-scan now reports needsGrant (enabled, fs.read/net unruled).
    let list = scan_json(&rig);
    let row = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "org.example.gr")
        .unwrap();
    assert_eq!(row["status"], "needsGrant");

    // Disable persists.
    set_enabled(&rig, "org.example.gr", false).unwrap();
    let text = std::fs::read_to_string(&rig.state_path).unwrap();
    assert!(text.contains("enabled = false"));
    free(rig);
}

#[test]
fn enable_refusals_are_typed() {
    let sys = tmp("sys-ref");
    let user = tmp("user-ref");
    let rig = rig_with(&sys, &user);
    // Before any scan:
    let (st, body) = set_enabled(&rig, "org.example.none", true).unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert!(body["message"].as_str().unwrap().contains("scan first"));
    // After scan, unknown id:
    let _ = scan_json(&rig);
    let (st, _) = set_enabled(&rig, "org.example.none", true).unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    // Disable with no stored state:
    let (st, _) = set_enabled(&rig, "org.example.none", false).unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    free(rig);
}
