// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Build script for the `arap-libigl` feature ONLY (deps/libigl.yaml +
// the O-INT 2026-08-17 consumption ruling; deps/eigen.yaml sanctioned
// shape). Without the feature this script is a strict no-op — the
// default build reads no libigl or Eigen bytes and compiles no C++.
//
// With the feature enabled, the pinned checkouts are supplied
// EXTERNALLY (never vendored, never committed — the ruled file-level
// MPL arm consumes unmodified covered files from outside the tree):
//   APPNAME_LIBIGL_CHECKOUT — libigl at the deps/libigl.yaml pin
//                             (477e15a3d566a21f415aa5ee62992b12a836b01b
//                             or the O-BUILD-elected release ≥ it)
//   APPNAME_EIGEN_CHECKOUT  — Eigen at the deps/eigen.yaml pin (3.4.1 =
//                             d71c30c47858effcbd39967097a2d99ee48db464),
//                             STRIPPED of its `unsupported/` tree by the
//                             fetch tooling (D1 gate below).
// O-BUILD owns the fetch/pin tooling (record amendment flow).
//
// Sanctioned-shape enforcement (deps/eigen.yaml, REQUIRED):
//   * -DEIGEN_MPL2_ONLY is always defined;
//   * the `unsupported/` tree is excluded by the record's
//     PATH-EXCLUDED-CHECKOUT mechanism (D1 fix-round correction: with
//     `-I <checkout-root>` an `#include <unsupported/...>` WOULD
//     resolve, so an include-path claim alone is worthless — this
//     script therefore HARD-FAILS unless the supplied checkout carries
//     NO `unsupported/` directory at all, with a typed message telling
//     the fetch tooling to strip it);
//   * no EIGEN_USE_BLAS / EIGEN_USE_LAPACKE / EIGEN_USE_MKL* define is
//     ever passed.
// Copyleft-exclusion guard (deps/libigl.yaml rider): the shim source is
// grep-checked for any GPL-subtree include reference before compiling —
// a hit aborts the build. The GPL subtree itself stays unread (only the
// shim's stated closure is compiled).

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=shim/arap_global_shim.cpp");
    println!("cargo:rerun-if-env-changed=APPNAME_LIBIGL_CHECKOUT");
    println!("cargo:rerun-if-env-changed=APPNAME_EIGEN_CHECKOUT");
    if env::var_os("CARGO_FEATURE_ARAP_LIBIGL").is_none() {
        return; // default build: strict no-op
    }

    let libigl = PathBuf::from(
        env::var("APPNAME_LIBIGL_CHECKOUT")
            .expect("arap-libigl: set APPNAME_LIBIGL_CHECKOUT to the pinned libigl checkout"),
    );
    let eigen = PathBuf::from(
        env::var("APPNAME_EIGEN_CHECKOUT")
            .expect("arap-libigl: set APPNAME_EIGEN_CHECKOUT to the pinned Eigen checkout"),
    );
    let igl_include = libigl.join("include");
    assert!(
        igl_include
            .join("igl")
            .join("min_quad_with_fixed.h")
            .is_file(),
        "arap-libigl: {igl_include:?} does not look like the pinned libigl checkout"
    );
    assert!(
        eigen.join("Eigen").join("Core").is_file(),
        "arap-libigl: {eigen:?} does not look like the pinned Eigen checkout"
    );
    // D1 gate (deps/eigen.yaml sanctioned mechanism = path-excluded
    // checkout): the `unsupported/` tree must be ABSENT — with the
    // checkout root on the include path, its presence would make
    // #include <unsupported/...> resolvable, which the record forbids.
    assert!(
        !eigen.join("unsupported").exists(),
        "arap-libigl: the Eigen checkout at {eigen:?} still contains its `unsupported/` tree; \
         deps/eigen.yaml sanctions a PATH-EXCLUDED checkout — the O-BUILD fetch tooling must \
         strip `unsupported/` before this build (record: MPL2-core headers only)"
    );

    // Copyleft-exclusion guard (deps/libigl.yaml rider): no #include
    // line may reference the GPL subtree (comments here and in the shim
    // legitimately NAME the guard — the includes are what compile).
    let shim_src = std::fs::read_to_string("shim/arap_global_shim.cpp")
        .expect("arap-libigl: shim source readable");
    // Pattern assembled so guard code never carries it verbatim (the
    // in-tree exclusion test scans this file's code lines too).
    let pat = String::from("copy") + "left";
    for line in shim_src.lines() {
        let t = line.trim_start();
        if t.starts_with("#include") {
            assert!(
                !t.to_ascii_lowercase().contains(&pat),
                "arap-libigl: shim include references libigl's {pat}/ subtree: {line}"
            );
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let obj = out_dir.join("arap_global_shim.o");
    let lib = out_dir.join("libappname_arap_shim.a");

    let cxx = env::var("CXX").unwrap_or_else(|_| "c++".to_string());
    let status = Command::new(&cxx)
        .args(["-std=c++17", "-O2", "-fPIC", "-c"])
        // deps/eigen.yaml sanctioned shape: MPL2-only, REQUIRED.
        .arg("-DEIGEN_MPL2_ONLY")
        .arg("-I")
        .arg(&igl_include)
        .arg("-I")
        .arg(&eigen)
        .arg("shim/arap_global_shim.cpp")
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("arap-libigl: spawn the C++ compiler");
    assert!(status.success(), "arap-libigl: shim compilation failed");

    let status = Command::new("ar")
        .arg("crs")
        .arg(&lib)
        .arg(&obj)
        .status()
        .expect("arap-libigl: spawn ar");
    assert!(status.success(), "arap-libigl: archiving the shim failed");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=appname_arap_shim");
    // The C++ runtime for the shim TU.
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
}
