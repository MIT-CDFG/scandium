// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! The plugin-lifecycle door — ABI 1.5 additive block.
//!
//! Spec: ES §25.2 (manifest), §25.3 (permissions — grants are the USER
//! decisions this door records), §25.6 (discovery, load order,
//! `appname.pluginstate.v1` persistence), §9.1/Appendix N (additive-
//! block discipline); §3.11 hard rule (the ABI is the ONLY entrance —
//! the Plugin Manager UI is a C-ABI client like every front-end, so
//! plugin lifecycle IS an ABI surface, never a shell-side side door);
//! plan §2.5 row "Plugins sandboxed tier v1" (lane O-SCRIPT).
//!
//! # Why these symbols exist (Appendix-N justification)
//!
//! Scan/enable/grant semantics are NORMATIVE (§25.6 shadowing order,
//! version pick, load-failure isolation, key-pin change refusal, §25.3
//! silent-grant class, grant persistence). One implementation lives in
//! `appname-script::plugin`; hosts consume it here — a shell-side
//! reimplementation would fork those rules exactly like the rejected
//! shell-side export encoding.
//!
//! # What is deliberately NOT here (record-first honesty)
//!
//! There is NO invoke symbol in the 1.5 block: executing a sandboxed
//! plugin needs the wasmtime/mlua bindings, which are staged-uncompiled
//! pending `deps/wasmtime.yaml` / `deps/mlua.yaml` (the appname-script
//! module docs). The invoke door lands as its own additive block WITH
//! the bindings, so no shipped symbol ever answers "not compiled in" —
//! an ABI symbol that cannot work in any build of this checkpoint would
//! be a broken contract, not an additive block. Scan/enable/grant are
//! fully real: they are what the Plugin Manager needs at install time
//! and everything they promise works in this build.
//!
//! # Handle model
//!
//! `AppnamePluginHost*` is an app-level handle (not per-document): one
//! registry cookie over the scan dirs + session store + state path the
//! embedder configured. Same misuse discipline as every handle (typed
//! errors, never UB; free is NULL-tolerant).

use std::ffi::c_char;
use std::path::PathBuf;

use serde_json::{json, Value};

use appname_script::plugin::lifecycle::{
    enable, scan, LoadRefusal, PluginRegistry, PluginStatus, ScanDirs, SignatureState,
};
use appname_script::plugin::state::StateFile;

use crate::mem::{AppnameError, AppnameString};
use crate::registry::{self, ErrorBody};
use crate::{guarded, guarded_status, invalid_arg, read_cstr, AppnameStatus};

/// Opaque plugin-host handle (scan config + grant store binding). Never
/// dereference; the value is a registry cookie.
#[repr(C)]
pub struct AppnamePluginHost {
    _private: [u8; 0],
}

/// The state behind one host handle.
pub(crate) struct PluginHostState {
    dirs: ScanDirs,
    state_path: PathBuf,
    /// Last scan result (None until the first
    /// `appname_plugin_host_scan`).
    registry: Option<PluginRegistry>,
}

/// The ABI this build presents to manifest `api` ranges: this crate's
/// own version constants (§25.2 "compared against `appname_version()`").
fn host_abi() -> (u32, u32) {
    let v = crate::appname_version();
    (v.abi_major, v.abi_minor)
}

fn parse_path(v: &Value, what: &str) -> Result<PathBuf, ErrorBody> {
    match v.as_str() {
        Some(s) if !s.is_empty() => Ok(PathBuf::from(s)),
        _ => Err(invalid_arg(&format!(
            "config_json.{what} must be a non-empty path string"
        ))),
    }
}

fn parse_path_array(v: &Value, what: &str) -> Result<Vec<PathBuf>, ErrorBody> {
    let arr = v
        .as_array()
        .ok_or_else(|| invalid_arg(&format!("config_json.{what} must be an array of paths")))?;
    arr.iter()
        .map(|p| parse_path(p, what))
        .collect::<Result<Vec<_>, _>>()
}

/// Render one scanned record as list JSON (`appname.pluginlist.v1`
/// entry shape — the Plugin Manager's row).
fn record_json(rec: &appname_script::plugin::lifecycle::PluginRecord) -> Value {
    let (status, detail): (&str, Value) = match &rec.status {
        PluginStatus::Enabled => ("enabled", Value::Null),
        PluginStatus::Disabled => ("disabled", Value::Null),
        PluginStatus::NeedsGrant(caps) => (
            "needsGrant",
            Value::Array(
                caps.iter()
                    .map(|c| Value::String(c.as_str().to_string()))
                    .collect(),
            ),
        ),
        PluginStatus::Failed(refusal) => {
            let class = match refusal {
                LoadRefusal::ManifestMissing => "manifestMissing",
                LoadRefusal::Manifest(_) => "manifest",
                LoadRefusal::DirNameMismatch { .. } => "dirNameMismatch",
                LoadRefusal::AbiOutOfRange { .. } => "abiOutOfRange",
                LoadRefusal::Signature(_) => "signature",
                LoadRefusal::KeyChanged => "keyChanged",
                LoadRefusal::EntryMissing(_) => "entryMissing",
            };
            (
                "failed",
                json!({ "class": class, "message": refusal.to_string() }),
            )
        }
        PluginStatus::Shadowed { by } => ("shadowed", Value::String(by.display().to_string())),
    };
    let mut entry = json!({
        "dir": rec.dir.display().to_string(),
        "status": status,
    });
    if !detail.is_null() {
        entry["detail"] = detail;
    }
    if let Some(m) = &rec.manifest {
        entry["id"] = Value::String(m.id.clone());
        entry["version"] = Value::String(m.version.clone());
        entry["entry"] = Value::String(m.entry.clone());
        entry["permissions"] = Value::Array(
            m.permissions
                .iter()
                .map(|c| Value::String(c.as_str().to_string()))
                .collect(),
        );
        entry["provides"] = Value::Array(
            m.provides
                .iter()
                .map(|p| json!({ "type": p.kind.as_str(), "id": p.id }))
                .collect(),
        );
    }
    entry["signature"] = match &rec.signature {
        None | Some(SignatureState::Absent) => Value::String("unsigned".into()),
        // Record-first honesty: ed25519 verification is held with its
        // record; a present signature buys NO trust yet (§25.5 consent
        // path = unsigned) but its key still pins.
        Some(SignatureState::PresentUnverified { .. }) => Value::String("presentUnverified".into()),
        Some(SignatureState::Malformed(_)) => Value::String("malformed".into()),
    };
    entry
}

fn list_json(registry: &PluginRegistry, state: &StateFile) -> Value {
    json!({
        "schema": "appname.pluginlist.v1",
        "plugins": registry.records.iter().map(record_json).collect::<Vec<_>>(),
        "granted": state.plugins.iter().map(|p| json!({
            "id": p.id,
            "enabled": p.enabled,
            "granted": p.granted.iter().map(|g| g.to_grant_string()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn state_err(e: appname_script::plugin::state::StateError) -> ErrorBody {
    ErrorBody::new(
        AppnameStatus::AppnameErrIo,
        json!({
            "code": "error.io.read_failed",
            "message": format!("plugin state file: {e}"),
        })
        .to_string(),
    )
}

// ---------------------------------------------------------------------------
// ABI 1.5 symbols.
// ---------------------------------------------------------------------------

/// ABI 1.5 additive block (gate on `appname_version().abi_minor >= 5`).
/// Create a plugin-host handle from a strict JSON config:
///
/// ```json
/// { "scanDirs":       ["/usr/share/appname/plugins", "~‐free literal paths"],
///   "sessionStoreDir": "/path/for/per-plugin-session-stores",
///   "statePath":       "/path/to/plugins-state.toml" }
/// ```
///
/// `scanDirs` is LOWEST precedence first — on an id collision a LATER
/// dir's plugin shadows an earlier dir's (put the user dir last; §25.6).
/// Paths are used verbatim (no tilde/env expansion — the embedder
/// resolves its platform dirs, §25.6 table). Unknown keys reject
/// `error.api.invalid_arg`; nothing touches the filesystem until scan.
///
/// # Safety
/// `config_json` a valid NUL-terminated C string; `out` writable;
/// `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_plugin_host_new(
    config_json: *const c_char,
    out: *mut *mut AppnamePluginHost,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        if out.is_null() {
            return Err(invalid_arg("out must not be NULL"));
        }
        // SAFETY: caller contract — valid NUL-terminated string.
        let text = unsafe { read_cstr(config_json, "config_json") }?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| invalid_arg(&format!("config_json is not valid JSON: {e}")))?;
        let obj = v
            .as_object()
            .ok_or_else(|| invalid_arg("config_json must be a JSON object"))?;
        for k in obj.keys() {
            if !matches!(k.as_str(), "scanDirs" | "sessionStoreDir" | "statePath") {
                return Err(invalid_arg(&format!(
                    "unknown config_json key {k:?} (supported: scanDirs, sessionStoreDir, statePath)"
                )));
            }
        }
        let scan_dirs = parse_path_array(
            obj.get("scanDirs")
                .ok_or_else(|| invalid_arg("config_json.scanDirs is required"))?,
            "scanDirs",
        )?;
        let session_root = parse_path(
            obj.get("sessionStoreDir")
                .ok_or_else(|| invalid_arg("config_json.sessionStoreDir is required"))?,
            "sessionStoreDir",
        )?;
        let state_path = parse_path(
            obj.get("statePath")
                .ok_or_else(|| invalid_arg("config_json.statePath is required"))?,
            "statePath",
        )?;
        // The session root rides in the state for the invoke block (the
        // gates are built per-plugin at instantiation); holding it here
        // pins the config shape now so 1.5 config never re-breaks.
        let _ = appname_script::plugin::session::SessionStore::new(session_root);
        let state = PluginHostState {
            dirs: ScanDirs { dirs: scan_dirs },
            state_path,
            registry: None,
        };
        // SAFETY: `out` non-NULL checked above; caller contract says
        // writable storage for one handle pointer.
        unsafe { *out = registry::mint_plugin_host(state) };
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Free a plugin-host handle. NULL is an OK no-op; an unknown or
/// already-freed handle reports `APPNAME_ERR_INVALID_ARG` (§9.3 free
/// family).
///
/// # Safety
/// `host` NULL or a handle from `appname_plugin_host_new`.
#[no_mangle]
pub unsafe extern "C" fn appname_plugin_host_free(host: *mut AppnamePluginHost) -> AppnameStatus {
    guarded_status(|| registry::close_plugin_host(host))
}

/// Scan the configured directories (ES §25.6 discovery + load order:
/// manifest parse, ABI range against THIS build's `appname_version()`,
/// signature classification, key-pin comparison, permission diff
/// against the stored grants). Load failures never abort the scan —
/// they are rows with `"status":"failed"`. On success `*out_list`
/// (when non-NULL) receives `appname.pluginlist.v1` JSON (free with
/// `appname_string_free`).
///
/// Signature note (record-first): ed25519 verification is not yet
/// active; `"signature":"presentUnverified"` rows follow the UNSIGNED
/// consent path (§25.5 warn-and-confirm) until the crypto record lands.
///
/// # Safety
/// `host` a live handle; `out_list` NULL or writable; `err` NULL or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn appname_plugin_host_scan(
    host: *mut AppnamePluginHost,
    out_list: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        let list = registry::with_plugin_host(host, |st| {
            let state = StateFile::load(&st.state_path).map_err(state_err)?;
            let reg = scan(&st.dirs, host_abi(), &state);
            let list = list_json(&reg, &state);
            st.registry = Some(reg);
            Ok(list)
        })?;
        if !out_list.is_null() {
            // SAFETY: non-NULL checked; caller contract says writable.
            unsafe { *out_list = registry::mint_string(list.to_string()) };
        }
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Enable or disable a scanned plugin — the DELIBERATE user step
/// (§25.5/§25.6: this is where the unsigned warn-and-confirm and the
/// permission listing have already happened in the embedder's UI).
/// Requires a prior scan on this handle.
///
/// Enabling records the §25.3 silent registration grants and the §25.5
/// key pin, persists `appname.pluginstate.v1` atomically, and returns
/// (in `*out_json`, when non-NULL) `{"toPrompt": ["fs.read", …]}` — the
/// requested capabilities still needing explicit per-grant rulings via
/// `appname_plugin_grant`. Disabling persists `enabled = false` (grants
/// are kept; revocation is explicit, §25.3). Enabling a failed or
/// unknown plugin rejects `error.api.invalid_arg` with the scan
/// refusal in the message.
///
/// # Safety
/// `host` a live handle; `plugin_id` a valid NUL-terminated C string;
/// `out_json` NULL or writable; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_plugin_set_enabled(
    host: *mut AppnamePluginHost,
    plugin_id: *const c_char,
    enabled: bool,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: caller contract — valid NUL-terminated string.
        let id = unsafe { read_cstr(plugin_id, "plugin_id") }?;
        let reply = registry::with_plugin_host(host, |st| {
            let reg = st
                .registry
                .as_ref()
                .ok_or_else(|| invalid_arg("scan first: no scan has run on this handle"))?;
            let mut state = StateFile::load(&st.state_path).map_err(state_err)?;
            let to_prompt = if enabled {
                enable(reg, &mut state, &id).map_err(|e| invalid_arg(&e))?
            } else {
                match state.get_mut(&id) {
                    Some(p) => p.enabled = false,
                    None => {
                        return Err(invalid_arg(&format!(
                            "plugin {id:?} has no stored state to disable"
                        )))
                    }
                }
                Vec::new()
            };
            state.save(&st.state_path).map_err(state_err)?;
            Ok(json!({
                "toPrompt": to_prompt
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
            }))
        })?;
        if !out_json.is_null() {
            // SAFETY: non-NULL checked; caller contract says writable.
            unsafe { *out_json = registry::mint_string(reply.to_string()) };
        }
        Ok(AppnameStatus::AppnameOk)
    })
}

/// Record one explicit grant for a plugin — a `appname.pluginstate.v1`
/// grant string (§25.3/§25.6): `"fs.read:<absolute dir>"`,
/// `"fs.write:<absolute dir>"`, `"net"`, `"clipboard"`, or a
/// registration capability. Malformed grant strings (relative fs paths,
/// unknown forms, a bare `"fs.read"`) reject `error.api.invalid_arg`.
/// Persists atomically. Granting does NOT bypass tier policy: the v1
/// sandboxed tier refuses `net` at petition time even when granted (the
/// §30.1 privacy hard-gate — the grant is recorded for when the
/// preference surfaces).
///
/// # Safety
/// `host` a live handle; `plugin_id`, `grant` valid NUL-terminated C
/// strings; `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_plugin_grant(
    host: *mut AppnamePluginHost,
    plugin_id: *const c_char,
    grant: *const c_char,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || plugin_grant_impl(host, plugin_id, grant, true))
}

/// Revoke one previously recorded grant (exact string match; §25.3:
/// revocation takes effect at next plugin load). Revoking a grant that
/// is not recorded rejects `error.api.invalid_arg`. Persists
/// atomically.
///
/// # Safety
/// As `appname_plugin_grant`.
#[no_mangle]
pub unsafe extern "C" fn appname_plugin_revoke(
    host: *mut AppnamePluginHost,
    plugin_id: *const c_char,
    grant: *const c_char,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || plugin_grant_impl(host, plugin_id, grant, false))
}

/// Shared body of grant/revoke (each wrapper runs it under its own
/// `guarded`).
fn plugin_grant_impl(
    host: *mut AppnamePluginHost,
    plugin_id: *const c_char,
    grant: *const c_char,
    add: bool,
) -> Result<AppnameStatus, ErrorBody> {
    // SAFETY: caller contract — valid NUL-terminated strings (the two
    // public wrappers document it; they forward their contract here).
    let id = unsafe { read_cstr(plugin_id, "plugin_id") }?;
    // SAFETY: as above.
    let grant_text = unsafe { read_cstr(grant, "grant") }?;
    let parsed =
        appname_script::plugin::permissions::Grant::parse(&grant_text).ok_or_else(|| {
            invalid_arg(&format!(
                "grant {grant_text:?} is not a valid appname.pluginstate.v1 grant string"
            ))
        })?;
    registry::with_plugin_host(host, |st| {
        let mut state = StateFile::load(&st.state_path).map_err(state_err)?;
        let p = state.get_mut(&id).ok_or_else(|| {
            invalid_arg(&format!(
                "plugin {id:?} has no stored state (enable it first)"
            ))
        })?;
        if add {
            p.granted.insert(parsed.clone());
        } else if !p.granted.remove(&parsed) {
            return Err(invalid_arg(&format!(
                "grant {grant_text:?} is not recorded for plugin {id:?}"
            )));
        }
        state.save(&st.state_path).map_err(state_err)?;
        Ok(())
    })?;
    Ok(AppnameStatus::AppnameOk)
}
