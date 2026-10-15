// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_doc_new_goldens — mechanical golden (re)generator (O-QA lane).
//!
//! Spec: plan §2.1 "Test harness v1" + §0.6 rule 6 (goldens change only
//! via O-QA review); goldens/README.md rules 1-5; rule 7 (no hand-authored
//! expected bytes — everything here comes out of the engine).
//!
//! Writes goldens/doc_new/: for every fixed matrix combo a package
//! (.appname), an export (.png), and a .meta sidecar (fixed input
//! envelope + sha256s — no timestamps, so re-runs are byte-identical);
//! plus goldens/selftest/reference.appname (the CLI selftest fixed-script
//! package, sha pinned in reports/O-CORE-log.md) and MANIFEST.sha256
//! files. Every artifact is generated TWICE through independent engines
//! and byte-compared before anything is written (goldens/README.md rule
//! 5). Run from anywhere:  cargo run -p appname-conformance --bin
//! regen_doc_new_goldens

use appname_conformance::goldens::{
    doc_new_matrix, produce, selftest_reference_package, SELFTEST_REFERENCE_BYTES,
    SELFTEST_REFERENCE_SHA256, SELFTEST_STATE_HASH_PREFIX,
};
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
    let root = paths::repo_root().join("goldens");
    let dn = root.join("doc_new");
    let st = root.join("selftest");
    std::fs::create_dir_all(&dn).unwrap();
    std::fs::create_dir_all(&st).unwrap();

    let mut manifest = Vec::new();

    for combo in doc_new_matrix() {
        // rule 5: two independent generations must agree byte-for-byte
        let (pkg_a, png_a) = produce(&combo.envelope);
        let (pkg_b, png_b) = produce(&combo.envelope);
        assert_eq!(pkg_a, pkg_b, "{}: package nondeterminism", combo.name);
        assert_eq!(png_a, png_b, "{}: export nondeterminism", combo.name);

        let pkg_path = dn.join(format!("{}.appname", combo.name));
        std::fs::write(&pkg_path, &pkg_a).unwrap();
        let pkg_sha = sha256(&pkg_path);
        manifest.push(format!("{pkg_sha}  {}.appname", combo.name));

        let export_lines;
        match &png_a {
            Ok(png) => {
                let png_path = dn.join(format!("{}.png", combo.name));
                std::fs::write(&png_path, png).unwrap();
                let png_sha = sha256(&png_path);
                manifest.push(format!("{png_sha}  {}.png", combo.name));
                export_lines = format!(
                    "export: {}.png ({} bytes)\nexport.sha256: {png_sha}\n",
                    combo.name,
                    png.len()
                );
                println!("golden {}: pkg {pkg_sha} png {png_sha}", combo.name);
            }
            Err(deferral) => {
                export_lines = format!(
                    "export: DEFERRED — engine answers \"{deferral}\" \
                     (non-rgba export needs the section 5/6 conversion \
                     pipeline, CP-0.5); the PNG golden joins in that wave\n"
                );
                println!("golden {}: pkg {pkg_sha} png deferred (CP-0.5)", combo.name);
            }
        }

        let meta = format!(
            "golden: {name}\n\
             producer: appname-conformance regen_doc_new_goldens \
             (Engine::apply_bytes + save_package_bytes + export_png_bytes; \
             two independent generations byte-compared)\n\
             input.envelope: {env}\n\
             package: {name}.appname ({pkg_len} bytes)\n\
             package.sha256: {pkg_sha}\n\
             {export_lines}\
             rule: goldens change only via O-QA review (plan section 0.6 \
             rule 6); regenerate with `cargo run -p appname-conformance \
             --bin regen_doc_new_goldens`\n",
            name = combo.name,
            env = combo.envelope,
            pkg_len = pkg_a.len(),
        );
        std::fs::write(dn.join(format!("{}.meta", combo.name)), meta).unwrap();
    }

    manifest.sort();
    std::fs::write(dn.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();

    // ---- selftest reference package (CP-0.3: doc.new + brush leg) ------
    let (ref_a, hash_a) = selftest_reference_package();
    let (ref_b, hash_b) = selftest_reference_package();
    assert_eq!(ref_a, ref_b, "selftest reference nondeterminism");
    assert_eq!(hash_a, hash_b);
    assert!(
        hash_a.starts_with(SELFTEST_STATE_HASH_PREFIX),
        "selftest state_hash drifted from the O-CORE-log record: {hash_a}"
    );
    assert_eq!(
        ref_a.len(),
        SELFTEST_REFERENCE_BYTES,
        "selftest reference size drifted from the O-CORE-log record"
    );
    let ref_path = st.join("reference.appname");
    std::fs::write(&ref_path, &ref_a).unwrap();
    let ref_sha = sha256(&ref_path);
    assert_eq!(
        ref_sha, SELFTEST_REFERENCE_SHA256,
        "selftest reference sha drifted from the O-CORE-log record — \
         engine output changed; route through O-QA review"
    );
    std::fs::write(
        st.join("reference.meta"),
        format!(
            "golden: selftest reference package (CP-0.3: doc.new + brush \
             leg)\n\
             producer: appname-conformance regen_doc_new_goldens; inputs \
             mirror the CLI selftest fixed script VERBATIM \
             (crates/appname-cli/src/selftest.rs): assembled engine, \
             fixed 3-point samples asset, doc.new 320x200 black (id \
             5e1f7e57-...-0001), brush.stroke hard-round-16 seed 7 \
             opacity 0.9 (id 5e1f7e57-...-0002)\n\
             package: reference.appname ({SELFTEST_REFERENCE_BYTES} bytes)\n\
             package.sha256: {SELFTEST_REFERENCE_SHA256}\n\
             state_hash: {hash_a}\n\
             pinned-by: reports/O-CORE-log.md stroke-wiring entry (the \
             pre-stroke reference 33818115..., 527133 B is RETIRED by \
             this revision)\n\
             rule: goldens change only via O-QA review (plan section 0.6 \
             rule 6)\n"
        ),
    )
    .unwrap();
    std::fs::write(
        st.join("MANIFEST.sha256"),
        format!("{ref_sha}  reference.appname\n"),
    )
    .unwrap();
    println!(
        "golden selftest reference: {ref_sha} ({} bytes)",
        ref_a.len()
    );
    println!("goldens written under {}", root.display());
}
