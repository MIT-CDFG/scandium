// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The CP-0.1 determinism self-check (plan slice-3 item 4): a
//! self-contained proof that
//! `script → save → reopen → replay → identical state_hash` and that the
//! re-export is **byte-identical across two fully independent runs** (the
//! property the wave-4 QA runners assert via checksum comparison).
//!
//! Self-contained by design: no golden files (goldens are O-QA's lane) —
//! every expectation is derived inside the run. Envelope ids are FIXED so
//! both runs derive identical document ids (§9.6 v5 derivation).

use appname_core::{export_png_bytes, load_package_with, save_package, Engine};

pub struct Row {
    pub name: &'static str,
    pub pass: bool,
    pub detail: String,
}

pub struct Report {
    pub pass: bool,
    pub rows: Vec<Row>,
    pub work_dir: String,
}

/// Fixed stroke samples (deterministic bytes; the assetId derives from
/// them, so the script below can reference it as a constant of the build).
fn fixed_samples() -> Vec<u8> {
    use appname_tools::brush::{encode_samples, InputSample};
    let pts = [(40.0, 30.0, 0.0), (160.0, 90.0, 0.05), (280.0, 150.0, 0.10)];
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
    encode_samples(&samples).expect("fixed samples encode")
}

/// The fixed script both runs execute (raw envelopes, WD-1): doc.new plus
/// a brush stroke (fixed seed, fixed samples asset — the CP-0.3 leg).
fn script_lines(samples_id: uuid::Uuid) -> Vec<String> {
    vec![
        r#"{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000001","kind":"doc.new","params":{"width":320,"height":200,"background":"black"}}"#.to_string(),
        format!(
            r#"{{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000002","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.25,0,0]}},"seed":7,"opacity":0.9}},"samples":"{samples_id}"}}"#,
            layer = doc_new_background_layer_id(),
            preset = super::stroke_kinds::preset_hard_round_16_id(),
        ),
    ]
}

/// The background layer id of the fixed doc.new envelope (§9.6 v5
/// derivation from the fixed envelope id — a build constant).
fn doc_new_background_layer_id() -> uuid::Uuid {
    appname_core::engine::doc_new::derive_id(
        uuid::Uuid::parse_str("5e1f7e57-0001-4000-8000-000000000001").unwrap(),
        "layer/0",
    )
}

fn one_run(dir: &std::path::Path, tag: &str) -> Result<(Vec<u8>, Vec<u8>, Engine), String> {
    let mut engine = super::full_assembly().build();
    let samples_id = engine.add_samples_asset(fixed_samples());
    for line in script_lines(samples_id) {
        engine
            .apply_json(&line)
            .map_err(|e| format!("script apply: {e}"))?;
    }
    let path = dir.join(format!("selftest-{tag}.appname"));
    save_package(&engine, &path).map_err(|e| format!("save: {e}"))?;
    let package_bytes = std::fs::read(&path).map_err(|e| format!("read back: {e}"))?;
    let export = export_png_bytes(&engine).map_err(|e| format!("export: {e}"))?;
    Ok((package_bytes, export, engine))
}

pub fn run(keep: bool) -> Report {
    let dir = std::env::temp_dir().join(format!("appname-selftest-{}", std::process::id()));
    let mut rows = Vec::new();
    let mut check = |name: &'static str, result: Result<String, String>| {
        let (pass, detail) = match result {
            Ok(d) => (true, d),
            Err(d) => (false, d),
        };
        rows.push(Row { name, pass, detail });
    };

    match std::fs::create_dir_all(&dir) {
        Ok(()) => {}
        Err(e) => {
            return Report {
                pass: false,
                rows: vec![Row {
                    name: "work directory",
                    pass: false,
                    detail: format!("creating {}: {e}", dir.display()),
                }],
                work_dir: dir.display().to_string(),
            }
        }
    }

    let run1 = one_run(&dir, "run1");
    match &run1 {
        Ok((package1, export1, engine1)) => {
            // 1. Reopen: state_hash identity across the package boundary.
            let path1 = dir.join("selftest-run1.appname");
            let reopened = load_package_with(&super::full_assembly(), &path1);
            check(
                "save → reopen: state_hash identity",
                match &reopened {
                    Ok(loaded) => {
                        if loaded.engine.state_hash() == engine1.state_hash()
                            && loaded.warnings.is_empty()
                        {
                            Ok(format!("hash {}", short(&engine1.state_hash().to_hex())))
                        } else {
                            Err("reopened state diverges (or load warnings)".into())
                        }
                    }
                    Err(e) => Err(format!("reopen: {e}")),
                },
            );

            // 2. Replay the reopened journal from scratch (§7.8 requirement).
            check(
                "reopen → replay journal: state_hash identity",
                match &reopened {
                    Ok(loaded) => match loaded.engine.replay_assembled(loaded.engine.journal()) {
                        Ok(replayed) if replayed.state_hash() == loaded.engine.state_hash() => {
                            Ok(format!("{} commands", loaded.engine.journal().len()))
                        }
                        Ok(_) => Err("replayed state diverges".into()),
                        Err(e) => Err(format!("replay: {e}")),
                    },
                    Err(e) => Err(format!("reopen: {e}")),
                },
            );

            // 3. Re-export from the reopened engine: byte identity.
            check(
                "reopen → re-export: byte identity",
                match &reopened {
                    Ok(loaded) => match export_png_bytes(&loaded.engine) {
                        Ok(bytes) if &bytes == export1 => Ok(format!("{} bytes", bytes.len())),
                        Ok(_) => Err("re-export bytes diverge".into()),
                        Err(e) => Err(format!("export: {e}")),
                    },
                    Err(e) => Err(format!("reopen: {e}")),
                },
            );

            // 4. Save → load → save package byte stability.
            check(
                "save → load → save: package byte identity",
                match &reopened {
                    Ok(loaded) => match appname_core::save_package_bytes(&loaded.engine) {
                        Ok(bytes) if &bytes == package1 => Ok(format!("{} bytes", bytes.len())),
                        Ok(_) => Err("second save diverges".into()),
                        Err(e) => Err(format!("save: {e}")),
                    },
                    Err(e) => Err(format!("reopen: {e}")),
                },
            );

            // 5. A fully independent second run: byte identity of package
            //    AND export (the QA shasum property).
            check(
                "independent second run: byte identity",
                match one_run(&dir, "run2") {
                    Ok((package2, export2, _)) => {
                        if &package2 == package1 && &export2 == export1 {
                            Ok("package + export identical".into())
                        } else {
                            Err("independent run diverges".into())
                        }
                    }
                    Err(e) => Err(e),
                },
            );
        }
        Err(e) => check("first run", Err(e.clone())),
    }

    if !keep {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let pass = rows.iter().all(|r| r.pass);
    Report {
        pass,
        rows,
        work_dir: dir.display().to_string(),
    }
}

fn short(hex: &str) -> String {
    hex.chars().take(12).collect()
}
