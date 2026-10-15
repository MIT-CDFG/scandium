// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-adjust` — the §17 adjustments corpus: pure working-space
//! functions compiled to LUTs where applicable (CP-0.7 wave 1, plan §2.4
//! "Adjustments corpus" groundwork).
//!
//! Spec: ES §17.1 ([NORMATIVE] adjustment model — pure functions in the
//! working space, per-channel LUT compilation, the shared histogram),
//! §17.2 ([NORMATIVE] exact math per block + the evaluation-domain rule:
//! kernels un-premultiply → apply → re-premultiply; display-referred
//! kernels wrap themselves in the §5 display transfer; `Lut1D` falls back
//! to the closed form for out-of-[0,1] f32 inputs — no silent
//! clamp-by-LUT), §17.2 Auto algorithms; DD ㉘ (LUT compilation), DD ⑯
//! context. OpNode wiring: the corpus registers through core's §12.1
//! OpImpl seam (`op_bridge`, CP-0.7 wave 2 — `adjustment.add` resolves
//! real kinds); the OpNode-independent `image.auto_*` bake kinds live in
//! `kinds` (the ACCEPTED wiring design: analyses → Levels params → the
//! existing edit machinery).
//!
//! Interim display transfer (documented divergence, the stroke/fill
//! precedent): the §5 working-space display transfer is the sRGB pair
//! (`appname-core io::srgb`) until the CMM display path wires through the
//! seam; every display-referred block routes through ONE pair of hooks
//! here, so the CMM swap is a two-line change.
//!
//! Rights: every §17.2 block is elementary tone/color arithmetic bound by
//! the spec's own math (L0 class; the hexcone HSL and PCHIP citations ride
//! the ES text — Fritsch & Carlson 1980 via the shared appname-tools
//! machinery). Select-and-screen judgment only, no release claim.

pub mod auto;
pub mod histogram;
pub mod kernel;
pub mod kinds;
pub mod op_bridge;
pub mod ops;

pub use kernel::{AdjKernel, Adjustment, ApplyStats};
pub use kinds::register;

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-adjust";

#[cfg(test)]
mod tests {
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-adjust");
    }
}
