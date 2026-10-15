// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! W08 plugin-safety GROUNDWORK — the hostile-plugin corpus, end to end
//! through the production stack: lifecycle scan → grant store → gate →
//! sandbox host → broker → the ONE engine door.
//!
//! Spec: ES §25.3 ("the plugin cannot touch fs/net (tested, §34)"),
//! §34.6 W8 rows 1–4; `bogos/tests/workflows/W08-plugin-safety.yaml`
//! steps 1–4 (step 5, plugin crash isolation, is the out-of-process
//! §25.5 tier — Post-1.0, recorded in the lane log). The workflow-level
//! `tests/workflows/w08_plugin_safety.sh` harness is O-QA's fence; the
//! proposed harness shape rides the lane log for their ratification —
//! THIS suite is the crate-side truth it will wrap.
//!
//! Host note (record-first): the VM bindings are staged, so the corpus
//! drives [`ScriptedStubHost`] — everything OUTSIDE the VM (the entire
//! capability surface W08 asserts on) is the production path; the suite
//! is written over `dyn SandboxHost` so the staged bindings slot in
//! without rewriting a case.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use appname_core::engine::kind_impl::EngineAssembly;
use appname_core::Engine;
use appname_script::plugin::gate::Gatekeeper;
use appname_script::plugin::host::{EngineBroker, HostError, PluginTrap, SandboxHost};
use appname_script::plugin::hosts::stub::ScriptedStubHost;
use appname_script::plugin::lifecycle::{enable, scan, PluginStatus, ScanDirs};
use appname_script::plugin::limits::ResourceLimits;
use appname_script::plugin::manifest::Manifest;
use appname_script::plugin::permissions::{Grant, GrantSet};
use appname_script::plugin::session::SessionStore;
use appname_script::plugin::state::StateFile;
use appname_script::EngineDoor;

/// The host ABI these tests present (the cabi door presents its own).
const ABI: (u32, u32) = (1, 5);

fn full_assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "appname-w08-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Install one W8 test plugin directory: manifest + stub-script entry.
/// The entry claims `plugin.wasm` (the manifest contract) while the
/// bytes are a stub script — the stub host interprets them; the real
/// wasm host will get real modules in the record-landing commit.
fn install_plugin(root: &Path, id: &str, perms: &str, script: &Value) -> PathBuf {
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
    std::fs::write(dir.join("plugin.wasm"), script.to_string()).unwrap();
    dir
}

/// The full production rig for one plugin: scan → enable (+ explicit
/// grants) → gate → engine → invoke via the stub host. Returns the
/// invocation result and the engine (for intact-document asserts).
struct Rig {
    engine: Engine,
    gate: Gatekeeper,
    entry: Vec<u8>,
    limits: ResourceLimits,
}

impl Rig {
    fn build(id: &str, perms: &str, script: &Value, explicit_grants: Vec<Grant>) -> Self {
        let sys = tmp("plugins");
        let dir = install_plugin(&sys, id, perms, script);
        let dirs = ScanDirs::new(vec![sys], tmp("user"), vec![]);
        let mut state = StateFile::default();
        let registry = scan(&dirs, ABI, &state);
        // The deliberate user steps: enable, then rule on the prompt.
        enable(&registry, &mut state, id).expect("enable");
        for g in explicit_grants {
            state.get_mut(id).unwrap().granted.insert(g);
        }
        let manifest = Manifest::parse(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let session = SessionStore::new(tmp("session")).for_plugin(id).unwrap();
        let granted: GrantSet = state.get(id).unwrap().granted.clone();
        let (gate, dropped) = Gatekeeper::new(&manifest, granted, session);
        assert!(dropped.is_empty(), "no grant dir drops expected in tests");
        // A real document behind the one door.
        let mut engine = full_assembly().build();
        engine
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": uuid::Uuid::new_v4().to_string(),
                    "kind": "doc.new",
                    "params": { "width": 16, "height": 16, "background": "white" }
                })
                .to_string(),
            )
            .expect("doc.new");
        Self {
            engine,
            gate,
            entry: std::fs::read(dir.join("plugin.wasm")).unwrap(),
            limits: ResourceLimits::V1_DEFAULT,
        }
    }

    fn invoke(&mut self) -> Result<Vec<Value>, HostError> {
        let mut host = ScriptedStubHost::new();
        host.instantiate(&self.entry, &self.limits)?;
        let mut door = EngineDoor(&mut self.engine);
        let mut broker = EngineBroker::new(&self.gate, &mut door, self.limits);
        let report = host.invoke("run", "{}", &mut broker)?;
        Ok(serde_json::from_str::<Vec<Value>>(&report).expect("stub report is a JSON array"))
    }

    fn journal_len(&self) -> u64 {
        self.engine.journal().len() as u64
    }
}

fn script(steps: Value) -> Value {
    json!({ "schema": "appname.stubscript.v1", "steps": steps })
}

// --- W8 step 2: fs escapes ------------------------------------------------

#[test]
fn w08_no_permission_plugin_cannot_read_home() {
    // W8 step 1+2: manifest requests NO fs permission; plugin reads $HOME.
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let mut rig = Rig::build(
        "org.hostile.nofs",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "fs_read", "path": format!("{home}/.ssh/id_ed25519") },
            { "op": "fs_read", "path": home }
        ])),
        vec![],
    );
    let report = rig.invoke().unwrap();
    for entry in &report {
        assert!(
            entry.get("denied").is_some(),
            "expected denial, got {entry}"
        );
        assert_eq!(
            entry["wireId"], "error.permission.filesystem_denied",
            "the denial carries the registered wire id (the app logs it, §25.3)"
        );
    }
}

#[test]
fn w08_traversal_out_of_session_store_denied() {
    let mut rig = Rig::build(
        "org.hostile.traversal",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "fs_read",  "path": "../../../../etc/passwd" },
            { "op": "fs_write", "path": "../outside.bin", "bytes": "aGk=" },
            { "op": "fs_read",  "path": "/etc/passwd" }
        ])),
        vec![],
    );
    let report = rig.invoke().unwrap();
    assert_eq!(report.len(), 3);
    for entry in &report {
        assert!(
            entry.get("denied").is_some(),
            "expected denial, got {entry}"
        );
    }
}

#[test]
fn w08_scoped_grant_in_scope_succeeds_out_of_scope_fails() {
    // W8 step 4: scoped fs grant — in-scope read succeeds, out-of-scope
    // read fails.
    let granted = tmp("granted-tex");
    std::fs::write(granted.join("tex.bin"), b"texture-bytes").unwrap();
    let inside = format!("{}/tex.bin", granted.display());
    let mut rig = Rig::build(
        "org.scoped.reader",
        r#"["filter.register", "fs.read"]"#,
        &script(json!([
            { "op": "fs_read", "path": inside },
            { "op": "fs_read", "path": "/etc/passwd" },
            { "op": "fs_write", "path": inside, "bytes": "aGk=" }
        ])),
        vec![Grant::FsRead(granted.clone())],
    );
    let report = rig.invoke().unwrap();
    assert_eq!(report[0]["ok"], "file:13B", "in-scope read succeeds");
    assert!(report[1].get("denied").is_some(), "out-of-scope read fails");
    assert!(
        report[2].get("denied").is_some(),
        "a READ grant buys no writes (scopes independent)"
    );
}

#[test]
fn w08_session_store_works_without_any_grant() {
    // The "fs = session store" half: relative petitions always land in
    // the plugin's own store, no grant needed, no escape possible.
    let mut rig = Rig::build(
        "org.wellbehaved.cache",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "fs_write", "path": "cache/result.bin", "bytes": "aGVsbG8=" },
            { "op": "fs_read",  "path": "cache/result.bin" }
        ])),
        vec![],
    );
    let report = rig.invoke().unwrap();
    assert_eq!(report[0]["ok"], "written");
    assert_eq!(report[1]["ok"], "file:5B");
}

#[cfg(unix)]
#[test]
fn w08_symlink_planted_in_session_store_cannot_escape() {
    let outside = tmp("symlink-target");
    std::fs::write(outside.join("secret.txt"), b"secret").unwrap();
    let mut rig = Rig::build(
        "org.hostile.symlink",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "fs_read", "path": "link/secret.txt" }
        ])),
        vec![],
    );
    // Plant the link as the plugin would (a prior sanctioned write can
    // only create files, so the link is planted out-of-band — the
    // stronger attacker).
    std::os::unix::fs::symlink(&outside, rig.gate.session_root().join("link")).unwrap();
    let report = rig.invoke().unwrap();
    assert!(report[0].get("denied").is_some(), "symlink escape refused");
}

// --- W8 step 3: net -------------------------------------------------------

#[test]
fn w08_network_denied_without_grant() {
    let mut rig = Rig::build(
        "org.hostile.phone",
        r#"["filter.register", "net"]"#,
        &script(json!([
            { "op": "net", "detail": "https://exfil.example/beacon" }
        ])),
        vec![],
    );
    let report = rig.invoke().unwrap();
    assert_eq!(report[0]["wireId"], "error.permission.network_denied");
}

#[test]
fn w08_network_denied_even_with_rogue_grant() {
    // v1 pin: net = DENY regardless of grants ([privacy] hard-gate).
    let mut rig = Rig::build(
        "org.hostile.phone2",
        r#"["filter.register", "net"]"#,
        &script(json!([ { "op": "net", "detail": "https://exfil.example" } ])),
        vec![Grant::Net],
    );
    let report = rig.invoke().unwrap();
    assert_eq!(report[0]["wireId"], "error.permission.network_denied");
}

#[test]
fn w08_denial_can_trap_typed() {
    // A guest that treats denial as fatal traps TYPED, not silently.
    let mut rig = Rig::build(
        "org.hostile.fatalist",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "trap_on_denial" },
            { "op": "net", "detail": "https://x" }
        ])),
        vec![],
    );
    match rig.invoke() {
        Err(HostError::Trap(PluginTrap::Denied(e))) => {
            assert_eq!(e.wire_id(), Some("error.permission.network_denied"));
        }
        other => panic!("expected typed denial trap, got {other:?}"),
    }
}

// --- W8: runaway loops ----------------------------------------------------

#[test]
fn w08_runaway_loop_exhausts_fuel_typed() {
    let mut rig = Rig::build(
        "org.hostile.spinner",
        r#"["filter.register"]"#,
        &script(json!([ { "op": "spin" } ])), // spin without fuel bound = consume all
        vec![],
    );
    rig.limits = ResourceLimits {
        fuel: Some(1_000_000),
        ..ResourceLimits::V1_DEFAULT
    };
    match rig.invoke() {
        Err(HostError::Trap(PluginTrap::FuelExhausted)) => {}
        other => panic!("expected FuelExhausted, got {other:?}"),
    }
}

#[test]
fn w08_deadline_trap_typed() {
    let mut rig = Rig::build(
        "org.hostile.sleeper",
        r#"["filter.register"]"#,
        &script(json!([ { "op": "sleep_ms", "ms": 30 } ])),
        vec![],
    );
    rig.limits = ResourceLimits {
        deadline_ms: 5,
        ..ResourceLimits::V1_DEFAULT
    };
    match rig.invoke() {
        Err(HostError::Trap(PluginTrap::DeadlineExceeded)) => {}
        other => panic!("expected DeadlineExceeded, got {other:?}"),
    }
}

// --- registration + the one door -----------------------------------------

#[test]
fn w08_registration_gated_and_prefixed() {
    let mut rig = Rig::build(
        "org.good.reg",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "register", "id": "org.good.reg.f", "capability": "filter.register" },
            { "op": "register", "id": "org.good.reg.ghost", "capability": "filter.register" },
            { "op": "register", "id": "org.good.reg.f", "capability": "tool.register" }
        ])),
        vec![],
    );
    let report = rig.invoke().unwrap();
    assert_eq!(
        report[0]["ok"], "registered:x.org.good.reg.f",
        "sanctioned registration lands under the x. prefix (§10.2)"
    );
    assert!(
        report[1].get("denied").is_some(),
        "not in provides → denied"
    );
    assert!(
        report[2].get("denied").is_some(),
        "ungranted capability → denied"
    );
}

#[test]
fn w08_mutations_ride_the_one_door_and_land_in_history() {
    let mut rig = Rig::build(
        "org.good.editor",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "apply", "envelope": {
                "schema": "appname.command.v1",
                "id": uuid::Uuid::new_v4().to_string(),
                "kind": "layer.add",
                "params": { "kind": "raster", "name": "from-plugin",
                            "newLayerId": uuid::Uuid::new_v4().to_string() }
            } }
        ])),
        vec![],
    );
    let before = rig.journal_len();
    let report = rig.invoke().unwrap();
    let ok = report[0]["ok"].as_str().expect("apply outcome");
    assert!(
        ok.starts_with("applied:"),
        "envelope crossed the door: {ok}"
    );
    let receipt: Value = serde_json::from_str(ok.strip_prefix("applied:").unwrap()).unwrap();
    assert_eq!(receipt["ok"], true, "receipt: {receipt}");
    assert_eq!(
        rig.journal_len(),
        before + 1,
        "the plugin edit is journaled like any other command (D-48)"
    );
}

#[test]
fn w08_garbage_envelope_refused_before_the_door() {
    let mut rig = Rig::build(
        "org.hostile.garbage",
        r#"["filter.register"]"#,
        &script(json!([
            { "op": "apply", "envelope": "not-an-object" },
            { "op": "apply", "envelope": { "schema": "appname.command.v1", "id": "not-a-uuid", "kind": "doc.new" } }
        ])),
        vec![],
    );
    let before = rig.journal_len();
    let report = rig.invoke().unwrap();
    // Non-object: the gate's own refusal.
    assert!(report[0].get("denied").is_some());
    // Object but invalid: the ENGINE's ordinary typed rejection —
    // exactly what any caller gets, no plugin-special path.
    let ok = report[1]["ok"].as_str().unwrap();
    let outcome: Value = serde_json::from_str(ok.strip_prefix("applied:").unwrap()).unwrap();
    assert_eq!(outcome["ok"], false);
    assert!(outcome["error"]["code"].is_string());
    assert_eq!(rig.journal_len(), before, "document unchanged");
}

// --- hostile packages at the lifecycle layer ------------------------------

#[test]
fn w08_hostile_manifests_fail_scan_without_aborting_it() {
    let sys = tmp("hostile-scan");
    // Traversal entry.
    let d1 = sys.join("org.evil.entry-1.0.0");
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::write(
        d1.join("manifest.json"),
        br#"{"schema":"appname.plugin.v1","id":"org.evil.entry","version":"1.0.0",
            "api":{"min":"1.0","max":"1.x"},"entry":"../../escape.wasm",
            "permissions":["filter.register"],
            "provides":[{"type":"filter","id":"org.evil.entry.f"}]}"#,
    )
    .unwrap();
    // Unknown permission.
    let d2 = sys.join("org.evil.perm-1.0.0");
    std::fs::create_dir_all(&d2).unwrap();
    std::fs::write(
        d2.join("manifest.json"),
        br#"{"schema":"appname.plugin.v1","id":"org.evil.perm","version":"1.0.0",
            "api":{"min":"1.0","max":"1.x"},"entry":"plugin.lua",
            "permissions":["kernel.everything"],
            "provides":[]}"#,
    )
    .unwrap();
    // A fine one alongside proves no abort.
    install_plugin(
        &sys,
        "org.fine.one",
        r#"["filter.register"]"#,
        &script(json!([])),
    );
    let dirs = ScanDirs::new(vec![sys], tmp("user-h"), vec![]);
    let registry = scan(&dirs, ABI, &StateFile::default());
    assert_eq!(registry.records.len(), 3);
    let failed = registry
        .records
        .iter()
        .filter(|r| matches!(r.status, PluginStatus::Failed(_)))
        .count();
    assert_eq!(failed, 2);
    assert!(registry.get("org.fine.one").is_some());
}
