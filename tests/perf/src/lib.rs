// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-perf` — deterministic-input criterion benches over PUBLIC
//! engine/compositor surfaces (O-QA perf mandate).
//!
//! Spec: plan §3.14 (perf harness) + §4.6 standing perf watch;
//! deps/criterion.yaml determinism requirement (fixed committed fixtures,
//! seeded inputs, no wall-clock/network workloads); the >15% wave-gate
//! runs `.claude/perf/compare_criterion.py` against the baselines under
//! `.claude/perf/baselines/<host-tag>/`.
//!
//! The lib target is deliberately empty (plus one smoke test) so the
//! member rides `cargo test --workspace`; the measurable content lives in
//! `benches/engine_benches.rs`.

/// Crate name marker exercised by the smoke test.
pub const CRATE_NAME: &str = "appname-perf";

#[cfg(test)]
mod tests {
    //! Spec: workspace-member smoke discipline (bootstrap precedent).
    #[test]
    fn smoke() {
        assert_eq!(super::CRATE_NAME, "appname-perf");
    }
}
