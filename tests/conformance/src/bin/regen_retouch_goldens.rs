// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_retouch_goldens — mechanical retouch-suite stroke-golden
//! (re)generator (O-QA, CP-1.0 golden round for the retouch row merged
//! d660d3b), per the stroke/caf pattern: twice-produced, byte-compared,
//! manifest-sha'd, one `.meta` sidecar per station carrying the fixed
//! inputs + the in-production D-48 evidence.
//!
//! Stations (6): retouch-clone (§21.1), retouch-heal-sampled (§45.1
//! Poisson membrane), retouch-heal-spot (§45.3 exemplar footprint),
//! retouch-dodge-burn (§21.5), retouch-smudge, retouch-blur-sharpen.
//!
//! Run:  cargo run -p appname-conformance --bin regen_retouch_goldens

use appname_conformance::paths;
use appname_conformance::retouch_goldens::produce_all;
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
    let dir = paths::repo_root().join("goldens").join("retouch");
    std::fs::create_dir_all(&dir).unwrap();

    // rule 5: two independent productions must agree byte-for-byte.
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
                 producer: appname-conformance regen_retouch_goldens via \
                 the ASSEMBLED engine; two independent productions \
                 byte-compared\n\
                 {meta}\
                 package: {name}.appname ({pkg_len} bytes)\n\
                 package.sha256: {pkg_sha}\n\
                 export: {name}.png ({png_len} bytes)\n\
                 export.sha256: {png_sha}\n\
                 seed-rationale: dynamics RNG is command-seeded (PCG32, \
                 §15.4; omitted → 0); Part 45 retouch stochastic choices \
                 derive from blake3(mergeKey) (§45 intro; absent → \
                 blake3(id)) — replay bit-identical either way; fixed ids \
                 + fixed seeds pin two-production byte-identity\n\
                 rule: goldens change only via O-QA review (plan section \
                 0.6 rule 6); regenerate with `cargo run -p \
                 appname-conformance --bin regen_retouch_goldens`\n",
                name = ga.name,
                meta = ga.meta,
                pkg_len = ga.package.len(),
                png_len = ga.export.len(),
            ),
        )
        .unwrap();
        println!("golden retouch/{}: pkg {pkg_sha} png {png_sha}", ga.name);
    }
    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();
    println!("retouch goldens written under {}", dir.display());
}
