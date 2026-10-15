// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Gate wiring for the C side of the ABI (plan §2.2 "C smoke test in gate";
//! ES Appendix N.2 seeds), executed by `cargo test --workspace` so
//! `scripts/gate.sh` runs it without modification:
//!
//! 1. `c_smoke_builds_and_passes` — compiles `tests/smoke.c` with the
//!    system `cc` against the committed `include/appname.h` + the built
//!    staticlib and runs it (black-box conformance host, N.1 discipline).
//! 2. `header_compiles_standalone_c99` — an empty TU including the header
//!    compiles under `-std=c99 -pedantic -Wall -Wextra -Werror`.
//! 3. `header_has_no_cbindgen_drift` — regenerates the header with the
//!    installed cbindgen and diffs against the committed file (N.2
//!    `abi_sym_header_drift` seed; the cbindgen binary is a build-time tool
//!    — deps/ record owned by O-BUILD).
//! 4. `cdylib_exports_exactly_the_v1_symbols` — `nm` census of the built
//!    cdylib against the frozen v1 symbol table (N.2
//!    `abi_sym_exports_match_golden` seed; the appname_ namespace is
//!    asserted exactly, non-appname platform noise is reported, not failed
//!    — full namespace hygiene lands with O-BUILD's exported-symbols list).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The frozen v1 symbol table (ABI 1.0 + the released 1.1 block + the 1.2
/// additive block) — single source of truth in the crate (ES Appendix N.2
/// rules: additions need a new additive header block + conformance tests).
use appname_cabi::V1_SYMBOLS;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn scratch_dir() -> PathBuf {
    let dir = workspace_root().join("target/cabi-smoke");
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// `cargo build -p appname-cabi` so the staticlib/cdylib artifacts exist
/// regardless of how this test binary itself was produced.
fn build_artifacts() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(&cargo)
        .args(["build", "-p", "appname-cabi"])
        .current_dir(workspace_root())
        .output()
        .expect("spawn cargo build");
    assert!(
        out.status.success(),
        "cargo build -p appname-cabi failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    workspace_root().join("target/debug")
}

fn run(cmd: &mut Command, what: &str) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("spawn {what}: {e}"));
    assert!(
        out.status.success(),
        "{what} failed (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// W-21 (O-WIN): the Windows arm. MSVC's `cl` stands in for `cc`: /std:c11 is
// the oldest C mode cl offers (no C99 switch); /W4 /WX is the -Wall -Wextra
// -Werror line, less C4456 (shadowed local), which -Wall -Wextra does not
// enable either; /MD matches the msvcrt the Rust staticlib links. The system
// libs are rustc's `--print native-static-libs` for this staticlib; z.lib is
// the static zlib libz-sys builds (freetype-sys emits `link-lib=z`).
// W-21 (O-WIN): cl cannot open `\\?\` verbatim paths (what canonicalize
// returns on Windows); hand it the plain form.
#[cfg(windows)]
fn plain(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
}

#[cfg(windows)]
fn libz_dir(debug: &Path) -> PathBuf {
    std::fs::read_dir(debug.join("build"))
        .expect("read target/debug/build")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("libz-sys-"))
        })
        .map(|p| p.join("out").join("lib"))
        .find(|p| p.join("z.lib").exists())
        .expect("libz-sys z.lib under target/debug/build")
}

#[cfg(windows)]
#[test]
fn c_smoke_builds_and_passes() {
    let root = workspace_root();
    let debug = build_artifacts();
    let staticlib = debug.join("appname_cabi.lib");
    assert!(
        staticlib.exists(),
        "staticlib missing at {}",
        staticlib.display()
    );

    let exe = scratch_dir().join("cabi_smoke.exe");
    let obj = scratch_dir().join("smoke.obj");
    run(
        Command::new("cl")
            .args(["/nologo", "/MD", "/std:c11", "/W4", "/WX", "/wd4456", "/I"])
            .arg(plain(&root.join("include")))
            .arg(plain(&root.join("crates/appname-cabi/tests/smoke.c")))
            .arg(plain(&staticlib))
            .arg(format!("/Fe{}", plain(&exe).display()))
            .arg(format!("/Fo{}", plain(&obj).display()))
            .arg("/link")
            .arg(format!("/LIBPATH:{}", plain(&libz_dir(&debug)).display()))
            .args([
                "z.lib",
                "kernel32.lib",
                "opengl32.lib",
                "legacy_stdio_definitions.lib",
                "ntdll.lib",
                "userenv.lib",
                "ws2_32.lib",
                "dbghelp.lib",
            ]),
        "cl compile+link of tests/smoke.c",
    );

    // smoke.c joins "<dir>/name"; a verbatim `\\?\` dir rejects the '/'.
    let stdout = run(
        Command::new(&exe).env("CABI_SMOKE_DIR", plain(&scratch_dir())),
        "cabi_smoke execution",
    );
    println!("--- cabi_smoke stdout ---\n{stdout}--- end ---");
    assert!(
        stdout.contains("cabi smoke: PASS"),
        "smoke did not print PASS:\n{stdout}"
    );
    assert!(
        !stdout.contains("FAIL"),
        "smoke printed a FAIL line:\n{stdout}"
    );
}

#[cfg(not(windows))]
#[test]
fn c_smoke_builds_and_passes() {
    let root = workspace_root();
    let debug = build_artifacts();
    let staticlib = debug.join("libappname_cabi.a");
    assert!(
        staticlib.exists(),
        "staticlib missing at {}",
        staticlib.display()
    );

    let exe = scratch_dir().join("cabi_smoke");
    run(
        Command::new("cc")
            .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(root.join("include"))
            .arg(root.join("crates/appname-cabi/tests/smoke.c"))
            .arg(&staticlib)
            // Apple frameworks the staticlib references once appname-render
            // carries the wgpu GPU backend (objc2 glue; O-RENDER slice D's
            // verified exact set). Inert-harmless before that merge: the
            // frameworks exist on every macOS and unused -framework flags
            // only lengthen the link line.
            .args([
                "-framework",
                "Foundation",
                "-framework",
                "Metal",
                "-framework",
                "QuartzCore",
                "-framework",
                "CoreFoundation",
                "-framework",
                "CoreGraphics",
            ])
            // System libs the staticlib references since the TEXTVEC
            // text-kinds wiring slice put appname-text (vendored
            // HarfBuzz C++ objects + FreeType with its png/zlib use)
            // into the production graph: the C++ runtime and zlib —
            // the same libs the -sys build scripts hand every Rust
            // link (mechanical cross-fence companion, O-TEXTVEC).
            .args(["-lc++", "-lz"])
            .arg("-o")
            .arg(&exe),
        "cc compile+link of tests/smoke.c",
    );

    let stdout = run(
        // Scratch dir for the smoke's own file artifacts (export target,
        // autosave log) — the exe inherits the test cwd otherwise.
        Command::new(&exe).env("CABI_SMOKE_DIR", scratch_dir()),
        "cabi_smoke execution",
    );
    // Echo the transcript into the test output (report evidence).
    println!("--- cabi_smoke stdout ---\n{stdout}--- end ---");
    assert!(
        stdout.contains("cabi smoke: PASS"),
        "smoke did not print PASS:\n{stdout}"
    );
    assert!(
        !stdout.contains("FAIL"),
        "smoke printed a FAIL line:\n{stdout}"
    );
}

#[test]
fn header_compiles_standalone_c99() {
    let root = workspace_root();
    let tu = scratch_dir().join("header_standalone.c");
    std::fs::write(
        &tu,
        // WORD 10 (4): \u{2D} is the hyphen -- the C file written below is byte-identical.
        "/* SPDX-License\u{2D}Identifier: Apache-2.0\n\
          * Copyright (c) 2026 the Appname contributors */\n\
         #include \"appname.h\"\n\
         int main(void) { return (int)appname_version().abi_major - 1; }\n",
    )
    .expect("write standalone TU");
    // W-21 (O-WIN): cl has no -pedantic C99 mode; /std:c11 /W4 /WX is its
    // strictest standard C compile.
    #[cfg(windows)]
    {
        let obj = scratch_dir().join("header_standalone.obj");
        run(
            Command::new("cl")
                .args(["/nologo", "/std:c11", "/W4", "/WX", "/c", "/I"])
                .arg(plain(&root.join("include")))
                .arg(plain(&tu))
                .arg(format!("/Fo{}", plain(&obj).display())),
            "standalone C11 /W4 /WX compile of appname.h",
        );
        return;
    }
    #[cfg(not(windows))]
    let obj = scratch_dir().join("header_standalone.o");
    #[cfg(not(windows))]
    run(
        Command::new("cc")
            .args([
                "-std=c99",
                "-pedantic",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-c",
                "-I",
            ])
            .arg(root.join("include"))
            .arg(&tu)
            .arg("-o")
            .arg(&obj),
        "standalone C99 -pedantic compile of appname.h",
    );
}

#[test]
fn header_has_no_cbindgen_drift() {
    let root = workspace_root();
    let cbindgen = which_cbindgen().unwrap_or_else(|| {
        panic!(
            "cbindgen not found on PATH or in ~/.cargo/bin — it is the header \
             generation tool (build-time dependency; deps/ record owned by \
             O-BUILD). Install with `cargo install cbindgen --version 0.29.4` \
             and re-run; the committed include/appname.h must never drift \
             silently (ES Appendix N.2 abi_sym_header_drift)."
        )
    });
    let regen = scratch_dir().join("appname.h.regen");
    run(
        Command::new(cbindgen)
            .arg("--config")
            .arg(root.join("crates/appname-cabi/cbindgen.toml"))
            .args(["--crate", "appname-cabi", "--output"])
            .arg(&regen)
            .arg(root.join("crates/appname-cabi")),
        "cbindgen regeneration",
    );
    let committed =
        std::fs::read_to_string(root.join("include/appname.h")).expect("read committed header");
    let fresh = std::fs::read_to_string(&regen).expect("read regenerated header");
    assert!(
        committed == fresh,
        "include/appname.h drifted from the crate: regenerate with \
         crates/appname-cabi/gen_header.sh and commit both (first divergence at byte {})",
        committed
            .bytes()
            .zip(fresh.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(0)
    );
}

fn which_cbindgen() -> Option<PathBuf> {
    if let Ok(out) = Command::new("cbindgen").arg("--version").output() {
        if out.status.success() {
            return Some(PathBuf::from("cbindgen"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    let candidate = Path::new(&home).join(".cargo/bin/cbindgen");
    candidate.exists().then_some(candidate)
}

#[test]
fn cdylib_exports_exactly_the_v1_symbols() {
    let debug = build_artifacts();
    let dylib = if cfg!(target_os = "macos") {
        debug.join("libappname_cabi.dylib")
    } else if cfg!(windows) {
        // W-21 (O-WIN): MSVC names the cdylib without the `lib` prefix.
        debug.join("appname_cabi.dll")
    } else {
        debug.join("libappname_cabi.so")
    };
    assert!(dylib.exists(), "cdylib missing at {}", dylib.display());

    // macOS: `nm -gU` = global defined symbols; strip the leading underscore
    // (the N.2 extraction recipe). Windows (W-21): `dumpbin /exports`, whose
    // table rows read `ordinal hint RVA name [= target]`.
    let out = if cfg!(windows) {
        run(
            Command::new("dumpbin")
                .args(["/nologo", "/exports"])
                .arg(&dylib),
            "dumpbin export extraction",
        )
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4 && f[0].parse::<u32>().is_ok()).then(|| f[3].to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
    } else {
        run(
            Command::new("nm").arg("-gU").arg(&dylib),
            "nm symbol extraction",
        )
    };
    let mut exported: Vec<String> = Vec::new();
    let mut foreign: Vec<String> = Vec::new();
    for line in out.lines() {
        let Some(name) = line.split_whitespace().last() else {
            continue;
        };
        let name = name.strip_prefix('_').unwrap_or(name);
        if let Some(stripped) = name.strip_prefix("appname_") {
            exported.push(format!("appname_{stripped}"));
        } else {
            foreign.push(name.to_string());
        }
    }
    exported.sort();
    exported.dedup();
    let expected: Vec<String> = V1_SYMBOLS.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        exported, expected,
        "cdylib appname_ symbol census diverges from the frozen v1 table"
    );
    if !foreign.is_empty() {
        // Namespace hygiene beyond appname_ (leaked runtime symbols) is
        // O-BUILD's exported-symbols-list work; report, don't fail (N.2
        // full enforcement arrives with the packaging lane).
        println!("note: non-appname exported symbols present: {foreign:?}");
    }
}

#[test]
fn header_declares_exactly_the_v1_symbols() {
    // Census the committed header: every `appname_*(` declaration.
    let root = workspace_root();
    let header =
        std::fs::read_to_string(root.join("include/appname.h")).expect("read committed header");
    let mut declared: Vec<String> = Vec::new();
    for line in header.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        let mut rest = trimmed;
        while let Some(pos) = rest.find("appname_") {
            let tail = &rest[pos..];
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let after = &tail[name.len()..];
            if after.starts_with('(') && !declared.contains(&name) {
                declared.push(name.clone());
            }
            rest = &tail[name.len().max(1)..];
        }
    }
    declared.sort();
    let expected: Vec<String> = V1_SYMBOLS.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        declared, expected,
        "header function census diverges from the frozen v1 table"
    );
}
