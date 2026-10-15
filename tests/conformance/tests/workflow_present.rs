// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Workflow-runner presence checks — ACTIVE since wave 1.
//!
//! Spec: plan §2.1 "Test harness v1" (determinism replay + package
//! round-trip runners). Since wave 4 the runners execute LIVE against the
//! built `appname` binary (both PASS; outputs recorded in the lane log);
//! the SKIP-exit-2 fallback remains for environments without the binary.
//!
//! These tests prove the runners are committed, executable, syntactically
//! valid bash, and keep that honest fallback. They do NOT execute the
//! runners end-to-end here — a full run mutates temp state and belongs to
//! the runners themselves, so under `cargo test` we only syntax-check.

use appname_conformance::paths;
use std::process::Command;

const RUNNERS: [&str; 2] = ["replay_determinism.sh", "package_roundtrip.sh"];

#[test]
fn runners_exist_and_are_executable() {
    for name in RUNNERS {
        let p = paths::workflows_dir().join(name);
        assert!(p.is_file(), "{} missing", p.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = p.metadata().unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "{} is not executable (mode {mode:o})",
                p.display()
            );
        }
    }
}

#[test]
fn runners_are_valid_bash() {
    for name in RUNNERS {
        let p = paths::workflows_dir().join(name);
        let out = Command::new("bash")
            .arg("-n")
            .arg(&p)
            .output()
            .expect("bash available");
        assert!(
            out.status.success(),
            "bash -n {}: {}",
            p.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn runners_carry_the_honest_skip_path() {
    for name in RUNNERS {
        let p = paths::workflows_dir().join(name);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("SKIP: appname-cli not built"),
            "{}: SKIP line lost",
            p.display()
        );
        assert!(
            text.contains("exit 2"),
            "{}: exit-2 environment class lost",
            p.display()
        );
        assert!(
            text.contains("shasum -a 256"),
            "{}: byte-identity via shasum -a 256 lost",
            p.display()
        );
        assert!(
            text.contains("mktemp -d"),
            "{}: temp-dir discipline lost",
            p.display()
        );
    }
}

/// The CP-1.0 W-ROW TRANSLATION WAVE (O-QA, exit-ledger row 2): the
/// fourteen previously-untranslated workflow rows, each committed as a
/// runnable script (runnable/partial rows) or a typed staged harness
/// (W08/W09 — exit 2 until their tiers merge). Presence + executability
/// + `bash -n` keep the roster honest in the standing gate; execution
/// evidence lives in the per-wave lane-log ledger.
const W_ROW_SCRIPTS: [&str; 14] = [
    "w08_plugin_safety.sh",
    "w09_accessibility.sh",
    "w10_raw_photo_subset.sh",
    "w11_print_points.sh",
    "w12_pano_scale_retouch.sh",
    "w13_batch_raw_subset.sh",
    "w15_multi_asset_export.sh",
    "w17_ora_interop_round_trip.sh",
    "w18_psd_edit_round_trip.sh",
    "w19_psb_streaming_smoke.sh",
    "w21_cli_export_matrix.sh",
    "w22_crash_fault_injection.sh",
    "w23_rtl_mid_edit_switch.sh",
    "w24_forced_cpu.sh",
];

#[test]
fn w_row_translations_exist_executable_and_parse() {
    for name in W_ROW_SCRIPTS {
        let p = paths::workflows_dir().join(name);
        assert!(p.is_file(), "{} missing", p.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = p.metadata().unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "{} is not executable (mode {mode:o})",
                p.display()
            );
        }
        let out = Command::new("bash")
            .arg("-n")
            .arg(&p)
            .output()
            .expect("bash available");
        assert!(
            out.status.success(),
            "bash -n {}: {}",
            p.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("exit 2"),
            "{}: exit-2 environment/staged class lost",
            p.display()
        );
    }
}
