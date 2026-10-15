// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §16.1 [NORMATIVE] — each filter declares its parameters (with a
//! schema for validation/UI), a GPU implementation, a CPU implementation,
//! an ROI mapping (output rect → required input rect), and a preview hint;
//! FilterIds are `filter.snake_case`, stable forever; params are camelCase
//! JSON validated against the filter's schema before execution (§10.2
//! discipline); every filter runs on premultiplied linear f32 RGBA
//! (§4.4); DD ⑯.
//!
//! CPU-first adaptation (slice-1 pin, lane log): the §16.1 sketch's
//! `fn gpu(&self) -> &'static str` (WGSL source/pipeline key) is carried as
//! [`Filter::gpu_key`] — the stable key O-RENDER's pipelines will register
//! under; no WGSL ships from this lane. The §16.1 `cpu` sketch operates on
//! working tiles through an `Apron`; the pure engines here take the
//! materialized ROI as a [`Plane`] view (`input.rect ⊇ roi(out_rect)`) and
//! write a full output plane — O-CORE's evaluator materializes aprons
//! through the tile system when the OpNode seam publishes (the §16.2
//! scheduling is the evaluator's, not the filter's).

use appname_core::IRect;
use appname_tools::resample::Plane;
use serde_json::Value;

use crate::registry::FilterError;

/// §16.3 conventions: per-filter edge policy vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeRule {
    /// Reads outside the canvas clamp to the nearest edge pixel.
    Clamp,
    /// Reads outside the canvas return `(0,0,0,0)` (correct under
    /// premultiplication).
    Transparent,
}

/// §16.1 `PreviewPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewPolicy {
    LiveFull,
    LiveProxy,
    RegionOnly,
    NoPreview,
}

/// §16.1 `FilterGroup` (the §16.3 catalog groups).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterGroup {
    Blur,
    Sharpen,
    Noise,
    Distort,
    Stylize,
    Render,
    Pixelate,
    Other,
}

/// §16.1 `FilterUi`.
#[derive(Clone, Copy, Debug)]
pub struct FilterUi {
    pub preview: PreviewPolicy,
    pub has_dialog: bool,
    pub group: FilterGroup,
    /// §10.3 `checkpoint:true` arm (§45.4 commit-record reconciliation):
    /// a whole-canvas checkpoint filter — class C with the snap-form
    /// inverse at the recorder (the §45.4 [NORMATIVE] resolution names
    /// `filter.liquify` the instance; every slice-1 corpus filter is
    /// class R and carries `false`). The recorder-side inverse is
    /// realized as COW tile deltas over the touched region — the exact
    /// equivalent of the snap over the write support (kinds.rs pin).
    pub checkpoint: bool,
}

/// The slice-1 execution context (§16.1 `FilterCtx`, CPU half): document
/// geometry for edge policy, the command-recorded seed for stochastic
/// filters, and the proxy LOD tag. The §16.1 `color: &ColorContext` slot
/// joins when the §5 working-space services wire through the OpNode seam
/// (handoff note — no filter in slice 1 needs it).
#[derive(Clone, Copy, Debug)]
pub struct FilterCtx {
    /// Document canvas rect (§16.3 edge-policy input).
    pub canvas: IRect,
    /// Command-recorded seed (§16.3 stochastic rule).
    pub seed: u64,
    /// 0 = full res; preview passes may run at proxy LOD (§12.3).
    pub lod: u8,
}

/// The §16.1 trait, CPU-first.
pub trait Filter: Send + Sync {
    /// `filter.snake_case`, stable forever (§16.1).
    fn id(&self) -> &'static str;

    /// JSON Schema for the params (§16.1; the §16.3 param tables).
    fn param_schema(&self) -> &'static str;

    /// UI hints (§16.1).
    fn ui_hints(&self) -> FilterUi;

    /// DD ⑯ ROI contract: the input rect this filter must read to produce
    /// `out` (apron for kernels; `ctx.canvas` for whole-image filters).
    fn roi(&self, out: IRect, params: &Value, ctx: &FilterCtx) -> IRect;

    /// The filter's DECLARED §16.3 edge policy ("reads outside clamp to
    /// the nearest edge pixel" / "return (0,0,0,0)") — anchored at the
    /// DOCUMENT data bounds (canvas ∪ the target layer's §2.5 content
    /// bounds, ES:2617 [NORMATIVE]; off-canvas content is first-class),
    /// so the EVALUATOR materializes aprons with it before `cpu` runs
    /// (CP-0.7 wave-2 fix rounds D1 + R1: the kernels' own `Plane::get`
    /// edge handling only extends the materialized plane, it cannot
    /// know the document).
    fn edge(&self) -> EdgeRule;

    /// The stable pipeline key the GPU pair registers under (§16.1 `gpu`;
    /// WGSL itself is O-RENDER's — slice-1 pin).
    fn gpu_key(&self) -> &'static str;

    /// CPU reference (§16.1/§34.3 golden): `input` covers at least
    /// `roi(out_rect)`; returns the output plane over `out_rect`.
    /// Premultiplied linear f32 in and out (§4.4).
    fn cpu(
        &self,
        input: &Plane,
        out_rect: IRect,
        params: &Value,
        ctx: &FilterCtx,
    ) -> Result<Plane, FilterError>;

    /// Validate `params` against the filter's schema semantics (ranges +
    /// cross-fields; stage-3-analog — the §40.6 registry route re-checks
    /// via `param_schema` when the command seam lands).
    fn validate(&self, params: &Value) -> Result<(), FilterError>;

    /// The CONTRACT validation surface (strictness ruling 2026-08-17,
    /// O-UI probe filing): the per-filter hand validator runs FIRST
    /// (it owns the precise taxonomy — missing_field, ranges, the
    /// recorded non-finite pins), THEN the registered schema's
    /// structural letter — unknown keys refuse typed per
    /// `additionalProperties: false`, required/type/enum rows hold,
    /// recursively — as the backstop. Every seam that admits wire
    /// params (the §40.6 command route, the §12.1 op bridge) calls
    /// THIS, so a filter cannot opt back into silent tolerance by
    /// under-implementing `validate`.
    fn validate_strict(&self, params: &Value) -> Result<(), FilterError> {
        // Hand validator FIRST: it owns the precise taxonomy for its
        // rows (ParamMissing keeps error.schema.missing_field, ranges
        // their messages); the shape check then closes everything the
        // hand pass tolerated (unknown keys, typo'd types, enum
        // strays) — and doubles as the required/type backstop for any
        // future registrant whose hand validator under-checks.
        self.validate(params)?;
        appname_tools::schema_shape::check_shape(self.param_schema(), params).map_err(|v| {
            FilterError::ParamOutOfRange {
                param: "params",
                detail: format!("{v}"),
            }
        })
    }
}
