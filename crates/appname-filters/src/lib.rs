// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-filters` — the §16.1 `Filter` trait, registry, and the CPU
//! reference corpus (CP-0.7 wave 1, plan §2.4 "Filter engine + corpus v1").
//!
//! Spec: ES §16.1 ([NORMATIVE] trait shape — params schema, WGSL GPU + SIMD
//! CPU pair, ROI mapping, preview hint; registry discoverability; "adding a
//! filter requires no core edits"), §16.2 (ROI-driven evaluation, separable
//! two-pass pattern, selection masking at the EVALUATOR — filters stay
//! selection-unaware), §16.3 (per-filter normative blocks + the corpus
//! conventions: edge policies clamp/transparent, premultiplied linear f32
//! working tiles, stochastic filters seeded `Pcg32::new(seed ^
//! blake3_64(tile.x, tile.y, lod), stream: pass_index)` [NORMATIVE] —
//! deterministic, replayable, tile-order independent); DD ⑯ (the ROI
//! contract), §34.3 (the CPU path is the parity oracle for the GPU pair).
//!
//! CP-0.7 slice-1 scope (O-INT brief): CPU-first — every filter DESIGNS its
//! GPU pair by carrying the stable pipeline key (`Filter::gpu_key`) the
//! WGSL half will register under, but no WGSL ships from this lane; the GPU
//! implementations + the ≤1/255 parity harness are O-RENDER's (handoff
//! flagged, lane log). The command seam is WIRED (CP-0.7 wave 2):
//! `kinds` carries the `filter.apply`/`filter.smart_add` KindImpls and
//! `op_bridge` registers the corpus through core's §12.1 OpImpl seam —
//! the engines here stay pure over `appname_tools::resample::Plane`
//! views with params validated against each filter's §16.3 schema.
//!
//! Determinism: same params + same input plane + same seed ⇒ identical
//! bytes (fixed tap orders, the §16.3 RNG rule, no libm beyond `exp`/`sin`
//! documented per filter — the brush-module caveat class).

pub mod corpus;
pub mod kinds;
pub mod op_bridge;
pub mod registry;
pub mod traits;

pub use kinds::register;
pub use registry::FilterRegistry;
pub use traits::{EdgeRule, Filter, FilterCtx, FilterGroup, FilterUi, PreviewPolicy};

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-filters";

#[cfg(test)]
mod tests {
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-filters");
    }
}
