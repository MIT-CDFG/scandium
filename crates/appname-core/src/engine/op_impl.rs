// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: DD ⑯ (`trait Filter { params(); roi(outRect)->inRect; gpu; cpu }`
//! — "a new filter is addable with no edits to core"), ES §12.1 (the ONE
//! op registry, `OpInfo` shape), §16.1 (the `Filter` trait the compute
//! engines implement), §12.3 (PreviewPolicy), DD ⑨.
//!
//! **THE OpNode↔Filter SEAM** (brush-handoff pattern, wave-1 contract):
//!
//! - O-TOOLS implements [`OpImpl`] per registered adjustment/filter — the
//!   COMPUTE half: params schema, ROI math, and the CPU reference kernel
//!   over storage-normalized f32 planes (GPU pairs land with O-RENDER's
//!   §34.3 parity harness).
//! - Core owns the STORE-AND-RECORD half: `OpNode`s in the document
//!   arena, stage-3 nested param validation through
//!   [`OpAssembly::validate_params`], per-op masks in the mask arena, and
//!   — at bake/commit sites (`layer.rasterize`, destructive
//!   `filter.apply`) — kernel invocation with results installed through
//!   the recorder tile doors, exactly like brush patches.
//! - Registration is CONSTRUCTION-ONLY on the [`OpAssembly`] carried by
//!   the engine assembly (arbitration-1 discipline: no runtime door;
//!   replay behavior is a pure function of assembly + journal).
//! - Live parametric COMPOSITING (an enabled adjustment recoloring its
//!   backdrop at render) is the compositor's consumption of the same
//!   registry — O-RENDER walks `Document::ops` and calls these kernels
//!   per §12.1's evaluation order; core never composites.
//! - DD ⑨ proxy protocol: `preview()` declares the §12.3 policy; the
//!   proxy loop itself is shell/render-side. Core's share of the identity
//!   guarantee is structural — committed params are the envelope's params
//!   verbatim (D-48), no re-quantization anywhere.

use serde_json::Value;

use crate::engine::error::EngineError;
use crate::geom::IRect;

/// §12.3 preview policy (DD ⑨), declared per op for the shell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreviewPolicy {
    LiveFull,
    LiveProxy,
    RegionOnly,
    NoPreview,
}

/// One registered parametric op (the §12.1 `OpInfo` reshaped as the DD-⑯
/// trait; `kernel_wgsl` joins with the GPU lane).
pub trait OpImpl: Send + Sync {
    /// The registry id (`adjustment.curves`, `filter.gaussian_blur`;
    /// unquoted so the i18n emission sweep doesn't read doc examples as
    /// MessageIds). Plugin ops MUST prefix `x.` + reverse-DNS (§12.1).
    fn op_id(&self) -> &'static str;

    /// Stage-3 nested validation of `OpNode::params` (§16.1; RFC 6901
    /// pointers in error fields).
    fn validate_params(&self, params: &Value) -> Result<(), EngineError>;

    /// The op's registered defaults (`adjustment.add` with `params: {}`).
    fn default_params(&self) -> Value;

    /// Input rect needed per output rect (§12.1 `roi_out_to_in`; identity
    /// for pointwise ops — blur inflates by radius).
    fn roi_out_to_in(&self, params: &Value, out: IRect) -> IRect;

    /// Pointwise ⇒ ROI is identity and the op may fuse (§4.6).
    fn pointwise(&self) -> bool;

    /// §12.3/DD ⑨ preview policy.
    fn preview(&self) -> PreviewPolicy;

    /// CPU reference kernel (§16.1 `cpu`): map `input` (storage-normalized
    /// premultiplied f32 RGBA, row-major over `rect`, already expanded per
    /// [`OpImpl::roi_out_to_in`]) to `output` over the same rect. MUST be
    /// deterministic (golden-tested, §34); bit-identical across calls.
    fn apply_cpu(
        &self,
        params: &Value,
        rect: IRect,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
    ) -> Result<(), EngineError>;
}

/// Construction-time op registration (mirrors `EngineAssembly` for kinds;
/// §12.1: duplicate ids panic, registration precedes the first document).
#[derive(Clone, Default)]
pub struct OpAssembly {
    ops: Vec<std::sync::Arc<dyn OpImpl>>,
}

impl OpAssembly {
    pub fn new() -> OpAssembly {
        OpAssembly::default()
    }

    pub fn with_op(mut self, op: std::sync::Arc<dyn OpImpl>) -> OpAssembly {
        assert!(
            !self.ops.iter().any(|o| o.op_id() == op.op_id()),
            "OpAssembly: op `{}` registered twice (§12.1)",
            op.op_id()
        );
        self.ops.push(op);
        self
    }

    pub fn get(&self, id: &str) -> Option<&std::sync::Arc<dyn OpImpl>> {
        self.ops.iter().find(|o| o.op_id() == id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.ops.iter().map(|o| o.op_id())
    }
}
