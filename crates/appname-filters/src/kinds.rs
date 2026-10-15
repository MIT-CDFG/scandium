// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `filter.apply.yaml` / `filter.smart_add.yaml` (params,
//! §40.6 family rules, lock tables, error additions), ES §16.1 (registry
//! route), §16.2 (ROI-driven evaluation; selection masking at the
//! EVALUATOR — `out' = lerp(in, out, sel(p))`; filters stay
//! selection-unaware), §16.5 (Last Filter / gallery are recorder-side),
//! §12.4 (rasterize-first detail on parametric targets), §11.1 (locks:
//! the section names `filter.apply` under PIXELS), §9.6 (client
//! creation UUIDs), §40.1 (stage order); DD ⑯.
//!
//! The two `filter.*` [`KindImpl`]s, exported for the assembly points via
//! [`register`] (arbitration-1 pattern). `filter.apply` is the
//! DESTRUCTIVE bake: registry lookup → stage-3 nested param validation →
//! stage-6 target/locks → ROI-expanded CPU evaluation → §16.2 masked
//! write-back through the shared selection-gated edit loop
//! (`appname_tools::kind_impls::edit_region_at`). Both kinds and the
//! §12.1 OpImpl bridge (`op_bridge`) are constructed from the ONE
//! [`slice1_registry`], so command validation and op registration cannot
//! drift apart.
//!
//! v1 scope (typed refusals name their rows, stroke-kind precedent):
//! * `blend` fade passes other than `normal` answer
//!   `error.command.not_implemented` — the 26 non-normal §4.5 blend
//!   formulas are the compositor corpus (O-RENDER's row); the §10.3 fade
//!   itself (opacity lerp atop the result) IS implemented and composes
//!   with the §16.2 selection mask as `lerp(in, out, sel·opacity)`.
//! * The §10.3 `checkpoint:true` FilterUi arm (whole-canvas ops) gained
//!   its named instance at the CP-1.0 warps row: `filter.liquify`
//!   (§45.4) declares `checkpoint: true` and executes through this same
//!   bake path. Recorded pin: §45.4's class-C "inverse snap" is realized
//!   as COW tile deltas over the touched region — the exact inverse
//!   equivalent of a snap over the write support (the §40.1 recorder
//!   already restores every touched tile byte-exactly; a whole-canvas
//!   snapshot would store a superset of the same bytes). The remaining
//!   corpus filters run class R unchanged.
//! * `filter.smart_add` is LIVE end-to-end (CP-0.7 tail flip,
//!   2026-08-14): the same complete validation (registry, nested params,
//!   §9.6 duplicate, §40.6 target/locks, index clamp), then ONE call
//!   through O-CORE's recorded insert door
//!   (`KindContext::insert_smart_filter`: arena `AddOp` + chain-splice
//!   `AddFilterRef`, one DeltaSet — exact mirror of the remove door);
//!   the receipt carries the created OpId. RENDERING the chain is the
//!   §12.1 render row: the reference export door refuses ENABLED chain
//!   entries typed (QA-F8 discipline), while DISABLED entries contribute
//!   identity and export fine (§12.4 bit-identity) — pinned in the
//!   battery.
//!
//! Recorded pins (lane log):
//! * Bake extent + edge anchor (R1 re-anchor, ES:2617 [NORMATIVE]): the
//!   read substrate is the DOCUMENT data bounds — canvas ∪ the target
//!   layer's §2.5 content bounds (off-canvas content is first-class;
//!   negative tile coords per the package format). The §16.2 output
//!   support is the content bounds DILATED by the mirrored ROI apron
//!   (a kernel reaching r px in pulls content r px out), intersected
//!   with the selection bounds when active; Clamp-policy filters
//!   additionally clamp the WRITE region to the same data bounds
//!   (beyond them output would be pure edge replication — artifact,
//!   not §2.5 content), while Transparent-policy output keeps the full
//!   dilation (real faded content). Reads bind through the declared
//!   §16.3 policy anchored at the data bounds; behavior is identical
//!   to canvas anchoring whenever content ⊆ canvas.
//! * TRANSPARENCY lock "constrains filters that write alpha" (§40.6):
//!   pinned as alpha-preservation — the filtered straight color lands
//!   re-premultiplied over each pixel's ORIGINAL alpha (source-atop
//!   analog, §11.1 brush precedent).
//! * `ctx.seed = 0`: stochastic filters ride their own `params.seed`
//!   (§16.2 convention; contract note). "Last Filter" fresh-seed minting
//!   is §16.5 recorder work (actions wave).

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use appname_core::graph::id::OpId;
use appname_core::{
    EngineAssembly, EngineError, ErrorCode, IRect, IVec2, KindContext, KindImpl, KindOutcome,
    LayerId, LockFlags, OpNode,
};
use appname_tools::kind_impls::{edit_region_at, finish_write_bounds};
use appname_tools::resample::{EdgePolicy, Plane};

use crate::corpus::slice1_registry;
use crate::op_bridge::{register_ops, wire_err};
use crate::registry::FilterRegistry;
use crate::traits::{Filter, FilterCtx};

pub struct FilterApplyKind {
    registry: Arc<FilterRegistry>,
}

pub struct FilterSmartAddKind {
    registry: Arc<FilterRegistry>,
}

/// Register the two filter kinds + the §12.1 filter-corpus ops onto an
/// assembly — everything derives from the ONE [`slice1_registry`]
/// (single-truth pin, module doc).
pub fn register(assembly: EngineAssembly) -> EngineAssembly {
    let registry = Arc::new(slice1_registry());
    let assembly = assembly
        .with_kind(Arc::new(FilterApplyKind {
            registry: Arc::clone(&registry),
        }))
        .with_kind(Arc::new(FilterSmartAddKind {
            registry: Arc::clone(&registry),
        }));
    register_ops(assembly, &registry)
}

// ---------------------------------------------------------------------------
// Shared stage plumbing.
// ---------------------------------------------------------------------------

/// Stage-3 registry lookup + nested param validation (§16.1/§40.6): the
/// filter's own schema re-validates `params.params` before any live-state
/// work, exactly like `adjustment.add`'s opKind arm.
fn resolve_filter<'r>(
    registry: &'r FilterRegistry,
    cx: &KindContext<'_>,
) -> Result<(&'r Arc<dyn Filter>, Value), EngineError> {
    let id = cx
        .params()
        .get("filterId")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::missing("filterId"))?;
    let filter = registry.get(id).ok_or_else(|| {
        EngineError::with_field(
            ErrorCode::UnknownOpKind,
            "filterId",
            format!("filterId {id:?} is not registered in the §16.1 filter registry"),
        )
    })?;
    let params = cx
        .params()
        .get("params")
        .cloned()
        .ok_or_else(|| EngineError::missing("params"))?;
    if !params.is_object() {
        return Err(EngineError::range(
            "params",
            "the filter's own params must be an object (§16.1)",
        ));
    }
    // Strictness ruling 2026-08-17: the registered schema's letter first
    // (unknown keys/types/enums typed), then the hand validator.
    filter.validate_strict(&params).map_err(wire_err)?;
    Ok((filter, params))
}

/// §11.1 transitive ALL gate through the ONE consolidated place
/// (`Document::effective_locks`, O-CORE door-slice handoff): the ancestry
/// union replaces the local walk — strictest-wins means any composition
/// of ancestor flags covering ALL binds identically.
fn refuse_all_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    if cx
        .document()
        .effective_locks(layer)
        .contains(LockFlags::ALL)
    {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "ALL lock along the ancestry blocks (§11.1 transitive rule)",
        ));
    }
    Ok(())
}

/// The §16.2 command seed pin: 0 (module doc — stochastic filters carry
/// their own `params.seed`).
fn filter_ctx(cx: &KindContext<'_>) -> FilterCtx {
    FilterCtx {
        canvas: cx.document().geometry().canvas_rect(),
        seed: 0,
        lod: 0,
    }
}

/// Fix-round D1 helper (R1 re-anchor): materialize an input plane over
/// `in_rect` under the filter's declared §16.3 edge policy, anchored at
/// the DOCUMENT data bounds (canvas ∪ the target layer's §2.5 content
/// bounds, ES:2617 [NORMATIVE]) — `Clamp` reads beyond the data bounds
/// re-read the nearest data-bounds pixel (so a solid border stays solid
/// under any kernel, and real off-canvas content is READ, never masked
/// by replication), `Transparent` reads return `(0,0,0,0)`; reads
/// INSIDE the data bounds always read true content (transparent where
/// no content exists — that is real data, not edge policy).
fn materialize_apron(
    read: impl Fn(IVec2) -> [f32; 4],
    in_rect: IRect,
    data_bounds: IRect,
    edge: crate::traits::EdgeRule,
) -> Plane {
    let mut p = Plane::new(in_rect);
    for y in in_rect.min.y..in_rect.max.y {
        for x in in_rect.min.x..in_rect.max.x {
            let at = match edge {
                crate::traits::EdgeRule::Clamp => IVec2::new(
                    x.clamp(data_bounds.min.x, data_bounds.max.x - 1),
                    y.clamp(data_bounds.min.y, data_bounds.max.y - 1),
                ),
                crate::traits::EdgeRule::Transparent => IVec2::new(x, y),
            };
            p.set(x, y, read(at));
        }
    }
    p
}

/// Mirrored-apron output dilation: out(p) reads in[p − r_left ..
/// p + r_right], so nonzero output extends left by the RIGHT margin and
/// right by the LEFT margin (and mirrored vertically) — exact for any
/// ROI shape the trait can declare (module-doc pin).
fn dilated_support(filter: &dyn Filter, bounds: IRect, params: &Value, ctx: &FilterCtx) -> IRect {
    let roi = filter.roi(bounds, params, ctx);
    let l = (bounds.min.x - roi.min.x).max(0);
    let r = (roi.max.x - bounds.max.x).max(0);
    let t = (bounds.min.y - roi.min.y).max(0);
    let b = (roi.max.y - bounds.max.y).max(0);
    IRect::new(
        bounds.min.x - r,
        bounds.min.y - b,
        bounds.max.x + l,
        bounds.max.y + t,
    )
}

// ---------------------------------------------------------------------------
// filter.apply — the destructive bake (§40.6).
// ---------------------------------------------------------------------------

impl KindImpl for FilterApplyKind {
    fn kind(&self) -> &'static str {
        "filter.apply"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (filter, params) = resolve_filter(&self.registry, cx)?;

        // §10.3 fade pass: opacity implemented; non-normal blend typed.
        // Fix-round D3 (brush decode_blend precedent): serde-parse against
        // the §4.5 wire vocabulary FIRST — an UNKNOWN name is a stage-3
        // schema violation (out_of_range), and only a valid-but-
        // unimplemented mode earns the typed NI.
        match cx.params().get("blend") {
            None => {}
            Some(v) => {
                let blend: appname_core::BlendMode = serde_json::from_value(v.clone())
                    .map_err(|e| EngineError::range("blend", format!("§4.5 wire name: {e}")))?;
                if blend != appname_core::BlendMode::Normal {
                    return Err(EngineError::new(
                        ErrorCode::NotImplemented,
                        format!(
                            "fade blend {blend:?} needs the §4.5 blend corpus (compositor row); \
                             the opacity fade and blend:normal are live"
                        ),
                    ));
                }
            }
        }
        let opacity = match cx.params().get("opacity") {
            None => 1.0f32,
            Some(v) => {
                let x = v.as_f64().unwrap_or(f64::NAN);
                let x = if x.is_nan() { 1.0 } else { x };
                if !(0.0..=1.0).contains(&x) {
                    return Err(EngineError::range("opacity", "fade opacity outside 0..=1"));
                }
                x as f32
            }
        };

        // Stage 6: target MUST be a Raster layer (§40.6).
        let target = cx.envelope().target.as_ref();
        let (layer, mask, channel) = match target {
            Some(t) => (t.layer_id, t.mask_id, t.channel_id),
            None => (None, None, None),
        };
        if mask.is_some() || channel.is_some() {
            return Err(EngineError::bad_target(
                "filter.apply targets target.layerId; mask/channel surfaces arrive with their \
                 rows (§40.6)",
            ));
        }
        let Some(layer) = layer else {
            return Err(EngineError::bad_target(
                "filter.apply requires target.layerId (§40.6)",
            ));
        };
        let lid = LayerId::from_uuid(layer);
        let node = cx.document().layer(lid).ok_or_else(|| {
            EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
        })?;
        if node.pixels().is_none() {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "the target MUST be a Raster layer — rasterize first (§12.4: the UI offers \
                 layer.rasterize + filter.apply as two recorded steps, §40.6)",
            ));
        }

        // §11.1: PIXELS blocks (the section names filter.apply); ALL
        // transitively; TRANSPARENCY constrains alpha writes (pin below).
        // Reads go through `Document::effective_locks` (O-CORE handoff):
        // a group's PIXELS/TRANSPARENCY now binds the subtree.
        let eff_locks = cx.document().effective_locks(lid);
        if eff_locks.contains(LockFlags::PIXELS) {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "the target layer's pixels are locked (§11.1 names filter.apply)",
            ));
        }
        let trans_locked = eff_locks.contains(LockFlags::TRANSPARENCY);
        refuse_all_locked(cx, lid)?;

        let ctx = filter_ctx(cx);
        let (bounds, unsigned) = {
            let buf = cx
                .document()
                .layer(lid)
                .and_then(|n| n.pixels())
                .expect("checked raster");
            (
                buf.bounds(),
                matches!(buf.kind(), k if appname_core::ChannelKind::is_unsigned_normalized(k)),
            )
        };
        if bounds.is_empty() {
            return Ok(KindOutcome::default()); // §40.1 empty push
        }

        // R1 re-anchor (ES:2617): the read substrate is the DOCUMENT
        // data bounds — canvas ∪ this layer's §2.5 content bounds.
        // Identical to the canvas whenever content ⊆ canvas.
        let data_bounds = ctx.canvas.union(&bounds);

        // Output support ∩ selection (§16.2 masking narrows the write).
        let mut region = dilated_support(filter.as_ref(), bounds, &params, &ctx);
        // Fix-round D1 consequential, R1 re-anchored (module-doc pin):
        // under the data-bounds-anchored Clamp policy, output beyond the
        // data bounds would derive solely from edge replication — an
        // artifact, not §2.5 content — so the write region clamps to the
        // data bounds for Clamp filters (off-canvas CONTENT still bakes:
        // it is inside the data bounds). Transparent-policy output
        // beyond them is real faded content and keeps the full dilation.
        if filter.edge() == crate::traits::EdgeRule::Clamp {
            region = region.intersect(&data_bounds);
        }
        let sel_active = cx.document().selection().active();
        if sel_active {
            region = region.intersect(&cx.document().selection().bounds());
        }
        if region.is_empty() || opacity == 0.0 {
            return Ok(KindOutcome::default()); // §40.1 empty push
        }

        // Materialize the ROI apron from the layer's true content (§16.2
        // evaluator duty) UNDER the filter's declared §16.3 edge policy —
        // fix-round D1 + R1: the policy anchors at the DOCUMENT data
        // bounds, so reads beyond them clamp to the nearest data-bounds
        // pixel (Clamp) or return (0,0,0,0) (Transparent) HERE; the
        // kernels' own `Plane::get` handling only extends this plane and
        // cannot know the document.
        let in_rect = filter.roi(region, &params, &ctx);
        let input = {
            let buf = cx
                .document()
                .layer(lid)
                .and_then(|n| n.pixels())
                .expect("checked raster");
            materialize_apron(
                |p| buf.read_pixel_f32(p),
                in_rect,
                data_bounds,
                filter.edge(),
            )
        };
        let filtered = filter
            .cpu(&input, region, &params, &ctx)
            .map_err(wire_err)?;

        // §16.2 masked write-back (+ §10.3 fade weight) through the
        // recorder doors; only changed tiles land (§40.1).
        let changed = edit_region_at(cx, lid, region, &mut |p, dst, cov| {
            let w = cov * opacity;
            if w <= 0.0 {
                return dst;
            }
            let out = filtered.get(p.x, p.y, EdgePolicy::Transparent);
            let mut v = if trans_locked {
                // TRANSPARENCY pin: filtered straight color over the
                // pixel's ORIGINAL alpha (alpha never written).
                // Fix-round D2 (§11.1 source-atop brush precedent): a
                // filtered source with zero alpha CONTRIBUTES NOTHING —
                // keep dst; unpremultiplying it would fabricate black.
                if out[3] <= 0.0 {
                    return dst;
                }
                let s = appname_core::io::srgb::unpremul(out);
                [s[0] * dst[3], s[1] * dst[3], s[2] * dst[3], dst[3]]
            } else {
                out
            };
            for c in 0..4 {
                v[c] = dst[c] + (v[c] - dst[c]) * w;
            }
            if unsigned {
                // §18.2 store discipline (install_plane parity).
                v[3] = v[3].clamp(0.0, 1.0);
                for c in 0..3 {
                    v[c] = v[c].clamp(0.0, v[3]);
                }
            }
            v
        })?;
        if !changed.is_empty() {
            // §2.4: blurs legitimately spread content past the old extent;
            // erasing filters shrink. BOTH are served pixel-exact below (the
            // old eager-grow + tile-granular-recompute pair is fully replaced
            // — an eager grow by the proud `changed` rect would stick on the
            // grow branch, which only ever unions).
            //
            // Q-1 determinism class (the FIFTH door): `changed` is computed
            // on the PRE-quantization f32 delta, so through a FEATHERED
            // selection the fringe (w = cov·opacity sub-quantum) counts
            // pixels that store back exactly as they were — the eager extent
            // runs proud of the persisted plane, and the tile-granular
            // `recompute_layer_bounds` this door used before cannot tighten
            // below an occupied tile. The loader recomputes pixel-exact, so
            // `region_hash` (which folds bounds) forked live-vs-reloaded and
            // the D-48 replay-as-recovery gate refused undo/redo (q-scenario
            // A3: 24 OP-filter × S-feathered × C-partial cells). Route
            // through the shared class door: TRANSPARENCY-locked filters
            // never write alpha (the trans_locked arm keeps dst[3]), so
            // coverage only ADDS and the scoped pixel-exact grow is exact;
            // an unlocked filter can REDUCE alpha (a blur recedes a content
            // edge inward), so only the full pixel-exact recompute matches
            // the loader's sub-tile shrink.
            finish_write_bounds(cx, lid, changed, !trans_locked)?;
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// filter.smart_add — non-destructive chain append (§40.6).
// ---------------------------------------------------------------------------

impl KindImpl for FilterSmartAddKind {
    fn kind(&self) -> &'static str {
        "filter.smart_add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (filter, params) = resolve_filter(&self.registry, cx)?;

        // §9.6 creation UUID (stage-3 duplicate check).
        let new_op = cx
            .params()
            .get("newOpId")
            .ok_or_else(|| EngineError::missing("newOpId"))?
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| EngineError::range("newOpId", "uuid string (§9.6)"))?;
        if cx.document().op(OpId::from_uuid(new_op)).is_some() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "newOpId",
                format!("newOpId {new_op} already exists (§9.6)"),
            ));
        }
        // §40.6: out-of-range index CLAMPS to 0..=len — never an error.
        let index_req = cx.params().get("index").and_then(Value::as_i64);

        // Stage 6: target MUST be a SmartObject (§40.6).
        let layer = cx
            .envelope()
            .target
            .as_ref()
            .and_then(|t| t.layer_id)
            .ok_or_else(|| {
                EngineError::bad_target("filter.smart_add requires target.layerId (§40.6)")
            })?;
        let lid = LayerId::from_uuid(layer);
        let node = cx.document().layer(lid).ok_or_else(|| {
            EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
        })?;
        let Some(so) = node.smart_object() else {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "the target MUST be a SmartObject — run smartobject.convert first (§40.6)",
            ));
        };
        let chain_len = so.smart_filters().len();
        let index = index_req
            .map(|i| i.clamp(0, chain_len as i64) as usize)
            .unwrap_or(chain_len); // §10.3 default: len(chain)
        let filter_id = filter.id();

        // §40.6 lock table: ALL blocks on the target (transitively per
        // §11.1); PIXELS does not — non-destructive.
        refuse_all_locked(cx, lid)?;

        // Execution tail (CP-0.7 tail flip): O-CORE's recorded insert
        // door — arena `AddOp` + chain-splice `AddFilterRef` in ONE
        // DeltaSet (mirror of the remove door; the recorded index is the
        // post-clamp position, so undo/redo is exact-inverse). The
        // OpNode's op id IS the registered `Filter::id()` — the §12.1
        // single-truth pin (op_bridge registers the corpus under the
        // same ids, so the chain entry resolves in the op registry).
        // Construction is inert (D-48); the door is the only mutation.
        let id = cx.insert_smart_filter(
            lid,
            index,
            OpNode::new(OpId::from_uuid(new_op), filter_id, params),
        )?;
        Ok(KindOutcome {
            created_ids: vec![id.as_uuid()],
        })
    }
}
