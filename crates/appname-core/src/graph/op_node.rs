// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.6 (`OpNode` + field semantics), §12.1 (parametric ops —
//! parameters, never baked pixels; the §12.1 evaluation order), §12.2
//! (smart filters: per-op masks), DD ⑨. CP-0.7 wave 1.
//!
//! An `OpNode` is one parametric adjustment/filter application: the
//! registered op id, its schema-validated params, the per-op mask
//! (§12.2), and the per-op fade controls (blend + opacity). Adjustment
//! LAYERS reference exactly one (`LayerKind::Adjustment(OpId)`, the n = 1
//! chain of §12.1); smart objects carry an ordered `smart_filters`
//! chain. Kernels live in the assembly's [`crate::engine::op_impl`] seam
//! (O-TOOLS computes; core validates, stores, and installs under the
//! recorder — the brush-handoff split).

use serde_json::Value;

use crate::graph::id::{MaskId, OpId};
use crate::graph::node::BlendMode;

/// §2.6 `OpNode` (wire + arena form). `op` is the registered op-registry
/// id (`adjustment.curves`, `filter.gaussian_blur` — §12.1's ONE
/// registry mirroring the command registry; unquoted here so the i18n
/// emission sweep doesn't read a doc example as a MessageId).
#[derive(Clone, Debug)]
pub struct OpNode {
    pub(crate) id: OpId,
    pub(crate) op: String,
    /// Validated against the op's registered JSON Schema before commit
    /// (§16.1); unknown fields preserved (§2.8).
    pub(crate) params: Value,
    /// The smart-filter mask (§12.2); resolves in the mask arena and is
    /// owned by exactly this op (§2.7 rule 6 extended).
    pub(crate) mask: Option<MaskId>,
    pub(crate) blend: BlendMode,
    pub(crate) opacity: f32,
    pub(crate) enabled: bool,
}

impl OpNode {
    /// Build a fresh op node with §2.6 defaults (no mask, Normal blend,
    /// opacity 1, enabled). Public for the assembly-side kinds that feed
    /// the recorded structural doors (`adjustment.add` in-crate,
    /// `filter.smart_add`'s `KindContext::insert_smart_filter` from
    /// O-TOOLS) — construction is inert (D-48: a detached node mutates
    /// nothing; document installation happens ONLY through the recorded
    /// `KindContext` doors).
    pub fn new(id: OpId, op: impl Into<String>, params: Value) -> OpNode {
        OpNode {
            id,
            op: op.into(),
            params,
            mask: None,
            blend: BlendMode::Normal,
            opacity: 1.0,
            enabled: true,
        }
    }

    pub fn id(&self) -> OpId {
        self.id
    }

    pub fn op(&self) -> &str {
        &self.op
    }

    pub fn params(&self) -> &Value {
        &self.params
    }

    pub fn mask(&self) -> Option<MaskId> {
        self.mask
    }

    pub fn blend(&self) -> BlendMode {
        self.blend
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The §12.1 fade/param value block (op.set_params' full-state delta
    /// unit; `op` and `id` are immutable, `mask` moves via op.set_mask).
    pub fn value_block(&self) -> OpValueBlock {
        OpValueBlock {
            params: self.params.clone(),
            blend: self.blend,
            opacity: self.opacity,
            enabled: self.enabled,
        }
    }

    pub(crate) fn apply_value_block(&mut self, b: &OpValueBlock) {
        self.params = b.params.clone();
        self.blend = b.blend;
        self.opacity = b.opacity;
        self.enabled = b.enabled;
    }

    pub(crate) fn set_mask(&mut self, mask: Option<MaskId>) {
        self.mask = mask;
    }
}

/// The mutable §2.6 op fields as one value block (SetOpValue delta unit).
#[derive(Clone, Debug, PartialEq)]
pub struct OpValueBlock {
    pub params: Value,
    pub blend: BlendMode,
    pub opacity: f32,
    pub enabled: bool,
}

impl crate::graph::arena::HasId for OpNode {
    fn uuid(&self) -> uuid::Uuid {
        self.id.as_uuid()
    }
}

/// Summary Debug discipline (§1.6) is satisfied: params are JSON values,
/// never pixel dumps.
impl core::fmt::Display for OpNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "OpNode({}, {}, enabled={})",
            self.id, self.op, self.enabled
        )
    }
}
