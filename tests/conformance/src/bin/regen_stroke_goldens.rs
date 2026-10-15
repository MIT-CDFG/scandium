// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_stroke_goldens — mechanical stroke-golden (re)generator plus the
//! runner samples fixtures (O-QA, CP-0.3 slice B).
//!
//! Spec: contracts brush.stroke.yaml / eraser.stroke.yaml; ES §15/§40.5;
//! plan §0.6 rule 6; goldens/README.md rules 1-5.
//!
//! Writes goldens/stroke/ (4 cases x package + export + meta, twice-
//! produced and byte-compared before writing) and the runner fixtures:
//! fixtures/samples/basic-5pt.samplesv1 (the fixed samples blob) and
//! fixtures/samples/stroke-script.template.jsonl (a brush.stroke script
//! with the content-derived assetId + built-in preset id baked in and a
//! <LAYER_ID> placeholder the runners substitute from `new --json`).
//!
//! Run:  cargo run -p appname-conformance --bin regen_stroke_goldens

use appname_conformance::assembly_point;
use appname_conformance::paths;
use appname_conformance::stroke_goldens::{fixed_samples_basic, produce_all};
use std::path::Path;
use std::process::Command;

fn sha256(path: &Path) -> String {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .expect("shasum available");
    assert!(out.status.success(), "shasum failed for {}", path.display());
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn main() {
    let dir = paths::repo_root().join("goldens").join("stroke");
    std::fs::create_dir_all(&dir).unwrap();

    // rule 5: two independent productions must agree byte-for-byte
    let a = produce_all();
    let b = produce_all();
    let mut manifest = Vec::new();
    for (ga, gb) in a.iter().zip(&b) {
        assert_eq!(ga.package, gb.package, "{}: package nondet", ga.name);
        assert_eq!(ga.export, gb.export, "{}: export nondet", ga.name);

        let pkg = dir.join(format!("{}.appname", ga.name));
        let png = dir.join(format!("{}.png", ga.name));
        std::fs::write(&pkg, &ga.package).unwrap();
        std::fs::write(&png, &ga.export).unwrap();
        let pkg_sha = sha256(&pkg);
        let png_sha = sha256(&png);
        manifest.push(format!("{pkg_sha}  {}.appname", ga.name));
        manifest.push(format!("{png_sha}  {}.png", ga.name));
        std::fs::write(
            dir.join(format!("{}.meta", ga.name)),
            format!(
                "golden: {name}\n\
                 producer: appname-conformance regen_stroke_goldens via \
                 the ASSEMBLED engine (assembly point compiled verbatim); \
                 two independent productions byte-compared\n\
                 {meta}\
                 package: {name}.appname ({pkg_len} bytes)\n\
                 package.sha256: {pkg_sha}\n\
                 export: {name}.png ({png_len} bytes)\n\
                 export.sha256: {png_sha}\n\
                 note: no lock golden — no public surface sets section \
                 11.1 locks until layer.set_props lands; the \
                 error.command.locked wire id is pinned in the verify \
                 suite instead (recorded decision)\n\
                 rule: goldens change only via O-QA review (plan section \
                 0.6 rule 6); regenerate with `cargo run -p \
                 appname-conformance --bin regen_stroke_goldens`\n",
                name = ga.name,
                meta = ga.meta,
                pkg_len = ga.package.len(),
                png_len = ga.export.len(),
            ),
        )
        .unwrap();
        println!("golden stroke/{}: pkg {pkg_sha} png {png_sha}", ga.name);
    }
    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();

    // ---- runner fixtures ------------------------------------------------
    let samples_dir = paths::repo_root().join("fixtures").join("samples");
    std::fs::create_dir_all(&samples_dir).unwrap();
    let blob = fixed_samples_basic();
    std::fs::write(samples_dir.join("basic-5pt.samplesv1"), &blob).unwrap();

    // the asset id is content-derived, so it can be baked into the script
    let mut probe = assembly_point::assembled_engine();
    let asset_id = probe.add_samples_asset(blob.clone());
    let preset = assembly_point::preset_hard_round_16_id();
    let template = format!(
        "# appname.samples.v1 stroke script TEMPLATE (runner fixture)\n\
         # <LAYER_ID> is substituted from `new --json` \
         receipt.createdIds[-1];\n\
         # the assetId below is content-derived from \
         basic-5pt.samplesv1 (add it first via `appname asset add`).\n\
         {{\"schema\":\"appname.command.v1\",\"id\":\"ad00e001-0000-4000-8000-00000000e001\",\"kind\":\"brush.stroke\",\"target\":{{\"layerId\":\"<LAYER_ID>\"}},\"params\":{{\"brushPreset\":\"{preset}\",\"color\":{{\"model\":\"rgb\",\"comps\":[1,0.2,0,0]}},\"seed\":11,\"opacity\":1}},\"samples\":\"{asset_id}\"}}\n"
    );
    std::fs::write(samples_dir.join("stroke-script.template.jsonl"), template).unwrap();
    println!(
        "runner fixtures written: basic-5pt.samplesv1 ({} bytes, asset \
         {asset_id}), stroke-script.template.jsonl",
        blob.len()
    );
}
