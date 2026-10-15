// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-render` — Appname compositor: CPU reference oracle (CP-0.3) +
//! wgpu render DAG (CP-0.5).
//!
//! Spec: ES §4 (compositing model §4.4, blend catalog §4.5, DAG §4.6,
//! groups §4.7, CPU reference path §4.8), §3.6/§3.7 (residency,
//! dirty-region tracking), §6.4 (premultiplied rules), §24 (viewport
//! consumer); DD ⑤⑧; parity contract ES §34.3.
//!
//! Contents (CP-0.3 "CPU compositor (reference)" + the CP-0.5 §2.3
//! rows):
//!
//! * [`TileRgba`] — the linear premultiplied f32 working tile (§4.8
//!   layout: flat interleaved, SIMD-friendly; explicit SIMD is a later
//!   pass).
//! * [`composite_tile`]/[`composite_tile_ctx`] — the full 27-mode §4.5
//!   catalog (Appendix-A formulas): 20 separable, 2 whole-pixel, 4
//!   non-separable, and Dissolve against the committed [`BN64`]
//!   blue-noise table ([`BLEND_SET_V1`] remains the CP-0.3 golden-suite
//!   mode set until O-QA's wave-2 extension).
//! * [`composite_document`]/[`recomposite_dirty`]/[`Composite`] — the
//!   naive reference walk (THE oracle): §4.7 pass-through vs isolated
//!   groups, §4.6 step-4b intermediates, typed refusals for everything
//!   not yet renderable. The `_with` doors ([`composite_document_with`],
//!   [`recomposite_dirty_with`], [`flatten_document_with`]) carry a §12.1
//!   op registry and SERVE parametric content (reopened-CP-0.7 §12.1
//!   slice): smart-object proxies as raster surfaces, enabled
//!   smart-filter chains via core's exported `ChainEvaluator`, adjustment
//!   layers as §17.1/§11.4 pass-through-over-backdrop at root scope —
//!   with core's typed-refusal matrix inherited verbatim for what
//!   §11/§12.1 do not define. Parametric serving is FORCED-CPU (W24
//!   first-class; recorded deferral in the compositor module doc).
//! * [`composite_subtree_with`]/[`flatten_subtree_with`] (+ bare-registry
//!   twins) — the subtree-composite door (O-SCRIPT ABI-1.4 handoff: the
//!   GROUP arm of `appname_doc_read_layer`): one group's subtree in
//!   isolation onto transparency, content-not-contribution (the group's
//!   own blend/opacity/visibility never applied; full §4.4/§4.7 semantics
//!   within), root-as-target ≡ the document composite by construction,
//!   refusal matrix inherited via the shared flatten walk.
//! * [`Renderer`]/[`RenderStats`] — the production DAG engine (§4.6):
//!   per-(node, tile) content-hash caching with §2.7 dedup, §4.1
//!   pull-driven ROI scheduling; output pinned bit-equal to the oracle
//!   for any cache state.
//! * [`parity`] — the §34.3 GPU/CPU parity harness: backend trait,
//!   deterministic 27-mode kernel corpus, max-abs/SSIM metrics plus the
//!   Dissolve coverage/own-baseline verdicts, all with tested detectors
//!   ([`CpuBackend`] is unconditional — W24 forced-CPU is first-class
//!   by construction).
//! * [`wgsl`] — the committed `cs_composite_full` shader contract
//!   ([`COMPOSITE_WGSL`]) with source-level drift pins + headless naga
//!   validation.
//! * [`gpu`] — the wgpu backend over that contract ([`GpuBackend`],
//!   §4.2 init with typed [`GpuUnavailable`] and the CPU floor on every
//!   failure); the live §34.3 gate is `tests/gpu_parity.rs` — GREEN on
//!   Metal (lane-log evidence).
//! * [`DirtyTracker`]/[`RegionSet`]/[`DirtyMap`] — §3.7 dirty-region
//!   tracking, shaped so command receipts feed it.
//!
//! **This CPU path is the permanent parity oracle** (ES §34.3): the CP-0.5
//! wgpu pipelines are asserted against it at ≤ 1/255 on the golden corpus,
//! forever. Correctness over speed; every unrenderable state is a typed
//! error, never approximate math.
//!
//! Determinism spine (D-48, ES §10): this crate only READS documents
//! (core's public surface); every document mutation is an
//! `appname.command.v1` envelope through the single apply entry — no
//! side-door mutation APIs in this crate, ever. Compositing itself is
//! deterministic: fixed walk order, fixed tile order, scalar-identical
//! math on every run and platform.

pub mod blend;
pub mod bn64;
pub mod compositor;
pub mod dag;
pub mod dirty;
pub mod error;
pub mod gpu;
pub mod parity;
pub mod tile_rgba;
pub mod wgsl;

pub use blend::{
    blend_implemented, composite_tile, composite_tile_ctx, render_separable_kernel,
    render_triple_kernel, routing, BlendRouting, DissolveCtx, SeparableKernel, TripleKernel,
    BLEND_SET_V1,
};
pub use bn64::{bn_offsets, bn_threshold, BN64};
pub use compositor::{
    composite_document, composite_document_with, composite_subtree, composite_subtree_with,
    flatten_document, flatten_document_with, flatten_subtree, flatten_subtree_with,
    recomposite_dirty, recomposite_dirty_with, AdjustmentItem, Composite, FlatLayer, IsolatedGroup,
    PageLeaf, RecompositeStats, RenderItem, SmartLeaf,
};
pub use dag::{RenderStats, Renderer, RENDER_TILE_CACHE_BUDGET_BYTES};
pub use dirty::{DirtyMap, DirtyRecord, DirtyTracker, RegionSet, COALESCE_SLOP, REGION_SET_CAP};
pub use error::CompositeError;
pub use gpu::{GpuBackend, GpuUnavailable};
pub use parity::{
    kernel_corpus, masked_corpus, masked_source, max_abs_8bit, parametric_station, run_parity,
    ssim, CompositeBackend, CpuBackend, KernelCase, ParityFailure, ParityReport,
};
pub use tile_rgba::{TileRgba, TILE_SCALARS};
pub use wgsl::{COMPOSITE_ENTRY, COMPOSITE_WGSL};

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-render";

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-render");
    }
}
