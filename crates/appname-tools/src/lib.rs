// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-tools` — Appname interactive tools: brush engine, selections,
//! geometry/transforms, retouch tiers.
//!
//! Spec: ES Part 15 (brush engine — CP-0.3), §13 (selections), §18–§22
//! (geometry/transforms/tools); DD ⑭⑮㉖.
//!
//! Determinism spine (D-48, ES §10): every document mutation is a
//! `appname.command.v1` envelope through the single apply entry — the
//! [`brush`] module computes **pure** results (dab plans, composited tile
//! patches), and the [`kind_impls`] module is the command-engine adapter
//! that installs them strictly through the `KindContext` doors core's apply
//! pipeline hands it; no side-door mutation APIs in this crate, ever.
//!
//! CP-0.3 content: the [`brush`] module — brush engine v1 (plan §2.2 row
//! "Brush engine v1"): the `appname.samples.v1` input-sample blob codec,
//! §15.4 dynamics (monotone-cubic response curves, PCG32 jitter), §15.3
//! centripetal Catmull-Rom stroke geometry with arc-length spacing, §15.5
//! round-dab coverage + two-buffer compositing, and eraser dst-out
//! presentation — plus [`kind_impls`], the two stroke `KindImpl`s + the
//! [`kind_impls::assembly`] helper the assembly points (cli/cabi) import
//! (integration-log wave-2 arbitration 1). Selections/geometry arrive
//! CP-0.5, filter/adjustment corpora CP-0.7, retouch CP-1.0 (plan §3.7).

pub mod blur;
pub mod brush;
// ★ HONEST WALL (operator-set, Tier-1 item 2): the §11.3 layer-effects
// stack is engine scaffolding built AHEAD of its spec rows, which land at
// the v5.6 fold-in. It is a PRIVATE module on purpose — `mod`, never
// `pub mod` — so the compiler, not a convention, guarantees no crate
// outside this one can reach a single item in it. There is no command
// kind, registry entry, params schema, ABI symbol or shell surface behind
// it, and there must be none until the rows land (spec-first). Its only
// callers are its own unit tests. Promoting this to `pub mod` is the first
// act of WIRING it, and belongs to that later, spec-backed slice.
mod effects;
pub mod geometry;
pub mod kind_impls;
pub mod kind_impls_retouch;
pub mod liquify;
pub mod puppet;
pub mod resample;
pub mod retouch;
pub mod schema_shape;
pub mod select;
pub mod warp;
pub mod xform;

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-tools";

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-tools");
    }
}
