// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `layer.add` / `layer.delete` / `layer.duplicate` /
//! `layer.reorder` / `layer.group` / `layer.ungroup` / `layer.rename` /
//! `layer.set_props` (+ align/distribute/transform in the geometry half of
//! this family, same module); ES §10.3 layer family, §2.6/§2.7 (tree
//! rules), §11.1 (transitive locks — strictest along the ancestry wins),
//! §40.7 family rules. CP-0.5 wave-4 kind tail.
//!
//! v1 scope (typed refusals with reasons): `layer.add` creates `raster`,
//! `group` and — via the §22 param registry
//! ([`crate::engine::fill_reg`]) — `fill` nodes;
//! `adjustment`/`text`/`shape`/`smart_object` still need their
//! §17/§23/§20/§12 param registries and answer `not_implemented` until
//! those rows. Coalescing for the class-M
//! `layer.set_props` awaits the §10.5 merge-key machinery (standing
//! divergence, mask.set_props precedent). Structural kinds produce
//! tree-topic events only — headless receipts carry no T topic (standing
//! divergence); dirty rects derive from tile deltas, so pure-structure
//! steps report none.

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::buffer::Buffer;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::graph::id::LayerId;
use crate::graph::node::{BlendMode, ColorLabel, LayerNode, LockFlags};
use crate::tile::model::ColorModelKind;

pub struct LayerAddKind;
pub struct LayerDeleteKind;
pub struct LayerDuplicateKind;
pub struct LayerReorderKind;
pub struct LayerGroupKind;
pub struct LayerUngroupKind;
pub struct LayerRenameKind;
pub struct LayerSetPropsKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(LayerAddKind),
        Arc::new(LayerDeleteKind),
        Arc::new(LayerDuplicateKind),
        Arc::new(LayerReorderKind),
        Arc::new(LayerGroupKind),
        Arc::new(LayerUngroupKind),
        Arc::new(LayerRenameKind),
        Arc::new(LayerSetPropsKind),
    ]
}

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

fn target_layer(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let id = cx
        .envelope()
        .target
        .as_ref()
        .and_then(|t| t.layer_id)
        .ok_or_else(|| EngineError::bad_target("this kind requires target.layerId (§40.7)"))?;
    let id = LayerId::from_uuid(id);
    if cx.document().layer(id).is_none() || id == cx.document().root() {
        return Err(EngineError::bad_target(format!(
            "target.layerId {id} does not resolve"
        )));
    }
    Ok(id)
}

/// §11.1 transitive rule: ALL anywhere along the ancestry (incl. self)
/// blocks; the strictest lock wins. (Door-slice consolidation: reads the
/// one-place effective set.)
fn ancestry_all_locked(cx: &KindContext<'_>, layer: LayerId) -> bool {
    cx.document()
        .effective_locks(layer)
        .contains(LockFlags::ALL)
}

fn refuse_all_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    if ancestry_all_locked(cx, layer) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "ALL lock along the ancestry blocks structural edits (§11.1 transitive rule)",
        ));
    }
    Ok(())
}

fn param_uuid(cx: &KindContext<'_>, name: &str) -> Result<Uuid, EngineError> {
    cx.params()
        .get(name)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| EngineError::missing(name))
}

fn fresh_layer_id(cx: &KindContext<'_>, field: &str, id: Uuid) -> Result<LayerId, EngineError> {
    let lid = LayerId::from_uuid(id);
    if cx.document().layer(lid).is_some() {
        return Err(EngineError::with_field(
            ErrorCode::DuplicateId,
            field,
            format!("{field} {id} already exists (§9.6: creation UUIDs are fresh)"),
        ));
    }
    Ok(lid)
}

/// Resolve `params.parent` (default root); must be a Group →
/// `target_kind_mismatch` (§40.7).
fn resolve_parent(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let parent = match cx.params().get("parent").and_then(Value::as_str) {
        Some(s) => {
            let id = LayerId::from_uuid(
                Uuid::parse_str(s).map_err(|_| EngineError::range("parent", "uuid"))?,
            );
            if cx.document().layer(id).is_none() {
                return Err(EngineError::bad_target(format!(
                    "parent {id} does not resolve"
                )));
            }
            id
        }
        None => cx.document().root(),
    };
    if !cx
        .document()
        .layer(parent)
        .map(|n| n.is_group())
        .unwrap_or(false)
    {
        return Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "params.parent must resolve to a Group (or the root) — §40.7",
        ));
    }
    Ok(parent)
}

/// `index` with the documented clamp `0..=len` (top default).
fn resolve_index(cx: &KindContext<'_>, parent: LayerId) -> usize {
    let len = cx
        .document()
        .children_of(parent)
        .map(<[_]>::len)
        .unwrap_or(0);
    match cx.params().get("index").and_then(Value::as_u64) {
        Some(i) => (i as usize).min(len),
        None => len,
    }
}

// ---------------------------------------------------------------------------
// layer.add
// ---------------------------------------------------------------------------

impl KindImpl for LayerAddKind {
    fn kind(&self) -> &'static str {
        "layer.add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let kind = cx
            .params()
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("kind"))?;
        let new_id = fresh_layer_id(cx, "newLayerId", param_uuid(cx, "newLayerId")?)?;
        let parent = resolve_parent(cx)?;
        refuse_all_locked(cx, parent)?;
        let index = resolve_index(cx, parent);
        // v1: raster/group take no kindParams; a non-empty block for them
        // is a shape violation at this stage.
        if cx
            .params()
            .get("kindParams")
            .and_then(Value::as_object)
            .map(|m| !m.is_empty())
            == Some(true)
            && matches!(kind, "raster" | "group")
        {
            return Err(EngineError::range(
                "kindParams",
                "raster/group take no kindParams",
            ));
        }
        let node = match kind {
            "raster" => {
                let name = cx
                    .params()
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Layer") // localized default is shell-side (divergence)
                    .to_string();
                let working = cx.document().color().working;
                let depth = cx.document().color().depth;
                LayerNode::new_raster(
                    new_id,
                    name,
                    Buffer::new(depth, ColorModelKind::Rgba, working),
                )
            }
            "group" => {
                let name = cx
                    .params()
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Group")
                    .to_string();
                LayerNode::new_group(new_id, name, false)
            }
            "fill" => {
                // §22 fill-layer param registry (§40.3: kindParams is
                // "re-validated recursively at stage 3 against the kind's
                // registry"). `kindParams` IS the frozen `$defs/fill`
                // object — see the fill_reg module header on that reading.
                let raw = cx
                    .params()
                    .get("kindParams")
                    .cloned()
                    .ok_or_else(|| EngineError::missing("kindParams"))?;
                let content = crate::engine::fill_reg::decode_fill_content(&raw, "kindParams")?;
                let name = cx
                    .params()
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Fill")
                    .to_string();
                LayerNode::new_fill(
                    new_id,
                    name,
                    crate::graph::node::FillLayerData::new(content, raw),
                )
            }
            "adjustment" | "text" | "shape" | "smart_object" => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!(
                        "layer kind \"{kind}\" needs its param registry \
                         (§17 adjustments / §23 text / §20 shapes / §12 smart objects)"
                    ),
                ))
            }
            other => {
                return Err(EngineError::range(
                    "kind",
                    format!("unknown layer kind \"{other}\""),
                ))
            }
        };
        let id = cx.insert_layer(parent, index, node)?;
        // Dirty emission — CONTRACT-MANDATED, the layer.set_props/O-UI
        // Finding-2 precedent: `layer.add.yaml`'s own dirtyRect row is the
        // authority and reads "D = new node bounds; adjustment/fill kinds:
        // downstream ROI (§11.6; unmasked = full canvas, §9.6 worked
        // example)". A freshly added fill layer carries no masks (layer.add
        // takes none), so the row's own stated value for this exact case is
        // the full canvas — no §11.6 ROI machinery is invented here.
        //
        // Structural kinds that DO carry pixels (raster/group) keep the
        // recorder's tile-delta derivation (module header) and are not
        // touched: `composite_affect_bounds` falls through to the canvas
        // only for a pixel-less non-group node, which is what a fill layer
        // is.
        if cx
            .document()
            .layer(id)
            .is_some_and(|n| n.fill_data().is_some())
        {
            let rect = composite_affect_bounds(cx.document(), id);
            cx.note_layer_dirty(id, rect);
        }
        Ok(KindOutcome {
            created_ids: vec![id.as_uuid()],
        })
    }
}

// ---------------------------------------------------------------------------
// layer.delete
// ---------------------------------------------------------------------------

impl KindImpl for LayerDeleteKind {
    fn kind(&self) -> &'static str {
        "layer.delete"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = target_layer(cx)?;
        // ALL on node or ancestry blocks; PIXELS/POSITION do not (§11.1
        // structural rule).
        refuse_all_locked(cx, layer)?;
        // The layer's ops leave with it (q-tutorial/Q-12).
        cx.remove_subtree_and_ops(layer)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// layer.duplicate
// ---------------------------------------------------------------------------

/// Collect the CLOSED identity set of a subtree (v5.1 CMD-001: layer
/// nodes + masks; op-stack entries join with §17).
fn subtree_identities(cx: &KindContext<'_>, root: LayerId) -> Vec<Uuid> {
    let doc = cx.document();
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        out.push(id.as_uuid());
        if let Some(n) = doc.layer(id) {
            for m in n.masks() {
                out.push(m.as_uuid());
            }
            // Door-slice riding item (O-UI filing): the contract's CLOSED
            // identity set is "layer nodes, their masks, their OP-STACK
            // ENTRIES" — an adjustment layer's op, a smart object's
            // smart-filter chain, and each op's own mask (an arena
            // identity that must remap with its op; pinned reading,
            // recorded). Without these the duplicate map was open: clones
            // ALIASED the source's OpIds (§2.7 single-owner violation).
            let ops: Vec<crate::graph::id::OpId> = n
                .adjustment_op()
                .into_iter()
                .chain(
                    n.smart_object()
                        .into_iter()
                        .flat_map(|so| so.smart_filters().iter().copied()),
                )
                .collect();
            for oid in ops {
                out.push(oid.as_uuid());
                if let Some(mid) = doc.op(oid).and_then(|o| o.mask()) {
                    out.push(mid.as_uuid());
                }
            }
            for c in n.children() {
                stack.push(*c);
            }
        }
    }
    out
}

impl KindImpl for LayerDuplicateKind {
    fn kind(&self) -> &'static str {
        "layer.duplicate"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let source = target_layer(cx)?;
        refuse_all_locked(cx, source)?;
        let map = cx
            .params()
            .get("idMap")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing("idMap"))?;
        // Parse the wire map (canonical lowercase uuids both sides).
        let mut id_map: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
        for (k, v) in map {
            let src = Uuid::parse_str(k).map_err(|_| {
                EngineError::range("idMap", format!("source key {k} is not a uuid"))
            })?;
            let dst = v
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::range("idMap", "target values must be uuids"))?;
            if dst.is_nil() {
                return Err(EngineError::range("idMap", "nil target uuid"));
            }
            id_map.insert(src, dst);
        }
        // Completeness over the CLOSED identity set; unknown keys refuse.
        let identities = subtree_identities(cx, source);
        for id in &identities {
            if !id_map.contains_key(id) {
                return Err(EngineError::with_field(
                    ErrorCode::MissingField,
                    "idMap",
                    format!("idMap misses subtree identity {id} (CMD-001 completeness)"),
                ));
            }
        }
        for k in id_map.keys() {
            if !identities.contains(k) {
                return Err(EngineError::range(
                    "idMap",
                    format!("idMap key {k} is not an identity of the source subtree"),
                ));
            }
        }
        // Target freshness + pairwise distinctness.
        let mut seen = std::collections::HashSet::new();
        for dst in id_map.values() {
            if !seen.insert(*dst) {
                return Err(EngineError::with_field(
                    ErrorCode::DuplicateId,
                    "idMap",
                    "idMap target uuids must be pairwise distinct",
                ));
            }
            if cx.document().layer(LayerId::from_uuid(*dst)).is_some()
                || cx
                    .document()
                    .mask(crate::graph::id::MaskId::from_uuid(*dst))
                    .is_some()
            {
                return Err(EngineError::with_field(
                    ErrorCode::DuplicateId,
                    "idMap",
                    format!("idMap target {dst} already exists (§9.6)"),
                ));
            }
        }
        // Deep-copy with remapping (tiles COW-shared through Buffer clones).
        type ClonedOps = Vec<(
            crate::graph::op_node::OpNode,
            Option<crate::graph::mask_node::MaskNode>,
        )>;
        fn clone_subtree(
            cx: &KindContext<'_>,
            id: LayerId,
            id_map: &std::collections::HashMap<Uuid, Uuid>,
            top_name: Option<&str>,
        ) -> (
            LayerNode,
            Vec<(LayerId, crate::graph::mask_node::MaskNode)>,
            ClonedOps,
            Vec<LayerId>,
        ) {
            let doc = cx.document();
            let src = doc.layer(id).expect("identity-checked");
            let new_id = LayerId::from_uuid(id_map[&id.as_uuid()]);
            let mut node = src.clone_shell(new_id);
            if let Some(name) = top_name {
                node.set_name(name);
            }
            let mut masks = Vec::new();
            for m in src.masks() {
                let src_mask = doc.mask(*m).expect("validated tree");
                let new_mask =
                    src_mask.clone_as(crate::graph::id::MaskId::from_uuid(id_map[&m.as_uuid()]));
                masks.push((new_id, new_mask));
            }
            // Fully-closed maps (riding item): op-stack entries clone under
            // their MAPPED ids — an adjustment layer's op and a smart
            // object's chain never alias the source's (§2.7 single owner).
            let mut ops: ClonedOps = Vec::new();
            let mut clone_op = |oid: crate::graph::id::OpId| -> crate::graph::id::OpId {
                let src_op = doc.op(oid).expect("validated tree");
                let new_oid = crate::graph::id::OpId::from_uuid(id_map[&oid.as_uuid()]);
                let mut op_clone = src_op.clone();
                op_clone.id = new_oid;
                let mask_clone = src_op.mask().map(|mid| {
                    doc.mask(mid)
                        .expect("validated tree")
                        .clone_as(crate::graph::id::MaskId::from_uuid(id_map[&mid.as_uuid()]))
                });
                op_clone.mask = None; // re-attached through the recorded door
                ops.push((op_clone, mask_clone));
                new_oid
            };
            match &mut node.kind {
                crate::graph::node::LayerKind::Adjustment(op) => {
                    *op = clone_op(*op);
                }
                crate::graph::node::LayerKind::SmartObject(so) => {
                    let remapped: Vec<crate::graph::id::OpId> =
                        so.smart_filters.iter().map(|o| clone_op(*o)).collect();
                    so.smart_filters = remapped;
                }
                _ => {}
            }
            (node, masks, ops, src.children().to_vec())
        }
        let new_name = cx
            .params()
            .get("newName")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let default_name = format!(
            "{} copy",
            cx.document().layer(source).expect("resolved").name()
        );
        let top_name = new_name.unwrap_or(default_name);
        let insert_above = cx
            .params()
            .get("insertAbove")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let parent = cx.document().parent_of(source).expect("non-root");
        let src_index = cx
            .document()
            .children_of(parent)
            .and_then(|c| c.iter().position(|k| *k == source))
            .expect("parent lists source");
        let at = if insert_above {
            src_index + 1
        } else {
            src_index
        };

        // Iterative insertion: clone top, insert, then children in order.
        // Op-stack entries insert BEFORE their owning layer (the layer's
        // op references must resolve when the node lands, §2.7).
        let mut created = Vec::new();
        let install_ops = |cx: &mut KindContext<'_>,
                           ops: ClonedOps,
                           created: &mut Vec<Uuid>|
         -> Result<(), EngineError> {
            for (op, mask) in ops {
                let oid = op.id();
                cx.insert_op_node(op)?;
                created.push(oid.as_uuid());
                if let Some(m) = mask {
                    let mid = m.id();
                    cx.set_op_mask_with_node(oid, m)?;
                    created.push(mid.as_uuid());
                }
            }
            Ok(())
        };
        let (top, top_masks, top_ops, top_children) =
            clone_subtree(cx, source, &id_map, Some(&top_name));
        install_ops(cx, top_ops, &mut created)?;
        let top_id = cx.insert_layer(parent, at, top)?;
        created.push(top_id.as_uuid());
        for (owner, mask) in top_masks {
            let mid = cx.attach_mask(owner, mask)?;
            created.push(mid.as_uuid());
        }
        let mut queue: Vec<(LayerId, Vec<LayerId>)> = vec![(top_id, top_children)];
        while let Some((new_parent, kids)) = queue.pop() {
            for (i, kid) in kids.iter().enumerate() {
                let (node, masks, kid_ops, grandkids) = clone_subtree(cx, *kid, &id_map, None);
                install_ops(cx, kid_ops, &mut created)?;
                let nid = cx.insert_layer(new_parent, i, node)?;
                created.push(nid.as_uuid());
                for (owner, mask) in masks {
                    let mid = cx.attach_mask(owner, mask)?;
                    created.push(mid.as_uuid());
                }
                queue.push((nid, grandkids));
            }
        }
        Ok(KindOutcome {
            created_ids: created,
        })
    }
}

// ---------------------------------------------------------------------------
// layer.reorder / layer.group / layer.ungroup
// ---------------------------------------------------------------------------

impl KindImpl for LayerReorderKind {
    fn kind(&self) -> &'static str {
        "layer.reorder"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = target_layer(cx)?;
        let parent = LayerId::from_uuid(param_uuid(cx, "parent")?);
        if cx.document().layer(parent).is_none() {
            return Err(EngineError::bad_target(format!(
                "parent {parent} does not resolve"
            )));
        }
        if !cx
            .document()
            .layer(parent)
            .map(|n| n.is_group())
            .unwrap_or(false)
        {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "params.parent must resolve to a Group (or the root) — §40.7",
            ));
        }
        let index = cx
            .params()
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| EngineError::missing("index"))? as usize;
        refuse_all_locked(cx, layer)?;
        refuse_all_locked(cx, parent)?;
        // Cycle: destination inside the moved subtree → bad_target.
        {
            let doc = cx.document();
            let mut probe = Some(parent);
            while let Some(p) = probe {
                if p == layer {
                    return Err(EngineError::bad_target(
                        "destination inside moved subtree (§40.7 cycle rule)",
                    ));
                }
                probe = doc.parent_of(p);
            }
        }
        cx.move_layer(layer, parent, index)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for LayerGroupKind {
    fn kind(&self) -> &'static str {
        "layer.group"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let ids = cx
            .params()
            .get("layerIds")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("layerIds"))?;
        if ids.is_empty() {
            return Err(EngineError::range(
                "layerIds",
                "layerIds must carry at least 1 member",
            ));
        }
        let mut members = Vec::with_capacity(ids.len());
        let mut seen = std::collections::HashSet::new();
        for v in ids {
            let id = v
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::range("layerIds", "members must be uuids"))?;
            if !seen.insert(id) {
                return Err(EngineError::range("layerIds", "members must be unique"));
            }
            let lid = LayerId::from_uuid(id);
            if cx.document().layer(lid).is_none() || lid == cx.document().root() {
                return Err(EngineError::bad_target(format!(
                    "layer {id} does not resolve"
                )));
            }
            members.push(lid);
        }
        // All one parent (§40.7).
        let parent = cx.document().parent_of(members[0]).expect("non-root");
        for m in &members {
            if cx.document().parent_of(*m) != Some(parent) {
                return Err(EngineError::bad_target("layers span parents (§40.7)"));
            }
        }
        refuse_all_locked(cx, parent)?;
        let group_id = fresh_layer_id(cx, "newGroupId", param_uuid(cx, "newGroupId")?)?;
        let name = cx
            .params()
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Group")
            .to_string();
        let isolated = cx
            .params()
            .get("isolated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Insert the group at the topmost member's stacking position.
        let siblings = cx.document().children_of(parent).expect("group").to_vec();
        let top_pos = members
            .iter()
            .filter_map(|m| siblings.iter().position(|s| s == m))
            .max()
            .expect("members are siblings");
        let gid = cx.insert_layer(
            parent,
            top_pos + 1,
            LayerNode::new_group(group_id, name, isolated),
        )?;
        // Move members in (preserve relative z-order bottom→top).
        let ordered: Vec<LayerId> = siblings
            .iter()
            .filter(|s| members.contains(s))
            .copied()
            .collect();
        for (i, m) in ordered.iter().enumerate() {
            cx.move_layer(*m, gid, i)?;
        }
        Ok(KindOutcome {
            created_ids: vec![gid.as_uuid()],
        })
    }
}

impl KindImpl for LayerUngroupKind {
    fn kind(&self) -> &'static str {
        "layer.ungroup"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let group = target_layer(cx)?;
        if !cx
            .document()
            .layer(group)
            .map(|n| n.is_group())
            .unwrap_or(false)
        {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "layer.ungroup targets a Group (§40.7)",
            ));
        }
        refuse_all_locked(cx, group)?;
        let parent = cx.document().parent_of(group).expect("non-root");
        let at = cx
            .document()
            .children_of(parent)
            .and_then(|c| c.iter().position(|k| *k == group))
            .expect("parent lists group");
        let kids = cx.document().children_of(group).expect("group").to_vec();
        // Splice children out at the group's position (order preserved);
        // group masks/effects are discarded by the splice (§40.7 note) —
        // the RemoveSubtree stash still restores them on undo.
        for (i, kid) in kids.iter().enumerate() {
            cx.move_layer(*kid, parent, at + i)?;
        }
        cx.remove_subtree(group)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// layer.rename / layer.set_props
// ---------------------------------------------------------------------------

impl KindImpl for LayerRenameKind {
    fn kind(&self) -> &'static str {
        "layer.rename"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = target_layer(cx)?;
        refuse_all_locked(cx, layer)?;
        let name = cx
            .params()
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("name"))?;
        let n = name.chars().count();
        if n == 0 || n > 255 {
            return Err(EngineError::range(
                "name",
                "name must be 1..=255 Unicode scalar values",
            ));
        }
        let mut after = cx.document().props_block(layer).expect("resolved");
        after.name = name.to_string();
        cx.set_layer_props(layer, after)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for LayerSetPropsKind {
    fn kind(&self) -> &'static str {
        "layer.set_props"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = target_layer(cx)?;
        let before = cx.document().props_block(layer).expect("resolved");
        let mut after = before.clone();
        let p = cx.params();
        // §11.1 escape hatch: ALL blocks UNLESS the param set ⊆ {locks}.
        let keys: Vec<&str> = p.keys().map(String::as_str).collect();
        let only_locks = keys.iter().all(|k| *k == "locks");
        if ancestry_all_locked(cx, layer) && !only_locks {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "ALL lock blocks prop edits except {locks} itself (§11.1 escape hatch)",
            ));
        }
        if let Some(b) = p.get("blend") {
            after.blend = serde_json::from_value::<BlendMode>(b.clone())
                .map_err(|e| EngineError::range("blend", format!("§4.5 wire name: {e}")))?;
        }
        if let Some(v) = p.get("opacity").and_then(Value::as_f64) {
            if !(0.0..=1.0).contains(&v) {
                return Err(EngineError::range("opacity", "0..=1"));
            }
            after.opacity = v as f32;
        }
        if let Some(v) = p.get("fillOpacity").and_then(Value::as_f64) {
            if !(0.0..=1.0).contains(&v) {
                return Err(EngineError::range("fillOpacity", "0..=1"));
            }
            after.fill_opacity = v as f32;
        }
        if let Some(v) = p.get("visible").and_then(Value::as_bool) {
            after.visible = v;
        }
        if let Some(l) = p.get("locks").and_then(Value::as_object) {
            let mut flags = LockFlags::empty();
            for (k, v) in l {
                if v.as_bool() == Some(true) {
                    flags = flags.union(match k.as_str() {
                        "transparency" => LockFlags::TRANSPARENCY,
                        "position" => LockFlags::POSITION,
                        "pixels" => LockFlags::PIXELS,
                        "all" => LockFlags::ALL,
                        other => {
                            return Err(EngineError::range(
                                "locks",
                                format!("unknown lock flag \"{other}\""),
                            ))
                        }
                    });
                }
            }
            after.locks = flags;
        }
        if let Some(v) = p.get("clipToBelow").and_then(Value::as_bool) {
            after.clip_to_below = v;
        }
        if let Some(v) = p.get("isolated").and_then(Value::as_bool) {
            if !cx
                .document()
                .layer(layer)
                .map(|n| n.is_group())
                .unwrap_or(false)
            {
                return Err(EngineError::new(
                    ErrorCode::TargetKindMismatch,
                    "isolated applies to Groups (§4.7)",
                ));
            }
            after.isolated = v;
        }
        if let Some(v) = p.get("colorLabel") {
            after.color_label = match v {
                Value::Null => None,
                other => Some(
                    serde_json::from_value::<ColorLabel>(other.clone())
                        .map_err(|e| EngineError::range("colorLabel", format!("{e}")))?,
                ),
            };
        }
        // Dirty emission — CONTRACT-MANDATED, not an arbitration (the
        // review corrected the first cut's record): the layer.set_props
        // contract's own dirtyRect row is the authority — "D = node
        // full bounds for rendering props (blend, opacity, fillOpacity,
        // visible, clipToBelow, isolated — §3.7 rule); locks/colorLabel
        // emit T only". This arm implements that row's letter (O-UI's
        // Finding-2 probe showed it was previously unimplemented — the
        // gap was in the code, not the spec; the fold-in candidate the
        // first cut minted is WITHDRAWN, rule-9 erratum in the lane
        // log). A §40.1 no-op set emits none (nothing changed).
        // Receipts/events are NOT journal content (§10.6: replay
        // re-executes ENVELOPES), so replay determinism is untouched —
        // asserted in-test.
        let visual_changed = before.blend != after.blend
            || before.opacity != after.opacity
            || before.fill_opacity != after.fill_opacity
            || before.visible != after.visible
            || before.clip_to_below != after.clip_to_below
            || before.isolated != after.isolated;
        cx.set_layer_props(layer, after)?;
        if visual_changed {
            let rect = composite_affect_bounds(cx.document(), layer);
            cx.note_layer_dirty(layer, rect);
        }
        Ok(KindOutcome::default())
    }
}

/// The region a node's VISUAL-prop change can stale (§9.5 emission,
/// §3.7 tracker merges downstream): raster → its content bounds; group
/// → the union of its subtree's regions; every parametric kind
/// (adjustment — which affects the composite BELOW —, text, shape,
/// smart object) → the canvas rect, conservatively. An empty region
/// (e.g. a blank raster layer) emits nothing — nothing presented can
/// have changed.
fn composite_affect_bounds(
    doc: &crate::graph::document::Document,
    layer: LayerId,
) -> crate::geom::IRect {
    let Some(node) = doc.layer(layer) else {
        return crate::geom::IRect::EMPTY;
    };
    if let Some(px) = node.pixels() {
        return px.bounds();
    }
    if node.is_group() {
        let mut out = crate::geom::IRect::EMPTY;
        if let Some(children) = doc.children_of(layer) {
            for child in children.iter().copied() {
                out = out.union(&composite_affect_bounds(doc, child));
            }
        }
        return out;
    }
    doc.geometry().canvas_rect()
}

// The align/distribute/transform (integer-translate) half lives in
// `kinds_layer_geom.rs`; merge/flatten in `kinds_merge.rs`.

// Duplicate-support shells on the graph nodes.
impl LayerNode {
    /// Clone this node under a NEW id with an empty mask list and no
    /// children (the duplicate walk re-attaches remapped copies); pixels
    /// clone COW (Arc-shared tiles).
    pub(crate) fn clone_shell(&self, id: LayerId) -> LayerNode {
        let mut n = self.clone();
        n.id = id;
        n.masks.clear();
        n.children.clear();
        n
    }

    pub(crate) fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }
}

impl crate::graph::mask_node::MaskNode {
    /// Clone under a new id (coverage tiles COW-shared).
    pub(crate) fn clone_as(&self, id: crate::graph::id::MaskId) -> Self {
        let mut m = self.clone();
        m.set_id(id);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;

    use serde_json::json;

    fn assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
            .with_kind(Arc::new(crate::engine::kinds::FillKind))
    }

    fn ready() -> (Engine, LayerId) {
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "1a7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 128, "height": 128, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        (e, bg)
    }

    fn env(n: u32, kind: &str, layer: Option<LayerId>, params: serde_json::Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("1a7e0000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.as_uuid().to_string()});
        }
        v.to_string()
    }

    fn children(e: &Engine) -> Vec<LayerId> {
        let d = e.document().unwrap();
        d.children_of(d.root()).unwrap().to_vec()
    }

    // -- §22 fill-layer param registry -----------------------------------

    fn add_fill(n: u32, kind_params: serde_json::Value) -> String {
        env(
            n,
            "layer.add",
            None,
            json!({
                "kind": "fill",
                "newLayerId": format!("1a7e0000-0000-4000-8000-0002{n:08x}"),
                "kindParams": kind_params,
            }),
        )
    }

    #[test]
    fn fill_layer_add_serves_every_frozen_arm() {
        // The Tier-1 deliverable: `layer.add{kind:"fill"}` used to refuse
        // NotImplemented for want of a param registry. All four arms the
        // frozen `$defs/fill` admits must now land as real nodes.
        let (mut e, _) = ready();
        let arms = [
            json!({"type": "none"}),
            json!({"type": "solid", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
            json!({"type": "pattern", "patternRef": "assets/abc"}),
            json!({
                "type": "gradient",
                "angleDeg": 45, "scale": 2, "reverse": true,
                "gradient": {
                    "kind": "radial", "space": "perceptual", "dither": false,
                    "stops": [
                        {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                        {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                    ]
                }
            }),
        ];
        for (i, arm) in arms.iter().enumerate() {
            let r = e
                .apply_json(&add_fill(100 + i as u32, arm.clone()))
                .unwrap();
            assert_eq!(
                r.created_ids.len(),
                1,
                "arm {i} must create exactly one node"
            );
        }
        let d = e.document().unwrap();
        let kinds: Vec<&str> = children(&e)
            .iter()
            .filter_map(|id| d.layer(*id))
            .map(|n| n.kind().kind_str())
            .collect();
        assert_eq!(
            kinds.iter().filter(|k| **k == "fill").count(),
            4,
            "four fill layers must be in the tree, got {kinds:?}"
        );
    }

    #[test]
    fn fill_layer_keeps_both_the_decoded_content_and_the_wire() {
        let (mut e, _) = ready();
        let arm = json!({
            "type": "gradient",
            "angleDeg": 45, "scale": 2, "reverse": true,
            "futureV56Field": "kept",
            "gradient": {
                "kind": "diamond",
                "stops": [
                    {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}, "midpoint": 0.25},
                    {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}
                ]
            }
        });
        e.apply_json(&add_fill(200, arm.clone())).unwrap();
        let d = e.document().unwrap();
        let id = *children(&e).last().unwrap();
        let data = d.layer(id).unwrap().fill_data().expect("a fill layer");
        // The DECODED half carries the typed params...
        let crate::engine::fill_reg::FillContent::Gradient {
            gradient,
            angle_deg,
            scale,
            reverse,
        } = data.content()
        else {
            panic!("expected a gradient fill")
        };
        assert_eq!((*angle_deg, *scale, *reverse), (45.0, 2.0, true));
        assert_eq!(
            gradient.kind(),
            crate::engine::gradient::GradientKind::Diamond
        );
        assert_eq!(gradient.stops()[0].midpoint, 0.25);
        // ...and the RAW half preserves §7.7 unknown fields verbatim.
        assert_eq!(
            data.wire(),
            &arm,
            "the wire object must round-trip verbatim"
        );
    }

    #[test]
    fn fill_layer_add_refuses_a_bad_registry_payload_typed() {
        let (mut e, _) = ready();
        for (n, arm, want) in [
            (300, json!({"type": "solid"}), "error.schema.missing_field"),
            (301, json!({"type": "plaid"}), "error.schema.out_of_range"),
            (
                302,
                json!({"type": "pattern", "patternRef": "assets/x", "scale": 0}),
                "error.schema.out_of_range",
            ),
            (
                303,
                json!({
                    "type": "gradient",
                    "gradient": {
                        "kind": "linear",
                        "stops": [{"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}}]
                    }
                }),
                "error.schema.out_of_range",
            ),
        ] {
            let e_ = e.apply_json(&add_fill(n, arm.clone())).unwrap_err();
            assert_eq!(e_.code.as_str(), want, "arm {arm} must refuse {want}");
        }
        // A refused add must leave NO node behind (atomicity).
        assert_eq!(children(&e).len(), 1, "only the doc.new background remains");
    }

    #[test]
    fn fill_layer_add_dirties_the_whole_canvas() {
        // `layer.add.yaml` dirtyRect row: "adjustment/fill kinds:
        // downstream ROI (§11.6; unmasked = full canvas)". A fill layer
        // writes no tiles, so the recorder's tile-delta derivation would
        // report NOTHING and the canvas would never repaint — the row has
        // to be implemented explicitly.
        let (mut e, _) = ready();
        let r = e
            .apply_json(&add_fill(
                400,
                json!({"type": "solid", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
            ))
            .unwrap();
        let canvas = e.document().unwrap().geometry().canvas_rect();
        assert_eq!(
            canvas,
            crate::geom::IRect::new(0, 0, 128, 128),
            "ready() doc"
        );
        assert_eq!(
            r.dirty.len(),
            1,
            "exactly one dirty rect, got {:?}",
            r.dirty
        );
        assert_eq!(
            r.dirty[0].rect, canvas,
            "a fill layer stales the whole canvas"
        );
        // A raster add is UNCHANGED by this arm: it carries pixels, so the
        // recorder's own tile-delta derivation still owns its dirty.
        let r2 = e
            .apply_json(&env(
                401,
                "layer.add",
                None,
                json!({"kind": "raster", "newLayerId": "1a7e0000-0000-4000-8000-000200000401"}),
            ))
            .unwrap();
        assert!(
            r2.dirty.is_empty(),
            "an empty raster add stales nothing, got {:?}",
            r2.dirty
        );
    }

    #[test]
    fn fill_layer_add_requires_kind_params() {
        let (mut e, _) = ready();
        let err = e
            .apply_json(&env(
                310,
                "layer.add",
                None,
                json!({
                    "kind": "fill",
                    "newLayerId": "1a7e0000-0000-4000-8000-000200000310",
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code.as_str(), "error.schema.missing_field");
    }

    #[test]
    fn the_other_parametric_kinds_still_refuse() {
        // The fill arm must not have opened the door for kinds whose
        // registries genuinely do not exist yet.
        let (mut e, _) = ready();
        for (n, kind) in [(320, "adjustment"), (321, "text"), (322, "shape")] {
            let err = e
                .apply_json(&env(
                    n,
                    "layer.add",
                    None,
                    json!({
                        "kind": kind,
                        "newLayerId": format!("1a7e0000-0000-4000-8000-0003{n:08x}"),
                        "newOpId": format!("1a7e0000-0000-4000-8000-0004{n:08x}"),
                        "kindParams": {},
                    }),
                ))
                .unwrap_err();
            assert_eq!(
                err.code.as_str(),
                "error.command.not_implemented",
                "layer kind {kind} must still refuse"
            );
        }
    }

    /// §9.5 dirty emission for prop mutations (O-UI Finding-2 fix): the
    /// per-prop dirty/non-dirty table, the undo/redo echo, and the
    /// replay-determinism non-interaction (receipts are not journal
    /// content, §10.6 — the journal replays envelopes only).
    #[test]
    fn set_props_visual_props_dirty_composite_bounds_nonvisual_do_not() {
        let (mut e, bg) = ready();
        let canvas = crate::geom::IRect::new(0, 0, 128, 128);
        // VISUAL props: each emits exactly the layer's composite bounds
        // (white background content = the canvas).
        for (n, params) in [
            (1, json!({"visible": false})),
            (2, json!({"visible": true})),
            (3, json!({"opacity": 0.5})),
            (4, json!({"blend": "multiply"})),
            (5, json!({"fillOpacity": 0.25})),
            (6, json!({"clipToBelow": true})),
        ] {
            let r = e
                .apply_json(&env(700 + n, "layer.set_props", Some(bg), params.clone()))
                .unwrap();
            assert_eq!(
                r.dirty,
                vec![crate::engine::DirtyRect {
                    layer: bg,
                    rect: canvas
                }],
                "visual prop {params} dirties the composite bounds (§9.5)"
            );
        }
        // NON-VISUAL props: locks + colorLabel emit NOTHING (the
        // contract: colorLabel is organizational only).
        for (n, params) in [
            (10, json!({"locks": {"pixels": true}})),
            (11, json!({"colorLabel": "red"})),
        ] {
            let r = e
                .apply_json(&env(700 + n, "layer.set_props", Some(bg), params.clone()))
                .unwrap();
            assert!(
                r.dirty.is_empty(),
                "non-visual prop {params} emits no dirty"
            );
        }
        // §40.1 no-op set (visible already true): no dirty either.
        let r = e
            .apply_json(&env(
                712,
                "layer.set_props",
                Some(bg),
                json!({"visible": true}),
            ))
            .unwrap();
        assert!(r.dirty.is_empty(), "no-op prop set emits no dirty");

        // UNDO/REDO ECHO (arbitration 2): the history row of a visual
        // step echoes the SAME region the apply receipt carried — the
        // O-UI probe pinned the prior empty shape.
        let r = e
            .apply_json(&env(
                713,
                "layer.set_props",
                Some(bg),
                json!({"visible": false}),
            ))
            .unwrap();
        let want = vec![crate::engine::DirtyRect {
            layer: bg,
            rect: canvas,
        }];
        assert_eq!(r.dirty, want);
        e.undo().unwrap();
        e.redo().unwrap();
        let rows = e.history();
        let row = rows.last().expect("history rows");
        assert_eq!(row.kind, "layer.set_props");
        assert_eq!(
            row.dirty, want,
            "undo/redo rows echo the visual-prop region (arbitration 2)"
        );

        // Replay determinism untouched: receipts are not journal content.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// The group arm of the §9.5 emission: a group's visual toggle
    /// dirties the union of its subtree's composite bounds; an empty
    /// group emits nothing.
    #[test]
    fn set_props_group_dirty_is_subtree_union_and_empty_group_is_silent() {
        let (mut e, _bg) = ready();
        // Empty group: isolated toggle → visual, but nothing presented
        // can change → no dirty.
        let g = Uuid::new_v4();
        e.apply_json(&env(
            720,
            "layer.add",
            None,
            json!({"kind": "group", "newLayerId": g.to_string()}),
        ))
        .unwrap();
        let g = LayerId::from_uuid(g);
        let r = e
            .apply_json(&env(
                721,
                "layer.set_props",
                Some(g),
                json!({"isolated": true}),
            ))
            .unwrap();
        assert!(r.dirty.is_empty(), "empty group: nothing to stale");
        // Put a painted raster child inside; the group's visible toggle
        // dirties the CHILD's bounds through the union walk.
        let child = Uuid::new_v4();
        e.apply_json(&env(
            722,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": child.to_string(),
                   "parent": g.as_uuid().to_string()}),
        ))
        .unwrap();
        let child_id = LayerId::from_uuid(child);
        // Paint a PARTIAL region through the test door (fill has no
        // region param; a journaled paint would cover the canvas and be
        // indistinguishable from the conservative rect) — replay is NOT
        // asserted in this test for that reason (the table test above
        // owns the replay leg).
        {
            let doc = e.doc_mut_for_test();
            let buf = doc
                .layer_mut(child_id)
                .unwrap()
                .pixels_mut()
                .expect("raster child");
            for y in 8..24 {
                for x in 8..24 {
                    buf.write_pixel_f32(crate::geom::IVec2::new(x, y), [1.0, 0.0, 0.0, 1.0]);
                }
            }
            buf.set_bounds_recorded(crate::geom::IRect::new(8, 8, 24, 24));
        }
        let child_bounds = e
            .document()
            .unwrap()
            .layer(child_id)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(child_bounds, crate::geom::IRect::new(8, 8, 24, 24));
        let r = e
            .apply_json(&env(
                724,
                "layer.set_props",
                Some(g),
                json!({"visible": false}),
            ))
            .unwrap();
        assert_eq!(
            r.dirty,
            vec![crate::engine::DirtyRect {
                layer: g,
                rect: child_bounds
            }],
            "group visual toggle dirties the subtree union (§9.5)"
        );
    }

    #[test]
    fn add_raster_and_group_round_trip_and_reject_typed() {
        let (mut e, _bg) = ready();
        let before = e.state_hash();
        let nl = Uuid::new_v4();
        let receipt = e
            .apply_json(&env(
                1,
                "layer.add",
                None,
                json!({"kind": "raster", "newLayerId": nl.to_string(), "name": "Sky"}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![nl]);
        assert_eq!(children(&e).len(), 2, "top of root by default");
        assert_eq!(
            e.document()
                .unwrap()
                .layer(LayerId::from_uuid(nl))
                .unwrap()
                .name(),
            "Sky"
        );
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        // group with explicit index 0 (bottom).
        let g = Uuid::new_v4();
        e.apply_json(&env(
            2,
            "layer.add",
            None,
            json!({"kind": "group", "newLayerId": g.to_string(), "index": 0}),
        ))
        .unwrap();
        assert_eq!(children(&e)[0], LayerId::from_uuid(g), "index 0 = bottom");
        // Typed refusals + duplicate + parent mismatch.
        let err = e
            .apply_json(&env(
                3,
                "layer.add",
                None,
                json!({"kind": "text", "newLayerId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        let err = e
            .apply_json(&env(
                4,
                "layer.add",
                None,
                json!({"kind": "raster", "newLayerId": nl.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);
        let err = e
            .apply_json(&env(
                5,
                "layer.add",
                None,
                json!({"kind": "raster", "newLayerId": Uuid::new_v4().to_string(),
                        "parent": nl.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::TargetKindMismatch,
            "raster parent refuses"
        );
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn delete_restores_whole_subtrees_with_pixels_and_masks() {
        let (mut e, bg) = ready();
        // Content on bg + a mask, then group it, then delete the group.
        e.apply_json(&env(
            10,
            "fill",
            Some(bg),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.9, 0.1, 0.2, 0]}}),
        ))
        .unwrap();
        let m = Uuid::new_v4();
        e.apply_json(&env(
            11,
            "mask.add",
            Some(bg),
            json!({"kind": "raster", "newMaskId": m.to_string()}),
        ))
        .unwrap();
        let g = Uuid::new_v4();
        e.apply_json(&env(
            12,
            "layer.group",
            None,
            json!({"layerIds": [bg.as_uuid().to_string()], "newGroupId": g.to_string()}),
        ))
        .unwrap();
        let with_all = e.state_hash();
        e.apply_json(&env(
            13,
            "layer.delete",
            Some(LayerId::from_uuid(g)),
            json!({}),
        ))
        .unwrap();
        assert_eq!(children(&e).len(), 0, "group + contents gone");
        assert_eq!(e.document().unwrap().mask_count(), 0);
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            with_all,
            "same nodes, tiles, and mask re-attach (§2.7)"
        );
        assert_eq!(e.document().unwrap().mask_count(), 1);
        e.redo().unwrap();
        assert_eq!(children(&e).len(), 0);
        // ALL lock along the ancestry blocks.
        e.undo().unwrap();
        e.doc_mut_for_test()
            .layer_mut(LayerId::from_uuid(g))
            .unwrap()
            .locks = LockFlags::ALL;
        let err = e
            .apply_json(&env(14, "layer.delete", Some(bg), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked, "transitive ALL (§11.1)");
    }

    #[test]
    fn duplicate_validates_the_idmap_and_shares_tiles() {
        let (mut e, bg) = ready();
        let m = Uuid::new_v4();
        e.apply_json(&env(
            20,
            "mask.add",
            Some(bg),
            json!({"kind": "raster", "newMaskId": m.to_string()}),
        ))
        .unwrap();
        // Complete map: bg + its mask.
        let (nb, nm) = (Uuid::new_v4(), Uuid::new_v4());
        let receipt = e
            .apply_json(&env(21, "layer.duplicate", Some(bg),
                json!({"idMap": {bg.as_uuid().to_string(): nb.to_string(), m.to_string(): nm.to_string()}})))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![nb, nm]);
        let d = e.document().unwrap();
        let dup = LayerId::from_uuid(nb);
        assert_eq!(d.layer(dup).unwrap().name(), "Background copy");
        assert_eq!(children(&e)[1], dup, "insertAbove default");
        assert_eq!(d.mask_count(), 2);
        // COW: duplicate's pixels read identically.
        assert_eq!(
            d.layer(dup).unwrap().pixels().unwrap().region_hash(),
            d.layer(bg).unwrap().pixels().unwrap().region_hash()
        );
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 1);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);

        // idMap rejects: incomplete, unknown key, collision.
        let err = e
            .apply_json(&env(22, "layer.duplicate", Some(bg), json!({"idMap": {}})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        let ghost = Uuid::new_v4();
        let err = e
            .apply_json(&env(
                23,
                "layer.duplicate",
                Some(bg),
                json!({"idMap": {bg.as_uuid().to_string(): Uuid::new_v4().to_string(),
                                  m.to_string(): Uuid::new_v4().to_string(),
                                  ghost.to_string(): Uuid::new_v4().to_string()}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "unknown source key");
        let err = e
            .apply_json(&env(
                24,
                "layer.duplicate",
                Some(bg),
                json!({"idMap": {bg.as_uuid().to_string(): nb.to_string(),
                                  m.to_string(): Uuid::new_v4().to_string()}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId, "target already exists");
    }

    #[test]
    fn reorder_group_ungroup_round_trip() {
        let (mut e, bg) = ready();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            30,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": a.to_string(), "name": "A"}),
        ))
        .unwrap();
        e.apply_json(&env(
            31,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": b.to_string(), "name": "B"}),
        ))
        .unwrap();
        let (a, b) = (LayerId::from_uuid(a), LayerId::from_uuid(b));
        assert_eq!(children(&e), vec![bg, a, b]);
        // reorder B to bottom (param inverse round trip).
        let root = e.document().unwrap().root();
        let before = e.state_hash();
        e.apply_json(&env(
            32,
            "layer.reorder",
            Some(b),
            json!({"parent": root.as_uuid().to_string(), "index": 0}),
        ))
        .unwrap();
        assert_eq!(children(&e), vec![b, bg, a]);
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "MoveNode param inverse");
        e.redo().unwrap();
        // group A+bg (spanning check ok — same parent), then ungroup.
        let g = Uuid::new_v4();
        e.apply_json(&env(
            33,
            "layer.group",
            None,
            json!({"layerIds": [bg.as_uuid().to_string(), a.as_uuid().to_string()],
                    "newGroupId": g.to_string()}),
        ))
        .unwrap();
        let g = LayerId::from_uuid(g);
        assert_eq!(children(&e), vec![b, g]);
        assert_eq!(
            e.document().unwrap().children_of(g).unwrap(),
            &[bg, a],
            "relative order kept"
        );
        let grouped = e.state_hash();
        e.apply_json(&env(34, "layer.ungroup", Some(g), json!({})))
            .unwrap();
        assert_eq!(
            children(&e),
            vec![b, bg, a],
            "children splice at the group's position"
        );
        e.undo().unwrap();
        assert_eq!(e.state_hash(), grouped);
        e.redo().unwrap();
        // cycle reject: move the (re-created) situation — move b into a raster? target mismatch.
        let err = e
            .apply_json(&env(
                35,
                "layer.reorder",
                Some(b),
                json!({"parent": a.as_uuid().to_string(), "index": 0}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        // group spanning parents reject.
        let g2 = Uuid::new_v4();
        e.apply_json(&env(
            36,
            "layer.group",
            None,
            json!({"layerIds": [a.as_uuid().to_string()], "newGroupId": g2.to_string()}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                37,
                "layer.group",
                None,
                json!({"layerIds": [a.as_uuid().to_string(), b.as_uuid().to_string()],
                        "newGroupId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget, "layers span parents");
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn rename_and_set_props_value_deltas() {
        let (mut e, bg) = ready();
        let before = e.state_hash();
        e.apply_json(&env(40, "layer.rename", Some(bg), json!({"name": "Sky"})))
            .unwrap();
        assert_eq!(e.document().unwrap().layer(bg).unwrap().name(), "Sky");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "SetLayerProps value inverse");
        e.redo().unwrap();
        let err = e
            .apply_json(&env(41, "layer.rename", Some(bg), json!({"name": ""})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // set_props: full-state per present field.
        e.apply_json(&env(
            42,
            "layer.set_props",
            Some(bg),
            json!({"opacity": 0.8, "blend": "multiply", "colorLabel": "red"}),
        ))
        .unwrap();
        let n = e.document().unwrap().props_block(bg).unwrap();
        assert_eq!(
            (n.opacity, n.blend, n.color_label),
            (0.8, BlendMode::Multiply, Some(ColorLabel::Red))
        );
        // colorLabel null clears.
        e.apply_json(&env(
            43,
            "layer.set_props",
            Some(bg),
            json!({"colorLabel": null}),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().props_block(bg).unwrap().color_label,
            None
        );
        // §11.1 escape hatch: under ALL only {locks} passes.
        e.apply_json(&env(
            44,
            "layer.set_props",
            Some(bg),
            json!({"locks": {"all": true}}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                45,
                "layer.set_props",
                Some(bg),
                json!({"opacity": 0.5}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        e.apply_json(&env(46, "layer.set_props", Some(bg), json!({"locks": {}})))
            .unwrap();
        assert_eq!(
            e.document().unwrap().props_block(bg).unwrap().locks,
            LockFlags::empty()
        );
        // range + enum rejects.
        let err = e
            .apply_json(&env(
                47,
                "layer.set_props",
                Some(bg),
                json!({"opacity": 1.5}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let err = e
            .apply_json(&env(
                48,
                "layer.set_props",
                Some(bg),
                json!({"blend": "colour_burn"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let err = e
            .apply_json(&env(
                49,
                "layer.set_props",
                Some(bg),
                json!({"isolated": true}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::TargetKindMismatch,
            "isolated is a group prop"
        );
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Door-slice riding item (O-UI filing): layer.duplicate's idMap is
    /// FULLY CLOSED — layer nodes + masks + OP-STACK ENTRIES. Duplicating
    /// an adjustment layer remaps its op under the client's mapped id
    /// (never aliasing the source's, §2.7 single owner); a map missing
    /// the op id fails completeness with missing_field.
    #[test]
    fn duplicate_remaps_op_stack_entries_fully_closed_map() {
        use crate::engine::op_impl::{OpImpl, PreviewPolicy};
        use serde_json::Value;
        struct ProbeOp;
        impl OpImpl for ProbeOp {
            fn op_id(&self) -> &'static str {
                "adjustment.invert"
            }
            fn validate_params(&self, _p: &Value) -> Result<(), EngineError> {
                Ok(())
            }
            fn default_params(&self) -> Value {
                json!({})
            }
            fn roi_out_to_in(&self, _p: &Value, out: crate::geom::IRect) -> crate::geom::IRect {
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
                _rect: crate::geom::IRect,
                input: &[[f32; 4]],
                output: &mut [[f32; 4]],
            ) -> Result<(), EngineError> {
                output.copy_from_slice(input);
                Ok(())
            }
        }
        let mut e = assembly().with_op(Arc::new(ProbeOp)).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "1a7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            1,
            "adjustment.add",
            Some(bg),
            json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                   "newOpId": no.to_string(), "params": {}}),
        ))
        .unwrap();

        // Completeness now spans the op id: a layer-only map refuses.
        let dup_l = Uuid::new_v4();
        let err = e
            .apply_json(&env(
                2,
                "layer.duplicate",
                Some(LayerId::from_uuid(nl)),
                json!({"idMap": {nl.to_string(): dup_l.to_string()}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::MissingField,
            "op id missing from the map"
        );
        assert!(
            err.message.contains(&no.to_string()),
            "names the op identity"
        );

        // The fully-closed map duplicates with a REMAPPED op.
        let dup_o = Uuid::new_v4();
        let pre = e.state_hash();
        let receipt = e
            .apply_json(&env(
                3,
                "layer.duplicate",
                Some(LayerId::from_uuid(nl)),
                json!({"idMap": {nl.to_string(): dup_l.to_string(),
                                  no.to_string(): dup_o.to_string()}}),
            ))
            .unwrap();
        assert!(receipt.created_ids.contains(&dup_l));
        assert!(
            receipt.created_ids.contains(&dup_o),
            "op-stack entry ids surface in created_ids (client enumerability \
             rides the required idMap; §26.1 replay uses the map verbatim)"
        );
        let doc = e.document().unwrap();
        assert_eq!(doc.op_count(), 2, "two DISTINCT op nodes — no aliasing");
        assert_eq!(
            doc.layer(LayerId::from_uuid(dup_l))
                .unwrap()
                .adjustment_op(),
            Some(crate::graph::id::OpId::from_uuid(dup_o)),
            "the duplicate references its OWN op under the mapped id"
        );
        // Undo/redo round-trips the coupled op+layer creation.
        let post = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre);
        assert_eq!(e.document().unwrap().op_count(), 1);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), post);
        // Replay determinism (the recorded map reproduces identical ids).
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// q-tutorial/Q-12 (operator WORD 14 (4): RELEASE-BLOCKING, "with the
    /// delete-then-undo discriminator as the test"). `layer.delete` on an
    /// adjustment layer detached the node but left its OP — and the op's
    /// mask — in the arenas: an orphan. Debug builds refused the commit
    /// (validate_tree's orphan check, "commit corrupted the tree"); release
    /// committed it, and the package's journal could no longer replay to
    /// the persisted state, so undo refused with bad_target while `info`
    /// still reported canUndo. The subtree's op-stack entries are part of
    /// the subtree (the same closed identity set layer.duplicate remaps).
    #[test]
    fn delete_carries_the_adjustment_op_and_its_mask_through_undo_and_redo() {
        use crate::engine::op_impl::{OpImpl, PreviewPolicy};
        use crate::geom::IRect;
        use serde_json::Value;
        struct InvertProbe;
        impl OpImpl for InvertProbe {
            fn op_id(&self) -> &'static str {
                "adjustment.invert"
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
                _p: &Value,
                _rect: IRect,
                input: &[[f32; 4]],
                output: &mut [[f32; 4]],
            ) -> Result<(), EngineError> {
                output.copy_from_slice(input);
                Ok(())
            }
        }
        let mut e = assembly().with_op(Arc::new(InvertProbe)).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "1a7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            1,
            "adjustment.add",
            Some(bg),
            json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                   "newOpId": no.to_string(), "params": {}}),
        ))
        .unwrap();
        // Give the op its own mask, so the op's mask must travel too.
        {
            let doc = e.doc_mut_for_test();
            let sel = doc.selection_mut();
            sel.mask_mut()
                .fill_rect(IRect::new(0, 0, 8, 8), [1.0, 0.0, 0.0, 0.0]);
            sel.set_bounds(IRect::new(0, 0, 8, 8));
        }
        e.apply_json(&env(
            2,
            "op.set_mask",
            None,
            json!({"opId": no.to_string(), "from": "selection"}),
        ))
        .unwrap();
        let oid = crate::graph::id::OpId::from_uuid(no);
        let adj = LayerId::from_uuid(nl);
        assert!(e.document().unwrap().op(oid).unwrap().mask().is_some());
        let (ops0, masks0) = {
            let d = e.document().unwrap();
            (d.op_count(), d.mask_count())
        };
        let with_adj = e.state_hash();

        // DELETE: the op and its mask leave with the layer — no orphans.
        e.apply_json(&env(3, "layer.delete", Some(adj), json!({})))
            .expect("delete of an adjustment layer commits");
        {
            let d = e.document().unwrap();
            assert!(d.layer(adj).is_none());
            assert_eq!(d.op_count(), ops0 - 1, "the op left with its layer");
            assert_eq!(d.mask_count(), masks0 - 1, "the op's mask left too");
            assert_eq!(d.validate_tree(), Ok(()), "no unreachable op or mask");
        }
        let deleted = e.state_hash();

        // UNDO — the discriminator: it must succeed and restore everything.
        e.undo().expect("undo of the delete succeeds (Q-12)");
        assert_eq!(e.state_hash(), with_adj, "same layer, op and op mask");
        {
            let d = e.document().unwrap();
            assert_eq!(d.layer(adj).unwrap().adjustment_op(), Some(oid));
            assert!(d.op(oid).unwrap().mask().is_some());
            assert_eq!(d.validate_tree(), Ok(()));
        }
        // REDO removes them again.
        e.redo().expect("redo of the delete succeeds");
        assert_eq!(e.state_hash(), deleted);
        assert_eq!(e.document().unwrap().validate_tree(), Ok(()));
        // No replay assertion here: the op mask's selection was seeded
        // through doc_mut_for_test, outside the journal. The journal half
        // is the next test's.
    }

    /// q-tutorial/Q-12, the journal half: release committed the orphan and
    /// the package's journal stopped reproducing its persisted state. A
    /// GROUP holding an adjustment layer is deleted, so the op collection
    /// walks the whole subtree, and replay must match after delete, undo
    /// and redo.
    #[test]
    fn deleting_a_group_with_an_adjustment_layer_replays_from_the_journal() {
        use crate::engine::op_impl::{OpImpl, PreviewPolicy};
        use crate::geom::IRect;
        use serde_json::Value;
        struct InvertProbe;
        impl OpImpl for InvertProbe {
            fn op_id(&self) -> &'static str {
                "adjustment.invert"
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
                _p: &Value,
                _rect: IRect,
                input: &[[f32; 4]],
                output: &mut [[f32; 4]],
            ) -> Result<(), EngineError> {
                output.copy_from_slice(input);
                Ok(())
            }
        }
        let mut e = assembly().with_op(Arc::new(InvertProbe)).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "1a7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let (nl, no, g) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            1,
            "adjustment.add",
            Some(bg),
            json!({"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                   "newOpId": no.to_string(), "params": {}}),
        ))
        .unwrap();
        e.apply_json(&env(
            2,
            "layer.group",
            None,
            json!({"layerIds": [nl.to_string()], "newGroupId": g.to_string()}),
        ))
        .unwrap();
        let with_group = e.state_hash();
        let replay_matches = |e: &Engine, when: &str| {
            let replayed = e.replay_assembled(e.journal()).unwrap();
            assert_eq!(replayed.state_hash(), e.state_hash(), "replay after {when}");
        };

        e.apply_json(&env(
            3,
            "layer.delete",
            Some(LayerId::from_uuid(g)),
            json!({}),
        ))
        .expect("delete of a group holding an adjustment layer commits");
        assert_eq!(
            e.document().unwrap().op_count(),
            0,
            "the nested op left too"
        );
        assert_eq!(e.document().unwrap().validate_tree(), Ok(()));
        replay_matches(&e, "delete");

        e.undo().expect("undo of the group delete succeeds (Q-12)");
        assert_eq!(e.state_hash(), with_group);
        assert_eq!(e.document().unwrap().op_count(), 1);
        e.redo().expect("redo succeeds");
        assert_eq!(e.document().unwrap().op_count(), 0);
        replay_matches(&e, "undo + redo");
    }
}
