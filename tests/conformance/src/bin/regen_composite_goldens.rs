// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_composite_goldens — mechanical composite SERVE-golden
//! (re)generator (O-QA, CP-1.0 golden round; the composite walls fell @
//! 3b21d8f), per the caf/stroke pattern: twice-produced, byte-compared,
//! manifest-sha'd. Each station's `.meta` records the export==readback
//! one-truth invariant the production `produce_all` re-asserts.
//!
//! Stations (2): composite-shape (two solid-fill vector paths) and
//! composite-text (Door-6 tofu page).
//!
//! Run:  cargo run -p appname-conformance --bin regen_composite_goldens

use appname_conformance::composite_goldens::produce_all;
use appname_conformance::paths;
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
    let dir = paths::repo_root().join("goldens").join("composite");
    std::fs::create_dir_all(&dir).unwrap();

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
                 producer: appname-conformance regen_composite_goldens via \
                 the ASSEMBLED engine; two independent productions \
                 byte-compared\n\
                 {meta}\
                 package: {name}.appname ({pkg_len} bytes)\n\
                 package.sha256: {pkg_sha}\n\
                 export: {name}.png ({png_len} bytes)\n\
                 export.sha256: {png_sha}\n\
                 rule: goldens change only via O-QA review (plan section \
                 0.6 rule 6); regenerate with `cargo run -p \
                 appname-conformance --bin regen_composite_goldens`\n",
                name = ga.name,
                meta = ga.meta,
                pkg_len = ga.package.len(),
                png_len = ga.export.len(),
            ),
        )
        .unwrap();
        println!("golden composite/{}: pkg {pkg_sha} png {png_sha}", ga.name);
    }
    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();
    println!("composite goldens written under {}", dir.display());
}
