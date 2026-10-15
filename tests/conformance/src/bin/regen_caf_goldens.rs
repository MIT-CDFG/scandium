// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_caf_goldens — mechanical CAF §45.3 behavioral-golden
//! (re)generator (O-QA, CP-1.0 ratification batch), per the stroke/edit
//! pattern: twice-produced, byte-compared, manifest-sha'd.
//!
//! FREEZE DISCIPLINE (goldens/README rules + the ledger trigger): this
//! freeze cites (1) the "CAF §45.3 SEEDED-STREAM MICRO-LAYOUT
//! RATIFICATION — 2026-08-16" entry (reports/O-QA-log.md) and (2) the
//! LIVE `caf_stream_conformance` test — both are stamped into every
//! .meta. Any future drift is an investigation, never a re-pin.
//!
//! Run:  cargo run -p appname-conformance --bin regen_caf_goldens

use appname_conformance::caf_goldens::produce_all;
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
    let dir = paths::repo_root().join("goldens").join("caf");
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
                 producer: appname-conformance regen_caf_goldens via the \
                 ASSEMBLED engine; two independent productions \
                 byte-compared\n\
                 freeze-citation-1: CAF §45.3 SEEDED-STREAM MICRO-LAYOUT \
                 RATIFICATION (CP-1.0, O-INT dispatch) — rule-9 block — \
                 2026-08-16 (reports/O-QA-log.md; ratified form (a), \
                 zero re-pins)\n\
                 freeze-citation-2: live conformance test \
                 tests/conformance/tests/caf_stream_conformance.rs \
                 (activated 2026-08-16; 12/12 double-derived vectors) — \
                 stream drift trips independently of this golden\n\
                 {meta}\
                 package: {name}.appname ({pkg_len} bytes)\n\
                 package.sha256: {pkg_sha}\n\
                 export: {name}.png ({png_len} bytes)\n\
                 export.sha256: {png_sha}\n\
                 deferrals (recorded): output new_layer/duplicate_layer \
                 goldens wait on the core layer-creation door (recorded \
                 escalation); sampleArea custom waits on the channel \
                 arena; target.maskId source waits on its contract \
                 reading\n\
                 rule: goldens change only via O-QA review (plan section \
                 0.6 rule 6); regenerate with `cargo run -p \
                 appname-conformance --bin regen_caf_goldens`\n",
                name = ga.name,
                meta = ga.meta,
                pkg_len = ga.package.len(),
                png_len = ga.export.len(),
            ),
        )
        .unwrap();
        println!("golden caf/{}: pkg {pkg_sha} png {png_sha}", ga.name);
    }
    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();
    println!("caf goldens written under {}", dir.display());
}
