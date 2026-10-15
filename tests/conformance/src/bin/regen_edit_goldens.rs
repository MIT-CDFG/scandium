// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_edit_goldens — mechanical fill/paste golden (re)generator (O-QA,
//! CP-0.5 wave 2), per the stroke pattern.
//!
//! Spec: contracts fill.yaml / edit.paste.yaml; plan §0.6 rule 6;
//! goldens/README.md rules 1-5. Writes goldens/edit/ (4 cases x package +
//! export + meta, twice-produced byte-compare) + MANIFEST.sha256.
//! Run:  cargo run -p appname-conformance --bin regen_edit_goldens

use appname_conformance::edit_goldens::produce_all;
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
    let dir = paths::repo_root().join("goldens").join("edit");
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
                 producer: appname-conformance regen_edit_goldens via the \
                 ASSEMBLED engine; two independent productions \
                 byte-compared\n\
                 {meta}\
                 package: {name}.appname ({pkg_len} bytes)\n\
                 package.sha256: {pkg_sha}\n\
                 export: {name}.png ({png_len} bytes)\n\
                 export.sha256: {png_sha}\n\
                 deferrals (recorded): edit.paste_into accept golden \
                 needs a live selection (selection.set pending; its \
                 selection_required gate is asserted as evidence in the \
                 verify suite); edit.cut accept golden waits for the \
                 selection wave\n\
                 rule: goldens change only via O-QA review (plan section \
                 0.6 rule 6); regenerate with `cargo run -p \
                 appname-conformance --bin regen_edit_goldens`\n",
                name = ga.name,
                meta = ga.meta,
                pkg_len = ga.package.len(),
                png_len = ga.export.len(),
            ),
        )
        .unwrap();
        println!("golden edit/{}: pkg {pkg_sha} png {png_sha}", ga.name);
    }
    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();
    println!("edit goldens written under {}", dir.display());
}
