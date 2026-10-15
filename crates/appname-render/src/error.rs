// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.5 (27-mode catalog — implemented in full since CP-0.5
//! slice A), §4.7 (isolation decision table — implemented at slice B),
//! §4.4 (clipping groups), §11.2 (masks), §18 (transforms), §5–§6 (color
//! pipeline); plan §2.2/§2.3: every not-yet-implemented construct is a
//! typed/documented refusal, never silent wrong math.
//!
//! Every state the CP-0.3 CPU compositor cannot render CORRECTLY is a typed
//! refusal naming the checkpoint where the capability lands. The compositor
//! never falls back to approximate math: a caller either gets the exact
//! ES §4.4 composite or an error it can surface.

use appname_core::{BlendMode, ColorModelKind, LayerId};

/// Typed refusal set of the reference compositor. `#[non_exhaustive]`:
/// CP-0.5 rows remove variants as they land (slice A removed the 19-mode
/// refusal in practice; slice B removed `GroupIsolationNotImplemented` —
/// §4.7 isolation is implemented); clipping, masks, transforms and
/// color-managed non-RGBA composition are still typed refusals below.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompositeError {
    /// RETIRED IN PRACTICE (CP-0.5 slice A): the full 27-mode catalog is
    /// implemented, so no path constructs this variant anymore. It stays
    /// compilable because the CP-0.3 conformance suite (O-QA's fence)
    /// matches on it; removal rides O-QA's wave-2 corpus extension
    /// (coordination note in the O-RENDER lane log).
    BlendModeNotImplemented {
        mode: BlendMode,
        /// The layer carrying the mode, when the refusal comes from a
        /// document walk (`None` from the raw tile API).
        layer: Option<LayerId>,
    },
    /// `clip_to_below` needs the ES §4.4 clipping-group intermediate
    /// (CP-0.5).
    ClippingNotImplemented { layer: LayerId },
    /// A mask construct this slice does not yet serve, refused typed
    /// (CP-0.5 mask row / Sub-slice A serves unfeathered raster masks on
    /// leaves and isolated-group intermediates; this variant covers the
    /// remaining deferrals — the subtree-composite door's masked GROUP
    /// target, whose isolated content is a recorded thumbnail deferral).
    MasksNotImplemented { layer: LayerId },
    /// A LIVE (non-destructive) non-identity transform PROP on a layer node
    /// needs the compositor's resample leg (ES §18) — not yet built. NOTE
    /// the DESTRUCTIVE bake IS served: `layer.transform` / `transform.free`
    /// resample layer content in place (§18.2/§18.3, lane/transform-183),
    /// leaving the transform prop identity, so a baked scale/rotate never
    /// reaches this refusal — only a live transform prop does.
    TransformNotImplemented { layer: LayerId },
    /// Non-RGBA document modes need the §5/§6 conversion pipeline (CP-0.5)
    /// — the same refusal core's CP-0.1 export makes.
    UnsupportedDocumentMode { mode: ColorModelKind },
    /// A source the v1 compositor has no working conversion for (non-RGBA
    /// layer buffer, future layer kinds).
    UnsupportedSource { layer: LayerId, detail: String },
    /// A parametric construct (§11/§12.1 — smart-filter chains, adjustment
    /// layers) the reopened-CP-0.7 §12.1 slice does NOT yet define, refused
    /// typed per the QA-F8 discipline (refuse-never-omit) — the same
    /// refusal matrix core's export door established: enabled ops whose
    /// kind is unregistered in the serving assembly, non-Normal per-op
    /// fade blends, feathered op masks, non-identity smart placement,
    /// non-default compositing props on adjustment layers, adjustments
    /// inside isolating groups (§4.7 rows named in `detail`).
    ParametricNotImplemented { layer: LayerId, detail: String },
    /// A §12.1 kernel/evaluator contract failure while serving parametric
    /// content (not a refusal: the construct is in scope, evaluation
    /// itself errored — surfaced typed, never swallowed).
    OpEvaluationFailed { layer: LayerId, detail: String },
}

impl core::fmt::Display for CompositeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CompositeError::BlendModeNotImplemented { mode, layer } => {
                write!(f, "blend mode {mode:?} is outside blend set v1 (CP-0.5 lands the full 27, ES §4.5)")?;
                if let Some(l) = layer {
                    write!(f, " [layer {l}]")?;
                }
                Ok(())
            }
            CompositeError::ClippingNotImplemented { layer } => write!(
                f,
                "layer {layer} sets clip_to_below; clipping groups (ES §4.4) land at CP-0.5"
            ),
            CompositeError::MasksNotImplemented { layer } => write!(
                f,
                "layer {layer} carries a mask on a GROUP; §11.2 Sub-slice A serves \
                 unfeathered raster masks on LEAVES, but a masked group is §4.7 row-4 \
                 isolation whose compositing has no CPU reference path yet (the render \
                 row) — refused typed on both paths, never a split"
            ),
            CompositeError::TransformNotImplemented { layer } => write!(
                f,
                "layer {layer} carries a non-identity transform prop; live (non-destructive) \
                 transform compositing (ES §18) is not yet implemented — bake it with \
                 layer.transform / transform.free, which resample content in place (§18.2/§18.3)"
            ),
            CompositeError::UnsupportedDocumentMode { mode } => write!(
                f,
                "document mode {mode:?} needs the §5/§6 conversion pipeline (CP-0.5)"
            ),
            CompositeError::UnsupportedSource { layer, detail } => {
                write!(f, "layer {layer}: {detail}")
            }
            CompositeError::ParametricNotImplemented { layer, detail } => {
                write!(f, "layer {layer}: {detail}")
            }
            CompositeError::OpEvaluationFailed { layer, detail } => {
                write!(f, "layer {layer}: §12.1 evaluation failed: {detail}")
            }
        }
    }
}

impl std::error::Error for CompositeError {}
