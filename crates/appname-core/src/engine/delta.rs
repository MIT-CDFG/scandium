// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10.1 (`DeltaRecorder`, `TileDelta`, `DeltaSet`), §10.4
//! (copy-on-write undo), §3.3 (capture timing); DD ⑰.
//!
//! Undo memory scales with **edited area**, not image size: deltas hold
//! `Arc<TileStorage>` clones of the slice-1 COW substrate — never
//! whole-layer copies (the deliberate divergence from the proto's
//! whole-layer snapshots, plan slice-2 row).
//!
//! Capture timing [NORMATIVE §3.3/§10.1]: `before` is captured on the
//! **first touch of a coord per command** (`will_edit` is idempotent per
//! `(surface, coord)` — the first call wins); `after` is re-read from the
//! live map once at commit, **post-prune** (§2.4 ordering), and no-ops
//! (`Arc::ptr_eq(before, after)`) are dropped. For tiles created or removed
//! by a command, the absent side is the canonical shared transparent tile;
//! restore prunes transparent tiles back out of the map (§2.4 sparsity).
//!
//! Dedup scoping (review requirement, slice-2 binding): the commit pipeline
//! interns changed tiles into a [`DedupIndex`] holding **one table per
//! `(ChannelKind, ColorModelKind)` encoding context**, so the slice-1 debug
//! tripwires (`DedupTable::intern` / `TileMap::insert_arc`) can never fire
//! from the command path.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use uuid::Uuid;

use crate::buffer::Buffer;
use crate::geom::IRect;
use crate::graph::document::Document;
use crate::graph::id::{LayerId, MaskId};
use crate::graph::mask_node::MaskNode;
use crate::graph::node::LayerNode;
use crate::hash::ContentHash;
use crate::tile::channel::ChannelKind;
use crate::tile::dedup::DedupTable;
use crate::tile::map::{TileCoord, TileMap};
use crate::tile::model::ColorModelKind;
use crate::tile::storage::TileStorage;

/// The canonical shared transparent payload (ES §10.1: the absent side of a
/// delta; one process-wide Arc so absent-vs-absent is `ptr_eq`-droppable).
pub(crate) fn transparent_tile() -> Arc<TileStorage> {
    static TRANSPARENT: OnceLock<Arc<TileStorage>> = OnceLock::new();
    TRANSPARENT
        .get_or_init(|| {
            Arc::new(TileStorage::transparent(
                ChannelKind::U8,
                ColorModelKind::Rgba,
            ))
        })
        .clone()
}

fn is_transparent_side(t: &Arc<TileStorage>) -> bool {
    Arc::ptr_eq(t, &transparent_tile())
        || matches!(**t, TileStorage::Uniform { value } if value.iter().all(|v| v.to_bits() == 0))
}

/// Surface-UUID → buffer resolution (ES §10.1: surfaces are disjoint UUIDv4
/// spaces — layers and mask coverages now; channels/selection join with
/// their arenas). Tree restoration additionally needs the document itself
/// (`document_mut`), which buffer-only test resolvers do not provide.
pub(crate) trait SurfaceResolver {
    fn buffer_mut(&mut self, surface: Uuid) -> Option<&mut Buffer>;
    fn buffer(&self, surface: Uuid) -> Option<&Buffer>;
    fn document_mut(&mut self) -> Option<&mut Document> {
        None
    }
}

impl SurfaceResolver for Document {
    fn buffer_mut(&mut self, surface: Uuid) -> Option<&mut Buffer> {
        if surface == self.selection_surface() {
            return Some(self.selection_mut().mask_mut());
        }
        let lid = LayerId::from_uuid(surface);
        if self
            .layer(lid)
            .map(|n| n.pixels().is_some())
            .unwrap_or(false)
        {
            return self.layer_mut(lid).ok()?.pixels_mut();
        }
        if self
            .layer(lid)
            .map(|n| n.smart_object().is_some())
            .unwrap_or(false)
        {
            return self
                .layer_mut(lid)
                .ok()?
                .smart_object_mut()
                .map(|so| &mut so.cached_composite);
        }
        if self
            .mask(crate::graph::id::MaskId::from_uuid(surface))
            .is_some()
        {
            return self
                .mask_mut(crate::graph::id::MaskId::from_uuid(surface))
                .map(|m| m.coverage_mut());
        }
        // §14.1 named channels are surfaces too (door C1).
        self.channel_mut(crate::graph::id::ChannelId::from_uuid(surface))
            .map(|c| c.buffer_mut())
    }
    fn buffer(&self, surface: Uuid) -> Option<&Buffer> {
        if surface == self.selection_surface() {
            return Some(self.selection().mask());
        }
        if let Some(node) = self.layer(LayerId::from_uuid(surface)) {
            if let Some(b) = node.pixels() {
                return Some(b);
            }
            if let Some(so) = node.smart_object() {
                return Some(so.cached_composite());
            }
        }
        if let Some(m) = self.mask(crate::graph::id::MaskId::from_uuid(surface)) {
            return Some(m.coverage());
        }
        self.channel(crate::graph::id::ChannelId::from_uuid(surface))
            .map(|c| c.buffer())
    }
    fn document_mut(&mut self) -> Option<&mut Document> {
        Some(self)
    }
}

/// Test/bench harness resolver: every surface is the one buffer.
#[cfg_attr(not(test), allow(dead_code))] // engine door for later slices (in-crate tests exercise it today)
pub(crate) struct SingleBuffer<'a>(pub &'a mut Buffer);

impl SurfaceResolver for SingleBuffer<'_> {
    fn buffer_mut(&mut self, _surface: Uuid) -> Option<&mut Buffer> {
        Some(self.0)
    }
    fn buffer(&self, _surface: Uuid) -> Option<&Buffer> {
        Some(self.0)
    }
}

/// One changed tile (ES §10.1).
pub struct TileDelta {
    pub surface: Uuid,
    pub coord: TileCoord,
    pub before: Arc<TileStorage>,
    pub after: Arc<TileStorage>,
}

/// The five §2.6/§11.2 mask node properties as one value block (the
/// `mask.set_props` full-state-per-present-field payload and its SetProp
/// delta unit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskProps {
    pub enabled: bool,
    pub inverted: bool,
    pub density: f32,
    pub feather: f32,
    pub linked: bool,
}

/// Structural op recorded as redo applied it (ES §10.1 `TreeOp` subset —
/// grows with the layer.* kinds).
#[derive(Debug, Clone, PartialEq)]
pub enum TreeOp {
    AddNode {
        parent: Option<LayerId>,
        index: usize,
        id: LayerId,
    },
    /// Mask attached to a layer (`mask.add`, `edit.paste_into`): undo
    /// detaches into the stash; redo re-attaches the SAME node (§2.7).
    AttachMask { layer: LayerId, mask: MaskId },
    /// Mask detached + removed (`mask.delete`, `mask.apply`): the removed
    /// node rides the stash from EXECUTION time (it left the arena), undo
    /// re-attaches it at its recorded stacking index, redo re-detaches.
    DetachMask {
        layer: LayerId,
        mask: MaskId,
        index: usize,
    },
    /// Property delta (`mask.set_props`): undo applies `before`, redo
    /// `after` — value-complete, no stash.
    SetMaskProps {
        mask: MaskId,
        before: MaskProps,
        after: MaskProps,
    },
    /// `doc.assign_profile` / the retag half of `doc.convert_profile`
    /// (§5.3): pure ColorState.working swap — zero tile deltas, param
    /// inverse (§40.2).
    SetWorkingProfile {
        before: crate::graph::id::ProfileId,
        after: crate::graph::id::ProfileId,
    },
    /// Document-geometry value delta (O-TOOLS doc.* seam: crop-hide is a
    /// SetProp-only inverse; §19.1).
    SetGeometry {
        before: crate::graph::document::DocumentGeometry,
        after: crate::graph::document::DocumentGeometry,
    },
    /// §12 op-node structural add (`adjustment.add`, and the arena half
    /// of `filter.smart_add`'s insert door): undo removes the op into the
    /// stash, redo re-inserts the SAME node (§2.7).
    AddOp { op: crate::graph::id::OpId },
    /// §12 op-node removal (`op.delete`; the adjustment coupled-delete
    /// pairs this with `RemoveSubtree`): the removed node rides the stash
    /// from execution time.
    RemoveOp { op: crate::graph::id::OpId },
    /// `op.set_params` / `op.toggle`: the §2.6 mutable-op value block.
    SetOpValue {
        op: crate::graph::id::OpId,
        before: Box<crate::graph::op_node::OpValueBlock>,
        after: Box<crate::graph::op_node::OpValueBlock>,
    },
    /// `op.set_mask`: the op's mask slot swap; created/removed mask NODES
    /// ride the stashes (created-side node stashed on undo, removed-side
    /// from execution).
    SetOpMask {
        op: crate::graph::id::OpId,
        before: Option<crate::graph::id::MaskId>,
        after: Option<crate::graph::id::MaskId>,
    },
    /// `op.reorder` within a smart-filter chain (param inverse).
    ReorderOp {
        layer: LayerId,
        op: crate::graph::id::OpId,
        from: usize,
        to: usize,
    },
    /// Splice an op id out of a smart-filter chain (`op.delete` on a
    /// filter; param inverse re-inserts at the recorded index).
    RemoveFilterRef {
        layer: LayerId,
        op: crate::graph::id::OpId,
        index: usize,
    },
    /// Splice an op id INTO a smart-filter chain (`filter.smart_add`'s
    /// chain half — O-TOOLS integration ask 2): exact mirror of
    /// [`TreeOp::RemoveFilterRef`] — revert removes the ref, restore
    /// re-inserts at the recorded (post-clamp) index.
    AddFilterRef {
        layer: LayerId,
        op: crate::graph::id::OpId,
        index: usize,
    },
    /// `smartobject.edit_contents`/`replace`: per-instance source swap
    /// (param inverse; edit_contents records one per instance).
    SetSmartSource {
        layer: LayerId,
        before: Uuid,
        after: Uuid,
    },
    /// §24.2 furniture value delta (guide.add/move/delete/clear, grid.set
    /// — param-inverse kinds expressed as full-state Guides blocks).
    SetGuides {
        before: Box<crate::graph::document::Guides>,
        after: Box<crate::graph::document::Guides>,
    },
    /// Subtree removal (`layer.delete`, `layer.ungroup`'s group disposal):
    /// the detached fragment rides the stash from execution time; undo
    /// re-attaches the SAME nodes (§2.7), redo re-detaches.
    RemoveSubtree {
        parent: LayerId,
        index: usize,
        root: LayerId,
    },
    /// Stacking move (`layer.reorder`, group membership moves): pure param
    /// inverse — undo moves back, redo forward. Indices are the positions
    /// AT APPLICATION TIME in each direction (§10.3 MoveNode).
    MoveNode {
        layer: LayerId,
        from: (LayerId, usize),
        to: (LayerId, usize),
    },
    /// Full §2.6 shared-prop block delta (`layer.rename`,
    /// `layer.set_props`, align/distribute translations): undo applies
    /// `before`, redo `after` — value-complete (§10.3 full-state rule).
    SetLayerProps {
        layer: LayerId,
        before: Box<LayerPropsBlock>,
        after: Box<LayerPropsBlock>,
    },
    /// §20 doc-path arena add (`vector.path_new`, `vector.shape_add`'s
    /// path half): undo removes the path into the stash, redo re-inserts
    /// the SAME node (§2.7) — the stashed node carries the value-captured
    /// sidecar, so neither direction re-runs geometry (D-48).
    AddPath { path: crate::graph::id::PathId },
    /// §20 doc-path removal (`vector.path_delete`, `path_bool`
    /// flatten-consumption): the removed node rides the stash from
    /// execution time; undo re-inserts it with geometry intact.
    RemovePath { path: crate::graph::id::PathId },
    /// `vector.path_edit` / `vector.edit` / `path_bool` doc-path bake:
    /// full path-state snapshot delta (paths are small — §10.5 SetProp
    /// shape, earliest-before/latest-after under merging; NEVER tiles).
    SetPathValue {
        path: crate::graph::id::PathId,
        before: Box<crate::graph::path_node::PathValueBlock>,
        after: Box<crate::graph::path_node::PathValueBlock>,
    },
    /// Shape-layer payload value delta (`vector.shape_set_style`,
    /// `path_bool` shape-mode `bool_ops` append): full §20.3 block,
    /// value-complete both sides.
    SetShapeData {
        layer: LayerId,
        before: Box<ShapeDataBlock>,
        after: Box<ShapeDataBlock>,
    },
    /// Text-layer payload value delta (`text.edit`/`set_style`/
    /// `set_paragraph` — TEXTVEC Door 7): full sidecar snapshot both sides
    /// (§10.3 full-state rule; sidecars are small JSON — SetProp shape,
    /// earliest-before/latest-after under §10.5 merging; NEVER tiles —
    /// rasterized glyph tiles are cache, §23.1).
    SetTextData {
        layer: LayerId,
        before: Box<crate::graph::node::TextLayerData>,
        after: Box<crate::graph::node::TextLayerData>,
    },
    /// `text.to_shape`'s recorded kind swap (§40.7 "undo: delta (SetProp
    /// kind + geometry payloads) — editability restored on undo"): the
    /// layer's CONTENT KIND swaps wholesale, value-complete both sides
    /// (class R — never merges; the new shape's path node rides a paired
    /// [`TreeOp::AddPath`] in the same DeltaSet).
    SetContentKind {
        layer: LayerId,
        before: Box<ContentKindBlock>,
        after: Box<ContentKindBlock>,
    },
    /// §14.1 channel-arena add (`channel.add`, `channel.duplicate`,
    /// `selection.save` creating, `channel.calculations` result:
    /// 'new_channel'): undo removes the channel into the stash, redo
    /// re-inserts the SAME node at its recorded list index (§2.7
    /// analogue; tiles ride the Arcs).
    AddChannel {
        channel: crate::graph::id::ChannelId,
    },
    /// `channel.delete`: the removed node + its §14.1 list index ride the
    /// stash from execution time.
    RemoveChannel {
        channel: crate::graph::id::ChannelId,
        index: usize,
    },
    /// `channel.edit{name}` rename (value delta, §40.4).
    SetChannelName {
        channel: crate::graph::id::ChannelId,
        before: String,
        after: String,
    },
    /// §29.2 slice-vector value delta (slice.add/edit/delete — the
    /// guides full-state pattern; slice.edit is class M and merges).
    SetSlices {
        before: Box<Vec<crate::graph::document::SliceRow>>,
        after: Box<Vec<crate::graph::document::SliceRow>>,
    },
    /// `meta.set` (§10.3/§40.8): one registered document-meta key,
    /// inverse = previous value (`None` = key absent). Class M — merges
    /// per key via the contract mergeKey.
    SetMetaKey {
        key: String,
        before: Box<Option<serde_json::Value>>,
        after: Box<Option<serde_json::Value>>,
    },
}

/// One side of a [`TreeOp::SetContentKind`] swap: the full parametric
/// content payload of the layer (only the text↔shape pair exists today —
/// `text.to_shape` forward, its undo backward).
#[derive(Debug, Clone, PartialEq)]
pub enum ContentKindBlock {
    Text(crate::graph::node::TextLayerData),
    Shape(ShapeContentBlock),
}

/// The FULL shape payload for a kind swap — unlike [`ShapeDataBlock`],
/// the creation-fixed `path` reference is part of the block (the swap
/// CREATES the shape content, it does not mutate an existing one).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeContentBlock {
    pub path: crate::graph::id::PathId,
    pub fill: serde_json::Value,
    pub stroke: serde_json::Value,
    pub bool_ops: Vec<(
        crate::engine::vector_ops::VectorBoolOp,
        crate::graph::id::PathId,
    )>,
}

/// The mutable §20.3 shape-layer payload as one value block (the `path`
/// reference is creation-fixed and not part of the mutable block).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeDataBlock {
    pub fill: serde_json::Value,
    pub stroke: serde_json::Value,
    pub bool_ops: Vec<(
        crate::engine::vector_ops::VectorBoolOp,
        crate::graph::id::PathId,
    )>,
}

/// The §2.6 shared node properties as one value block.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPropsBlock {
    pub name: String,
    pub blend: crate::graph::node::BlendMode,
    pub opacity: f32,
    pub fill_opacity: f32,
    pub visible: bool,
    pub locks: crate::graph::node::LockFlags,
    pub clip_to_below: bool,
    pub color_label: Option<crate::graph::node::ColorLabel>,
    pub transform: crate::geom::Mat3,
    /// Group isolation (§4.7); carried but only meaningful on groups.
    pub isolated: bool,
}

/// Undo-time stash riding beside a tree op (§2.7: the SAME node re-inserts
/// on redo, tiles and masks riding along — zero pixel copies).
pub(crate) enum TreeUndoStash {
    Node {
        node: LayerNode,
        masks: Vec<MaskNode>,
    },
    Mask(MaskNode),
    Subtree(crate::graph::document::SubtreeFragment),
    Op(crate::graph::op_node::OpNode),
    /// SetOpMask sides: (created-side node, removed-side node).
    OpMaskSwap {
        created: Option<MaskNode>,
        removed: Option<MaskNode>,
    },
    /// AddPath/RemovePath sides: the SAME node re-inserts (§2.7), its
    /// value-captured sidecar riding along (D-48).
    Path(crate::graph::path_node::PathNode),
    /// AddChannel/RemoveChannel sides: the SAME node re-inserts at its
    /// recorded §14.1 list index, tiles riding the Arcs.
    Channel(crate::graph::channel_node::NamedChannel),
}

/// Sealed recording of one command's effects (ES §10.1).
pub struct DeltaSet {
    pub tiles: Vec<TileDelta>,
    /// Ops in application order (`tree_after` of §10.1); undo applies the
    /// derived inverse list (reverse order).
    pub tree_after: Vec<TreeOp>,
    /// Parallel to `tree_after`: the stashed nodes while a step is undone.
    pub(crate) tree_stash: Vec<Option<TreeUndoStash>>,
    /// Per-surface content bounds (surface, before, after) for restore
    /// re-tightening (§2.4).
    pub bounds: Vec<(Uuid, IRect, IRect)>,
}

impl DeltaSet {
    /// §10.4 byte accounting: nominal storage size of the `before` tiles
    /// (`after` is the live document and never accounted).
    pub fn accounted_bytes(&self, kind: ChannelKind, model: ColorModelKind) -> usize {
        self.tiles.len() * TileStorage::nominal_byte_len(kind, model)
    }

    /// §10.5 class-M coalescing, the DeltaSet half [NORMATIVE letter]:
    /// "the merged `DeltaSet` keeps the **earliest** `before` and the
    /// **latest** `after` per `(surface, coord)` and per `SetProp` prop,
    /// and concatenates structural ops." `newer` is folded INTO `self`
    /// (the earlier step). Value-block tree ops merge per target (earliest
    /// before already sits in `self`; the after side is replaced);
    /// structural ops append with their stashes.
    pub(crate) fn merge_from(&mut self, newer: DeltaSet) {
        // Tiles: earliest before + latest after per (surface, coord).
        for nd in newer.tiles {
            match self
                .tiles
                .iter_mut()
                .find(|d| d.surface == nd.surface && d.coord == nd.coord)
            {
                Some(prev) => prev.after = nd.after,
                None => self.tiles.push(nd),
            }
        }
        // Bounds: earliest before + latest after per surface.
        for (surface, b_before, b_after) in newer.bounds {
            match self.bounds.iter_mut().find(|(s, _, _)| *s == surface) {
                Some((_, _, after)) => *after = b_after,
                None => self.bounds.push((surface, b_before, b_after)),
            }
        }
        // Tree ops: SetProp-class value blocks merge per target; the rest
        // concatenate (stashes ride along, parallel arrays preserved).
        debug_assert_eq!(newer.tree_after.len(), newer.tree_stash.len());
        for (op, stash) in newer.tree_after.into_iter().zip(newer.tree_stash) {
            let merged = match &op {
                TreeOp::SetLayerProps { layer, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetLayerProps {
                            layer: pl,
                            after: pa,
                            ..
                        } if pl == layer => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetOpValue { op: oid, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetOpValue {
                            op: po, after: pa, ..
                        } if po == oid => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetMaskProps { mask, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetMaskProps {
                            mask: pm,
                            after: pa,
                            ..
                        } if pm == mask => {
                            *pa = *after;
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetPathValue { path, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetPathValue {
                            path: pp,
                            after: pa,
                            ..
                        } if pp == path => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetShapeData { layer, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetShapeData {
                            layer: pl,
                            after: pa,
                            ..
                        } if pl == layer => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetTextData { layer, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetTextData {
                            layer: pl,
                            after: pa,
                            ..
                        } if pl == layer => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetGeometry { after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetGeometry { after: pa, .. } => {
                            *pa = *after;
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetGuides { after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetGuides { after: pa, .. } => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetWorkingProfile { after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetWorkingProfile { after: pa, .. } => {
                            *pa = *after;
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetSmartSource { layer, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetSmartSource {
                            layer: pl,
                            after: pa,
                            ..
                        } if pl == layer => {
                            *pa = *after;
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetSlices { after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetSlices { after: pa, .. } => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetMetaKey { key, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetMetaKey {
                            key: pk, after: pa, ..
                        } if pk == key => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                TreeOp::SetChannelName { channel, after, .. } => {
                    self.tree_after.iter_mut().find_map(|p| match p {
                        TreeOp::SetChannelName {
                            channel: pc,
                            after: pa,
                            ..
                        } if pc == channel => {
                            *pa = after.clone();
                            Some(())
                        }
                        _ => None,
                    })
                }
                _ => None, // structural: concatenate below
            };
            if merged.is_none() {
                self.tree_after.push(op);
                self.tree_stash.push(stash);
            }
        }
    }

    /// Swap every `before` back in (undo direction): tile/bounds restore
    /// FIRST (while the touched nodes still exist), then the derived tree
    /// inverse list in REVERSE order — an `AddNode` undoes by detaching the
    /// node's masks and removing the node, both stashed for redo (§2.7:
    /// the SAME node re-inserts with tiles and masks intact, zero pixel
    /// copies). Buffer-only resolvers (no `document_mut`) skip tree ops.
    pub(crate) fn restore_before(&mut self, resolver: &mut impl SurfaceResolver) {
        self.apply_side(resolver, true);
        for i in (0..self.tree_after.len()).rev() {
            let Some(doc) = resolver.document_mut() else {
                continue;
            };
            match self.tree_after[i].clone() {
                TreeOp::AddNode { id, .. } => {
                    let mask_ids: Vec<_> = doc
                        .layer(id)
                        .map(|n| n.masks().to_vec())
                        .unwrap_or_default();
                    let mut masks = Vec::with_capacity(mask_ids.len());
                    for m in mask_ids {
                        masks.push(doc.detach_mask(id, m).expect("recorded mask resolves"));
                    }
                    let node = doc.remove_layer(id).expect("recorded AddNode is removable");
                    self.tree_stash[i] = Some(TreeUndoStash::Node { node, masks });
                }
                TreeOp::AttachMask { layer, mask } => {
                    let node = doc
                        .detach_mask(layer, mask)
                        .expect("recorded mask resolves");
                    self.tree_stash[i] = Some(TreeUndoStash::Mask(node));
                }
                TreeOp::DetachMask { layer, mask, index } => {
                    let Some(TreeUndoStash::Mask(node)) = self.tree_stash[i].take() else {
                        unreachable!("DetachMask carries its node from execution");
                    };
                    debug_assert_eq!(node.id(), mask);
                    doc.attach_mask_at(layer, index, node)
                        .expect("re-attach under §2.7");
                }
                TreeOp::SetMaskProps { mask, before, .. } => {
                    doc.mask_mut(mask)
                        .expect("recorded mask resolves")
                        .set_props(before);
                }
                TreeOp::SetWorkingProfile { before, .. } => {
                    doc.set_working_profile(before);
                }
                TreeOp::SetGeometry { before, .. } => {
                    doc.set_geometry_recorded(before);
                }
                TreeOp::RemoveSubtree { parent, index, .. } => {
                    let Some(TreeUndoStash::Subtree(frag)) = self.tree_stash[i].take() else {
                        unreachable!("RemoveSubtree carries its fragment from execution");
                    };
                    doc.attach_subtree(parent, index, frag)
                        .expect("re-attach under §2.7");
                }
                TreeOp::MoveNode { layer, from, .. } => {
                    doc.move_child(layer, from.0, from.1)
                        .expect("param inverse move");
                }
                TreeOp::SetLayerProps {
                    layer, ref before, ..
                } => {
                    doc.apply_props_block(layer, before)
                        .expect("recorded layer resolves");
                }
                TreeOp::SetGuides { ref before, .. } => {
                    doc.set_guides((**before).clone());
                }
                TreeOp::AddOp { op } => {
                    let node = doc.remove_op(op).expect("recorded op resolves");
                    self.tree_stash[i] = Some(TreeUndoStash::Op(node));
                }
                TreeOp::RemoveOp { op } => {
                    let Some(TreeUndoStash::Op(node)) = self.tree_stash[i].take() else {
                        unreachable!("RemoveOp carries its node from execution");
                    };
                    debug_assert_eq!(node.id(), op);
                    doc.insert_op(node).expect("re-insert under §2.7");
                }
                TreeOp::SetOpValue { op, ref before, .. } => {
                    doc.op_mut(op)
                        .expect("recorded op resolves")
                        .apply_value_block(before);
                }
                TreeOp::SetOpMask { op, before, after } => {
                    // Undo: put `before` back. Remove the after-node (stash
                    // it as created-side), re-insert the removed-side node.
                    let mut created = None;
                    if let Some(a) = after {
                        created = doc.detach_op_mask(a);
                    }
                    let removed_node = match self.tree_stash[i].take() {
                        Some(TreeUndoStash::OpMaskSwap { removed, .. }) => removed,
                        None => None,
                        _ => unreachable!("SetOpMask stash shape"),
                    };
                    if let (Some(_b), Some(node)) = (before, removed_node) {
                        doc.attach_op_mask(op, node).expect("re-insert under §2.7");
                    }
                    doc.op_mut(op)
                        .expect("recorded op resolves")
                        .set_mask(before);
                    self.tree_stash[i] = Some(TreeUndoStash::OpMaskSwap {
                        created,
                        removed: None,
                    });
                }
                TreeOp::ReorderOp {
                    layer, op, from, ..
                } => {
                    doc.reorder_smart_filter(layer, op, from)
                        .expect("param inverse");
                }
                TreeOp::SetSmartSource { layer, before, .. } => {
                    doc.set_smart_source(layer, before)
                        .expect("recorded layer resolves");
                }
                TreeOp::RemoveFilterRef { layer, op, index } => {
                    doc.insert_smart_filter_ref(layer, op, index)
                        .expect("param inverse");
                }
                TreeOp::AddFilterRef { layer, op, .. } => {
                    doc.remove_smart_filter_ref(layer, op)
                        .expect("param inverse");
                }
                TreeOp::AddPath { path } => {
                    let node = doc
                        .remove_path(path)
                        .expect("recorded AddPath is removable");
                    self.tree_stash[i] = Some(TreeUndoStash::Path(node));
                }
                TreeOp::RemovePath { path } => {
                    let Some(TreeUndoStash::Path(node)) = self.tree_stash[i].take() else {
                        unreachable!("RemovePath carries its node from execution");
                    };
                    debug_assert_eq!(node.id(), path);
                    doc.insert_path(node).expect("re-insert under §2.7");
                }
                TreeOp::SetPathValue {
                    path, ref before, ..
                } => {
                    doc.path_mut(path)
                        .expect("recorded path resolves")
                        .apply_value_block(before);
                }
                TreeOp::SetShapeData {
                    layer, ref before, ..
                } => {
                    doc.apply_shape_data_block(layer, before)
                        .expect("recorded shape layer resolves");
                }
                TreeOp::SetTextData {
                    layer, ref before, ..
                } => {
                    doc.apply_text_data_block(layer, before)
                        .expect("recorded text layer resolves");
                }
                TreeOp::SetContentKind {
                    layer, ref before, ..
                } => {
                    doc.apply_content_kind_block(layer, before)
                        .expect("recorded layer resolves");
                }
                TreeOp::AddChannel { channel } => {
                    let (_, node) = doc
                        .remove_channel(channel)
                        .expect("recorded AddChannel is removable");
                    self.tree_stash[i] = Some(TreeUndoStash::Channel(node));
                }
                TreeOp::RemoveChannel { channel, index } => {
                    let Some(TreeUndoStash::Channel(node)) = self.tree_stash[i].take() else {
                        unreachable!("RemoveChannel carries its node from execution");
                    };
                    debug_assert_eq!(node.id(), channel);
                    doc.insert_channel_at(index, node)
                        .expect("re-insert under §2.7");
                }
                TreeOp::SetChannelName {
                    channel,
                    ref before,
                    ..
                } => {
                    doc.channel_mut(channel)
                        .expect("recorded channel resolves")
                        .set_name(before.clone());
                }
                TreeOp::SetSlices { ref before, .. } => {
                    *doc.slices_mut() = (**before).clone();
                }
                TreeOp::SetMetaKey {
                    ref key,
                    ref before,
                    ..
                } => match &**before {
                    Some(v) => {
                        doc.meta_mut().insert(key.clone(), v.clone());
                    }
                    None => {
                        doc.meta_mut().remove(key);
                    }
                },
            }
        }
    }

    /// Swap every `after` back in (redo direction): tree ops replay
    /// FORWARD first (the stashed node re-inserts under its recorded
    /// parent/index, masks re-attach), then the after-Arc swaps run —
    /// idempotent over the re-inserted surfaces.
    pub(crate) fn restore_after(&mut self, resolver: &mut impl SurfaceResolver) {
        for i in 0..self.tree_after.len() {
            let Some(doc) = resolver.document_mut() else {
                continue;
            };
            match self.tree_after[i].clone() {
                TreeOp::AddNode { parent, index, id } => {
                    let Some(TreeUndoStash::Node { node, masks }) = self.tree_stash[i].take()
                    else {
                        unreachable!("undone AddNode carries its stash");
                    };
                    let parent = parent.expect("v1 restorable AddNode records a parent");
                    doc.insert_child(parent, index, node)
                        .expect("re-insertion under §2.7");
                    for m in masks {
                        doc.attach_mask(id, m).expect("mask re-attach under §2.7");
                    }
                }
                TreeOp::AttachMask { layer, .. } => {
                    let Some(TreeUndoStash::Mask(node)) = self.tree_stash[i].take() else {
                        unreachable!("undone AttachMask carries its stash");
                    };
                    doc.attach_mask(layer, node).expect("re-attach under §2.7");
                }
                TreeOp::DetachMask { layer, mask, .. } => {
                    let node = doc
                        .detach_mask(layer, mask)
                        .expect("recorded mask resolves");
                    self.tree_stash[i] = Some(TreeUndoStash::Mask(node));
                }
                TreeOp::SetMaskProps { mask, after, .. } => {
                    doc.mask_mut(mask)
                        .expect("recorded mask resolves")
                        .set_props(after);
                }
                TreeOp::SetWorkingProfile { after, .. } => {
                    doc.set_working_profile(after);
                }
                TreeOp::SetGeometry { after, .. } => {
                    doc.set_geometry_recorded(after);
                }
                TreeOp::RemoveSubtree { root, .. } => {
                    let frag = doc.detach_subtree(root).expect("recorded subtree resolves");
                    self.tree_stash[i] = Some(TreeUndoStash::Subtree(frag));
                }
                TreeOp::MoveNode { layer, to, .. } => {
                    doc.move_child(layer, to.0, to.1)
                        .expect("recorded move re-applies");
                }
                TreeOp::SetLayerProps {
                    layer, ref after, ..
                } => {
                    doc.apply_props_block(layer, after)
                        .expect("recorded layer resolves");
                }
                TreeOp::SetGuides { ref after, .. } => {
                    doc.set_guides((**after).clone());
                }
                TreeOp::AddOp { .. } => {
                    let Some(TreeUndoStash::Op(node)) = self.tree_stash[i].take() else {
                        unreachable!("undone AddOp carries its stash");
                    };
                    doc.insert_op(node).expect("re-insert under §2.7");
                }
                TreeOp::RemoveOp { op } => {
                    let node = doc.remove_op(op).expect("recorded op resolves");
                    self.tree_stash[i] = Some(TreeUndoStash::Op(node));
                }
                TreeOp::SetOpValue { op, ref after, .. } => {
                    doc.op_mut(op)
                        .expect("recorded op resolves")
                        .apply_value_block(after);
                }
                TreeOp::SetOpMask { op, before, after } => {
                    let mut removed = None;
                    if let Some(b) = before {
                        removed = doc.detach_op_mask(b);
                    }
                    let created_node = match self.tree_stash[i].take() {
                        Some(TreeUndoStash::OpMaskSwap { created, .. }) => created,
                        None => None,
                        _ => unreachable!("SetOpMask stash shape"),
                    };
                    if let (Some(_a), Some(node)) = (after, created_node) {
                        doc.attach_op_mask(op, node).expect("re-insert under §2.7");
                    }
                    doc.op_mut(op)
                        .expect("recorded op resolves")
                        .set_mask(after);
                    self.tree_stash[i] = Some(TreeUndoStash::OpMaskSwap {
                        created: None,
                        removed,
                    });
                }
                TreeOp::ReorderOp { layer, op, to, .. } => {
                    doc.reorder_smart_filter(layer, op, to)
                        .expect("recorded move re-applies");
                }
                TreeOp::SetSmartSource { layer, after, .. } => {
                    doc.set_smart_source(layer, after)
                        .expect("recorded layer resolves");
                }
                TreeOp::RemoveFilterRef { layer, op, .. } => {
                    doc.remove_smart_filter_ref(layer, op)
                        .expect("recorded move re-applies");
                }
                TreeOp::AddFilterRef { layer, op, index } => {
                    doc.insert_smart_filter_ref(layer, op, index)
                        .expect("recorded move re-applies");
                }
                TreeOp::AddPath { .. } => {
                    let Some(TreeUndoStash::Path(node)) = self.tree_stash[i].take() else {
                        unreachable!("undone AddPath carries its stash");
                    };
                    doc.insert_path(node).expect("re-insert under §2.7");
                }
                TreeOp::RemovePath { path } => {
                    let node = doc.remove_path(path).expect("recorded path resolves");
                    self.tree_stash[i] = Some(TreeUndoStash::Path(node));
                }
                TreeOp::SetPathValue {
                    path, ref after, ..
                } => {
                    doc.path_mut(path)
                        .expect("recorded path resolves")
                        .apply_value_block(after);
                }
                TreeOp::SetShapeData {
                    layer, ref after, ..
                } => {
                    doc.apply_shape_data_block(layer, after)
                        .expect("recorded shape layer resolves");
                }
                TreeOp::SetTextData {
                    layer, ref after, ..
                } => {
                    doc.apply_text_data_block(layer, after)
                        .expect("recorded text layer resolves");
                }
                TreeOp::SetContentKind {
                    layer, ref after, ..
                } => {
                    doc.apply_content_kind_block(layer, after)
                        .expect("recorded layer resolves");
                }
                TreeOp::AddChannel { .. } => {
                    let Some(TreeUndoStash::Channel(node)) = self.tree_stash[i].take() else {
                        unreachable!("undone AddChannel carries its stash");
                    };
                    // Re-append: an AddChannel always appended (creation
                    // order = list order, §14.1), so the tail index is
                    // exact.
                    doc.insert_channel(node).expect("re-insert under §2.7");
                }
                TreeOp::RemoveChannel { channel, .. } => {
                    let (index, node) = doc
                        .remove_channel(channel)
                        .expect("recorded channel resolves");
                    debug_assert_eq!(
                        index,
                        match self.tree_after[i] {
                            TreeOp::RemoveChannel { index, .. } => index,
                            _ => unreachable!(),
                        }
                    );
                    self.tree_stash[i] = Some(TreeUndoStash::Channel(node));
                }
                TreeOp::SetChannelName {
                    channel, ref after, ..
                } => {
                    doc.channel_mut(channel)
                        .expect("recorded channel resolves")
                        .set_name(after.clone());
                }
                TreeOp::SetSlices { ref after, .. } => {
                    *doc.slices_mut() = (**after).clone();
                }
                TreeOp::SetMetaKey {
                    ref key, ref after, ..
                } => match &**after {
                    Some(v) => {
                        doc.meta_mut().insert(key.clone(), v.clone());
                    }
                    None => {
                        doc.meta_mut().remove(key);
                    }
                },
            }
        }
        self.apply_side(resolver, false);
    }

    fn apply_side(&self, resolver: &mut impl SurfaceResolver, before: bool) {
        for d in &self.tiles {
            let Some(buf) = resolver.buffer_mut(d.surface) else {
                continue;
            };
            let side = if before { &d.before } else { &d.after };
            if is_transparent_side(side) {
                buf.remove_tile(d.coord);
            } else {
                buf.install_tile(d.coord, side.clone());
            }
        }
        for (surface, b_before, b_after) in &self.bounds {
            if let Some(buf) = resolver.buffer_mut(*surface) {
                buf.set_bounds_recorded(if before { *b_before } else { *b_after });
            }
        }
    }
}

/// Per-command recorder (ES §10.1): created by the engine per execution;
/// execution routes every mutation through it; the engine seals the
/// recording after execution returns.
pub struct DeltaRecorder {
    /// (surface, coord) → captured before Arc; first call wins.
    before: HashMap<(Uuid, TileCoord), Arc<TileStorage>>,
    /// Capture order for deterministic delta listing.
    order: Vec<(Uuid, TileCoord)>,
    /// Ops with an optional execution-time stash (`DetachMask` carries the
    /// node it removed from the arena).
    tree: Vec<(TreeOp, Option<TreeUndoStash>)>,
    bounds_before: Vec<(Uuid, IRect)>,
    /// §10.3 D-event hints for composite changes NO surface delta encodes
    /// (chain-edit kinds: an op's params/mask/order/enabled flip changes
    /// pixels only at re-render). Receipt-side only — never part of the
    /// persisted `DeltaSet` (history spill and undo inverses are
    /// untouched); the engine merges these into the receipt dirty rects,
    /// which the history rows then echo on undo/redo (arbitration 2).
    dirty_hints: Vec<crate::engine::DirtyRect>,
}

impl DeltaRecorder {
    pub fn new() -> DeltaRecorder {
        DeltaRecorder {
            before: HashMap::new(),
            order: Vec::new(),
            tree: Vec::new(),
            bounds_before: Vec::new(),
            dirty_hints: Vec::new(),
        }
    }

    /// Capture the `before` Arc of a tile about to change. Idempotent per
    /// `(surface, coord)`: the FIRST call wins, so the captured state is
    /// always the pre-command state no matter how often a stroke revisits a
    /// tile (ES §10.1).
    pub fn will_edit(&mut self, surface: Uuid, map: &TileMap, c: TileCoord) {
        if !self.before.contains_key(&(surface, c)) {
            self.order.push((surface, c));
            let captured = map.get(c).cloned().unwrap_or_else(transparent_tile);
            self.before.insert((surface, c), captured);
        }
    }

    /// Record the pre-command content bounds of a surface (first call wins).
    pub fn will_edit_bounds(&mut self, surface: Uuid, buffer: &Buffer) {
        if !self.bounds_before.iter().any(|(s, _)| *s == surface) {
            self.bounds_before.push((surface, buffer.bounds()));
        }
    }

    /// Record a structural op execution has just applied (ES §10.1).
    pub fn tree(&mut self, op: TreeOp) {
        self.tree.push((op, None));
    }

    /// Push a §10.3 D-event hint (chain-edit kinds; deduplicated exact).
    pub(crate) fn dirty_hint(&mut self, hint: crate::engine::DirtyRect) {
        if !hint.rect.is_empty() && !self.dirty_hints.contains(&hint) {
            self.dirty_hints.push(hint);
        }
    }

    /// Drain the D-event hints (engine commit, before `finish` seals).
    pub(crate) fn take_dirty_hints(&mut self) -> Vec<crate::engine::DirtyRect> {
        std::mem::take(&mut self.dirty_hints)
    }

    /// Record a structural op that removed a node from the arena — the
    /// removed node rides the delta from execution time (`DetachMask`).
    pub(crate) fn tree_with_stash(&mut self, op: TreeOp, stash: TreeUndoStash) {
        self.tree.push((op, Some(stash)));
    }

    /// Seal the recording (ES §10.1 `finish`): prune each touched coord
    /// (§2.4: pruning happens BEFORE after-images are recorded), re-read the
    /// live Arc as `after`, intern changed payloads into the per-context
    /// dedup table (§3.8), drop no-ops (`Arc::ptr_eq` / absent-to-absent),
    /// and record the bounds pairs.
    pub(crate) fn finish(
        self,
        resolver: &mut impl SurfaceResolver,
        dedup: &mut DedupIndex,
    ) -> DeltaSet {
        let mut tiles = Vec::new();
        for (surface, coord) in &self.order {
            let Some(buf) = resolver.buffer_mut(*surface) else {
                continue;
            };
            buf.prune_tile(*coord);
            let before = self.before[&(*surface, *coord)].clone();
            let after = match buf.get_tile(*coord).cloned() {
                Some(arc) => {
                    let hash = arc.content_hash(buf.kind(), buf.model());
                    match dedup.intern(buf.kind(), buf.model(), hash, &arc) {
                        Some(existing) => {
                            buf.install_tile(*coord, existing.clone());
                            existing
                        }
                        None => arc,
                    }
                }
                None => transparent_tile(),
            };
            let no_op = Arc::ptr_eq(&before, &after)
                || (is_transparent_side(&before) && is_transparent_side(&after));
            if !no_op {
                tiles.push(TileDelta {
                    surface: *surface,
                    coord: *coord,
                    before,
                    after,
                });
            }
        }
        let mut bounds = Vec::new();
        for (surface, before_bounds) in &self.bounds_before {
            if let Some(buf) = resolver.buffer(*surface) {
                bounds.push((*surface, *before_bounds, buf.bounds()));
            }
        }
        let (tree_after, tree_stash): (Vec<_>, Vec<_>) = self.tree.into_iter().unzip();
        DeltaSet {
            tiles,
            tree_after,
            tree_stash,
            bounds,
        }
    }
}

impl Default for DeltaRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-`(kind, model)` dedup scoping (review minor 2 requirement): one
/// [`DedupTable`] per encoding context, so content-hash equality is only
/// ever consulted within one context and the slice-1 debug tripwires stay
/// silent on the command path.
#[derive(Default)]
pub struct DedupIndex {
    tables: HashMap<(ChannelKind, ColorModelKind), DedupTable>,
}

impl DedupIndex {
    pub fn new() -> DedupIndex {
        DedupIndex::default()
    }

    pub(crate) fn intern(
        &mut self,
        kind: ChannelKind,
        model: ColorModelKind,
        hash: ContentHash,
        tile: &Arc<TileStorage>,
    ) -> Option<Arc<TileStorage>> {
        self.tables
            .entry((kind, model))
            .or_default()
            .intern(hash, tile)
    }

    pub fn context_count(&self) -> usize {
        self.tables.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::IVec2;
    use crate::graph::id::ProfileId;
    use crate::tile::TILE_I32;

    fn buffer() -> Buffer {
        Buffer::new(ChannelKind::U8, ColorModelKind::Rgba, ProfileId::nil())
    }

    fn coord(x: i32, y: i32) -> TileCoord {
        TileCoord::new(0, x, y)
    }

    #[test]
    fn will_edit_first_touch_wins() {
        let mut b = buffer();
        let surface = Uuid::new_v4();
        b.fill_rect(IRect::new(0, 0, 256, 256), [0.25, 0.25, 0.25, 1.0]);
        let original = b.get_tile(coord(0, 0)).unwrap().clone();

        let mut rec = DeltaRecorder::new();
        rec.will_edit(surface, b.tiles(), coord(0, 0));
        // Mutate, then touch again: the captured before must stay the
        // pre-command payload (§3.3: only the first dab pays).
        b.write_pixel_f32(IVec2::new(1, 1), [0.5, 0.5, 0.5, 1.0]);
        rec.will_edit(surface, b.tiles(), coord(0, 0));
        assert!(Arc::ptr_eq(&rec.before[&(surface, coord(0, 0))], &original));
        assert_eq!(rec.order.len(), 1);
    }

    #[test]
    fn finish_drops_no_op_touches() {
        let mut b = buffer();
        let surface = Uuid::new_v4();
        let mut rec = DeltaRecorder::new();
        rec.will_edit_bounds(surface, &b);
        // Touched but never mutated: absent before, absent after.
        rec.will_edit(surface, b.tiles(), coord(3, 3));
        let set = rec.finish(&mut SingleBuffer(&mut b), &mut DedupIndex::new());
        assert!(
            set.tiles.is_empty(),
            "absent-to-absent and ptr-equal deltas drop"
        );
    }

    #[test]
    fn cow_delta_restore_round_trips_content_hash() {
        let mut b = buffer();
        let surface = Uuid::new_v4();
        b.fill_rect(IRect::new(0, 0, 300, 300), [0.25, 0.25, 0.25, 1.0]);
        let h_pre = b.region_hash();

        // One command: capture-then-edit through the recorder discipline.
        let mut rec = DeltaRecorder::new();
        rec.will_edit_bounds(surface, &b);
        for (tx, ty) in IRect::new(100, 100, 500, 400).tiles(TILE_I32) {
            rec.will_edit(surface, b.tiles(), coord(tx, ty));
        }
        b.fill_rect(IRect::new(100, 100, 500, 400), [0.5, 0.1, 0.1, 1.0]);
        let mut set = rec.finish(&mut SingleBuffer(&mut b), &mut DedupIndex::new());
        let h_post = b.region_hash();
        assert_ne!(h_pre, h_post);
        assert!(!set.tiles.is_empty());

        // Undo: befores swap back in; identity returns exactly (§10.4).
        set.restore_before(&mut SingleBuffer(&mut b));
        assert_eq!(b.region_hash(), h_pre, "undo restores byte identity");

        // Redo: afters swap back in.
        set.restore_after(&mut SingleBuffer(&mut b));
        assert_eq!(b.region_hash(), h_post, "redo restores byte identity");
    }

    #[test]
    fn created_tiles_restore_to_absent() {
        let mut b = buffer();
        let surface = Uuid::new_v4();
        let mut rec = DeltaRecorder::new();
        rec.will_edit_bounds(surface, &b);
        for (tx, ty) in IRect::new(0, 0, 256, 256).tiles(TILE_I32) {
            rec.will_edit(surface, b.tiles(), coord(tx, ty));
        }
        b.fill_rect(IRect::new(0, 0, 256, 256), [0.5, 0.5, 0.5, 1.0]);
        let mut set = rec.finish(&mut SingleBuffer(&mut b), &mut DedupIndex::new());
        assert_eq!(set.tiles.len(), 1);
        assert!(is_transparent_side(&set.tiles[0].before));

        set.restore_before(&mut SingleBuffer(&mut b));
        assert!(
            b.tiles().is_empty(),
            "created tile pruned back out on undo (§2.4)"
        );
        assert_eq!(
            b.bounds(),
            IRect::EMPTY,
            "bounds re-tighten from the recorded pair"
        );
    }

    #[test]
    fn commit_dedups_identical_payloads_within_one_context() {
        // Two coords filled with identical content through the recorder →
        // after commit both map entries share ONE Arc (§3.8 "identical
        // content is stored once"), and no slice-1 debug tripwire fires.
        let mut b = buffer();
        let surface = Uuid::new_v4();
        let mut rec = DeltaRecorder::new();
        rec.will_edit_bounds(surface, &b);
        for (tx, ty) in IRect::new(0, 0, 512, 256).tiles(TILE_I32) {
            rec.will_edit(surface, b.tiles(), coord(tx, ty));
        }
        // The fill fast path installs one fresh Uniform Arc per covered
        // tile — identical content, distinct allocations, which is exactly
        // what commit-time interning collapses.
        b.fill_rect(IRect::new(0, 0, 512, 256), [0.5, 0.25, 0.125, 1.0]);
        let mut dedup = DedupIndex::new();
        let set = rec.finish(&mut SingleBuffer(&mut b), &mut dedup);
        assert_eq!(set.tiles.len(), 2);
        let a0 = b.get_tile(coord(0, 0)).unwrap();
        let a1 = b.get_tile(coord(1, 0)).unwrap();
        assert!(
            Arc::ptr_eq(a0, a1),
            "identical content interned to one payload"
        );
        assert_eq!(dedup.context_count(), 1);
    }

    #[test]
    fn dedup_index_scopes_per_context() {
        let mut dedup = DedupIndex::new();
        let u8_rgba = Arc::new(TileStorage::materialize(
            [0.5, 0.5, 0.5, 1.0],
            ChannelKind::U8,
            ColorModelKind::Rgba,
        ));
        let h8 = u8_rgba.content_hash(ChannelKind::U8, ColorModelKind::Rgba);
        assert!(dedup
            .intern(ChannelKind::U8, ColorModelKind::Rgba, h8, &u8_rgba)
            .is_none());

        let twin = Arc::new(TileStorage::materialize(
            [0.5, 0.5, 0.5, 1.0],
            ChannelKind::U8,
            ColorModelKind::Rgba,
        ));
        let hit = dedup
            .intern(
                ChannelKind::U8,
                ColorModelKind::Rgba,
                twin.content_hash(ChannelKind::U8, ColorModelKind::Rgba),
                &twin,
            )
            .expect("same-context dedup hit");
        assert!(Arc::ptr_eq(&hit, &u8_rgba));

        // A different (kind, model) context gets its own table: no
        // cross-context hash consultation, no slice-1 tripwire.
        let f32_gray = Arc::new(TileStorage::from(crate::tile::tile::Tile::<
            f32,
            crate::tile::model::model::Gray,
        >::zeroed()));
        let hg = f32_gray.content_hash(ChannelKind::F32, ColorModelKind::Gray);
        assert!(dedup
            .intern(ChannelKind::F32, ColorModelKind::Gray, hg, &f32_gray)
            .is_none());
        assert_eq!(dedup.context_count(), 2);
    }

    #[test]
    fn accounted_bytes_is_nominal_before_size() {
        let set = DeltaSet {
            tiles: vec![
                TileDelta {
                    surface: Uuid::new_v4(),
                    coord: coord(0, 0),
                    before: transparent_tile(),
                    after: transparent_tile(),
                },
                TileDelta {
                    surface: Uuid::new_v4(),
                    coord: coord(1, 0),
                    before: transparent_tile(),
                    after: transparent_tile(),
                },
            ],
            tree_after: vec![],
            tree_stash: vec![],
            bounds: vec![],
        };
        assert_eq!(
            set.accounted_bytes(ChannelKind::U8, ColorModelKind::Rgba),
            2 * 256 * 256 * 4
        );
    }
}
