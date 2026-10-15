// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §12.1 (the ONE op registry), §16.1 (the `Filter` trait the
//! compute engines implement; the registry makes filters discoverable to
//! smart filters), DD ⑯; core seam `appname-core/src/engine/op_impl.rs`
//! (brush-handoff pattern, CP-0.7 wave-1 contract).
//!
//! The O-TOOLS compute half of the OpNode seam for the FILTER corpus:
//! each §16.3 [`Filter`] wraps as a core [`OpImpl`] and registers
//! construction-only through [`register_ops`], so `filter.smart_add` /
//! `op.set_params` stage-3 nested validation resolves real filter kinds
//! and the §12.1 evaluation sites (compositor smart-filter chains,
//! `layer.rasterize` bakes) can invoke the CPU reference kernels. First
//! production registrants through the seam together with the
//! `appname-adjust` bridge (reviewer finding, CP-0.7 wave 1).
//!
//! Bridge pins (recorded, not invented):
//! * `apply_cpu`'s plane contract, taken literally from the seam doc:
//!   `input` is row-major over `rect` "already expanded per
//!   `roi_out_to_in`", and output covers the SAME rect — the evaluator
//!   materializes aprons (§16.2), so taps beyond `rect` fall to the
//!   filter's §16.3 edge policy against `ctx.canvas := rect`. Exact for
//!   every slice-1 filter (none is canvas-dependent); the whole-canvas
//!   filter class (Liquify, CP-1.0) revisits the ctx plumbing with
//!   O-CORE.
//! * `ctx.seed = 0`: the §16.3 stochastic rule reads the per-filter
//!   `seed` PARAM (the §16.2 PCG32 convention rides `params.seed`);
//!   "Last Filter" fresh-seed minting is the recorder's (§16.5), not the
//!   kernel's.
//! * `default_params` = `{}` (§16.3 per-param defaults are the registered
//!   default set).
//! * `pointwise` = ROI-identity probed on a unit rect (exact for the
//!   slice-1 corpus: params never change WHETHER a filter is pointwise,
//!   only how far its apron reaches).
//! * GPU pairs stay O-RENDER's §34.3 handoff (`Filter::gpu_key` carries
//!   the pipeline key; CPU is the parity oracle — recorded deferral).

use std::sync::Arc;

use serde_json::{json, Value};

use appname_core::engine::op_impl::{OpImpl, PreviewPolicy};
use appname_core::{EngineAssembly, EngineError, IRect};
use appname_tools::resample::{EdgePolicy, Plane};

use crate::registry::{FilterError, FilterRegistry};
use crate::traits::{Filter, FilterCtx};

/// Map the crate's typed failures onto the wire taxonomy (§40.6):
/// unknown id → `error.schema.unknown_op_kind`, ranges →
/// `error.schema.out_of_range`, absent required →
/// `error.schema.missing_field`, ROI contract breach → typed
/// `bad_target` (programming error surfaced, never a panic).
pub(crate) fn wire_err(e: FilterError) -> EngineError {
    match e {
        FilterError::UnknownFilter(id) => EngineError::with_field(
            appname_core::ErrorCode::UnknownOpKind,
            "filterId",
            format!("filter id {id:?} is not registered (§16.1)"),
        ),
        FilterError::ParamOutOfRange { param, detail } => EngineError::range(param, detail),
        FilterError::ParamMissing(p) => EngineError::missing(p),
        FilterError::RoiNotCovered => EngineError::bad_target(
            "input plane does not cover the filter's declared ROI (DD ⑯ evaluator contract)",
        ),
    }
}

/// One filter wrapped as the core seam's [`OpImpl`].
pub struct FilterOp(pub Arc<dyn Filter>);

impl FilterOp {
    fn ctx_for(&self, rect: IRect) -> FilterCtx {
        FilterCtx {
            canvas: rect, // bridge pin (module doc)
            seed: 0,      // §16.3: stochastic seed rides params.seed
            lod: 0,
        }
    }
}

impl OpImpl for FilterOp {
    fn op_id(&self) -> &'static str {
        self.0.id()
    }

    fn validate_params(&self, params: &Value) -> Result<(), EngineError> {
        // Strictness ruling 2026-08-17: schema-letter shape first.
        self.0.validate_strict(params).map_err(wire_err)
    }

    fn default_params(&self) -> Value {
        json!({})
    }

    fn roi_out_to_in(&self, params: &Value, out: IRect) -> IRect {
        let ctx = self.ctx_for(out);
        self.0.roi(out, params, &ctx)
    }

    fn pointwise(&self) -> bool {
        let probe = IRect::new(0, 0, 1, 1);
        let ctx = self.ctx_for(probe);
        self.0.roi(probe, &json!({}), &ctx) == probe
    }

    fn preview(&self) -> PreviewPolicy {
        // §12.3 vocabulary is shared verbatim between the two seam halves.
        match self.0.ui_hints().preview {
            crate::traits::PreviewPolicy::LiveFull => PreviewPolicy::LiveFull,
            crate::traits::PreviewPolicy::LiveProxy => PreviewPolicy::LiveProxy,
            crate::traits::PreviewPolicy::RegionOnly => PreviewPolicy::RegionOnly,
            crate::traits::PreviewPolicy::NoPreview => PreviewPolicy::NoPreview,
        }
    }

    fn apply_cpu(
        &self,
        params: &Value,
        rect: IRect,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
    ) -> Result<(), EngineError> {
        let w = (rect.max.x - rect.min.x).max(0) as usize;
        let h = (rect.max.y - rect.min.y).max(0) as usize;
        let n = w * h;
        if input.len() != n || output.len() != n {
            return Err(EngineError::bad_target(format!(
                "op plane size mismatch: rect carries {n} pixels, input {} / output {}",
                input.len(),
                output.len()
            )));
        }
        let mut plane = Plane::new(rect);
        let mut i = 0;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                plane.set(x, y, input[i]);
                i += 1;
            }
        }
        let ctx = self.ctx_for(rect);
        let out = self.0.cpu(&plane, rect, params, &ctx).map_err(wire_err)?;
        let mut i = 0;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                output[i] = out.get(x, y, EdgePolicy::Transparent);
                i += 1;
            }
        }
        Ok(())
    }
}

/// Register a filter registry's corpus onto an engine assembly
/// (construction-only, §12.1; DD ⑯ — no core edits).
pub fn register_ops(assembly: EngineAssembly, registry: &FilterRegistry) -> EngineAssembly {
    registry.filters().fold(assembly, |a, f| {
        a.with_op(Arc::new(FilterOp(Arc::clone(f))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::slice1_registry;

    #[test]
    fn corpus_registers_through_the_seam_with_stable_ids() {
        let a = register_ops(EngineAssembly::new(), &slice1_registry());
        let ids: Vec<&str> = a.ops().ids().collect();
        assert_eq!(
            ids,
            vec![
                "filter.add_noise",
                "filter.box_blur",
                "filter.custom",
                "filter.gaussian_blur",
                "filter.high_pass",
                "filter.liquify",
                "filter.median",
                "filter.unsharp_mask",
            ],
            "the seven slice-1 filters + the CP-1.0 liquify checkpoint filter, deterministic order"
        );
        // Pointwise probe: noise is pointwise, gaussian is not.
        assert!(a.ops().get("filter.add_noise").unwrap().pointwise());
        assert!(!a.ops().get("filter.gaussian_blur").unwrap().pointwise());
    }

    #[test]
    fn bridge_validation_and_roi_map_the_trait_faithfully() {
        let a = register_ops(EngineAssembly::new(), &slice1_registry());
        let g = a.ops().get("filter.gaussian_blur").unwrap();
        g.validate_params(&serde_json::json!({"radius": 4.0}))
            .unwrap();
        let err = g
            .validate_params(&serde_json::json!({"radius": -3.0}))
            .unwrap_err();
        assert_eq!(err.code, appname_core::ErrorCode::OutOfRange);
        // ROI inflates by the §16.3 apron (radius 12 → σ 4 → ⌈3σ⌉ = 12).
        let out = IRect::new(0, 0, 8, 8);
        let roi = g.roi_out_to_in(&serde_json::json!({"radius": 12.0}), out);
        assert_eq!(roi, out.inflate(12, 12), "apron inflation");
    }

    #[test]
    fn bridge_apply_cpu_runs_a_kernel_over_the_seam_planes() {
        let a = register_ops(EngineAssembly::new(), &slice1_registry());
        let b = a.ops().get("filter.box_blur").unwrap();
        // 4×1 plane: an impulse blurs into its neighbors.
        let rect = IRect::new(0, 0, 4, 1);
        let input = [[0.0f32; 4], [1.0, 1.0, 1.0, 1.0], [0.0; 4], [0.0; 4]];
        let mut out = [[0.0f32; 4]; 4];
        b.apply_cpu(&serde_json::json!({"radius": 1}), rect, &input, &mut out)
            .unwrap();
        assert!(out[0][3] > 0.0 && out[2][3] > 0.0, "energy spread: {out:?}");
        assert!(out[1][3] < 1.0, "impulse lowered");
    }
}
