// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-actions` — the headless action/batch runner (W20 groundwork).
//!
//! Spec: ES §26.2 (batch semantics + report), §26.3 (the `--batch` CLI
//! mode this binary grounds; the flag wiring on the MAIN `appname` CLI is
//! O-CORE's `appname-cli` and consumes `appname_script::batch` — this
//! binary exists so the batch stack is drivable headless inside the
//! O-SCRIPT fence today).
//!
//! ```text
//! appname-actions validate --action a.json
//! appname-actions run --action a.json --in x.png --in y.appname \
//!     --out "web/{name}-{date:yyyy-MM-dd}.{ext}" [--format png]
//!     [--on-error continue|halt] [--overwrite] [--json]
//!     [--var /params/points='[[0,0],[1,1]]'] [--active-layer UUID]
//! ```
//!
//! Exit codes (§26.3 discipline): 0 success; 2 validation/usage error;
//! 7 batch completed with per-file failures; 1 unexpected failure.

use std::process::ExitCode;

use serde_json::{Map, Value};
use uuid::Uuid;

use appname_script::batch::{now_civil_utc, run_batch, BatchJob, OnError, PngExporter};
use appname_script::{Action, ReplayOptions};

/// The full kind assembly: THE production constructor. [MECHANICAL
/// CROSS-FENCE COMPANION, RATIFIED by O-SCRIPT (return leg,
/// reports/O-SCRIPT-log.md; core door slice 2, item D): this site is
/// the one that HAD drifted (O-INT MAJOR-1 — missing
/// adjust/filters/vector wraps forked the batch replay truth); the
/// single-source constructor retires the class. The chain lives in
/// `appname_assembly::PRODUCTION_CHAIN`; the crate's parity test
/// enumerates this call site.]
fn full_assembly() -> appname_core::engine::kind_impl::EngineAssembly {
    appname_assembly::production_assembly()
}

fn usage() -> &'static str {
    "usage:\n  appname-actions validate --action <a.json>\n  appname-actions run \
     --action <a.json> --in <file>... --out <template> [--format png]\n      \
     [--on-error continue|halt] [--overwrite] [--json]\n      [--var <pointer>=<json>]... \
     [--active-layer <uuid>]"
}

fn fail2(msg: &str) -> ExitCode {
    eprintln!("error: {msg}\n{}", usage());
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("validate") => validate(&args[1..]),
        Some("run") => run(&args[1..]),
        _ => fail2("expected a subcommand: validate | run"),
    }
}

fn load_action(path: &str) -> Result<Action, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    Action::from_json(&text).map_err(|e| format!("{}: {}", e.code.as_str(), e.message))
}

fn validate(args: &[String]) -> ExitCode {
    let mut action_path = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--action" => action_path = it.next().cloned(),
            other => return fail2(&format!("unknown argument {other:?}")),
        }
    }
    let Some(path) = action_path else {
        return fail2("--action is required");
    };
    match load_action(&path) {
        Ok(a) => {
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "name": a.name,
                    "steps": a.steps.len(),
                    "commandSteps": a.command_step_count(),
                    "assets": a.assets.len(),
                })
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "ok": false, "error": e }));
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> ExitCode {
    let mut action_path = None;
    let mut inputs = Vec::new();
    let mut out_template = None;
    let mut format = "png".to_string();
    let mut on_error = OnError::Continue;
    let mut overwrite = false;
    let mut json_mode = false;
    let mut vars: Map<String, Value> = Map::new();
    let mut active_layer = None;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--action" => action_path = it.next().cloned(),
            "--in" => match it.next() {
                Some(p) => inputs.push(std::path::PathBuf::from(p)),
                None => return fail2("--in needs a path"),
            },
            "--out" => out_template = it.next().cloned(),
            "--format" => match it.next() {
                Some(f) => format = f.clone(),
                None => return fail2("--format needs a codec id"),
            },
            "--on-error" => match it.next().map(String::as_str) {
                Some("continue") => on_error = OnError::Continue,
                Some("halt") => on_error = OnError::Halt,
                other => return fail2(&format!("--on-error continue|halt, got {other:?}")),
            },
            "--overwrite" => overwrite = true,
            "--json" => json_mode = true,
            "--var" => match it.next() {
                Some(kv) => match kv.split_once('=') {
                    Some((pointer, raw)) => match serde_json::from_str(raw) {
                        Ok(v) => {
                            vars.insert(pointer.to_string(), v);
                        }
                        Err(e) => {
                            return fail2(&format!("--var {pointer}: value is not JSON: {e}"))
                        }
                    },
                    None => return fail2("--var expects <pointer>=<json>"),
                },
                None => return fail2("--var needs <pointer>=<json>"),
            },
            "--active-layer" => match it.next().map(|s| Uuid::parse_str(s)) {
                Some(Ok(u)) => active_layer = Some(u),
                _ => return fail2("--active-layer needs a UUID"),
            },
            other => return fail2(&format!("unknown argument {other:?}")),
        }
    }
    let Some(action_path) = action_path else {
        return fail2("--action is required");
    };
    let Some(out_template) = out_template else {
        return fail2("--out is required");
    };
    if inputs.is_empty() {
        return fail2("at least one --in is required");
    }
    let action = match load_action(&action_path) {
        Ok(a) => a,
        Err(e) => return fail2(&e),
    };

    let job = BatchJob {
        action: &action,
        inputs,
        output_template: out_template,
        format,
        on_error,
        overwrite,
        replay: ReplayOptions { vars, active_layer },
        started: now_civil_utc(),
        clock: None, // wall clock: the report's `finished` is the real finish time
    };
    match run_batch(&job, &full_assembly(), &mut PngExporter) {
        Ok(report) => {
            if json_mode {
                println!("{}", report.to_value());
            } else {
                for r in &report.results {
                    match (&r.output, &r.error) {
                        (Some(out), None) => {
                            println!("ok    {} -> {}", r.input.display(), out.display())
                        }
                        (_, Some(e)) => {
                            println!("error {} : {}", r.input.display(), e.message)
                        }
                        _ => {}
                    }
                }
                let v = report.to_value();
                println!("summary: {}", v["summary"]);
            }
            if report.all_ok() {
                ExitCode::SUCCESS
            } else {
                // §26.3 / W20: batch completed with per-file failures.
                ExitCode::from(7)
            }
        }
        Err(e) => {
            // Job-level validation errors are exit 2; IO on the report is 1.
            let code = if e.code == appname_core::ErrorCode::IoWriteFailed {
                1
            } else {
                2
            };
            eprintln!("error: {}: {}", e.code.as_str(), e.message);
            ExitCode::from(code)
        }
    }
}
