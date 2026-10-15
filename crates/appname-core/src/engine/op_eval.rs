// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §12.1 (evaluation order within a node — the NORMATIVE chain
//! formula), §12.2 (smart-filter chains evaluate over the shared source),
//! §12.4 (disabled == never-applied, bit-identical), §11.2 (per-op mask
//! coverage steps), §17.1 (an adjustment is the n = 1 chain over its
//! backdrop); DD ⑨⑯. CP-0.7 reopened §12.1 slice.
//!
//! **The op-EVALUATION door.** Kernels reach core exactly the way
//! `KindImpl`s do: the ASSEMBLY carries them ([`OpAssembly`], registered
//! construction-only by the op bridges in `appname-adjust` /
//! `appname-filters` — core never depends on the tools crates). This
//! module consumes that registry to compute EFFECTIVE pixels: given a
//! materialized `in₀` plane and an op chain, it runs the §12.1 formula
//! verbatim —
//!
//! ```text
//! for each opᵢ in chain order, skipping enabled == false:
//!     fᵢ  = kernel(opᵢ.params, inᵢ₋₁ over ROI expanded by roi_out_to_in)
//!     mᵢ  = opᵢ.mask coverage per §11.2 (no mask ⇒ 1)
//!     inᵢ = lerp(inᵢ₋₁, blend(inᵢ₋₁, fᵢ, opᵢ.blend), opᵢ.opacity · mᵢ)
//! ```
//!
//! with the nested-ROI discipline: rects are composed BACKWARD through
//! the enabled ops (`Rᵢ₋₁ = roiᵢ(Rᵢ)`), each step evaluates over its
//! exact input rect and CROPS to the next, so every retained pixel is
//! exact (kernel taps never read fabricated edges inside the kept
//! region). Everything here is a pure function of (params, inputs) —
//! deterministic, no clock, no RNG beyond param-seeded kernels (D-48).
//!
//! Typed refusals (refuse-never-omit, the QA-F8 discipline): an ENABLED
//! op whose kind is not in this assembly's registry (crafted packages,
//! bare-core assemblies); a non-Normal per-op fade blend (the §4.5 blend
//! kernels are the render row); a feathered op mask (the §11.2 feather
//! Gaussian is the render row). A DISABLED op never refuses — it is
//! skipped before any of these checks, which is what makes §12.4
//! bit-identity structural.

use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::op_impl::{OpAssembly, OpImpl};
use crate::geom::{IRect, IVec2};
use crate::graph::document::Document;
use crate::graph::id::OpId;
use crate::graph::node::BlendMode;
use crate::graph::op_node::OpNode;

/// A rectangular plane of storage-normalized premultiplied f32 RGBA
/// pixels, row-major over `rect` — the §12.1 evaluation currency (the
/// same plane shape [`OpImpl::apply_cpu`] speaks).
#[derive(Clone, Debug)]
pub struct OpPlane {
    pub rect: IRect,
    pub px: Vec<[f32; 4]>,
}

impl OpPlane {
    /// A fully transparent plane over `rect`.
    pub fn transparent(rect: IRect) -> OpPlane {
        let n = (rect.width().max(0) as usize) * (rect.height().max(0) as usize);
        OpPlane {
            rect,
            px: vec![[0.0; 4]; n],
        }
    }

    /// Read one pixel; outside `rect` reads transparent (absent tiles
    /// read 0, §11.2).
    pub fn get(&self, p: IVec2) -> [f32; 4] {
        if p.x < self.rect.min.x
            || p.x >= self.rect.max.x
            || p.y < self.rect.min.y
            || p.y >= self.rect.max.y
        {
            return [0.0; 4];
        }
        let w = self.rect.width() as usize;
        let i = (p.y - self.rect.min.y) as usize * w + (p.x - self.rect.min.x) as usize;
        self.px[i]
    }

    /// The sub-plane over `to` (values copied; `to` need not be contained
    /// — uncovered pixels read transparent).
    pub fn crop(&self, to: IRect) -> OpPlane {
        if to == self.rect {
            return self.clone();
        }
        let mut out = OpPlane::transparent(to);
        let mut i = 0;
        for y in to.min.y..to.max.y {
            for x in to.min.x..to.max.x {
                out.px[i] = self.get(IVec2::new(x, y));
                i += 1;
            }
        }
        out
    }
}

/// The §12.1 chain evaluator: the assembly's op registry + the document
/// (for per-op mask resolution). Construct per evaluation site.
pub struct ChainEvaluator<'a> {
    ops: &'a OpAssembly,
    doc: &'a Document,
}

impl<'a> ChainEvaluator<'a> {
    pub fn new(ops: &'a OpAssembly, doc: &'a Document) -> ChainEvaluator<'a> {
        ChainEvaluator { ops, doc }
    }

    /// The ENABLED ops of `chain`, in chain order, each resolved against
    /// the registry. A disabled op is skipped before any check (§12.4);
    /// an enabled op with an unregistered kind refuses typed.
    #[allow(clippy::type_complexity)]
    fn enabled_ops(
        &self,
        chain: &[OpId],
    ) -> Result<Vec<(&'a OpNode, &'a std::sync::Arc<dyn OpImpl>)>, EngineError> {
        let mut out = Vec::new();
        for oid in chain {
            let node = self.doc.op(*oid).ok_or_else(|| {
                EngineError::bad_target(format!("chain op {oid} does not resolve"))
            })?;
            if !node.enabled() {
                continue; // §12.4: identity, structurally
            }
            let Some(imp) = self.ops.get(node.op()) else {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!(
                        "enabled op \"{}\" is not registered in this assembly's §12.1 op \
                         registry — its kernel cannot evaluate (the compute half arrives \
                         through the assembly wraps); refused typed, never omitted",
                        node.op()
                    ),
                ));
            };
            if node.blend() != BlendMode::Normal {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!(
                        "op \"{}\" carries a non-Normal fade blend; the §4.5 blend kernels \
                         at the op fade are the render row — refused typed, never omitted",
                        node.op()
                    ),
                ));
            }
            if let Some(mid) = node.mask() {
                if let Some(mask) = self.doc.mask(mid) {
                    if mask.enabled() && mask.feather() > 0.0 {
                        return Err(EngineError::new(
                            ErrorCode::NotImplemented,
                            format!(
                                "op \"{}\" carries a feathered mask; the §11.2 feather \
                                 Gaussian is the render row — refused typed, never omitted",
                                node.op()
                            ),
                        ));
                    }
                }
            }
            out.push((node, imp));
        }
        Ok(out)
    }

    /// §12.1 input rect: the source rect needed to produce `out` through
    /// the enabled tail of `chain` (rects composed backward, disabled ops
    /// contributing identity).
    pub fn chain_input_rect(&self, chain: &[OpId], out: IRect) -> Result<IRect, EngineError> {
        let ops = self.enabled_ops(chain)?;
        let mut rect = out;
        for (node, imp) in ops.iter().rev() {
            rect = imp.roi_out_to_in(node.params(), rect);
        }
        Ok(rect)
    }

    /// Evaluate the §12.1 chain over a materialized `in₀`. `plane.rect`
    /// MUST equal [`Self::chain_input_rect`]`(chain, out)`; the result
    /// spans exactly `out`. Pure in (params, inputs) — deterministic.
    pub fn eval_chain(
        &self,
        chain: &[OpId],
        plane: OpPlane,
        out: IRect,
    ) -> Result<OpPlane, EngineError> {
        let ops = self.enabled_ops(chain)?;
        // Nested rects: r[0] = input rect, r[n] = out.
        let mut rects = vec![out; ops.len() + 1];
        for i in (0..ops.len()).rev() {
            let (node, imp) = &ops[i];
            rects[i] = imp.roi_out_to_in(node.params(), rects[i + 1]);
        }
        if plane.rect != rects[0] {
            return Err(EngineError::bad_target(format!(
                "eval_chain plane rect {:?} does not match the chain input rect {:?} \
                 (evaluator contract)",
                plane.rect, rects[0]
            )));
        }
        let mut cur = plane;
        for (i, (node, imp)) in ops.iter().enumerate() {
            // fᵢ over the current (already expanded) rect.
            let mut f = vec![[0.0f32; 4]; cur.px.len()];
            imp.apply_cpu(node.params(), cur.rect, &cur.px, &mut f)?;
            // inᵢ = lerp(inᵢ₋₁, fᵢ, opacity · mᵢ), §4.5 Normal fade.
            let opacity = node.opacity().clamp(0.0, 1.0);
            let mask = node.mask().and_then(|m| self.doc.mask(m));
            let mut merged = OpPlane {
                rect: cur.rect,
                px: Vec::with_capacity(cur.px.len()),
            };
            let mut idx = 0;
            for y in cur.rect.min.y..cur.rect.max.y {
                for x in cur.rect.min.x..cur.rect.max.x {
                    let t = opacity * mask_coverage(mask, IVec2::new(x, y));
                    let a = cur.px[idx];
                    let b = f[idx];
                    let px = if t >= 1.0 {
                        b
                    } else if t <= 0.0 {
                        a
                    } else {
                        [
                            a[0] + (b[0] - a[0]) * t,
                            a[1] + (b[1] - a[1]) * t,
                            a[2] + (b[2] - a[2]) * t,
                            a[3] + (b[3] - a[3]) * t,
                        ]
                    };
                    merged.px.push(px);
                    idx += 1;
                }
            }
            // Drop the approximate border band: keep the exact interior.
            cur = merged.crop(rects[i + 1]);
        }
        if cur.rect != out {
            cur = cur.crop(out);
        }
        Ok(cur)
    }
}

/// FI-13 [NORMATIVE] — fold an Adjustment node's OWN `opacity` and
/// layer-mask coverage into §12.1's lerp weight (ENGINEERING-SPEC
/// :5956-5962, frozen at `v5.6-freeze`).
///
/// ```text
/// w = node.opacity · Π coverage(node.masks) · opᵢ.opacity · mᵢ
/// ```
///
/// [`ChainEvaluator::eval_chain`] has already applied `opᵢ.opacity · mᵢ`.
/// This applies the remaining two NODE factors as a second lerp against
/// the SAME backdrop, which composes to exactly `w` rather than
/// approximating it: with both sites using the `a + (b - a)·t` form,
///
/// ```text
/// lerp(a, lerp(a, b, x), y) = a + ((a + (b-a)x) - a)·y = a + (b-a)·x·y
/// ```
///
/// The deviation is at FLOAT-EPSILON SCALE — one unseeded 100k-draw sweep
/// over the unit square measured 4.441e-16, BUT THAT SAMPLE IS NOT A PIN:
/// the draws were unseeded, so a re-run gives a different value and a
/// reader cannot tell a changed implementation from a different probe.
/// WHAT CARRIES THE MEANING IS THE PROPERTY, NOT THE SAMPLE: the identity
/// holds to float precision. And since 2026-08-28 it matters less still —
/// ruling 2 established that the Normal collapse is STRUCTURAL (a Normal
/// blend returns its source), which needs no floating-point argument at
/// all. `eval_chain`
/// therefore needs no modification, and an Adjustment layer is always the
/// n = 1 case (§12.1), so no chain-composition question arises.
///
/// THIS IS THE SINGLE IMPLEMENTATION FOR BOTH DOORS. The render
/// compositor and the reference export door both call it; neither
/// re-derives it (one-math-library, the same discipline
/// [`layer_effective_coverage`] carries).
///
/// §11.2's route — multiplying coverage into a source's composited alpha
/// before a §4.4 source-over — MUST NOT be used here (:5960): it raises
/// alpha above `αb` at every non-zero coverage, which an in-place
/// adjustment must never do. This route cannot, because a lerp between
/// the backdrop and a result computed FROM that backdrop returns exactly
/// the backdrop at `w = 0` and introduces no alpha the endpoints do not
/// already carry.
///
/// No-op when the node weight is identically 1 (no enabled layer masks
/// and `opacity == 1.0`), so the pre-FI-13 served path stays bit-identical.
pub fn fold_adjustment_node_weight(
    backdrop: &OpPlane,
    served: &mut OpPlane,
    node_opacity: f32,
    masks: &[&crate::graph::mask_node::MaskNode],
) {
    if masks.is_empty() && node_opacity == 1.0 {
        return;
    }
    // A HARD assert, not debug_assert: this is a normative compositing rule
    // and the check is once per adjustment, not per pixel. A rect mismatch
    // would not fail loudly on its own — the two planes are indexed by a
    // shared counter, so a mismatch silently pairs the wrong pixels wherever
    // the lengths happen to allow it.
    assert_eq!(
        backdrop.rect, served.rect,
        "FI-13 fold: backdrop and served must span the same rect"
    );
    let node_opacity = node_opacity.clamp(0.0, 1.0);
    // Sub-slice B changed `layer_effective_coverage` to take a RESOLVED mask
    // stack (it may carry a canonical feathered plane). Resolve ONCE per
    // adjustment — never inside the loop below, which is per pixel.
    let resolved = crate::graph::mask_eval::ResolvedMasks::resolve(masks);
    // RULING 5 (operator, 2026-08-28): A NaN WEIGHT EVALUATES AS ZERO, LOUDLY.
    //
    // BEFORE THIS, THERE WAS NO NaN HANDLING HERE AT ALL, and the default was the
    // worst available: `t >= 1.0` is FALSE for NaN and `t <= 0.0` is ALSO FALSE, so a
    // NaN weight fell through to the general lerp and PROPAGATED INTO THE PIXEL
    // SILENTLY. `f32::clamp` does not help -- it returns NaN for a NaN input.
    //
    // §12.1 defines w as a product of scalars and says NOTHING about their domain; NaN
    // is representable in every factor (node.opacity, coverage, opᵢ.opacity, mᵢ) and the
    // letter excludes none of them. The ruling supplies the answer the spec lacks.
    //
    // ZERO means THE BACKDROP, UNMODIFIED -- the same branch as t <= 0.0, so a NaN can
    // never be worse than "the adjustment did nothing". LOUDLY means a diagnostic, and
    // it fires ONCE PER FOLD, not once per pixel: a per-pixel warning on a 1200x800
    // canvas is a million lines, which is indistinguishable from silence to a reader.
    let mut nan_reported = false;
    let mut idx = 0;
    for y in served.rect.min.y..served.rect.max.y {
        for x in served.rect.min.x..served.rect.max.x {
            let mut t = node_opacity
                * crate::graph::mask_eval::layer_effective_coverage(&resolved, IVec2::new(x, y));
            if t.is_nan() {
                if !nan_reported {
                    nan_reported = true;
                    eprintln!(
                        "appname: FI-13 weight is NaN at ({x},{y}) -- node_opacity={node_opacity}, \
                         masks={} -- EVALUATING AS ZERO (backdrop unmodified) per ruling 5. \
                         Reported once per fold; further NaN pixels in this adjustment are \
                         handled identically and NOT reported.",
                        masks.len()
                    );
                }
                t = 0.0;
            }
            let a = backdrop.px[idx];
            let b = served.px[idx];
            // Deliberately the same shape as the per-op fade above: the two
            // lerps must agree channel-for-channel for the identity to hold
            // in floating point as well as in algebra.
            served.px[idx] = if t >= 1.0 {
                b
            } else if t <= 0.0 {
                a
            } else {
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                    a[3] + (b[3] - a[3]) * t,
                ]
            };
            idx += 1;
        }
    }
}

/// §11.2 per-op mask coverage at `p`, document space (op masks are
/// unlinked v1; the serving door refuses non-identity layer transforms,
/// so layer-local == document space). Delegates to the ONE shared §11.2
/// routine ([`crate::graph::mask_eval::resolve_effective_coverage`] —
/// sample → density → invert, the O-INT-ruled order) so the op leg and
/// the CP-0.5 layer-mask row can never diverge. A disabled mask
/// contributes 1 (the §11.2 no-op); feather is refused typed upstream.
fn mask_coverage(mask: Option<&crate::graph::mask_node::MaskNode>, p: IVec2) -> f32 {
    let Some(mask) = mask else { return 1.0 };
    if !mask.enabled() {
        return 1.0;
    }
    crate::graph::mask_eval::resolve_effective_coverage(mask, p)
}

/// Probe `OpImpl`s for in-crate tests (the compute half without the
/// tools crates — the seam's construction-only registration exercised
/// with deterministic toy kernels).
#[cfg(test)]
pub(crate) mod probes {
    use super::*;
    use crate::engine::op_impl::PreviewPolicy;
    use serde_json::{json, Value};

    /// Pointwise probe: scale RGB by `params.factor` (alpha kept) — a
    /// stand-in for the §17 adjustments.
    pub(crate) struct ScaleOp;
    impl OpImpl for ScaleOp {
        fn op_id(&self) -> &'static str {
            "x.test.scale"
        }
        fn validate_params(&self, _p: &Value) -> Result<(), EngineError> {
            Ok(())
        }
        fn default_params(&self) -> Value {
            json!({})
        }
        fn roi_out_to_in(&self, _p: &Value, out: IRect) -> IRect {
            out
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
            _rect: IRect,
            input: &[[f32; 4]],
            output: &mut [[f32; 4]],
        ) -> Result<(), EngineError> {
            let f = params.get("factor").and_then(Value::as_f64).unwrap_or(0.5) as f32;
            for (o, i) in output.iter_mut().zip(input) {
                *o = [i[0] * f, i[1] * f, i[2] * f, i[3]];
            }
            Ok(())
        }
    }

    /// ROI probe: 3×1 horizontal box average (radius 1) — a stand-in for
    /// the convolution filters; NOT commuting with ScaleOp+offsets.
    pub(crate) struct SpreadOp;
    impl OpImpl for SpreadOp {
        fn op_id(&self) -> &'static str {
            "x.test.spread"
        }
        fn validate_params(&self, _p: &Value) -> Result<(), EngineError> {
            Ok(())
        }
        fn default_params(&self) -> Value {
            json!({})
        }
        fn roi_out_to_in(&self, _p: &Value, out: IRect) -> IRect {
            out.inflate(1, 0)
        }
        fn pointwise(&self) -> bool {
            false
        }
        fn preview(&self) -> PreviewPolicy {
            PreviewPolicy::LiveProxy
        }
        fn apply_cpu(
            &self,
            _params: &Value,
            rect: IRect,
            input: &[[f32; 4]],
            output: &mut [[f32; 4]],
        ) -> Result<(), EngineError> {
            let w = rect.width() as i64;
            for (i, o) in output.iter_mut().enumerate() {
                let x = (i as i64) % w;
                let mut acc = [0.0f32; 4];
                for dx in -1..=1i64 {
                    let sx = x + dx;
                    if sx < 0 || sx >= w {
                        continue; // edge taps transparent (§16.3 policy)
                    }
                    let s = input[(i as i64 + dx) as usize];
                    for c in 0..4 {
                        acc[c] += s[c];
                    }
                }
                *o = [acc[0] / 3.0, acc[1] / 3.0, acc[2] / 3.0, acc[3] / 3.0];
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {

    /// RULING 5 (operator, 2026-08-28): A NaN WEIGHT EVALUATES AS ZERO, LOUDLY.
    ///
    /// R5-P1, registered BEFORE the code (INTEGRATION-log 248.2): a NaN in any weight
    /// factor yields w = 0 — THE BACKDROP, UNMODIFIED — and emits a diagnostic.
    /// FALSIFIER: a NaN that CHANGES A PIXEL, or one silently clamped with NO
    /// diagnostic. **ZERO-QUIETLY IS NOT THE RULING**, and it is exactly what a reader
    /// would implement from "evaluates as zero" alone.
    ///
    /// THE PRIOR BEHAVIOUR IS WHAT MAKES THIS WORTH A TEST: `t >= 1.0` is FALSE for NaN
    /// and `t <= 0.0` is ALSO FALSE, so a NaN fell through to the general lerp and
    /// `a + (b - a) * NaN` POISONED EVERY CHANNEL. The failure was not loud and wrong,
    /// it was quiet and wrong.
    ///
    /// NaN CANNOT ARRIVE THROUGH THE WIRE — JSON has no NaN literal and the opacity
    /// schema is 0..=1 — so this drives the crate-internal door directly. THAT IS THE
    /// ONLY PATH IT CAN COME FROM, which is also why no integration test would ever
    /// have caught it.
    #[test]
    fn ruling5_nan_weight_evaluates_as_zero_leaving_the_backdrop() {
        let rect = IRect::new(0, 0, 2, 2);
        let backdrop = OpPlane {
            rect,
            px: vec![[0.25, 0.5, 0.75, 1.0]; 4],
        };
        let mut served = OpPlane {
            rect,
            px: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        };

        // CONTROL FIRST: a FINITE weight must actually MOVE the pixel, or the
        // assertion below would pass on a fold that does nothing at all.
        let mut control = served.clone();
        fold_adjustment_node_weight(&backdrop, &mut control, 1.0, &[]);
        assert_eq!(
            control.px[0], served.px[0],
            "CONTROL at w=1: the served value stands unchanged"
        );
        let mut half = OpPlane {
            rect,
            px: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        };
        fold_adjustment_node_weight(&backdrop, &mut half, 0.5, &[]);
        assert!(
            (half.px[0][0] - 0.625).abs() < 1e-6,
            "CONTROL at w=0.5: the fold demonstrably moves the pixel (got {}), so a \
             NaN landing on the backdrop below is a RESULT and not an inert fold",
            half.px[0][0]
        );

        // PROBE: NaN node_opacity.
        fold_adjustment_node_weight(&backdrop, &mut served, f32::NAN, &[]);
        for (i, px) in served.px.iter().enumerate() {
            assert_eq!(
                *px, backdrop.px[i],
                "ruling 5: a NaN weight yields w=0 -- THE BACKDROP, UNMODIFIED. Before \
                 this, NaN fell through both guards into the lerp and poisoned every \
                 channel: pixel {i} is {px:?}"
            );
            assert!(
                px.iter().all(|c| c.is_finite()),
                "ruling 5: NO NaN REACHES A PIXEL: {px:?}"
            );
        }
    }
    use super::probes::{ScaleOp, SpreadOp};
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn probe_assembly() -> OpAssembly {
        OpAssembly::new()
            .with_op(Arc::new(ScaleOp))
            .with_op(Arc::new(SpreadOp))
    }

    fn engine_with_ops(nodes: Vec<OpNode>) -> crate::engine::Engine {
        let mut e = crate::engine::Engine::new();
        e.apply_bytes(br#"{"schema":"appname.command.v1","id":"0eef0000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":8,"height":8}}"#)
            .unwrap();
        let d = e.doc_mut_for_test();
        for n in nodes {
            d.insert_op(n).unwrap();
        }
        e
    }

    fn opid(tail: u32) -> OpId {
        OpId::from_uuid(uuid::Uuid::from_u128(0x0eef_0000 + tail as u128))
    }

    fn uniform_plane(rect: IRect, px: [f32; 4]) -> OpPlane {
        let mut p = OpPlane::transparent(rect);
        p.px.iter_mut().for_each(|v| *v = px);
        p
    }

    #[test]
    fn disabled_op_is_bit_identical_to_never_applied() {
        let a = probe_assembly();
        let mut n = OpNode::new(opid(1), "x.test.scale", json!({"factor": 0.25}));
        let mut block = n.value_block();
        block.enabled = false;
        n.apply_value_block(&block);
        let e = engine_with_ops(vec![n]);
        let doc = e.document().unwrap();
        let ev = ChainEvaluator::new(&a, doc);
        let rect = IRect::new(0, 0, 4, 4);
        let src = uniform_plane(rect, [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(ev.chain_input_rect(&[opid(1)], rect).unwrap(), rect);
        let out = ev.eval_chain(&[opid(1)], src.clone(), rect).unwrap();
        assert_eq!(out.px, src.px, "§12.4: disabled == never-applied, bitwise");
    }

    #[test]
    fn chain_order_is_honored_and_non_commuting() {
        // scale∘spread vs spread∘scale differ on an impulse edge.
        let a = probe_assembly();
        let scale = OpNode::new(opid(1), "x.test.scale", json!({"factor": 0.5}));
        let spread = OpNode::new(opid(2), "x.test.spread", json!({}));
        let e = engine_with_ops(vec![scale, spread]);
        let doc = e.document().unwrap();
        let ev = ChainEvaluator::new(&a, doc);
        let out_rect = IRect::new(0, 0, 4, 1);
        // Impulse at x = 1 over the widest needed rect.
        let mk = |rect: IRect| {
            let mut p = OpPlane::transparent(rect);
            let w = rect.width() as usize;
            let i = (0 - rect.min.y) as usize * w + (1 - rect.min.x) as usize;
            p.px[i] = [1.0, 1.0, 1.0, 1.0];
            p
        };
        let ab = [opid(1), opid(2)]; // scale then spread
        let ba = [opid(2), opid(1)]; // spread then scale
        let r_ab = ev.chain_input_rect(&ab, out_rect).unwrap();
        let r_ba = ev.chain_input_rect(&ba, out_rect).unwrap();
        assert_eq!(r_ab, out_rect.inflate(1, 0), "spread expands the input");
        assert_eq!(r_ba, out_rect.inflate(1, 0));
        let o_ab = ev.eval_chain(&ab, mk(r_ab), out_rect).unwrap();
        let o_ba = ev.eval_chain(&ba, mk(r_ba), out_rect).unwrap();
        // Same math either way for a LINEAR pair — prove order plumbing
        // with values instead: scale-then-spread at x=0 is (0.5)/3;
        // spread-then-scale is (1/3)·0.5 — equal; so distinguish via the
        // t-fade below instead. Here pin exactness of both paths.
        let third: f32 = 1.0 / 3.0;
        assert!((o_ab.px[0][0] - 0.5 * third).abs() < 1e-6);
        assert!((o_ba.px[0][0] - third * 0.5).abs() < 1e-6);
        // Order DOES matter under a per-op fade: fade the SECOND op to
        // 0.5 in both chains — lerp against different in₁ planes.
        let mut spread_faded = OpNode::new(opid(3), "x.test.spread", json!({}));
        let mut b = spread_faded.value_block();
        b.opacity = 0.5;
        spread_faded.apply_value_block(&b);
        let mut scale_faded = OpNode::new(opid(4), "x.test.scale", json!({"factor": 0.5}));
        let mut b = scale_faded.value_block();
        b.opacity = 0.5;
        scale_faded.apply_value_block(&b);
        let e2 = engine_with_ops(vec![
            OpNode::new(opid(1), "x.test.scale", json!({"factor": 0.5})),
            OpNode::new(opid(2), "x.test.spread", json!({})),
            spread_faded,
            scale_faded,
        ]);
        let doc2 = e2.document().unwrap();
        let ev2 = ChainEvaluator::new(&a, doc2);
        let c1 = [opid(1), opid(3)]; // scale, then half-faded spread
        let c2 = [opid(2), opid(4)]; // spread, then half-faded scale
        let r1 = ev2.chain_input_rect(&c1, out_rect).unwrap();
        let r2 = ev2.chain_input_rect(&c2, out_rect).unwrap();
        let o1 = ev2.eval_chain(&c1, mk(r1), out_rect).unwrap();
        let o2 = ev2.eval_chain(&c2, mk(r2), out_rect).unwrap();
        assert!(
            (o1.px[1][0] - o2.px[1][0]).abs() > 1e-3,
            "chain order distinguishes faded non-commuting ops: {} vs {}",
            o1.px[1][0],
            o2.px[1][0]
        );
    }

    #[test]
    fn per_op_mask_gates_the_fade_with_density_and_invert() {
        let a = probe_assembly();
        let mut e = crate::engine::Engine::new();
        e.apply_bytes(br#"{"schema":"appname.command.v1","id":"0eef0000-0000-4000-8000-000000000002","kind":"doc.new","params":{"width":8,"height":8}}"#)
            .unwrap();
        let (oid, mid) = {
            let d = e.doc_mut_for_test();
            let oid = opid(1);
            d.insert_op(OpNode::new(oid, "x.test.scale", json!({"factor": 0.0})))
                .unwrap();
            // Mask: left half 1.0, right half absent (reads 0).
            let mut cov = crate::mask::Selection::new_mask_buffer();
            for y in 0..2 {
                for x in 0..2 {
                    cov.write_pixel_f32(IVec2::new(x, y), [1.0, 0.0, 0.0, 0.0]);
                }
            }
            cov.recompute_bounds_pixel_exact();
            let mid = crate::graph::id::MaskId::from_uuid(uuid::Uuid::from_u128(7));
            d.attach_op_mask(oid, crate::graph::mask_node::MaskNode::new_raster(mid, cov))
                .unwrap();
            d.op_mut(oid).unwrap().set_mask(Some(mid));
            (oid, mid)
        };
        let rect = IRect::new(0, 0, 4, 1);
        let src = uniform_plane(rect, [0.8, 0.8, 0.8, 1.0]);
        {
            let doc = e.document().unwrap();
            let ev = ChainEvaluator::new(&a, doc);
            let out = ev.eval_chain(&[oid], src.clone(), rect).unwrap();
            assert_eq!(out.px[0], [0.0, 0.0, 0.0, 1.0], "masked-in: op applies");
            assert_eq!(out.px[3], [0.8, 0.8, 0.8, 1.0], "masked-out: identity");
        }
        // Invert flips the gate; density 0 disables the mask (⇒ 1).
        {
            let d = e.doc_mut_for_test();
            let mut props = d.mask(mid).unwrap().props();
            props.inverted = true;
            d.mask_mut(mid).unwrap().set_props(props);
        }
        {
            let doc = e.document().unwrap();
            let ev = ChainEvaluator::new(&a, doc);
            let out = ev.eval_chain(&[oid], src.clone(), rect).unwrap();
            assert_eq!(out.px[0], [0.8, 0.8, 0.8, 1.0], "inverted: left identity");
            assert_eq!(out.px[3], [0.0, 0.0, 0.0, 1.0], "inverted: right applies");
        }
        {
            let d = e.doc_mut_for_test();
            let mut props = d.mask(mid).unwrap().props();
            props.inverted = false;
            props.density = 0.0;
            d.mask_mut(mid).unwrap().set_props(props);
        }
        let doc = e.document().unwrap();
        let ev = ChainEvaluator::new(&a, doc);
        let out = ev.eval_chain(&[oid], src, rect).unwrap();
        assert_eq!(
            out.px[3],
            [0.0, 0.0, 0.0, 1.0],
            "density 0 = mask off (coverage 1 everywhere, §11.2 step 3)"
        );
    }

    #[test]
    fn typed_refusals_enabled_unregistered_blend_feather() {
        let a = probe_assembly();
        let rect = IRect::new(0, 0, 2, 2);

        // Enabled + unregistered kind → typed.
        let e = engine_with_ops(vec![OpNode::new(opid(1), "adjustment.curves", json!({}))]);
        let ev = ChainEvaluator::new(&a, e.document().unwrap());
        let err = ev.chain_input_rect(&[opid(1)], rect).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);

        // DISABLED + unregistered kind → identity, no refusal (§12.4).
        let mut n = OpNode::new(opid(2), "adjustment.curves", json!({}));
        let mut b = n.value_block();
        b.enabled = false;
        n.apply_value_block(&b);
        let e = engine_with_ops(vec![n]);
        let ev = ChainEvaluator::new(&a, e.document().unwrap());
        let src = uniform_plane(rect, [0.3, 0.3, 0.3, 1.0]);
        let out = ev.eval_chain(&[opid(2)], src.clone(), rect).unwrap();
        assert_eq!(out.px, src.px);

        // Non-Normal fade blend → typed.
        let mut n = OpNode::new(opid(3), "x.test.scale", json!({}));
        let mut b = n.value_block();
        b.blend = BlendMode::Multiply;
        n.apply_value_block(&b);
        let e = engine_with_ops(vec![n]);
        let ev = ChainEvaluator::new(&a, e.document().unwrap());
        let err = ev
            .eval_chain(&[opid(3)], uniform_plane(rect, [0.5; 4]), rect)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);

        // Feathered op mask → typed.
        let mut e = crate::engine::Engine::new();
        e.apply_bytes(br#"{"schema":"appname.command.v1","id":"0eef0000-0000-4000-8000-000000000003","kind":"doc.new","params":{"width":8,"height":8}}"#)
            .unwrap();
        let oid = opid(4);
        {
            let d = e.doc_mut_for_test();
            d.insert_op(OpNode::new(oid, "x.test.scale", json!({})))
                .unwrap();
            let cov = crate::mask::Selection::new_mask_buffer();
            let mid = crate::graph::id::MaskId::from_uuid(uuid::Uuid::from_u128(9));
            let mut mask = crate::graph::mask_node::MaskNode::new_raster(mid, cov);
            let mut props = mask.props();
            props.feather = 2.0;
            mask.set_props(props);
            d.attach_op_mask(oid, mask).unwrap();
            d.op_mut(oid).unwrap().set_mask(Some(mid));
        }
        let ev = ChainEvaluator::new(&a, e.document().unwrap());
        let err = ev
            .eval_chain(&[oid], uniform_plane(rect, [0.5; 4]), rect)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    #[test]
    fn roi_expansion_keeps_edge_pixels_exact() {
        // A spread op whose source has content OUTSIDE the out rect: the
        // nested-rect discipline must pull it in (input rect inflated),
        // so the out-rect edge pixel sees the neighbor.
        let a = probe_assembly();
        let e = engine_with_ops(vec![OpNode::new(opid(1), "x.test.spread", json!({}))]);
        let ev = ChainEvaluator::new(&a, e.document().unwrap());
        let out_rect = IRect::new(0, 0, 2, 1);
        let in_rect = ev.chain_input_rect(&[opid(1)], out_rect).unwrap();
        assert_eq!(in_rect, IRect::new(-1, 0, 3, 1));
        // Content only at x = -1 (outside out): impulse.
        let mut src = OpPlane::transparent(in_rect);
        src.px[0] = [0.9, 0.9, 0.9, 0.9];
        let out = ev.eval_chain(&[opid(1)], src, out_rect).unwrap();
        assert!(
            (out.px[0][0] - 0.3).abs() < 1e-6,
            "x=0 sees the x=-1 neighbor through the apron: {}",
            out.px[0][0]
        );
        assert_eq!(out.rect, out_rect);
    }
}
