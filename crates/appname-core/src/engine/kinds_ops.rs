// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `adjustment.add` / `op.set_params` / `op.set_mask` /
//! `op.reorder` / `op.toggle` / `op.delete` / `smartobject.convert` /
//! `smartobject.edit_contents` / `smartobject.replace` /
//! `smartobject.rasterize` / `layer.rasterize`; ES §12.1/§12.2 (op model +
//! §40.6 family rules), §11.5 (smart objects), §12.4 (rasterize = class
//! C), §9.6 (engine-derived ids). CP-0.7 wave-1 kind family.
//!
//! v1 scope (typed refusals name their rows): op params validate against
//! the assembly's §12.1 registry (no ops registered ⇒ `adjustment.add`
//! answers bad_target naming the registry — O-TOOLS' compute half fills
//! it); `op.set_mask{maskId}` (ownership transfer from the §14 library)
//! and `smartobject.convert` of multi-layer/group selections (needs the
//! compositor flatten) are typed-pending; smart-object sources are
//! embedded raster assets (nested `.appname` = the render row; linked =
//! shell-side drift detection). `op.set_params`/`guide-style` coalescing
//! awaits §10.5 (standing divergence). Rasterize bakes the CACHED
//! composite (§12.4: bit-identical to the composite at commit — for v1
//! smart objects the cache IS the composite; op-chain rendering joins
//! with the compositor row) under identity/integer-translate placement
//! (general placement = the tools resample engines).

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::engine::doc_new::derive_id;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::graph::id::{LayerId, MaskId, OpId};
use crate::graph::node::{LayerNode, LockFlags, SmartObjectData, SmartSource};
use crate::graph::op_node::OpNode;

pub struct AdjustmentAddKind;
pub struct OpSetParamsKind;
pub struct OpSetMaskKind;
pub struct OpReorderKind;
pub struct OpToggleKind;
pub struct OpDeleteKind;
pub struct SmartConvertKind;
pub struct SmartEditContentsKind;
pub struct SmartReplaceKind;
pub struct SmartRasterizeKind;
pub struct LayerRasterizeKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(AdjustmentAddKind),
        Arc::new(OpSetParamsKind),
        Arc::new(OpSetMaskKind),
        Arc::new(OpReorderKind),
        Arc::new(OpToggleKind),
        Arc::new(OpDeleteKind),
        Arc::new(SmartConvertKind),
        Arc::new(SmartEditContentsKind),
        Arc::new(SmartReplaceKind),
        Arc::new(SmartRasterizeKind),
        Arc::new(LayerRasterizeKind),
    ]
}

fn param_uuid(cx: &KindContext<'_>, name: &str) -> Result<Uuid, EngineError> {
    cx.params()
        .get(name)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| EngineError::missing(name))
}

/// §40.6: the OpNode addressed by params.opId, plus its OWNING layer
/// (adjustment layer or smart-filter host) — the lock/dirty anchor.
fn resolve_op(cx: &KindContext<'_>) -> Result<(OpId, LayerId), EngineError> {
    let oid = OpId::from_uuid(param_uuid(cx, "opId")?);
    if cx.document().op(oid).is_none() {
        return Err(EngineError::bad_target(format!(
            "params.opId {oid} does not resolve"
        )));
    }
    let owner = cx
        .document()
        .z_order_walk()
        .into_iter()
        .find(|lid| {
            let n = cx.document().layer(*lid).expect("walk resolves");
            n.adjustment_op() == Some(oid)
                || n.smart_object()
                    .map(|so| so.smart_filters().contains(&oid))
                    .unwrap_or(false)
        })
        .ok_or_else(|| EngineError::bad_target(format!("op {oid} has no owning layer")))?;
    Ok((oid, owner))
}

fn refuse_all_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    // Door-slice lock consolidation: the one-place effective set (§11.1).
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

/// §40.7 PIXELS gate on the target node (fix-round M6): the smartobject
/// content-rewriting kinds and `layer.rasterize` block on a PIXELS lock in
/// addition to the transitive ALL rule (house pattern:
/// `kinds::require_pixels_unlocked`).
fn refuse_pixels_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    // Door-slice lock consolidation: PIXELS is transitive like every flag
    // (§11.1 "locks on a Group apply transitively"; strictest wins).
    if cx
        .document()
        .effective_locks(layer)
        .contains(LockFlags::PIXELS)
    {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the target layer's pixels are locked (§11.1; §40.7: PIXELS blocks — rendered \
             content is rewritten)",
        ));
    }
    Ok(())
}

impl KindImpl for AdjustmentAddKind {
    fn kind(&self) -> &'static str {
        "adjustment.add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let op_kind = cx
            .params()
            .get("opKind")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("opKind"))?
            .to_string();
        // §12.1 stage-3 nested validation against the assembly registry.
        // Fix-round M9 (contract §40.6 addition): an unregistered opKind
        // answers `error.schema.unknown_op_kind`, not bad_target.
        let Some(op_impl) = cx.op_registry().get(&op_kind).cloned() else {
            return Err(EngineError::with_field(
                ErrorCode::UnknownOpKind,
                "opKind",
                format!(
                    "opKind \"{op_kind}\" is not registered in this assembly's §12.1 op registry \
                     (the compute half is the O-TOOLS seam)"
                ),
            ));
        };
        let params = cx.params().get("params").cloned().unwrap_or(Value::Null);
        let params = if params.is_null() || params.as_object().map(|m| m.is_empty()) == Some(true) {
            op_impl.default_params()
        } else {
            params
        };
        op_impl.validate_params(&params)?;
        let new_layer = param_uuid(cx, "newLayerId")?;
        let new_op = param_uuid(cx, "newOpId")?;
        let lid = LayerId::from_uuid(new_layer);
        let oid = OpId::from_uuid(new_op);
        if cx.document().layer(lid).is_some() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "newLayerId",
                format!("newLayerId {new_layer} already exists (§9.6)"),
            ));
        }
        if cx.document().op(oid).is_some() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "newOpId",
                format!("newOpId {new_op} already exists (§9.6)"),
            ));
        }
        // Insertion anchor: target.layerId (immediately above it) or
        // params.parent/index (layer.add form; parent default root, top).
        let (parent, index) = match cx.envelope().target.as_ref().and_then(|t| t.layer_id) {
            Some(anchor) => {
                let anchor = LayerId::from_uuid(anchor);
                let parent = cx.document().parent_of(anchor).ok_or_else(|| {
                    EngineError::bad_target(format!("target.layerId {anchor} does not resolve"))
                })?;
                let at = cx
                    .document()
                    .children_of(parent)
                    .and_then(|c| c.iter().position(|k| *k == anchor))
                    .expect("parent lists anchor");
                (parent, at + 1)
            }
            None => {
                let parent = match cx.params().get("parent").and_then(Value::as_str) {
                    Some(s) => {
                        let p = LayerId::from_uuid(
                            Uuid::parse_str(s).map_err(|_| EngineError::range("parent", "uuid"))?,
                        );
                        if !cx
                            .document()
                            .layer(p)
                            .map(|n| n.is_group())
                            .unwrap_or(false)
                        {
                            return Err(EngineError::new(
                                ErrorCode::TargetKindMismatch,
                                "params.parent must resolve to a Group (§40.6)",
                            ));
                        }
                        p
                    }
                    None => cx.document().root(),
                };
                let len = cx
                    .document()
                    .children_of(parent)
                    .map(<[_]>::len)
                    .unwrap_or(0);
                let index = cx
                    .params()
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| (i as usize).min(len))
                    .unwrap_or(len);
                (parent, index)
            }
        };
        refuse_all_locked(cx, parent)?;
        // Fix-round m14: the contract declares NO `name` param
        // (additionalProperties:false — a `name` key never survives stage
        // 3), so the layer names after its opKind; localized display
        // naming is the shell's (§27).
        let name = op_kind.clone();
        // Fix-round M7: contract `clipToBelow` (default false, stage-3
        // filled) lands on the new adjustment layer's §2.6 prop block.
        let clip_to_below = cx
            .params()
            .get("clipToBelow")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        cx.insert_op_node(OpNode::new(oid, op_kind, params))?;
        let mut node = LayerNode::new_adjustment(lid, name, oid);
        node.clip_to_below = clip_to_below;
        cx.insert_layer(parent, index, node)?;
        // §10.3 D over the op's downstream ROI (§11.6): the new enabled
        // adjustment recolors the below-stack extent at re-render.
        cx.note_op_composite_dirty(oid);
        Ok(KindOutcome {
            created_ids: vec![new_layer, new_op],
        })
    }
}

impl KindImpl for OpSetParamsKind {
    fn kind(&self) -> &'static str {
        "op.set_params"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (oid, owner) = resolve_op(cx)?;
        refuse_all_locked(cx, owner)?;
        let params = cx
            .params()
            .get("params")
            .cloned()
            .ok_or_else(|| EngineError::missing("params"))?;
        let op_kind = cx.document().op(oid).expect("resolved").op().to_string();
        // Fix-round m12: nested validation NEVER silently skips. An op
        // whose kind is absent from this assembly's §12.1 registry cannot
        // re-validate its params — a typed refusal naming the registry
        // (mirroring adjustment.add's posture; `unknown_op_kind` is
        // contract-documented UNREACHABLE for this kind because the op's
        // kind was fixed at creation, so this answers bad_target).
        let Some(op_impl) = cx.op_registry().get(&op_kind) else {
            return Err(EngineError::bad_target(format!(
                "op {oid} has kind \"{op_kind}\", which is not registered in this assembly's \
                 §12.1 op registry — params cannot re-validate (the compute half is the \
                 O-TOOLS seam)"
            )));
        };
        op_impl.validate_params(&params)?;
        // Full-state block: params ONLY.
        //
        // RULING 4 (operator, 2026-08-28). Two branches lived here reading
        // `cx.params().get("blend")` and `get("opacity")` under a comment calling them
        // "optional fade fields (§40.6)". THEY WERE UNREACHABLE: `cx.params()` IS THE
        // ENVELOPE OBJECT, and op.set_params.yaml:25 declares it
        // `additionalProperties: false` with properties opId and params AND NOTHING ELSE.
        //
        // MEASURED BEFORE DELETING, WITH A CONTROL (INTEGRATION-log 250):
        //   control  well-formed envelope    -> rc 0, "Applied op.set_params"
        //   probe    same + top-level blend  -> rc 1, error.schema.out_of_range:
        //            unknown parameter `blend` (additionalProperties: false) field=blend
        // Stage 3 refuses the key BY NAME, so the handler never saw it. THE CONTROL IS
        // WHAT MAKES THAT A RESULT: without it, a refusal for bad_target would have read
        // the same and proved nothing.
        //
        // A CONTRACT AND ITS IMPLEMENTATION DISAGREED HERE WITH NO TEST FAILING, because
        // the stricter one silently absorbed the looser (G-519). An op's fade fields are
        // set through the OpNode value block, never through this command.
        let mut after = cx.document().op(oid).expect("resolved").value_block();
        after.params = params;
        cx.set_op_value(oid, after)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for OpToggleKind {
    fn kind(&self) -> &'static str {
        "op.toggle"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (oid, owner) = resolve_op(cx)?;
        refuse_all_locked(cx, owner)?;
        let enabled = cx
            .params()
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| EngineError::missing("enabled"))?;
        let mut after = cx.document().op(oid).expect("resolved").value_block();
        after.enabled = enabled;
        // §12.4 bit-identity: toggling off must equal never-applied — holds
        // structurally (a disabled op is skipped in the §12.1 chain).
        cx.set_op_value(oid, after)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for OpSetMaskKind {
    fn kind(&self) -> &'static str {
        "op.set_mask"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (oid, owner) = resolve_op(cx)?;
        refuse_all_locked(cx, owner)?;
        if cx.params().get("maskId").is_some() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "op.set_mask{maskId} (ownership transfer) arrives with the §14 mask/channel \
                 library",
            ));
        }
        match cx.params().get("from").and_then(Value::as_str) {
            Some("none") => cx.set_op_mask(oid, None)?,
            Some("selection") => {
                if !cx.document().selection().active() {
                    return Err(EngineError::new(
                        ErrorCode::SelectionRequired,
                        "op.set_mask from:'selection' requires an active selection (§40.6)",
                    ));
                }
                // §9.6 engine-derived creation id (deterministic per
                // envelope for replay).
                let mid = MaskId::from_uuid(derive_id(
                    cx.envelope().id,
                    &format!("op_mask/{}", oid.as_uuid()),
                ));
                let sel = cx.document().selection();
                let mut coverage = crate::mask::Selection::new_mask_buffer();
                let tiles: Vec<_> = sel
                    .mask()
                    .tiles()
                    .iter()
                    .map(|(c, a)| (c, Arc::clone(a)))
                    .collect();
                for (c, a) in tiles {
                    coverage.install_tile(c, a);
                }
                coverage.set_bounds_recorded(sel.bounds());
                let node = crate::graph::mask_node::MaskNode::new_raster(mid, coverage);
                cx.set_op_mask_with_node(oid, node)?;
            }
            Some(other) => return Err(EngineError::range("from", format!("unknown \"{other}\""))),
            None => return Err(EngineError::missing("from")),
        }
        Ok(KindOutcome::default())
    }
}

impl KindImpl for OpReorderKind {
    fn kind(&self) -> &'static str {
        "op.reorder"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (oid, owner) = resolve_op(cx)?;
        refuse_all_locked(cx, owner)?;
        let index = cx
            .params()
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::missing("index"))? as usize;
        if cx
            .document()
            .layer(owner)
            .expect("resolved")
            .smart_object()
            .is_none()
        {
            // Fix-round m10: op.reorder's contract error list is the §40.6
            // base set — target_kind_mismatch is not on it; the wrong-op-
            // shape target answers bad_target.
            return Err(EngineError::bad_target(
                "op.reorder addresses a smart-filter CHAIN member; an adjustment layer's single \
                 op has no order (§12.2)",
            ));
        }
        // QA-F12 closure: the contract's `index` row is explicit — "must
        // lie within 0..len(chain)−1 → error.schema.out_of_range (§40.6)
        // — the upper bound is a live-state check, not statically
        // expressible". The former CLAMP borrowed the §40.6
        // filter.smart_add precedent, but that precedent is an INSERT
        // position (0..=len is a valid domain); a REORDER position beyond
        // the chain end has no reading other than out-of-range.
        let chain_len = cx
            .document()
            .layer(owner)
            .expect("resolved")
            .smart_object()
            .expect("checked above")
            .smart_filters
            .len();
        if index >= chain_len {
            return Err(EngineError::range(
                "index",
                format!(
                    "`index` must lie within 0..{} (chain length {chain_len}), got {index} \
                     (§40.6 live-state bound)",
                    chain_len.saturating_sub(1)
                ),
            ));
        }
        cx.reorder_op(owner, oid, index)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for OpDeleteKind {
    fn kind(&self) -> &'static str {
        "op.delete"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (oid, owner) = resolve_op(cx)?;
        refuse_all_locked(cx, owner)?;
        let is_adjustment = cx
            .document()
            .layer(owner)
            .expect("resolved")
            .adjustment_op()
            == Some(oid);
        if is_adjustment {
            // §10.3 D, noted BEFORE the coupled delete (the owner must
            // still stand in the walk); a disabled op deletes invisibly
            // (§12.4 identity before and after).
            if cx.document().op(oid).map(|n| n.enabled()).unwrap_or(false) {
                cx.note_op_composite_dirty(oid);
            }
            // §40.6 coupled delete: the adjustment layer dies with its op,
            // one DeltaSet (subtree first, then the op — undo re-inserts
            // the op before the layer references it, reverse order).
            cx.remove_subtree(owner)?;
            cx.remove_op_node(oid)?;
        } else {
            cx.remove_smart_filter(owner, oid)?;
            cx.remove_op_node(oid)?;
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// smartobject.* + rasterize
// ---------------------------------------------------------------------------

impl KindImpl for SmartConvertKind {
    fn kind(&self) -> &'static str {
        "smartobject.convert"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let ids = cx
            .params()
            .get("layerIds")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("layerIds"))?;
        if ids.len() != 1 {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "multi-layer convert needs the compositor flatten (§11.5); v1 converts one \
                 raster layer",
            ));
        }
        let src = LayerId::from_uuid(
            ids[0]
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::range("layerIds", "members must be uuids"))?,
        );
        let Some(node) = cx.document().layer(src) else {
            return Err(EngineError::bad_target(format!(
                "layer {src} does not resolve"
            )));
        };
        if node.pixels().is_none() {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "v1 converts RASTER layers (parametric/group sources need the nested-composite \
                 render row)",
            ));
        }
        refuse_all_locked(cx, src)?;
        // §11.2 / q-scenario/Q-5 — enforce a typed refusal instead of silently
        // dropping the mask. `remove_subtree(src)` below consumes the source
        // and the new smart object is built with NO mask (place_transform
        // IDENTITY, no mask arena) — the mask is neither carried onto the SO
        // nor promoted to it, so every pixel it hid becomes visible with
        // ok:true. Until mask-preserving convert is built, REFUSE, matching the
        // structural-command discipline every sibling keeps (layer.group
        // preserves the mask; layer.duplicate refuses on an unmapped mask).
        if layer_has_enabled_mask(cx, src) {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "smartobject.convert on a MASKED layer would drop the mask: the wrapped layer's \
                 mask is neither carried onto the new smart object nor promoted to it, so the \
                 pixels it hides would silently become visible. Refused typed rather than serve \
                 mask-less pixels (QA-F8); apply or remove the mask before converting \
                 (cf. layer.group, which preserves it).",
            ));
        }
        // QA-F6 [RULED, door-slice escalation 1]: the contract marks
        // newLayerId OPTIONAL ("wrapper fills when omitted") — the
        // HEADLESS door fills DETERMINISTICALLY (uuidv5 over the envelope
        // id, the door-slice mechanism), so journal replay and action
        // playback reproduce identical ids with no wrapper present
        // (D-48). Present params behave exactly as before.
        let new_layer = crate::engine::kinds_vector::creation_id(cx, "newLayerId")?;
        let lid = LayerId::from_uuid(new_layer);
        if cx.document().layer(lid).is_some() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "newLayerId",
                format!("newLayerId {new_layer} already exists (§9.6)"),
            ));
        }
        // Source pixels → PNG asset (content-addressed; §11.5 dedup falls
        // out of the store) → embedded source; the cache IS those pixels
        // (§12.4 bit-identity at commit).
        let png = cx.encode_layer_png(src)?;
        // QA-F7 [RULED, door-slice escalation 2]: asset ids are
        // CONTENT-DERIVED (§7.4 — dedup is structural). An absent
        // newAssetId needs no fill: the content-derived id IS the id. A
        // PRESENT newAssetId is checked accept-if-equal — silent ignore
        // is the banned shape; a mismatch refuses TYPED (bad_target
        // family, §40.1 stage-4 id-valued-param row) BEFORE any mutation.
        // Fold-in candidate registered O-INT-side (drop the param or
        // document accept-if-equal contract-side).
        let content_id = crate::engine::assets::asset_id_for(&png);
        if let Some(requested) = cx.params().get("newAssetId") {
            let requested = requested
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::range("newAssetId", "must be a uuid"))?;
            if requested != content_id {
                return Err(EngineError::bad_target(format!(
                    "newAssetId {requested} does not match the content-derived asset id \
                     {content_id} — asset ids are content-addressed (§7.4); omit \
                     newAssetId or pass the content-derived value (QA-F7 ruling: \
                     accept-if-equal, never silent ignore)"
                )));
            }
        }
        let asset = cx.ingest_raster_asset(png)?;
        debug_assert_eq!(asset, content_id, "§7.4 content addressing");
        let name = cx
            .document()
            .layer(src)
            .expect("resolved")
            .name()
            .to_string();
        let pixels = cx
            .document()
            .layer(src)
            .expect("resolved")
            .pixels()
            .expect("raster")
            .clone();
        let parent = cx.document().parent_of(src).expect("non-root");
        let at = cx
            .document()
            .children_of(parent)
            .and_then(|c| c.iter().position(|k| *k == src))
            .expect("parent lists source");
        cx.remove_subtree(src)?;
        cx.insert_layer(
            parent,
            at,
            LayerNode::new_smart_object(
                lid,
                name,
                SmartObjectData {
                    source: SmartSource::Embedded(asset),
                    cached_composite: pixels,
                    smart_filters: Vec::new(),
                    place_transform: crate::geom::Mat3::IDENTITY,
                },
            ),
        )?;
        Ok(KindOutcome {
            created_ids: vec![new_layer],
        })
    }
}

/// True iff `layer` carries at least one ENABLED mask (a disabled mask is a
/// §11.2 no-op). The q-scenario/Q-5 refusal on `smartobject.convert` keys on
/// this, mirroring the compositor's enabled-mask coverage resolution.
fn layer_has_enabled_mask(cx: &KindContext<'_>, layer: LayerId) -> bool {
    let doc = cx.document();
    doc.layer(layer)
        .map(|n| {
            n.masks()
                .iter()
                .any(|m| doc.mask(*m).is_some_and(|mn| mn.enabled()))
        })
        .unwrap_or(false)
}

fn resolve_smart_target(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let lid = cx
        .envelope()
        .target
        .as_ref()
        .and_then(|t| t.layer_id)
        .map(LayerId::from_uuid)
        .ok_or_else(|| EngineError::bad_target("this kind requires target.layerId"))?;
    let Some(node) = cx.document().layer(lid) else {
        return Err(EngineError::bad_target(format!(
            "target.layerId {lid} does not resolve"
        )));
    };
    if node.smart_object().is_none() {
        return Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "the target is not a smart object (§11.5)",
        ));
    }
    Ok(lid)
}

/// Swap sources on EVERY instance of `before` (§11.5 edit-contents rule:
/// instance invalidation keys on asset identity).
fn swap_all_instances(
    cx: &mut KindContext<'_>,
    before: Uuid,
    after: Uuid,
) -> Result<usize, EngineError> {
    let instances: Vec<LayerId> = cx
        .document()
        .z_order_walk()
        .into_iter()
        .filter(|lid| {
            cx.document()
                .layer(*lid)
                .and_then(|n| n.smart_object())
                .map(|so| matches!(so.source(), SmartSource::Embedded(a) if *a == before))
                .unwrap_or(false)
        })
        .collect();
    let count = instances.len();
    for lid in instances {
        cx.set_smart_source(lid, before, after)?;
        cx.refresh_smart_cache(lid)?;
    }
    Ok(count)
}

impl KindImpl for SmartEditContentsKind {
    fn kind(&self) -> &'static str {
        "smartobject.edit_contents"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let target = resolve_smart_target(cx)?;
        // Fix-round M6: contract lockBehavior — PIXELS blocks (content
        // swap rewrites rendered pixels) and ALL blocks transitively.
        refuse_all_locked(cx, target)?;
        refuse_pixels_locked(cx, target)?;
        let before = param_uuid(cx, "assetBefore")?;
        let after = param_uuid(cx, "assetAfter")?;
        let SmartSource::Embedded(cur) = cx
            .document()
            .layer(target)
            .expect("resolved")
            .smart_object()
            .expect("smart")
            .source();
        if *cur != before {
            return Err(EngineError::bad_target(format!(
                "assetBefore {before} does not match the target's current source"
            )));
        }
        if cx.asset_kind(after).is_none() {
            return Err(EngineError::bad_target(format!(
                "assetAfter {after} does not resolve to an ingested asset (§9.6 rule 2)"
            )));
        }
        let n = swap_all_instances(cx, before, after)?;
        debug_assert!(n >= 1, "target matched");
        Ok(KindOutcome::default())
    }
}

impl KindImpl for SmartReplaceKind {
    fn kind(&self) -> &'static str {
        "smartobject.replace"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let target = resolve_smart_target(cx)?;
        refuse_all_locked(cx, target)?;
        // Fix-round M6: PIXELS blocks too (contract §40.7 — the relinkOnly
        // exception rides the linked-source row, which is typed-pending).
        refuse_pixels_locked(cx, target)?;
        let source = cx
            .params()
            .get("source")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing("source"))?;
        if source.get("path").is_some() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "linked sources (path/relinkOnly) are shell-side drift-detection work (§11.5)",
            ));
        }
        let asset = source
            .get("assetId")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| EngineError::missing("source.assetId"))?;
        if !matches!(
            cx.asset_kind(asset),
            Some(crate::engine::assets::AssetKind::RasterImage)
        ) {
            return Err(EngineError::bad_target(format!(
                "source.assetId {asset} does not resolve to an ingested raster asset"
            )));
        }
        // Replace affects THIS instance only (contrast edit_contents).
        let SmartSource::Embedded(before) = *cx
            .document()
            .layer(target)
            .expect("resolved")
            .smart_object()
            .expect("smart")
            .source();
        cx.set_smart_source(target, before, asset)?;
        cx.refresh_smart_cache(target)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for SmartRasterizeKind {
    fn kind(&self) -> &'static str {
        "smartobject.rasterize"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let target = resolve_smart_target(cx)?;
        refuse_all_locked(cx, target)?;
        // Fix-round M6: PIXELS blocks too (contract: content is rewritten
        // as raster buffers).
        refuse_pixels_locked(cx, target)?;
        cx.checkpoint_rasterize(target)
    }
}

impl KindImpl for LayerRasterizeKind {
    fn kind(&self) -> &'static str {
        "layer.rasterize"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = cx
            .envelope()
            .target
            .as_ref()
            .and_then(|t| t.layer_id)
            .map(LayerId::from_uuid)
            .ok_or_else(|| EngineError::bad_target("layer.rasterize requires target.layerId"))?;
        let Some(node) = cx.document().layer(lid) else {
            return Err(EngineError::bad_target(format!(
                "target.layerId {lid} does not resolve"
            )));
        };
        // Fix-round M6: contract lockBehavior — "PIXELS or ALL block
        // (content is rewritten as raster buffers)" — gates EVERY arm
        // (the contract's `locked` fixture pins exactly this).
        refuse_all_locked(cx, lid)?;
        refuse_pixels_locked(cx, lid)?;
        // QA-F3 closure — contract letter: "target MUST be parametric
        // (text/shape/fill/smart object) or carry effects when
        // what:'effects' → error.command.target_kind_mismatch
        // (already-raster, no effects)". Effects/styles are not in the
        // engine yet (§11.3 row pending), so a plain raster target can
        // never carry them: what:'effects' on a raster is exactly the
        // contract's mismatch case, not a no-op accept.
        let what = cx
            .params()
            .get("what")
            .and_then(Value::as_str)
            .unwrap_or("content");
        if what == "effects" && node.kind().kind_str() == "raster" {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "layer.rasterize{what:\"effects\"} on an already-raster layer with no effects \
                 (§40.3 contract row: error.command.target_kind_mismatch)",
            ));
        }
        match node.kind().kind_str() {
            "smartObject" => cx.checkpoint_rasterize(lid),
            "raster" => {
                // Rasterizing a raster is the documented no-op push (§12.4).
                Ok(KindOutcome::default())
            }
            "adjustment" => Err(EngineError::new(
                ErrorCode::NotImplemented,
                "rasterizing an adjustment bakes its backdrop effect — the compositor row",
            )),
            // The Door-6 SERVE (composite-leg slice; the retape commit's
            // NI wire survives only on service-less assemblies via
            // require_text_layout): layout_and_compose's premultiplied
            // linear page bakes into a raster buffer through the class-C
            // snapshot door. D-48: the page is VALUE-CAPTURED through
            // the capture table (framing in text_ops) — replay consumes
            // the capture and NEVER re-runs layout (§23.2 purity is in
            // (sidecar, FONTS); the font environment may drift between
            // record and replay — the to_shape rationale verbatim).
            "text" => {
                if cx.document().color().mode != crate::tile::model::ColorModelKind::Rgba {
                    return Err(EngineError::new(
                        ErrorCode::NotImplemented,
                        "text rasterization into non-rgba document modes needs the §6.2 \
                         page-marshalling arm — typed refusal, recorded",
                    ));
                }
                let page = match cx.vector_capture() {
                    Some(g) => crate::engine::text_ops::decode_page_capture(&g.sidecar)?,
                    None => {
                        let svc = cx.require_text_layout()?;
                        let sidecar = cx
                            .document()
                            .layer(lid)
                            .and_then(|n| n.text_data())
                            .expect("kind_str said text")
                            .sidecar()
                            .to_vec();
                        let (page, _metrics) = svc.layout_and_compose(&sidecar)?;
                        cx.record_vector_capture(std::sync::Arc::new(
                            crate::engine::vector_ops::GeometryResult {
                                sidecar: crate::engine::text_ops::encode_page_capture(&page),
                                bounds: page.rect,
                            },
                        ));
                        page
                    }
                };
                let buf = crate::io::png_io::buffer_from_text_page(
                    &page,
                    cx.document().color().depth,
                    cx.document().color().working,
                );
                cx.checkpoint_rasterize_text(lid, buf)
            }
            other => Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                format!("layer kind \"{other}\" has no rasterize arm"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::op_impl::{OpImpl, PreviewPolicy};
    use crate::engine::Engine;
    use crate::geom::IRect;
    use serde_json::json;

    /// Test-only reference op (the seam's in-crate probe; production
    /// kernels are O-TOOLS').
    struct TestInvert;
    impl OpImpl for TestInvert {
        fn op_id(&self) -> &'static str {
            "adjustment.invert"
        }
        fn validate_params(&self, p: &Value) -> Result<(), EngineError> {
            if p.as_object().map(|m| m.is_empty()) == Some(true) || p.is_null() {
                Ok(())
            } else {
                Err(EngineError::range(
                    "params",
                    "adjustment.invert takes no params",
                ))
            }
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
            _p: &Value,
            _rect: IRect,
            input: &[[f32; 4]],
            output: &mut [[f32; 4]],
        ) -> Result<(), EngineError> {
            for (o, i) in output.iter_mut().zip(input) {
                let a = i[3];
                *o = [a - i[0], a - i[1], a - i[2], a];
            }
            Ok(())
        }
    }

    fn assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
            .with_op(Arc::new(TestInvert))
            // Production fill lives tools-side; the dagger battery uses core's
            // reference FillKind (in-crate fixture door, fill-flip precedent).
            .with_kind(Arc::new(crate::engine::kinds::FillKind))
    }

    fn ready() -> (Engine, LayerId) {
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        (e, bg)
    }

    fn env(n: u32, kind: &str, layer: Option<LayerId>, params: Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("0b7e0000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.as_uuid().to_string()});
        }
        v.to_string()
    }

    #[test]
    fn adjustment_add_and_op_family_round_trip() {
        let (mut e, bg) = ready();
        let before = e.state_hash();
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        let receipt = e
            .apply_json(&env(
                1,
                "adjustment.add",
                Some(bg),
                json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                    "newOpId": no.to_string(), "params": {}}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![nl, no]);
        let oid = OpId::from_uuid(no);
        assert_eq!(e.document().unwrap().op_count(), 1);
        assert!(
            e.document()
                .unwrap()
                .layer(LayerId::from_uuid(nl))
                .unwrap()
                .adjustment_op()
                == Some(oid)
        );
        let with_adj = e.state_hash();
        assert_ne!(before, with_adj, "ops fold into state_hash when present");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "AddOp + AddNode reverse");
        assert_eq!(e.document().unwrap().op_count(), 0);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), with_adj);

        // toggle: §12.4 bit-identity is structural — hash changes (enabled
        // folds) and undo restores exactly.
        e.apply_json(&env(
            2,
            "op.toggle",
            None,
            json!({"opId": no.to_string(), "enabled": false}),
        ))
        .unwrap();
        assert!(!e.document().unwrap().op(oid).unwrap().enabled());
        e.undo().unwrap();
        assert_eq!(e.state_hash(), with_adj);
        e.redo().unwrap();

        // set_params validates against the registry.
        let err = e
            .apply_json(&env(
                3,
                "op.set_params",
                None,
                json!({"opId": no.to_string(), "params": {"bogusField": 1}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "registry validation refuses"
        );
        // Contract schema carries only {opId, params} (fade fields ride
        // §2.6 mutation notes but not the published wire — engine keeps
        // the block door ready); same-params set is the §40.1 no-op push.
        e.apply_json(&env(
            4,
            "op.set_params",
            None,
            json!({"opId": no.to_string(), "params": {}}),
        ))
        .unwrap();

        // set_mask from:selection (engine-derived id), then from:none.
        {
            let doc = e.doc_mut_for_test();
            let sel = doc.selection_mut();
            sel.mask_mut()
                .fill_rect(IRect::new(0, 0, 8, 8), [1.0, 0.0, 0.0, 0.0]);
            sel.set_bounds(IRect::new(0, 0, 8, 8));
        }
        e.apply_json(&env(
            5,
            "op.set_mask",
            None,
            json!({"opId": no.to_string(), "from": "selection"}),
        ))
        .unwrap();
        let mid = e
            .document()
            .unwrap()
            .op(oid)
            .unwrap()
            .mask()
            .expect("mask set");
        assert_eq!(
            e.document().unwrap().mask(mid).unwrap().coverage().bounds(),
            IRect::new(0, 0, 8, 8)
        );
        let masked = e.state_hash();
        e.apply_json(&env(
            6,
            "op.set_mask",
            None,
            json!({"opId": no.to_string(), "from": "none"}),
        ))
        .unwrap();
        assert!(e.document().unwrap().op(oid).unwrap().mask().is_none());
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            masked,
            "removed mask node restores from the stash"
        );
        e.redo().unwrap();

        // delete: coupled — the adjustment layer dies with its op.
        e.apply_json(&env(7, "op.delete", None, json!({"opId": no.to_string()})))
            .unwrap();
        assert_eq!(e.document().unwrap().op_count(), 0);
        assert!(e
            .document()
            .unwrap()
            .layer(LayerId::from_uuid(nl))
            .is_none());
        e.undo().unwrap();
        assert_eq!(
            e.document().unwrap().op_count(),
            1,
            "coupled delete reverses whole"
        );
        e.redo().unwrap();
        // maskId form + unregistered op kind: typed.
        let err = e
            .apply_json(&env(
                8,
                "adjustment.add",
                Some(bg),
                json!({"opKind": "adjustment.curves", "newLayerId": Uuid::new_v4().to_string(),
                    "newOpId": Uuid::new_v4().to_string(), "params": {}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::UnknownOpKind,
            "fix-round M9: unregistered opKind answers the §40.6 contract id"
        );
        // Fix-round m10: op.reorder on an adjustment layer's op — the
        // contract error list is the base set, so bad_target (not
        // target_kind_mismatch). Recreate the op first (env 7 deleted it,
        // then redid the delete).
        e.undo().unwrap();
        let err = e
            .apply_json(&env(
                9,
                "op.reorder",
                None,
                json!({"opId": no.to_string(), "index": 0}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::BadTarget,
            "fix-round m10: an id from the op.reorder contract's error list"
        );
    }

    #[test]
    fn smartobject_family_round_trips() {
        let (mut e, bg) = ready();
        // convert the (white) background raster layer.
        let nl = Uuid::new_v4();
        let receipt = e
            .apply_json(&env(
                20,
                "smartobject.convert",
                None,
                json!({"layerIds": [bg.as_uuid().to_string()], "newLayerId": nl.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![nl]);
        let so = LayerId::from_uuid(nl);
        let d = e.document().unwrap();
        let data = d.layer(so).unwrap().smart_object().expect("converted");
        let SmartSource::Embedded(asset_a) = *data.source();
        assert_eq!(
            data.cached_composite().region_hash(),
            // §12.4 bit-identity: the cache IS the source pixels.
            d.layer(so)
                .unwrap()
                .smart_object()
                .unwrap()
                .cached_composite()
                .region_hash()
        );
        let converted = e.state_hash();
        e.undo().unwrap();
        assert!(
            e.document().unwrap().layer(bg).is_some(),
            "original restores"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), converted);

        // replace with a different asset (this instance only).
        let png2 = {
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[0u8, 0, 255, 255].repeat(4)).unwrap();
            drop(wr);
            out
        };
        let asset_b = e.add_raster_asset(png2).unwrap();
        e.apply_json(&env(
            21,
            "smartobject.replace",
            Some(so),
            json!({"source": {"assetId": asset_b.to_string()}}),
        ))
        .unwrap();
        let d = e.document().unwrap();
        let SmartSource::Embedded(cur) = *d.layer(so).unwrap().smart_object().unwrap().source();
        assert_eq!(cur, asset_b);
        let px = d
            .layer(so)
            .unwrap()
            .smart_object()
            .unwrap()
            .cached_composite()
            .read_pixel_f32(crate::geom::IVec2::new(0, 0));
        assert!(px[2] > 0.9, "cache refreshed from the new source: {px:?}");
        e.undo().unwrap();
        let d = e.document().unwrap();
        let SmartSource::Embedded(back) = *d.layer(so).unwrap().smart_object().unwrap().source();
        assert_eq!(
            back, asset_a,
            "SetSmartSource param inverse + cache tiles restore"
        );
        e.redo().unwrap();

        // edit_contents swaps EVERY instance of assetBefore.
        e.apply_json(&env(
            22,
            "smartobject.edit_contents",
            Some(so),
            json!({"assetBefore": asset_b.to_string(), "assetAfter": asset_a.to_string()}),
        ))
        .unwrap();
        let d = e.document().unwrap();
        let SmartSource::Embedded(cur) = *d.layer(so).unwrap().smart_object().unwrap().source();
        assert_eq!(cur, asset_a);

        // rasterize (class C): kind swaps to Raster; snapshot undo.
        let pre = e.state_hash();
        e.apply_json(&env(23, "smartobject.rasterize", Some(so), json!({})))
            .unwrap();
        let d = e.document().unwrap();
        assert!(d.layer(so).unwrap().pixels().is_some(), "raster now");
        assert!(d.layer(so).unwrap().smart_object().is_none());
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "snapshot inverse");
        e.redo().unwrap();
        // layer.rasterize on a raster = documented no-op push.
        let l = e.journal().len();
        e.apply_json(&env(24, "layer.rasterize", Some(so), json!({})))
            .unwrap();
        assert_eq!(e.journal().len(), l + 1);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "family replays deterministically"
        );
    }

    #[test]
    fn history_daggers() {
        let (mut e, bg) = ready();
        e.apply_json(&env(
            30,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
        ))
        .unwrap();
        let red = e.state_hash();
        let hist_len = e.history().len();
        // R-dagger: journals as a MARKER, NO history entry, redo intact.
        e.apply_json(&env(
            31,
            "history.snapshot",
            None,
            json!({"name": "before blue"}),
        ))
        .unwrap();
        assert_eq!(e.snapshots().count(), 1);
        assert_eq!(e.history().len(), hist_len, "no history entry (†)");
        e.apply_json(&env(
            32,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0, 0, 1, 0]}}),
        ))
        .unwrap();
        let blue = e.state_hash();
        // goto{snapshot}: C restore push, single-step undoable.
        e.apply_json(&env(
            33,
            "history.goto",
            None,
            json!({"snapshot": "before blue"}),
        ))
        .unwrap();
        assert_eq!(e.state_hash(), red, "snapshot restore");
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            blue,
            "restore is single-step undoable (§10.4)"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), red);
        // goto{position}: † cursor run, pushes nothing.
        let jl = e.journal().len();
        e.apply_json(&env(34, "history.goto", None, json!({"position": 1})))
            .unwrap();
        assert_eq!(e.journal().len(), jl, "position form journals nothing (†)");
        // replay determinism: marker + restore replay to the same state.
        let replayed = e.replay_assembled(&e.journal()[..jl]).unwrap();
        assert_eq!(
            replayed.state_hash(),
            red,
            "marker-recreated snapshot resolves at replay"
        );
        // unknown snapshot name typed.
        let err = e
            .apply_json(&env(35, "history.goto", None, json!({"snapshot": "nope"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    /// Fix-round BLOCKER-1 regression (the reviewer's CLI repro in engine
    /// form): `history.goto{position}` must restore the EXACT state at
    /// each undo-stack position — below and above † markers — with
    /// state-hash assertions after every goto. The `history_daggers`
    /// journal-replay test above structurally cannot catch a shifted
    /// journal↔history correspondence; this one pins the states.
    #[test]
    fn history_goto_position_restores_exact_states_across_markers() {
        let (mut e, bg) = ready();
        let h_white = e.state_hash(); // position 1 (doc.new applied)
        e.apply_json(&env(
            40,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
        ))
        .unwrap();
        let h_red = e.state_hash(); // position 2
        e.apply_json(&env(41, "history.snapshot", None, json!({"name": "m1"})))
            .unwrap();
        e.apply_json(&env(
            42,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0, 0, 1, 0]}}),
        ))
        .unwrap();
        let h_blue = e.state_hash(); // position 3
        e.apply_json(&env(43, "history.snapshot", None, json!({"name": "m2"})))
            .unwrap();
        // journal: [doc.new, fill red, M1, fill blue, M2] — stack depth 3;
        // the † markers occupy NO position slot and stay out of history().
        assert_eq!(e.journal().len(), 5);
        assert_eq!(e.history().len(), 3, "markers filtered (§40.1)");

        // The FAILED repro: fill red → snapshot → fill blue →
        // goto{position:2} must be RED (the corrupted correspondence
        // exported white — it undid down to doc.new).
        e.apply_json(&env(44, "history.goto", None, json!({"position": 2})))
            .unwrap();
        assert_eq!(e.state_hash(), h_red, "position 2 = red (below a marker)");

        // Redo back across the marker.
        e.apply_json(&env(45, "history.goto", None, json!({"position": 3})))
            .unwrap();
        assert_eq!(e.state_hash(), h_blue, "position 3 = blue (across M1)");

        // Undo below BOTH markers.
        e.apply_json(&env(46, "history.goto", None, json!({"position": 1})))
            .unwrap();
        assert_eq!(e.state_hash(), h_white, "position 1 = white");

        // And forward again to the top.
        e.apply_json(&env(47, "history.goto", None, json!({"position": 3})))
            .unwrap();
        assert_eq!(e.state_hash(), h_blue);

        // M3: beyond the stack answers out_of_range and moves nothing.
        let err = e
            .apply_json(&env(48, "history.goto", None, json!({"position": 4})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "M3: 4 > undoDepth 3");
        assert_eq!(e.state_hash(), h_blue, "refusal leaves the state alone");

        // The position runs journaled as the runs themselves — nothing
        // appended; full-journal replay still lands on blue with both
        // marker-recreated snapshots resolving.
        assert_eq!(e.journal().len(), 5);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), h_blue);
        assert_eq!(replayed.snapshots().count(), 2);
    }

    /// Truncation-battery scaffold: a solid color fill on `layer`.
    fn fill_env(n: u32, layer: LayerId, comps: [f32; 4]) -> String {
        env(
            n,
            "fill",
            Some(layer),
            json!({"content": "color", "color": {"model": "rgb", "comps": comps}}),
        )
    }

    /// Fix-round truncation MAJOR (the reviewer's CLI repro in engine
    /// form): `fill red → history.snapshot "s" → goto{position:1} → fill
    /// green` truncates the marker out of the journal — the session
    /// snapshot must be PRUNED with it, so the in-session
    /// `goto{snapshot:"s"}` answers `error.command.bad_target` exactly as
    /// a fresh replay would (D-48 in-session ≡ replay). The FAILED run
    /// kept the stale entry, the goto succeeded and journaled, and the
    /// saved package permanently refused replay-as-recovery
    /// (`appname undo` → bad_target at journal entry 2).
    #[test]
    fn truncated_marker_prunes_snapshot_and_saved_package_replays() {
        let (mut e, bg) = ready();
        let h_white = e.state_hash();
        e.apply_json(&fill_env(70, bg, [1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        e.apply_json(&env(71, "history.snapshot", None, json!({"name": "s"})))
            .unwrap();
        assert_eq!(e.snapshots().count(), 1);
        e.apply_json(&env(72, "history.goto", None, json!({"position": 1})))
            .unwrap();
        assert_eq!(e.state_hash(), h_white);
        // The truncating apply: drops fill red AND the "s" marker.
        e.apply_json(&fill_env(73, bg, [0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        let h_green = e.state_hash();
        assert_eq!(e.journal().len(), 2, "journal = [doc.new, fill green]");
        assert_eq!(
            e.snapshots().count(),
            0,
            "the pruned marker takes its session snapshot with it"
        );
        // In-session goto answers what a fresh replay would.
        let err = e
            .apply_json(&env(74, "history.goto", None, json!({"snapshot": "s"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert_eq!(e.state_hash(), h_green, "refusal leaves the state alone");
        assert_eq!(e.journal().len(), 2, "refusal journals nothing");
        // The saved package replays (the FAILED run refused here forever).
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&assembly(), &bytes)
            .unwrap()
            .engine;
        let mut replayed = loaded.replay_assembled(loaded.journal()).unwrap();
        assert_eq!(
            replayed.state_hash_persistent(),
            loaded.state_hash_persistent(),
            "replay-as-recovery gate holds"
        );
        // Cross-boundary undo works again: green → white.
        replayed.undo().unwrap();
        assert_eq!(replayed.state_hash(), h_white);
    }

    /// Truncation dropping MULTIPLE markers prunes every snapshot whose
    /// marker fell inside the range.
    #[test]
    fn truncation_dropping_multiple_markers_prunes_all_their_snapshots() {
        let (mut e, bg) = ready();
        e.apply_json(&fill_env(75, bg, [1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        e.apply_json(&env(76, "history.snapshot", None, json!({"name": "a"})))
            .unwrap();
        e.apply_json(&env(77, "history.snapshot", None, json!({"name": "b"})))
            .unwrap();
        assert_eq!(e.snapshots().count(), 2);
        e.apply_json(&env(78, "history.goto", None, json!({"position": 1})))
            .unwrap();
        e.apply_json(&fill_env(79, bg, [0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(e.snapshots().count(), 0, "both markers truncated");
        for (n, name) in [(80, "a"), (81, "b")] {
            let err = e
                .apply_json(&env(n, "history.goto", None, json!({"snapshot": name})))
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::BadTarget, "stale {name:?} refused");
        }
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        assert_eq!(replayed.snapshots().count(), 0);
    }

    /// Truncation dropping one marker while an EARLIER snapshot's marker
    /// survives below the cursor: only the truncated one is pruned — the
    /// surviving snapshot still resolves and restores.
    #[test]
    fn truncation_spares_snapshot_whose_marker_survives() {
        let (mut e, bg) = ready();
        e.apply_json(&fill_env(82, bg, [1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        let h_red = e.state_hash();
        e.apply_json(&env(83, "history.snapshot", None, json!({"name": "keep"})))
            .unwrap();
        e.apply_json(&fill_env(84, bg, [0.0, 0.0, 1.0, 0.0]))
            .unwrap();
        e.apply_json(&env(85, "history.snapshot", None, json!({"name": "drop"})))
            .unwrap();
        // Rewind to red: "keep"'s marker stays below the cursor, "drop"'s
        // sits on the undone tail.
        e.apply_json(&env(86, "history.goto", None, json!({"position": 2})))
            .unwrap();
        assert_eq!(e.state_hash(), h_red);
        e.apply_json(&fill_env(87, bg, [0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(
            e.snapshots().collect::<Vec<_>>(),
            vec!["keep"],
            "only the truncated marker's snapshot is pruned"
        );
        let err = e
            .apply_json(&env(88, "history.goto", None, json!({"snapshot": "drop"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        e.apply_json(&env(89, "history.goto", None, json!({"snapshot": "keep"})))
            .unwrap();
        assert_eq!(e.state_hash(), h_red, "surviving snapshot restores");
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        assert_eq!(replayed.snapshots().count(), 1);
    }

    /// The adjacent interleaving with NO truncating apply: after a plain
    /// undo the marker sits on the undone tail — `goto{snapshot}` must
    /// refuse (committing it would truncate its own marker, journaling an
    /// envelope no fresh replay could satisfy: the same defect through the
    /// goto door). The refusal moves nothing; redo past the marker makes
    /// the snapshot reachable again.
    #[test]
    fn snapshot_with_marker_on_undone_tail_refuses_until_redo() {
        let (mut e, bg) = ready();
        let h_white = e.state_hash();
        e.apply_json(&fill_env(90, bg, [1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        let h_red = e.state_hash();
        e.apply_json(&env(91, "history.snapshot", None, json!({"name": "s"})))
            .unwrap();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), h_white);
        let jl = e.journal().len();
        let err = e
            .apply_json(&env(92, "history.goto", None, json!({"snapshot": "s"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget, "marker on the undone tail");
        assert_eq!(e.state_hash(), h_white, "refusal moves nothing");
        assert_eq!(e.journal().len(), jl, "refusal journals nothing");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), h_red);
        e.apply_json(&env(93, "history.goto", None, json!({"snapshot": "s"})))
            .unwrap();
        assert_eq!(e.state_hash(), h_red, "reachable again past the marker");
        // Ruling a′ boundary (residual-MAJOR round): a TIP-captured
        // snapshot keeps FULL goto behavior through further undo/redo
        // cycles — the a′ guard bites only mid-stack captures.
        e.apply_json(&fill_env(94, bg, [0.0, 0.0, 1.0, 0.0]))
            .unwrap();
        let h_blue = e.state_hash();
        e.apply_json(&env(95, "history.goto", None, json!({"snapshot": "s"})))
            .unwrap();
        assert_eq!(
            e.state_hash(),
            h_red,
            "tip capture restores after later edits"
        );
        e.undo().unwrap();
        assert_eq!(e.state_hash(), h_blue, "restore stays single-step undoable");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), h_red);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Re-review residual MAJOR (the reviewer's CLI repro in engine form),
    /// ruling a′ — the narrow goto-door guard: `fill red →
    /// goto{position:1} → history.snapshot "s"` captures WHITE (cursor
    /// state) but its † marker journals at the TIP, above the redo tail;
    /// after `goto{position:2}` absorbs the marker, a fresh replay would
    /// recreate "s" from RED. The FAILED run let `goto{snapshot:"s"}`
    /// restore white and journal it — the persisted package then replayed
    /// red at the restore, and the recovery gate refused the package
    /// forever (state-hash mismatch). Now the goto door answers typed
    /// `error.command.bad_target`, journals nothing, and the saved
    /// package replays clean.
    #[test]
    fn mid_stack_captured_snapshot_refuses_goto_and_package_replays() {
        let (mut e, bg) = ready();
        e.apply_json(&fill_env(100, bg, [1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        let h_red = e.state_hash();
        e.apply_json(&env(101, "history.goto", None, json!({"position": 1})))
            .unwrap();
        let h_white = e.state_hash();
        // Mid-stack capture (cursor below the tip): LEGAL per §10.4
        // snapshot-anytime — the capture is not the defect, the door is.
        e.apply_json(&env(102, "history.snapshot", None, json!({"name": "s"})))
            .unwrap();
        assert_eq!(e.snapshots().count(), 1, "mid-stack capture stays legal");
        // Redo absorbs the marker into the applied range.
        e.apply_json(&env(103, "history.goto", None, json!({"position": 2})))
            .unwrap();
        assert_eq!(e.state_hash(), h_red);
        let jl = e.journal().len();
        let err = e
            .apply_json(&env(104, "history.goto", None, json!({"snapshot": "s"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget, "ruling a′ goto-door guard");
        assert_eq!(e.state_hash(), h_red, "refusal moves nothing");
        assert_eq!(e.journal().len(), jl, "refusal journals nothing");
        // The saved package replays clean (the FAILED run refused here
        // forever with the state-hash mismatch).
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&assembly(), &bytes)
            .unwrap()
            .engine;
        let mut replayed = loaded.replay_assembled(loaded.journal()).unwrap();
        assert_eq!(
            replayed.state_hash_persistent(),
            loaded.state_hash_persistent(),
            "replay-as-recovery gate holds"
        );
        // Cross-boundary undo works: red → white.
        replayed.undo().unwrap();
        assert_eq!(replayed.state_hash(), h_white);
    }

    /// The a′ asymmetry the reviewer verified: a mid-stack snapshot that
    /// is NEVER targeted is benign — its marker recreates a (tip-state)
    /// snapshot at replay that nothing uses, so save → reopen → replay
    /// stays clean.
    #[test]
    fn untargeted_mid_stack_marker_is_benign_at_replay() {
        let (mut e, bg) = ready();
        e.apply_json(&fill_env(110, bg, [1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        e.apply_json(&env(111, "history.goto", None, json!({"position": 1})))
            .unwrap();
        e.apply_json(&env(112, "history.snapshot", None, json!({"name": "m"})))
            .unwrap();
        e.apply_json(&env(113, "history.goto", None, json!({"position": 2})))
            .unwrap();
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&assembly(), &bytes)
            .unwrap()
            .engine;
        let replayed = loaded.replay_assembled(loaded.journal()).unwrap();
        assert_eq!(
            replayed.state_hash_persistent(),
            loaded.state_hash_persistent(),
            "benign-marker: untargeted mid-stack snapshot replays clean"
        );
        assert_eq!(
            replayed.snapshots().count(),
            1,
            "the marker still recreates a snapshot at replay"
        );
    }

    /// Fix-round BLOCKER-1 (reopened-package variant) + M3: on a reopened
    /// package the in-process history is empty (§10.4), so every position
    /// but the current one is in-range-but-unreachable —
    /// `error.schema.out_of_range` (the FAILED run aborted with
    /// `error.history.nothing_to_undo`).
    #[test]
    fn history_goto_position_on_reopened_package_is_typed_out_of_range() {
        let (mut e, bg) = ready();
        e.apply_json(&env(
            50,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
        ))
        .unwrap();
        e.apply_json(&env(51, "history.snapshot", None, json!({"name": "m"})))
            .unwrap();
        e.apply_json(&env(
            52,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0, 0, 1, 0]}}),
        ))
        .unwrap();
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(e.assembly(), &bytes).unwrap();
        let mut r = loaded.engine;
        let live = r.state_hash_persistent();

        let err = r
            .apply_json(&env(53, "history.goto", None, json!({"position": 2})))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "below the live-state floor: out_of_range, not a nothing_to_undo abort"
        );
        assert_eq!(r.state_hash_persistent(), live, "state untouched");

        // The current position (3 = doc.new + two fills; the marker holds
        // no slot) is reachable as a no-op accept.
        r.apply_json(&env(54, "history.goto", None, json!({"position": 3})))
            .unwrap();
        assert_eq!(r.state_hash_persistent(), live);

        // Beyond the stack: the old clamp silently accepted this (M3).
        let err = r
            .apply_json(&env(55, "history.goto", None, json!({"position": 9})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    /// Fix-round M4 (top-level oneOf at stage 5) + M7 (scope) + m13
    /// (default snapshot name = registered MessageId in key-echo form).
    #[test]
    fn dagger_oneof_scope_and_default_name_are_contract_typed() {
        let (mut e, _bg) = ready();
        // history.goto{position,snapshot} both → missing_field (pinned).
        let err = e
            .apply_json(&env(
                60,
                "history.goto",
                None,
                json!({"position": 1, "snapshot": "x"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "M4: both branches");
        // Neither → missing_field (pinned).
        let err = e
            .apply_json(&env(61, "history.goto", None, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "M4: neither branch");
        // op.set_mask{maskId,from} both → missing_field at stage 5
        // (pinned; the old answer was not_implemented from stage 6).
        let err = e
            .apply_json(&env(
                62,
                "op.set_mask",
                None,
                json!({"opId": Uuid::new_v4().to_string(),
                        "maskId": Uuid::new_v4().to_string(), "from": "none"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "M4: set_mask oneOf");
        // M7: scope:"merged" is a typed refusal, never silently ignored.
        let err = e
            .apply_json(&env(
                63,
                "history.snapshot",
                None,
                json!({"name": "s", "scope": "merged"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented, "M7: merged scope");
        assert_eq!(e.snapshots().count(), 0, "refusal captures nothing");
        // m13: the default name is the contract's registered MessageId
        // (panel.history.snapshot.new) + args in the house key-echo form.
        e.apply_json(&env(64, "history.snapshot", None, json!({})))
            .unwrap();
        assert_eq!(e.snapshots().next(), Some("panel.history.snapshot.new n=1"));
    }

    /// Fix-round M6: the content-rewriting family blocks on a PIXELS lock
    /// (contracts: smartobject.edit_contents/replace/rasterize +
    /// layer.rasterize — the layer.rasterize contract's `locked` fixture
    /// pins the PIXELS-on-node case).
    #[test]
    fn pixels_lock_blocks_the_content_rewriting_family() {
        let (mut e, bg) = ready();
        let nl = Uuid::new_v4();
        e.apply_json(&env(
            70,
            "smartobject.convert",
            None,
            json!({"layerIds": [bg.as_uuid().to_string()], "newLayerId": nl.to_string()}),
        ))
        .unwrap();
        let so = LayerId::from_uuid(nl);
        e.apply_json(&env(
            71,
            "layer.set_props",
            Some(so),
            json!({"locks": {"pixels": true}}),
        ))
        .unwrap();
        for (n, kind, params) in [
            (
                72,
                "smartobject.edit_contents",
                json!({"assetBefore": Uuid::new_v4().to_string(),
                        "assetAfter": Uuid::new_v4().to_string()}),
            ),
            (
                73,
                "smartobject.replace",
                json!({"source": {"assetId": Uuid::new_v4().to_string()}}),
            ),
            (74, "smartobject.rasterize", json!({})),
            (75, "layer.rasterize", json!({})),
        ] {
            let err = e.apply_json(&env(n, kind, Some(so), params)).unwrap_err();
            assert_eq!(err.code, ErrorCode::Locked, "{kind}: PIXELS blocks (§40.7)");
        }
        // Unlock: rasterize goes through again.
        e.apply_json(&env(
            76,
            "layer.set_props",
            Some(so),
            json!({"locks": {"pixels": false}}),
        ))
        .unwrap();
        e.apply_json(&env(77, "smartobject.rasterize", Some(so), json!({})))
            .unwrap();
    }

    /// Fix-round M7 (clipToBelow lands on the node) + m14 (no dead `name`
    /// read — the contract declares no such param; the layer names after
    /// its opKind).
    #[test]
    fn adjustment_add_applies_clip_to_below_and_names_after_op_kind() {
        let (mut e, bg) = ready();
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            80,
            "adjustment.add",
            Some(bg),
            json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                    "newOpId": no.to_string(), "params": {}, "clipToBelow": true}),
        ))
        .unwrap();
        let node = e.document().unwrap().layer(LayerId::from_uuid(nl)).unwrap();
        assert!(node.clip_to_below(), "M7: clipToBelow applied");
        assert_eq!(node.name(), "adjustment.invert", "m14: named after opKind");
        e.undo().unwrap();
        e.redo().unwrap();
        assert!(
            e.document()
                .unwrap()
                .layer(LayerId::from_uuid(nl))
                .unwrap()
                .clip_to_below(),
            "prop survives the round trip"
        );
        // Default stays false when omitted.
        let (nl2, no2) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            81,
            "adjustment.add",
            Some(bg),
            json!({"opKind": "adjustment.invert", "newLayerId": nl2.to_string(),
                    "newOpId": no2.to_string(), "params": {}}),
        ))
        .unwrap();
        assert!(!e
            .document()
            .unwrap()
            .layer(LayerId::from_uuid(nl2))
            .unwrap()
            .clip_to_below());
    }

    /// Fix-round m12: `op.set_params` on an op whose kind is absent from
    /// the assembly registry refuses typed — it must never skip nested
    /// validation.
    /// RULING 4 (operator, 2026-08-28): `op.set_params` carries NO top-level fade
    /// fields. Two branches once read `blend`/`opacity` off the ENVELOPE object, which
    /// op.set_params.yaml:25 declares `additionalProperties: false` — so they were
    /// UNREACHABLE and were deleted.
    ///
    /// ★ WHAT THIS TEST GUARDS, STATED EXACTLY BECAUSE I FIRST CLAIMED MORE: IT GUARDS
    /// THE CONTRACT, NOT THE CODE. Verified by mutation — re-adding a `blend` branch to
    /// the handler leaves this test GREEN, because stage 3 rejects the key before the
    /// handler runs. So it CANNOT detect the dead code returning. What it DOES detect is
    /// the precondition failing: if someone opens `additionalProperties`, this goes red,
    /// and only then would a re-added branch become live.
    ///
    /// THE CONTROL IS A DIFFERENT ERROR CODE, NOT A SUCCESS. This module's assembly
    /// registers NO ops, so nothing here can succeed — which is the stronger
    /// discriminator: STAGE-3 VALIDATION RUNS BEFORE TARGET RESOLUTION, so two envelopes
    /// identical but for one key refuse for DIFFERENT reasons:
    ///   without `blend` -> BadTarget  (reached resolution, failed there)
    ///   with    `blend` -> OutOfRange (`error.schema.out_of_range`; the ENUM VARIANT,
    ///                     not the spec-side name — I wrote `Schema` first and the
    ///                     compiler refused it: cite a check by its IMPLEMENTATION)
    /// A single refusal proves nothing; THE PAIR proves the schema arm fires, and FIRST.
    #[test]
    fn op_set_params_has_no_top_level_fade_fields() {
        let (mut e, _bg) = ready();
        let oid = Uuid::parse_str("0b7e0000-0000-4000-8000-00000000d001").unwrap();
        let base = json!({"opId": oid.to_string(), "params": {"anything": 1}});

        let control = e
            .apply_json(&env(93, "op.set_params", None, base.clone()))
            .unwrap_err();
        assert_eq!(
            control.code,
            ErrorCode::BadTarget,
            "CONTROL: without a fade field this envelope REACHES target resolution and \
             fails there -- which is what makes the probe's different code mean \
             something: {control:?}"
        );

        let mut probed = base;
        probed["blend"] = json!("multiply");
        let err = e
            .apply_json(&env(94, "op.set_params", None, probed))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "ruling 4: a top-level fade field is a STAGE-3 SCHEMA violation refused \
             BEFORE target resolution -- not bad_target, never an accepted field: {err:?}"
        );
    }

    #[test]
    fn op_set_params_refuses_unregistered_op_kind_instead_of_skipping() {
        let (mut e, _bg) = ready();
        let oid = Uuid::parse_str("0b7e0000-0000-4000-8000-00000000c001").unwrap();
        let lid = Uuid::parse_str("0b7e0000-0000-4000-8000-00000000c002").unwrap();
        {
            // Crate-door fixture: an op kind this assembly does not carry.
            let doc = e.doc_mut_for_test();
            let root = doc.root();
            let n = doc.children_of(root).unwrap().len();
            doc.insert_op(OpNode::new(
                OpId::from_uuid(oid),
                "adjustment.curves",
                json!({"channels": {"master": {"points": [[0, 0], [1, 1]]}}}),
            ))
            .unwrap();
            doc.insert_child(
                root,
                n,
                LayerNode::new_adjustment(LayerId::from_uuid(lid), "Curves", OpId::from_uuid(oid)),
            )
            .unwrap();
        }
        let err = e
            .apply_json(&env(
                90,
                "op.set_params",
                None,
                json!({"opId": oid.to_string(), "params": {"anything": 1}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::BadTarget,
            "m12: typed refusal naming the registry, never a silent skip"
        );
    }

    /// The `insert_smart_filter` door (O-TOOLS ask 2): arena insert +
    /// chain splice in ONE DeltaSet, §40.6 index clamp, undo/redo
    /// exact-inverse, replay-deterministic, typed refusal off-target —
    /// exercised through a probe KindImpl carrying `filter.smart_add`'s
    /// registered name/params (the production impl is O-TOOLS'; its tail
    /// flips onto this door).
    #[test]
    fn insert_smart_filter_door_round_trips() {
        struct SmartAddProbe;
        impl KindImpl for SmartAddProbe {
            fn kind(&self) -> &'static str {
                "filter.smart_add"
            }
            fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
                let new_op = cx
                    .params()
                    .get("newOpId")
                    .and_then(Value::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| EngineError::missing("newOpId"))?;
                let filter = cx
                    .params()
                    .get("filterId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| EngineError::missing("filterId"))?
                    .to_string();
                let params = cx.params().get("params").cloned().unwrap_or(json!({}));
                let layer = cx
                    .envelope()
                    .target
                    .as_ref()
                    .and_then(|t| t.layer_id)
                    .map(LayerId::from_uuid)
                    .ok_or_else(|| EngineError::bad_target("requires target.layerId"))?;
                let index = cx
                    .params()
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i as usize)
                    .unwrap_or(usize::MAX); // default len(chain): clamps
                let id = cx.insert_smart_filter(
                    layer,
                    index,
                    OpNode::new(OpId::from_uuid(new_op), filter, params),
                )?;
                Ok(KindOutcome {
                    created_ids: vec![id.as_uuid()],
                })
            }
        }

        let mut e = assembly().with_kind(Arc::new(SmartAddProbe)).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];

        // Off-target: a raster layer refuses typed, nothing recorded.
        let op_a = Uuid::new_v4();
        let err = e
            .apply_json(&env(
                40,
                "filter.smart_add",
                Some(bg),
                json!({"filterId": "adjustment.invert", "params": {}, "newOpId": op_a.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);

        let nl = Uuid::new_v4();
        e.apply_json(&env(
            41,
            "smartobject.convert",
            None,
            json!({"layerIds": [bg.as_uuid().to_string()], "newLayerId": nl.to_string()}),
        ))
        .unwrap();
        let so = LayerId::from_uuid(nl);
        let pre = e.state_hash();

        // Append (index defaults to len via clamp).
        let receipt = e
            .apply_json(&env(
                42,
                "filter.smart_add",
                Some(so),
                json!({"filterId": "adjustment.invert", "params": {}, "newOpId": op_a.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![op_a]);
        // Splice a second entry at the head; out-of-range would clamp.
        let op_b = Uuid::new_v4();
        e.apply_json(&env(
            43,
            "filter.smart_add",
            Some(so),
            json!({"filterId": "adjustment.invert", "params": {}, "newOpId": op_b.to_string(),
                   "index": 0}),
        ))
        .unwrap();
        let chain = |e: &Engine| -> Vec<Uuid> {
            e.document()
                .unwrap()
                .layer(so)
                .unwrap()
                .smart_object()
                .unwrap()
                .smart_filters()
                .iter()
                .map(|o| o.as_uuid())
                .collect()
        };
        assert_eq!(chain(&e), vec![op_b, op_a], "head splice + append order");
        assert!(e.document().unwrap().op(OpId::from_uuid(op_a)).is_some());
        let post = e.state_hash();

        // Undo is exact-inverse: chain ref out FIRST, then the op node.
        e.undo().unwrap();
        assert_eq!(chain(&e), vec![op_a], "AddFilterRef reverted");
        assert!(
            e.document().unwrap().op(OpId::from_uuid(op_b)).is_none(),
            "AddOp reverted (node stashed)"
        );
        e.undo().unwrap();
        assert_eq!(chain(&e), Vec::<Uuid>::new());
        assert_eq!(e.state_hash(), pre, "undo restores byte identity");

        // Redo re-inserts the SAME nodes at the recorded indices.
        e.redo().unwrap();
        e.redo().unwrap();
        assert_eq!(chain(&e), vec![op_b, op_a]);
        assert_eq!(e.state_hash(), post, "redo restores byte identity");

        // Replay determinism (D-48): the journal re-derives the state.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "replay is byte-identical"
        );
    }

    /// §10.3 D-emission pins (the reopened-§12.1 slice, R1-updated):
    /// chain-edit kinds flip from `dirty: []` to real regions the moment
    /// the composite door serves parametric content. ADJUSTMENT ops
    /// (R1 ruling): UNMASKED → full canvas (contract letter), MASKED →
    /// below-stack extent (recorded conservative-superset divergence),
    /// attributed to the adjustment LAYER; smart-chain members dirty the
    /// owning INSTANCE's bounds; disabled-to-disabled edits emit nothing
    /// (§12.4 identity both sides); undo/redo rows echo the receipt
    /// rects (arbitration 2).
    #[test]
    fn chain_edit_kinds_emit_composite_dirty_rects() {
        use crate::engine::DirtyRect;
        let (mut e, bg) = ready();
        let bg_bounds = e
            .document()
            .unwrap()
            .layer(bg)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(bg_bounds, IRect::new(0, 0, 64, 64), "white bg spans canvas");

        // adjustment.add (UNMASKED): D = FULL CANVAS, owner-attributed
        // (R1 ruling — the adjustment.add.yaml contract letter; equal to
        // bg_bounds here only because the white bg spans the canvas).
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        let receipt = e
            .apply_json(&env(
                60,
                "adjustment.add",
                Some(bg),
                json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                       "newOpId": no.to_string(), "params": {}}),
            ))
            .unwrap();
        let want = vec![DirtyRect {
            layer: LayerId::from_uuid(nl),
            rect: e.document().unwrap().geometry().canvas_rect(),
        }];
        assert_eq!(
            receipt.dirty, want,
            "full canvas (unmasked arm, R1), adjustment-attributed"
        );
        assert_eq!(e.undo().unwrap().dirty, want, "undo echoes (arbitration 2)");
        assert_eq!(e.redo().unwrap().dirty, want, "redo echoes");

        // op.toggle off: still a composite change (enabled → disabled).
        let receipt = e
            .apply_json(&env(
                61,
                "op.toggle",
                None,
                json!({"opId": no.to_string(), "enabled": false}),
            ))
            .unwrap();
        assert_eq!(receipt.dirty, want, "disabling an enabled op is a D change");

        // op.set_params on the DISABLED op: identity before and after —
        // no D (§12.4); the edit still journals.
        let receipt = e
            .apply_json(&env(
                62,
                "op.set_params",
                None,
                json!({"opId": no.to_string(), "params": {}}),
            ))
            .unwrap();
        assert_eq!(receipt.dirty, vec![], "disabled-to-disabled emits nothing");

        // op.toggle back on, then op.delete: both emit.
        let receipt = e
            .apply_json(&env(
                63,
                "op.toggle",
                None,
                json!({"opId": no.to_string(), "enabled": true}),
            ))
            .unwrap();
        assert_eq!(receipt.dirty, want);
        let receipt = e
            .apply_json(&env(64, "op.delete", None, json!({"opId": no.to_string()})))
            .unwrap();
        assert_eq!(receipt.dirty, want, "coupled delete dirties the extent");

        // Smart-chain path: convert, splice a chain member through the
        // recorded door (setup via the arena doors), then command-edit it
        // — D over the instance bounds, instance-attributed.
        let snl = Uuid::new_v4();
        e.apply_json(&env(
            65,
            "smartobject.convert",
            None,
            json!({"layerIds": [bg.as_uuid().to_string()], "newLayerId": snl.to_string()}),
        ))
        .unwrap();
        let so = LayerId::from_uuid(snl);
        let coid = OpId::from_uuid(Uuid::new_v4());
        {
            let d = e.doc_mut_for_test();
            d.insert_op(OpNode::new(coid, "adjustment.invert", json!({})))
                .unwrap();
            d.insert_smart_filter_ref(so, coid, 0).unwrap();
        }
        let so_bounds = e
            .document()
            .unwrap()
            .layer(so)
            .unwrap()
            .smart_object()
            .unwrap()
            .cached_composite()
            .bounds();
        let want_so = vec![DirtyRect {
            layer: so,
            rect: so_bounds,
        }];
        let receipt = e
            .apply_json(&env(
                66,
                "op.toggle",
                None,
                json!({"opId": coid.as_uuid().to_string(), "enabled": false}),
            ))
            .unwrap();
        assert_eq!(
            receipt.dirty, want_so,
            "instance bounds, instance-attributed"
        );
    }

    /// R1 ruling pins (O-INT, 2026-08-16 — changed expectations BY
    /// RULING, cite the log addendum): the two adjustment dirty arms,
    /// distinguished on a doc whose content does NOT span the canvas.
    /// (1) The reviewer's probe: a BOTTOM-stack unmasked adjustment
    /// (nothing below) previously emitted EMPTY — it now emits FULL
    /// CANVAS. (2) An unmasked adjustment over 1-tile content emits full
    /// canvas, not the content extent. (3) A MASKED adjustment keeps the
    /// below-stack extent (the recorded conservative-superset
    /// divergence).
    #[test]
    fn adjustment_dirty_full_canvas_unmasked_below_extent_masked() {
        use crate::engine::DirtyRect;
        use crate::geom::IVec2;
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000000000002",
                "kind": "doc.new",
                "params": {"width": 96, "height": 96, "background": "transparent"},
            })
            .to_string(),
        )
        .unwrap();
        let canvas = e.document().unwrap().geometry().canvas_rect();
        let bg = e.document().unwrap().z_order_walk()[0];
        // Small content: an 8×8 white patch at (10,10) — content bounds
        // are far smaller than the canvas (test-door doctoring; the
        // emission math reads live bounds, not deltas).
        {
            let d = e.doc_mut_for_test();
            let buf = d
                .layer_mut(bg)
                .map_err(|err| format!("{err:?}"))
                .unwrap()
                .pixels_mut()
                .unwrap();
            for y in 10..18 {
                for x in 10..18 {
                    buf.write_pixel_f32(IVec2::new(x, y), [1.0, 1.0, 1.0, 1.0]);
                }
            }
            buf.recompute_bounds_pixel_exact();
        }
        let content = IRect::new(10, 10, 18, 18);
        assert_eq!(
            e.document()
                .unwrap()
                .layer(bg)
                .unwrap()
                .pixels()
                .unwrap()
                .bounds(),
            content
        );

        // (1) Reviewer probe: BOTTOM-stack unmasked adjustment → FULL
        // CANVAS (was: empty).
        let (bl, bo) = (Uuid::new_v4(), Uuid::new_v4());
        let root = e.document().unwrap().root();
        let receipt = e
            .apply_json(&env(
                70,
                "adjustment.add",
                None,
                json!({"opKind": "adjustment.invert", "newLayerId": bl.to_string(),
                       "newOpId": bo.to_string(), "params": {},
                       "parent": root.as_uuid().to_string(), "index": 0}),
            ))
            .unwrap();
        assert_eq!(
            receipt.dirty,
            vec![DirtyRect {
                layer: LayerId::from_uuid(bl),
                rect: canvas
            }],
            "R1 probe flip: bottom-stack unmasked adjustment emits full canvas, not empty"
        );

        // (2) Unmasked adjustment ABOVE the small content: full canvas,
        // NOT the content extent (the under-inclusive pre-R1 shape).
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        let receipt = e
            .apply_json(&env(
                71,
                "adjustment.add",
                Some(bg),
                json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                       "newOpId": no.to_string(), "params": {}}),
            ))
            .unwrap();
        let adj = LayerId::from_uuid(nl);
        assert_eq!(
            receipt.dirty,
            vec![DirtyRect {
                layer: adj,
                rect: canvas
            }],
            "unmasked arm: full canvas even over 1-tile content"
        );

        // (3) MASKED arm: install an op mask from a selection, then edit
        // — below-stack extent (the content rect; invert is pointwise).
        {
            let d = e.doc_mut_for_test();
            let sel = d.selection_mut();
            sel.mask_mut()
                .fill_rect(IRect::new(12, 12, 13, 13), [1.0, 0.0, 0.0, 0.0]);
            sel.set_bounds(IRect::new(12, 12, 13, 13));
        }
        let receipt = e
            .apply_json(&env(
                72,
                "op.set_mask",
                None,
                json!({"opId": no.to_string(), "from": "selection"}),
            ))
            .unwrap();
        let want_masked = vec![DirtyRect {
            layer: adj,
            rect: content,
        }];
        assert_eq!(
            receipt.dirty, want_masked,
            "masked arm: below-stack extent (recorded conservative-superset divergence)"
        );
        let receipt = e
            .apply_json(&env(
                73,
                "op.toggle",
                None,
                json!({"opId": no.to_string(), "enabled": false}),
            ))
            .unwrap();
        assert_eq!(
            receipt.dirty, want_masked,
            "masked edit keeps the extent arm"
        );
        // Back to UNMASKED (clear the mask): the arm flips back to full
        // canvas on the next composite-changing edit.
        e.apply_json(&env(
            74,
            "op.toggle",
            None,
            json!({"opId": no.to_string(), "enabled": true}),
        ))
        .unwrap();
        let receipt = e
            .apply_json(&env(
                75,
                "op.set_mask",
                None,
                json!({"opId": no.to_string(), "from": "none"}),
            ))
            .unwrap();
        assert_eq!(
            receipt.dirty,
            vec![DirtyRect {
                layer: adj,
                rect: canvas
            }],
            "clearing the mask re-enters the unmasked full-canvas arm"
        );
    }

    /// CP-1.0 door slice, QA-F12-family closure battery (O-WATCH tracked
    /// pre-tag condition): every arm here pins the CONTRACT LETTER the
    /// finding named, plus a no-mutation check on each reject.
    #[test]
    fn qa_f12_family_contract_letter_closures() {
        // --- Chain prime (SmartAddProbe pattern, §12.2). -------------------
        struct SmartAddProbe;
        impl KindImpl for SmartAddProbe {
            fn kind(&self) -> &'static str {
                "filter.smart_add"
            }
            fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
                let new_op = cx
                    .params()
                    .get("newOpId")
                    .and_then(Value::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| EngineError::missing("newOpId"))?;
                let filter = cx
                    .params()
                    .get("filterId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| EngineError::missing("filterId"))?
                    .to_string();
                let params = cx.params().get("params").cloned().unwrap_or(json!({}));
                let layer = cx
                    .envelope()
                    .target
                    .as_ref()
                    .and_then(|t| t.layer_id)
                    .map(LayerId::from_uuid)
                    .ok_or_else(|| EngineError::bad_target("requires target.layerId"))?;
                let id = cx.insert_smart_filter(
                    layer,
                    usize::MAX, // append (insert positions clamp, §40.6)
                    OpNode::new(OpId::from_uuid(new_op), filter, params),
                )?;
                Ok(KindOutcome {
                    created_ids: vec![id.as_uuid()],
                })
            }
        }

        let mut e = assembly().with_kind(Arc::new(SmartAddProbe)).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];

        // QA-F3: layer.rasterize{what:"effects"} on an already-raster layer
        // with no effects → error.command.target_kind_mismatch (contract
        // row verbatim), NOT a no-op accept.
        let pre = e.state_hash();
        let err = e
            .apply_json(&env(
                60,
                "layer.rasterize",
                Some(bg),
                json!({"what": "effects"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch, "QA-F3");
        assert_eq!(e.state_hash(), pre, "QA-F3 reject mutated nothing");
        // …while what:"content" on a raster stays the documented §12.4
        // no-op push.
        e.apply_json(&env(
            61,
            "layer.rasterize",
            Some(bg),
            json!({"what": "content"}),
        ))
        .unwrap();
        e.undo().unwrap();

        // QA-F2: history.snapshot{name:""} → error.schema.out_of_range
        // (contract minLength: 1; stage-5 length row).
        let err = e
            .apply_json(&env(62, "history.snapshot", None, json!({"name": ""})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "QA-F2");

        // QA-F11: §9.6 creation-uuid uniqueness is GLOBAL — a newOpId equal
        // to an existing LAYER id refuses error.schema.duplicate_id.
        let pre = e.state_hash();
        let nl = Uuid::new_v4();
        let err = e
            .apply_json(&env(
                63,
                "adjustment.add",
                Some(bg),
                json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                       "newOpId": bg.as_uuid().to_string(), "params": {}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId, "QA-F11 cross-arena");
        assert_eq!(e.state_hash(), pre, "QA-F11 reject rolled back cleanly");

        // QA-F4: smartobject.convert{layerIds:[]} → error.schema.
        // out_of_range at stage 5 (minItems), BEFORE the arm's v1
        // single-source not_implemented gate.
        let err = e
            .apply_json(&env(
                64,
                "smartobject.convert",
                None,
                json!({"layerIds": []}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "QA-F4");
        assert!(err.message.contains("layerIds"), "QA-F4 names the field");

        // --- Prime a real smart-filter chain for QA-F12/QA-F5. -------------
        let so_id = Uuid::new_v4();
        e.apply_json(&env(
            65,
            "smartobject.convert",
            None,
            json!({"layerIds": [bg.as_uuid().to_string()], "newLayerId": so_id.to_string()}),
        ))
        .unwrap();
        let so = LayerId::from_uuid(so_id);
        let op_a = Uuid::new_v4();
        e.apply_json(&env(
            66,
            "filter.smart_add",
            Some(so),
            json!({"filterId": "adjustment.invert", "params": {}, "newOpId": op_a.to_string()}),
        ))
        .unwrap();

        // QA-F12: op.reorder index beyond the chain end → error.schema.
        // out_of_range (contract: index must lie within 0..len(chain)−1;
        // the former CLAMP-and-accept is the recorded divergence).
        let pre = e.state_hash();
        let err = e
            .apply_json(&env(
                67,
                "op.reorder",
                Some(so),
                json!({"opId": op_a.to_string(), "index": 9999}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "QA-F12");
        assert!(err.message.contains("index"), "QA-F12 names the field");
        assert_eq!(e.state_hash(), pre, "QA-F12 reject mutated nothing");
        // In-range reorder still works (chain of 1: index 0 is the only
        // legal position — a same-position reorder is the §40.1 no-op push).
        e.apply_json(&env(
            68,
            "op.reorder",
            Some(so),
            json!({"opId": op_a.to_string(), "index": 0}),
        ))
        .unwrap();

        // QA-F5: smartobject.replace source oneOf BOTH-set → error.schema.
        // missing_field at stage 5, BEFORE the arm's linked-source
        // not_implemented gate (the contract's source-oneof-both-set row).
        let err = e
            .apply_json(&env(
                69,
                "smartobject.replace",
                Some(so),
                json!({"source": {"assetId": Uuid::new_v4().to_string(),
                                   "path": "/tmp/x.png"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "QA-F5 both-set");
        // …while the single-member linked form still reaches the arm's
        // typed pending answer (the linked-source row is NOT this slice).
        let err = e
            .apply_json(&env(
                70,
                "smartobject.replace",
                Some(so),
                json!({"source": {"path": "/tmp/x.png"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented, "linked row unchanged");
    }

    /// Door-slice escalations 1+2 AS RULED (O-INT, 2026-08-16):
    /// smartobject.convert's creation params to the contract letter.
    /// QA-F6: newLayerId OPTIONAL — absent fills DETERMINISTICALLY from
    /// the envelope id (replay-stable, D-48). QA-F7: newAssetId is
    /// accept-if-equal against the §7.4 content-derived id — mismatch
    /// refuses TYPED (never the silent ignore), equal accepts, absent
    /// binds the content id inherently.
    #[test]
    fn qa_f6_f7_ruled_convert_creation_params() {
        let (mut e, bg) = ready();
        // Prime content on a fresh raster layer.
        let lid = Uuid::new_v4();
        e.apply_json(&env(
            1,
            "layer.add",
            None,
            json!({"kind": "raster", "name": "art", "newLayerId": lid.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            2,
            "fill",
            Some(LayerId::from_uuid(lid)),
            json!({"content": "foreground"}),
        ))
        .unwrap();

        // QA-F6: newLayerId ABSENT → deterministic fill; the receipt
        // carries the derived id; replay reproduces it byte-identically.
        let receipt = e
            .apply_json(&env(
                3,
                "smartobject.convert",
                None,
                json!({"layerIds": [lid.to_string()]}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids.len(), 1, "derived newLayerId surfaces");
        let derived = receipt.created_ids[0];
        let so = LayerId::from_uuid(derived);
        assert!(e.document().unwrap().layer(so).is_some());
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "QA-F6 ruling: envelope-id-derived fill is replay-stable (D-48)"
        );
        // The content-derived asset id (for the accept-if-equal arm).
        let crate::graph::node::SmartSource::Embedded(content_id) = *e
            .document()
            .unwrap()
            .layer(so)
            .unwrap()
            .smart_object()
            .unwrap()
            .source();
        e.undo().unwrap(); // back to the raster layer

        // QA-F7: MISMATCHING newAssetId refuses typed, nothing mutated.
        let pre = e.state_hash();
        let err = e
            .apply_json(&env(
                4,
                "smartobject.convert",
                None,
                json!({"layerIds": [lid.to_string()],
                       "newLayerId": Uuid::new_v4().to_string(),
                       "newAssetId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget, "QA-F7 accept-if-equal");
        assert!(
            err.message.contains("content-derived"),
            "the refusal names content addressing"
        );
        assert_eq!(e.state_hash(), pre, "QA-F7 reject mutated nothing");

        // …and the EQUAL value accepts (same content after the undo).
        let nl2 = Uuid::new_v4();
        e.apply_json(&env(
            5,
            "smartobject.convert",
            None,
            json!({"layerIds": [lid.to_string()], "newLayerId": nl2.to_string(),
                   "newAssetId": content_id.to_string()}),
        ))
        .unwrap();
        assert!(e
            .document()
            .unwrap()
            .layer(LayerId::from_uuid(nl2))
            .is_some());
        let _ = bg;
    }

    /// Add a raster layer with foreground content; returns its id.
    fn primed_raster(e: &mut Engine, n: u32) -> LayerId {
        let lid = Uuid::new_v4();
        e.apply_json(&env(
            n,
            "layer.add",
            None,
            json!({"kind": "raster", "name": "art", "newLayerId": lid.to_string()}),
        ))
        .unwrap();
        let layer = LayerId::from_uuid(lid);
        e.apply_json(&env(
            n + 1,
            "fill",
            Some(layer),
            json!({"content": "foreground"}),
        ))
        .unwrap();
        layer
    }

    /// q-scenario/Q-5 — `smartobject.convert` on a MASKED layer must REFUSE
    /// typed rather than silently drop the mask (the hidden pixels would
    /// become visible with ok:true). Sibling-consistent with the structural
    /// family that preserves masks (layer.group) or refuses over them
    /// (layer.duplicate). The unmasked path is unaffected, and a DISABLED
    /// mask is exempt (a §11.2 no-op — nothing to fall behind).
    #[test]
    fn q5_smartobject_convert_on_masked_layer_refuses() {
        let (mut e, _bg) = ready();
        let layer = primed_raster(&mut e, 1);
        let mask = Uuid::new_v4();
        e.apply_json(&env(
            3,
            "mask.add",
            Some(layer),
            json!({"kind": "raster", "newMaskId": mask.to_string()}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 1);

        // The masked convert REFUSES typed and mutates nothing.
        let pre = e.state_hash();
        let err = e
            .apply_json(&env(
                4,
                "smartobject.convert",
                None,
                json!({"layerIds": [layer.as_uuid().to_string()]}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::NotImplemented,
            "Q-5: masked convert refuses typed"
        );
        assert!(err.message.contains("mask"), "the refusal names the mask");
        assert_eq!(e.state_hash(), pre, "Q-5: the refusal mutated nothing");
        assert!(
            e.document().unwrap().layer(layer).is_some(),
            "source layer intact after refusal"
        );
        assert_eq!(
            e.document().unwrap().mask_count(),
            1,
            "mask intact after refusal"
        );

        // Control 1: an UNMASKED raster layer still converts (the fix is
        // scoped to the masked path — unmasked convert is unaffected).
        let plain = primed_raster(&mut e, 10);
        let receipt = e
            .apply_json(&env(
                12,
                "smartobject.convert",
                None,
                json!({"layerIds": [plain.as_uuid().to_string()]}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids.len(), 1, "unmasked convert serves");
        assert!(e
            .document()
            .unwrap()
            .layer(LayerId::from_uuid(receipt.created_ids[0]))
            .and_then(|n| n.smart_object())
            .is_some());

        // Control 2: DISABLE the mask on the first layer — a disabled mask is
        // a §11.2 no-op, so convert is exempt and succeeds.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-00010000001e",
                "kind": "mask.set_props",
                "target": {"layerId": layer.as_uuid().to_string(), "maskId": mask.to_string()},
                "params": {"enabled": false},
            })
            .to_string(),
        )
        .unwrap();
        let receipt = e
            .apply_json(&env(
                31,
                "smartobject.convert",
                None,
                json!({"layerIds": [layer.as_uuid().to_string()]}),
            ))
            .unwrap();
        assert_eq!(
            receipt.created_ids.len(),
            1,
            "Q-5: a DISABLED mask does not block convert (§11.2 no-op)"
        );
    }
}
