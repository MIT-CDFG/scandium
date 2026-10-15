// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §12.1 (the ONE op registry; `OpInfo`), §16.1/§17.1 trait
//! split, DD ⑯ ("a new filter is addable with no edits to core"), DD ⑨
//! (§12.3 preview policy); core seam `appname-core/src/engine/op_impl.rs`
//! (brush-handoff pattern, CP-0.7 wave-1 contract).
//!
//! The O-TOOLS compute half of the OpNode seam for the ADJUSTMENTS corpus:
//! each §17.2 [`Adjustment`] is wrapped as a core [`OpImpl`] and
//! registered construction-only through [`register_ops`], so
//! `adjustment.add` / `op.set_params` stage-3 nested validation resolves
//! against real registered kinds and the §12.1 bake/composite sites can
//! invoke the CPU reference kernels. This crate is the FIRST production
//! registrant through the seam (reviewer finding, CP-0.7 wave 1).
//!
//! Bridge pins (recorded, not invented):
//! * `default_params` = `{}` — every §17.2 param carries its own
//!   block-documented default, so the empty object IS the registered
//!   default set (`adjustment.add` with `params: {}` compiles to the
//!   block's identity/default form).
//! * Adjustments are pointwise (§17.1 pure working-space functions): ROI
//!   is identity and fusion is allowed (§4.6).
//! * `PreviewPolicy::LiveFull` for the whole corpus (LUT-compiled or
//!   cheap per-pixel forms; DD ⑨ full-res live preview).
//! * `apply_cpu` compiles the kernel per invocation (deterministic; the
//!   §17.1 LUT cache is the evaluator's concern, not the registry's).
//! * GPU pairs: deferred to O-RENDER's §34.3 parity harness with the
//!   filter corpus (standing slice-1 handoff; CPU is the oracle).

use std::sync::Arc;

use serde_json::{json, Value};

use appname_core::engine::op_impl::{OpImpl, PreviewPolicy};
use appname_core::{EngineAssembly, EngineError, IRect};

use crate::kernel::{apply_kernel_pixel, AdjError, Adjustment};
use crate::ops::slice1_adjustments;

/// Map the crate's typed param failures onto the wire taxonomy (the
/// stroke-kind handoff rule: ranges → `error.schema.out_of_range`, absent
/// required → `error.schema.missing_field`).
fn wire_err(e: AdjError) -> EngineError {
    match e {
        AdjError::ParamOutOfRange { param, detail } => EngineError::range(param, detail),
        AdjError::ParamMissing(p) => EngineError::missing(p),
    }
}

/// One adjustment wrapped as the core seam's [`OpImpl`].
pub struct AdjustmentOp(pub Arc<dyn Adjustment>);

impl OpImpl for AdjustmentOp {
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

    fn roi_out_to_in(&self, _params: &Value, out: IRect) -> IRect {
        out // pointwise (§17.1)
    }

    fn pointwise(&self) -> bool {
        true
    }

    fn preview(&self) -> PreviewPolicy {
        PreviewPolicy::LiveFull
    }

    fn apply_cpu(
        &self,
        params: &Value,
        rect: IRect,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
    ) -> Result<(), EngineError> {
        let n =
            (rect.max.x - rect.min.x).max(0) as usize * (rect.max.y - rect.min.y).max(0) as usize;
        if input.len() != n || output.len() != n {
            return Err(EngineError::bad_target(format!(
                "op plane size mismatch: rect carries {n} pixels, input {} / output {}",
                input.len(),
                output.len()
            )));
        }
        let kernel = self.0.compile(params).map_err(wire_err)?;
        for (o, i) in output.iter_mut().zip(input.iter()) {
            let (v, _fallback) = apply_kernel_pixel(&kernel, *i);
            *o = v;
        }
        Ok(())
    }
}

/// Register the §17.2 adjustments corpus onto an engine assembly
/// (construction-only, §12.1). The assembly points wrap their existing
/// chain in this call — no core edits (DD ⑯).
pub fn register_ops(assembly: EngineAssembly) -> EngineAssembly {
    slice1_adjustments()
        .into_iter()
        .fold(assembly, |a, adj| a.with_op(Arc::new(AdjustmentOp(adj))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::engine::op_impl::OpAssembly;

    #[test]
    fn corpus_registers_and_ids_are_sorted_adjustment_namespace() {
        let a = register_ops(EngineAssembly::new());
        let ids: Vec<&str> = a.ops().ids().collect();
        assert_eq!(
            ids,
            vec![
                "adjustment.brightness_contrast",
                "adjustment.curves",
                "adjustment.exposure",
                "adjustment.hue_saturation",
                "adjustment.invert",
                "adjustment.levels",
                "adjustment.posterize",
                "adjustment.threshold",
            ],
            "the eight slice-1 adjustments, deterministic order"
        );
        let _bare = OpAssembly::new(); // seam type stays constructible
    }

    #[test]
    fn bridge_validates_and_applies_through_the_seam_shape() {
        let a = register_ops(EngineAssembly::new());
        let inv = a
            .ops()
            .get("adjustment.invert")
            .expect("registered")
            .clone();
        assert!(inv.pointwise());
        inv.validate_params(&json!({})).unwrap();
        // Levels rejects a §17.2 gap violation through the wire mapping.
        let lv = a.ops().get("adjustment.levels").expect("registered");
        let err = lv
            .validate_params(&json!({"r": {"inBlack": 0.5, "inWhite": 0.5}}))
            .unwrap_err();
        assert_eq!(err.code, appname_core::ErrorCode::OutOfRange);
        // apply_cpu: invert is display-referred (§17.2) — linear 0.25 →
        // decode(1 − encode(0.25)) at α 1.
        let rect = IRect::new(0, 0, 2, 1);
        let input = [[0.25, 0.25, 0.25, 1.0], [0.0, 0.0, 0.0, 0.0]];
        let mut out = [[0.0f32; 4]; 2];
        inv.apply_cpu(&json!({}), rect, &input, &mut out).unwrap();
        let expect =
            appname_core::io::srgb::srgb_decode(1.0 - appname_core::io::srgb::srgb_encode(0.25));
        assert!(
            (out[0][0] - expect).abs() < 2e-3,
            "inverted: {} vs {expect}",
            out[0][0]
        );
        assert_eq!(out[1], [0.0, 0.0, 0.0, 0.0], "empty pixel stays empty");
        // Size mismatch is a typed error, never a panic.
        let mut small = [[0.0f32; 4]; 1];
        assert!(inv.apply_cpu(&json!({}), rect, &input, &mut small).is_err());
    }
}
