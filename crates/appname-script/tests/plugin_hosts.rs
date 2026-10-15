// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! W08 corpus over the REAL sandbox bindings (wasmtime + mlua) — the
//! parity proof that the gate/broker stack the stub host exercised in
//! `plugin_safety.rs` behaves identically when a real VM crosses it.
//!
//! Fixtures are genuine WAT and Lua sources compiled by each host
//! (wasmtime via the elected `wat` feature; mlua directly). The report
//! shape is one spelling across hosts (`stub::render_reply`), so the
//! assertions are the same regardless of which VM ran.
//!
//! Gated on the tier features (default on): a `--no-default-features`
//! build drops these tiers (§48.4) and this file compiles to nothing.

#![cfg(any(feature = "wasm-plugins", feature = "lua-plugins"))]

use std::path::PathBuf;

use serde_json::{json, Value};

use appname_core::engine::kind_impl::EngineAssembly;
use appname_core::Engine;
use appname_script::plugin::gate::Gatekeeper;
use appname_script::plugin::host::{EngineBroker, HostError, PluginTrap, SandboxHost};
use appname_script::plugin::limits::ResourceLimits;
use appname_script::plugin::manifest::Manifest;
use appname_script::plugin::permissions::{Grant, GrantSet};
use appname_script::plugin::session::SessionStore;
use appname_script::EngineDoor;

fn full_assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "appname-hosts-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A production rig around one host: gate (session store + explicit
/// grants) + a real engine behind the one door.
struct Rig {
    engine: Engine,
    gate: Gatekeeper,
    limits: ResourceLimits,
}

impl Rig {
    fn new(id: &str, perms: &[&str], grants: Vec<Grant>) -> Self {
        let manifest = Manifest::parse(
            json!({
                "schema": "appname.plugin.v1",
                "id": id,
                "version": "1.0.0",
                "api": { "min": "1.0", "max": "1.x" },
                "entry": "plugin.wasm",
                "permissions": perms,
                "provides": [ { "type": "filter", "id": format!("{id}.f") } ],
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let mut set: GrantSet = grants.into_iter().collect();
        set.grant_silent(&manifest.permissions);
        let session = SessionStore::new(tmp("session")).for_plugin(id).unwrap();
        let (gate, dropped) = Gatekeeper::new(&manifest, set, session);
        assert!(dropped.is_empty());
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
            .unwrap();
        Self {
            engine,
            gate,
            limits: ResourceLimits::V1_DEFAULT,
        }
    }

    fn run(&mut self, host: &mut dyn SandboxHost, entry: &[u8]) -> Result<Vec<Value>, HostError> {
        host.instantiate(entry, &self.limits)?;
        let mut door = EngineDoor(&mut self.engine);
        let mut broker = EngineBroker::new(&self.gate, &mut door, self.limits);
        let report = host.invoke("run", "{}", &mut broker)?;
        Ok(serde_json::from_str(&report).unwrap())
    }

    fn journal_len(&self) -> u64 {
        self.engine.journal().len() as u64
    }
}

/// One WAT module: a memory, a data segment holding `data`, and a `run`
/// that calls `import` (from module "appname") with (0, data.len).
#[cfg(feature = "wasm-plugins")]
fn wat_one_str(import: &str, data: &str) -> String {
    format!(
        r#"(module
  (import "appname" "{import}" (func $f (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{data}")
  (func (export "run") (drop (call $f (i32.const 0) (i32.const {len})))))"#,
        len = data.len()
    )
}

#[cfg(feature = "wasm-plugins")]
fn wasm() -> appname_script::plugin::hosts::wasm_host::WasmHost {
    appname_script::plugin::hosts::wasm_host::WasmHost::new().unwrap()
}

// ---------------------------------------------------------------------------
// WASM (wasmtime) parity
// ---------------------------------------------------------------------------

#[cfg(feature = "wasm-plugins")]
mod wasm_tier {
    use super::*;

    #[test]
    fn fs_read_of_home_denied_with_wire_id() {
        let mut rig = Rig::new("org.w.nofs", &["filter.register"], vec![]);
        let entry = wat_one_str("fs_read", "/etc/passwd");
        let report = rig.run(&mut wasm(), entry.as_bytes()).unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(
            report[0]["wireId"], "error.permission.filesystem_denied",
            "{report:?}"
        );
    }

    #[test]
    fn net_denied() {
        let mut rig = Rig::new("org.w.net", &["filter.register", "net"], vec![Grant::Net]);
        let entry = wat_one_str("net", "https://exfil.example");
        let report = rig.run(&mut wasm(), entry.as_bytes()).unwrap();
        assert_eq!(report[0]["wireId"], "error.permission.network_denied");
    }

    #[test]
    fn session_store_write_then_read_ok() {
        let mut rig = Rig::new("org.w.cache", &["filter.register"], vec![]);
        // Two-import module: write "hi" to cache/x, then read it back.
        let entry = r#"(module
  (import "appname" "fs_write" (func $w (param i32 i32 i32 i32) (result i32)))
  (import "appname" "fs_read"  (func $r (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "cache/x")
  (data (i32.const 16) "hi")
  (func (export "run")
    (drop (call $w (i32.const 0) (i32.const 7) (i32.const 16) (i32.const 2)))
    (drop (call $r (i32.const 0) (i32.const 7)))))"#;
        let report = rig.run(&mut wasm(), entry.as_bytes()).unwrap();
        assert_eq!(report[0]["ok"], "written");
        assert_eq!(report[1]["ok"], "file:2B");
    }

    #[test]
    fn apply_rides_the_one_door_and_journals() {
        let mut rig = Rig::new("org.w.editor", &["filter.register"], vec![]);
        let env = json!({
            "schema": "appname.command.v1",
            "id": uuid::Uuid::new_v4().to_string(),
            "kind": "layer.add",
            "params": { "kind": "raster", "name": "from-wasm",
                        "newLayerId": uuid::Uuid::new_v4().to_string() }
        })
        .to_string();
        // Escape the envelope JSON into a WAT data string.
        let escaped: String = env
            .chars()
            .flat_map(|c| match c {
                '"' => vec!['\\', '"'],
                '\\' => vec!['\\', '\\'],
                other => vec![other],
            })
            .collect();
        let entry = format!(
            r#"(module
  (import "appname" "apply" (func $a (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{escaped}")
  (func (export "run") (drop (call $a (i32.const 0) (i32.const {len})))))"#,
            len = env.len()
        );
        let before = rig.journal_len();
        let report = rig.run(&mut wasm(), entry.as_bytes()).unwrap();
        let ok = report[0]["ok"].as_str().unwrap();
        assert!(ok.starts_with("applied:"), "{ok}");
        assert_eq!(rig.journal_len(), before + 1, "plugin edit journaled");
    }

    #[test]
    fn runaway_loop_traps_fuel_typed() {
        let mut rig = Rig::new("org.w.spin", &["filter.register"], vec![]);
        rig.limits = ResourceLimits {
            fuel: Some(100_000),
            ..ResourceLimits::V1_DEFAULT
        };
        let entry = r#"(module
  (memory (export "memory") 1)
  (func (export "run") (loop $l (br $l))))"#;
        match rig.run(&mut wasm(), entry.as_bytes()) {
            Err(HostError::Trap(PluginTrap::FuelExhausted)) => {}
            other => panic!("expected FuelExhausted, got {other:?}"),
        }
    }

    #[test]
    fn bad_module_is_typed_bad_entry() {
        let mut rig = Rig::new("org.w.bad", &["filter.register"], vec![]);
        match rig.run(&mut wasm(), b"not wasm at all") {
            Err(HostError::BadEntry(_)) => {}
            other => panic!("expected BadEntry, got {other:?}"),
        }
    }

    #[test]
    fn scoped_grant_in_and_out_of_scope() {
        let granted = tmp("granted");
        std::fs::write(granted.join("tex.bin"), b"texture!").unwrap();
        let inside = format!("{}/tex.bin", granted.display());
        // W-22: a Windows path's backslashes are WAT string escapes (and
        // `wat_one_str` passes the SOURCE length), so spell the path with
        // `/` — still a drive-letter absolute path on Windows.
        #[cfg(windows)]
        let inside = inside.replace('\\', "/");
        let mut rig = Rig::new(
            "org.w.scoped",
            &["filter.register", "fs.read"],
            vec![Grant::FsRead(granted.clone())],
        );
        let ok = wat_one_str("fs_read", &inside);
        let report = rig.run(&mut wasm(), ok.as_bytes()).unwrap();
        assert_eq!(report[0]["ok"], "file:8B");

        let mut rig2 = Rig::new(
            "org.w.scoped2",
            &["filter.register", "fs.read"],
            vec![Grant::FsRead(granted)],
        );
        #[cfg(not(windows))]
        let outside = "/etc/passwd";
        #[cfg(windows)]
        let outside = "C:/Windows/win.ini";
        let bad = wat_one_str("fs_read", outside);
        let report = rig2.run(&mut wasm(), bad.as_bytes()).unwrap();
        assert!(report[0].get("denied").is_some());
    }

    #[test]
    fn deadline_trap_typed() {
        let mut rig = Rig::new("org.w.slow", &["filter.register"], vec![]);
        // High fuel so the epoch DEADLINE fires, not fuel.
        rig.limits = ResourceLimits {
            fuel: Some(u64::MAX),
            deadline_ms: 5,
            ..ResourceLimits::V1_DEFAULT
        };
        let entry = r#"(module
  (memory (export "memory") 1)
  (func (export "run") (loop $l (br $l))))"#;
        match rig.run(&mut wasm(), entry.as_bytes()) {
            Err(HostError::Trap(PluginTrap::DeadlineExceeded)) => {}
            other => panic!("expected DeadlineExceeded, got {other:?}"),
        }
    }

    #[test]
    fn memory_cap_trap_typed() {
        let mut rig = Rig::new("org.w.hog", &["filter.register"], vec![]);
        // 2 MiB cap (32 pages); the module grows past it. trap_on_grow_
        // failure(true) turns the refused grow into a typed trap.
        rig.limits = ResourceLimits {
            fuel: Some(u64::MAX),
            deadline_ms: 60_000,
            memory_max_bytes: 2 * 1024 * 1024,
        };
        // grow by 1000 pages (64 MiB) in a loop until it traps.
        let entry = r#"(module
  (memory (export "memory") 1)
  (func (export "run")
    (loop $l
      (drop (memory.grow (i32.const 1000)))
      (br $l))))"#;
        match rig.run(&mut wasm(), entry.as_bytes()) {
            Err(HostError::Trap(PluginTrap::MemoryLimit)) => {}
            other => panic!("expected MemoryLimit, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Lua (mlua) parity
// ---------------------------------------------------------------------------

#[cfg(feature = "lua-plugins")]
mod lua_tier {
    use super::*;
    use appname_script::plugin::hosts::lua_host::LuaHost;

    fn lua() -> LuaHost {
        LuaHost::new()
    }

    #[test]
    fn fs_read_of_home_denied_with_wire_id() {
        let mut rig = Rig::new("org.l.nofs", &["filter.register"], vec![]);
        let src = r#"function run(args) appname.fs_read("/etc/passwd") end"#;
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert_eq!(report[0]["wireId"], "error.permission.filesystem_denied");
    }

    #[test]
    fn net_denied_even_with_rogue_grant() {
        let mut rig = Rig::new("org.l.net", &["filter.register", "net"], vec![Grant::Net]);
        let src = r#"function run(args) appname.net("https://x") end"#;
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert_eq!(report[0]["wireId"], "error.permission.network_denied");
    }

    #[test]
    fn session_store_round_trips() {
        let mut rig = Rig::new("org.l.cache", &["filter.register"], vec![]);
        let src = r#"
            function run(args)
              appname.fs_write("cache/x", "hello")
              appname.fs_read("cache/x")
            end"#;
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert_eq!(report[0]["ok"], "written");
        assert_eq!(report[1]["ok"], "file:5B");
    }

    /// D1 REGRESSION (critical sandbox escape): the always-loaded base
    /// library must NOT expose any fs/code-load global, and the never-
    /// loaded libraries must be absent. `run()` asserts each is nil; if
    /// any survives, the assert raises and the invocation faults — so a
    /// GREEN run is the proof of confinement. Every stripped name from
    /// `STRIPPED_BASE_GLOBALS` + `string.dump` + the unloaded libs.
    #[test]
    fn base_library_escape_hatches_are_stripped() {
        let mut rig = Rig::new("org.l.harden", &["filter.register"], vec![]);
        let src = r#"
            function run(args)
              -- code-load / fs escape hatches (the proven-live vector):
              assert(load == nil,          "load must be stripped")
              assert(loadstring == nil,    "loadstring must be stripped")
              assert(loadfile == nil,      "loadfile must be stripped")
              assert(dofile == nil,        "dofile must be stripped")
              assert(collectgarbage == nil,"collectgarbage must be stripped")
              assert(require == nil,       "require must be absent")
              assert(package == nil,       "package must be absent")
              assert(module == nil,        "module must be absent")
              assert(string.dump == nil,   "string.dump must be stripped")
              -- never-opened libraries:
              assert(io == nil,    "io must be absent")
              assert(os == nil,    "os must be absent")
              assert(debug == nil, "debug must be absent")
              assert(coroutine == nil, "coroutine must be absent")
              assert(utf8 == nil,  "utf8 must be absent")
              -- justified survivors are present (spot-check):
              assert(type(pcall) == "function", "pcall must survive")
              assert(type(string.format) == "function", "string.format must survive")
            end"#;
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert!(
            report.is_empty(),
            "no petitions; a green run proves it: {report:?}"
        );
    }

    /// D1: a plugin that TRIES the escape faults typed — it cannot reach
    /// the filesystem through `dofile`/`loadfile` (they are nil, so the
    /// call errors), and nothing was read or executed.
    #[test]
    fn dofile_and_loadfile_attempts_refuse() {
        // Plant a real file the escape would have executed/read.
        let evil = tmp("evil");
        std::fs::write(evil.join("payload.lua"), b"return 42").unwrap();
        let abs = format!("{}/payload.lua", evil.display());
        // W-22: a Windows path's backslashes are Lua string escapes —
        // spell them `\\` so the literal carries the real path.
        #[cfg(windows)]
        let abs = abs.replace('\\', "\\\\");
        for attempt in [
            format!(r#"function run(a) dofile("{abs}") end"#),
            format!(r#"function run(a) loadfile("{abs}") end"#),
            r#"function run(a) load("return 1")() end"#.to_string(),
        ] {
            let mut rig = Rig::new("org.l.escape", &["filter.register"], vec![]);
            match rig.run(&mut lua(), attempt.as_bytes()) {
                // Calling a nil global raises "attempt to call a nil value".
                Err(HostError::Trap(PluginTrap::Faulted(m))) => {
                    assert!(m.contains("nil"), "expected nil-call fault, got: {m}");
                }
                other => panic!("escape attempt {attempt:?} must fault, got {other:?}"),
            }
        }
    }

    #[test]
    fn deadline_trap_typed() {
        let mut rig = Rig::new("org.l.slow", &["filter.register"], vec![]);
        // High fuel so the DEADLINE (not fuel) is what fires.
        rig.limits = ResourceLimits {
            fuel: Some(u64::MAX),
            deadline_ms: 5,
            ..ResourceLimits::V1_DEFAULT
        };
        let src = r#"function run(args) while true do end end"#;
        match rig.run(&mut lua(), src.as_bytes()) {
            Err(HostError::Trap(PluginTrap::DeadlineExceeded)) => {}
            other => panic!("expected DeadlineExceeded, got {other:?}"),
        }
    }

    #[test]
    fn memory_cap_trap_typed() {
        let mut rig = Rig::new("org.l.hog", &["filter.register"], vec![]);
        // Small memory cap, generous fuel/deadline so MEMORY is what trips.
        rig.limits = ResourceLimits {
            fuel: Some(u64::MAX),
            deadline_ms: 60_000,
            memory_max_bytes: 512 * 1024,
        };
        // Grow an ever-longer string past the cap.
        let src = r#"
            function run(args)
              local s = "x"
              while true do s = s .. s end
            end"#;
        match rig.run(&mut lua(), src.as_bytes()) {
            Err(HostError::Trap(PluginTrap::MemoryLimit)) => {}
            other => panic!("expected MemoryLimit, got {other:?}"),
        }
    }

    #[test]
    fn traversal_denied() {
        let mut rig = Rig::new("org.l.trav", &["filter.register"], vec![]);
        let src = r#"function run(args) appname.fs_read("../../../etc/passwd") end"#;
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert!(report[0].get("denied").is_some());
    }

    #[test]
    fn apply_rides_the_one_door_and_journals() {
        let mut rig = Rig::new("org.l.editor", &["filter.register"], vec![]);
        let env = json!({
            "schema": "appname.command.v1",
            "id": uuid::Uuid::new_v4().to_string(),
            "kind": "layer.add",
            "params": { "kind": "raster", "name": "from-lua",
                        "newLayerId": uuid::Uuid::new_v4().to_string() }
        })
        .to_string();
        let src = format!("function run(args) appname.apply([==[{env}]==]) end");
        let before = rig.journal_len();
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert!(report[0]["ok"].as_str().unwrap().starts_with("applied:"));
        assert_eq!(rig.journal_len(), before + 1);
    }

    #[test]
    fn runaway_loop_traps_fuel_typed() {
        let mut rig = Rig::new("org.l.spin", &["filter.register"], vec![]);
        rig.limits = ResourceLimits {
            fuel: Some(50_000),
            ..ResourceLimits::V1_DEFAULT
        };
        let src = r#"function run(args) while true do end end"#;
        match rig.run(&mut lua(), src.as_bytes()) {
            Err(HostError::Trap(PluginTrap::FuelExhausted)) => {}
            other => panic!("expected FuelExhausted, got {other:?}"),
        }
    }

    #[test]
    fn syntax_error_is_typed_bad_entry() {
        let mut rig = Rig::new("org.l.bad", &["filter.register"], vec![]);
        match rig.run(&mut lua(), b"function run( this is not lua") {
            Err(HostError::BadEntry(_)) => {}
            other => panic!("expected BadEntry, got {other:?}"),
        }
    }

    #[test]
    fn registration_gated_and_prefixed() {
        let mut rig = Rig::new("org.l.reg", &["filter.register"], vec![]);
        let src = r#"
            function run(args)
              appname.register("org.l.reg.f", "filter.register")
              appname.register("org.l.reg.ghost", "filter.register")
            end"#;
        let report = rig.run(&mut lua(), src.as_bytes()).unwrap();
        assert_eq!(report[0]["ok"], "registered:x.org.l.reg.f");
        assert!(report[1].get("denied").is_some());
    }
}
