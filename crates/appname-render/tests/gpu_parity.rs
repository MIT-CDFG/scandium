// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The live §34.3 gate: GPU backend vs the CPU oracle over the in-crate
//! kernel corpus. Adapter absence is a STATE, not a failure (W24 —
//! headless CI stays green on the CPU floor; the §34.3 runner model
//! puts hardware legs on their own tiers); any present adapter must
//! pass the full tolerance table.

use appname_render::{
    kernel_corpus, masked_corpus, parametric_station, run_parity, CpuBackend, GpuBackend,
    GpuUnavailable,
};

#[test]
fn gpu_parity_gate_on_available_adapter() {
    let mut gpu = match GpuBackend::new() {
        Ok(g) => g,
        Err(GpuUnavailable::NoAdapter) => {
            // W24 legitimizes adapter ABSENCE only. On the §34.3 runner
            // tiers (lavapipe / Metal / WARP) the adapter is part of the
            // runner contract: APPNAME_REQUIRE_ADAPTER=1 makes absence a
            // hard failure there, so a misconfigured runner can never
            // green the per-PR gate silently (slice-D review MAJOR-1).
            if std::env::var_os("APPNAME_REQUIRE_ADAPTER").is_some() {
                panic!(
                    "APPNAME_REQUIRE_ADAPTER is set but no adapter was found — \
                     §34.3 runner misconfiguration"
                );
            }
            eprintln!("gpu parity: no adapter — CPU path active (W24)");
            return;
        }
        Err(state @ (GpuUnavailable::Device(_) | GpuUnavailable::Pipeline(_))) => {
            // An adapter EXISTS but the backend failed to build: that is
            // a §34.3 parity-blocking condition (driver-side validation
            // failure), never a skip.
            panic!("adapter present but GPU backend init failed — §34.3 gate: {state:?}");
        }
    };
    // The kernel corpus PLUS the §34.3 parametric station (composite-legs
    // slice): shape §20.2 / text §23.2 page content rides the SAME live
    // GPU-vs-CPU gate raster/blend/group content always did — the whole
    // point of the slice (parametric content gains parity it lacked).
    let mut corpus = kernel_corpus();
    corpus.extend(parametric_station());
    // §11.2 mask row: the coverage-multiply (`src *= E`) rides the SAME
    // live GPU-vs-CPU ≤1/255 gate — masked content gains the parity it
    // lacked (a single correctly-rounded multiply, provably exact).
    corpus.extend(masked_corpus());
    let report = run_parity(&mut gpu, &mut CpuBackend, &corpus);
    eprintln!(
        "gpu parity [{}]: {report:?}",
        appname_render::CompositeBackend::name(&gpu)
    );
    report.gate().unwrap_or_else(|e| {
        panic!("§34.3 parity gate FAILED on a live adapter: {e:?} — full report: {report:?}")
    });
}
