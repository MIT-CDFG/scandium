// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.6 (`Document`, document-level invariants), §2.7 (tree
//! integrity and `validate_tree`), §19.1 (document geometry), §5 sketch
//! (color state tags); schema `contracts/schemas/document.v1.json`
//! (`geometry`, `colorState`, `root`, `layers`).
//!
//! CP-0.1 document: geometry + color tags + the layer tree (raster/group) +
//! the selection slot. Arenas for ops/masks/paths/channels/assets, guides,
//! and metadata attach with their owning rows (§2.6 invariant 4 fixes only
//! ownership and addressing).
//!
//! Mutation doors are `pub(crate)` (plan rule 5 / D-48): slice 2's command
//! engine (`doc.new` first) is the only public write path. In debug builds
//! every structural door re-validates the tree after mutating (§2.7 rule b).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::geom::{IRect, IVec2};
use crate::graph::arena::{Arena, ArenaError};
use crate::graph::id::{DocumentId, LayerId, ProfileId};
use crate::graph::node::{BlendMode, LayerNode, LockFlags};
use crate::mask::Selection;
use crate::tile::channel::ChannelKind;
use crate::tile::model::ColorModelKind;

/// Document geometry (ES §19.1, CP-0.1 subset). `image_size` — the size of
/// the composited/exported result — equals `canvas_size` until the
/// non-destructive crop lands (`crop_rect`, §19.1); it is derived state but
/// serialized anyway (fast open, external tooling).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentGeometry {
    /// Pixels of the composited document (`[w, h]` on the wire).
    pub image_size: IVec2,
    /// Working area (`[w, h]`; may differ from content).
    pub canvas_size: IVec2,
    /// Print resolution metadata; `> 0`, default 72.0 (§19.1).
    ///
    /// Wire spelling is the published-schema value `resolutionPPI`
    /// (`document.v1.json` geometry.required — the §2.8 precedence note:
    /// where a published JSON Schema pins a spelling, the published value is
    /// normative and beats the default `camelCase` rename, which would emit
    /// `resolutionPpi`).
    #[serde(rename = "resolutionPPI")]
    pub resolution_ppi: f64,
    /// §19.1 non-destructive crop: hides, never deletes. `image_size`
    /// equals `crop_rect.size` while active, else `canvas_size` (derived,
    /// serialized anyway, validated on load — §19.1). Wire: `cropRect`
    /// `[x0, y0, x1, y1]`, emitted only when active (document.v1.json's
    /// geometry object lags §19.1 here — schema-gap flagged for the
    /// fold-in list; absent-when-None keeps every existing package
    /// byte-identical AND schema-valid).
    #[serde(rename = "cropRect", skip_serializing_if = "Option::is_none", default)]
    pub crop_rect: Option<crate::geom::IRect>,
}

impl DocumentGeometry {
    /// Fresh-document geometry: image = canvas (no crop yet), 72 PPI
    /// default (§19.1).
    pub fn new(width: i32, height: i32) -> DocumentGeometry {
        DocumentGeometry {
            image_size: IVec2::new(width, height),
            canvas_size: IVec2::new(width, height),
            resolution_ppi: 72.0,
            crop_rect: None,
        }
    }

    /// §19.1 invariants: `canvas_size >= 1×1`, `resolution_ppi > 0` and
    /// finite, and the DERIVED `image_size` rule — `crop_rect.size` while
    /// a non-destructive crop is active, else `canvas_size` (serialized
    /// anyway; validated here on load, §19.1).
    pub fn validate(&self) -> Result<(), GraphError> {
        if self.canvas_size.x < 1 || self.canvas_size.y < 1 {
            return Err(GraphError::InvalidGeometry(
                "canvas_size must be at least 1x1",
            ));
        }
        if !(self.resolution_ppi.is_finite() && self.resolution_ppi > 0.0) {
            return Err(GraphError::InvalidGeometry(
                "resolution_ppi must be finite and > 0",
            ));
        }
        match self.crop_rect {
            Some(r) => {
                if r.is_empty() {
                    return Err(GraphError::InvalidGeometry(
                        "crop_rect must be non-empty (§19.1)",
                    ));
                }
                if self.image_size != IVec2::new(r.width(), r.height()) {
                    return Err(GraphError::InvalidGeometry(
                        "image_size must equal crop_rect.size while a crop is active (§19.1)",
                    ));
                }
            }
            None => {
                if self.image_size != self.canvas_size {
                    return Err(GraphError::InvalidGeometry(
                        "image_size must equal canvas_size when no crop is active (§19.1)",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn canvas_rect(&self) -> IRect {
        IRect::new(0, 0, self.canvas_size.x, self.canvas_size.y)
    }

    /// §19.1 OUTPUT FRAME — the one rect every composite/export door frames
    /// on: the non-destructive crop sub-region while a crop is active
    /// (origin `+crop_rect.min`, size `crop_rect.size`), else the full
    /// canvas. This is the single source of the "one compositing truth":
    /// the CPU reference walk, render's compositor, the CLI/cabi export
    /// doors and canvas readback all route their frame through it, so a
    /// cropped document flattens/exports to an identical sub-region across
    /// every door — pixels outside the crop are HIDDEN, never deleted.
    /// `crop_rect == None` returns `canvas_rect()` verbatim, keeping every
    /// crop-free path byte-identical.
    pub fn image_rect(&self) -> IRect {
        self.crop_rect.unwrap_or_else(|| self.canvas_rect())
    }

    /// Reset to a FULL-CANVAS frame of `size`: `canvas == image` and NO
    /// active crop (§19.1). The canvas-geometry kinds (resize, canvas
    /// resize, rotate, and the destructive crop/trim) call this so a
    /// previously-active non-destructive `crop_rect` can never survive as a
    /// stale window into a canvas that has since been reflowed — those ops
    /// redefine the canvas, which supersedes the crop framing.
    pub fn set_full_canvas(&mut self, size: IVec2) {
        self.canvas_size = size;
        self.image_size = size;
        self.crop_rect = None;
    }
}

/// Rendering intent tags (schema `colorState.renderingIntent`; semantics with
/// the color lane, ES §5.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderingIntent {
    Perceptual,
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

/// Document color state tags (schema `colorState`; the CMM behind them is
/// the CP-0.5 color lane, ES §5).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorState {
    /// Working profile id (profile resolution lands with §5.2).
    pub working: ProfileId,
    pub mode: ColorModelKind,
    pub depth: ChannelKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rendering_intent: Option<RenderingIntent>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub black_point_compensation: Option<bool>,
}

/// Structural mutation failures at the graph doors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum GraphError {
    UnknownLayer(LayerId),
    NotAGroup(LayerId),
    IndexOutOfRange { len: usize, index: usize },
    RootImmutable,
    HasChildren(LayerId),
    DuplicateId(LayerId),
    InvalidGeometry(&'static str),
    DepthNotStorageBacked(ChannelKind),
}

impl From<ArenaError> for GraphError {
    fn from(e: ArenaError) -> GraphError {
        match e {
            ArenaError::DuplicateId(u) => GraphError::DuplicateId(LayerId::from_uuid(u)),
        }
    }
}

/// Tree-integrity violations (ES §2.7). `#[non_exhaustive]`: the mask/op
/// resolution variants grow with their arenas.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TreeError {
    MissingRoot,
    RootNotGroup,
    /// Root must be undecorated (§2.6 invariant 2).
    RootDecorated,
    DanglingChild {
        parent: LayerId,
        child: LayerId,
    },
    SharedChild {
        child: LayerId,
    },
    Cycle {
        at: LayerId,
    },
    NonGroupWithChildren {
        id: LayerId,
    },
    DanglingMask {
        owner: LayerId,
    },
    SharedMask {
        mask: crate::graph::id::MaskId,
    },
    UnreachableMasks(Vec<crate::graph::id::MaskId>),
    UnreachableNodes(Vec<LayerId>),
    /// §2.6 invariant 3/7: an `Adjustment(OpId)` / `smart_filters` entry /
    /// `OpNode::mask` fails to resolve.
    DanglingOp {
        owner: LayerId,
    },
    DanglingOpMask {
        op: crate::graph::id::OpId,
    },
    /// An op referenced by more than one owner (adjustment layer or
    /// smart-filter slot).
    SharedOp {
        op: crate::graph::id::OpId,
    },
    UnreachableOps(Vec<crate::graph::id::OpId>),
}

/// The document (ES §2.6, CP-0.1 field subset): geometry, color tags, the
/// layer tree, and the selection slot.
#[derive(Clone)]
pub struct Document {
    id: DocumentId,
    geometry: DocumentGeometry,
    color: ColorState,
    /// A `Group` node containing the stack (§2.6 invariant 1-2).
    root: LayerId,
    layers: Arena<LayerNode>,
    /// Mask nodes (§2.6/§2.7 rule 6: each referenced by exactly one
    /// OWNER — a layer or an op, §12.2).
    masks: Arena<crate::graph::mask_node::MaskNode>,
    /// §2.6 parametric op nodes (§12: parameters, never baked pixels).
    ops: Arena<crate::graph::op_node::OpNode>,
    /// §20 doc-level path arena (TEXTVEC Door 2): opaque
    /// `appname.vector.v1` sidecars + content hashes, persisted as
    /// `vector/{pathId}.json` per the document.v1 `pathRef` convention.
    paths: Arena<crate::graph::path_node::PathNode>,
    /// §14.1 named-channel arena (door C1): alpha channels now, spot room
    /// reserved (§14.3). The arena is unordered; `channel_order` carries
    /// the §14.1 list order (creation order — no reorder kind in v1.0).
    channels: Arena<crate::graph::channel_node::NamedChannel>,
    channel_order: Vec<crate::graph::id::ChannelId>,
    /// §29.2 export slices ("slices are document data") — the guides
    /// pattern: plain document furniture, index-addressed (§10.3),
    /// full-vector value deltas.
    slices: Vec<SliceRow>,
    selection: Selection,
    /// Deterministic selection-surface id (§10.1 disjoint UUID spaces),
    /// derived from the document id at construction.
    selection_surface: Uuid,
    /// §24.2 guides + grid furniture (persisted per the §7.3 `guides`
    /// object; the grid block rides inside it — schema-gap flagged, the
    /// $def permits additional properties like geometry's cropRect).
    guides: Guides,
    /// §5.1/§7.5 document profile store: `ProfileId` → resolved entry.
    /// Builtins carry no bytes; embedded ICC bytes persist as
    /// `profiles/{id}.icc` (§7.5) and re-embed on save. Content-derived
    /// ids make identical bytes dedup by construction (O-COLOR need 1/2).
    profiles: std::collections::BTreeMap<ProfileId, DocProfile>,
    /// §7.3 open document-meta object (document.v1.json `meta`,
    /// `additionalProperties: true`). CP-0.7 opening scope: the import-
    /// provenance slot — §42.4 persists a compatibility report under
    /// `meta["compatReport"]`, and §42.2 parks `meta["psd.*"]` carriage
    /// blobs — written through the engine's IO-style door
    /// ([`crate::engine::Engine::set_import_meta`]), persisted in
    /// document.json, and travelling with the document (clone/duplicate).
    /// The REGISTERED user-facing keys (§10.3 `meta.set`) stay envelope
    /// territory and are refused at the provenance door.
    meta: serde_json::Map<String, serde_json::Value>,
}

/// One document-profile entry (O-COLOR CORE-SIDE needs 1–2).
#[derive(Clone, Debug)]
pub struct DocProfile {
    /// appname-color content identity (`ProfileUid`).
    pub uid: appname_color::ProfileUid,
    pub source: DocProfileSource,
}

#[derive(Clone, Debug)]
pub enum DocProfileSource {
    /// A §5.1 builtin working space — no bytes to embed.
    Builtin(appname_color::BuiltinSpace),
    /// Embedded ICC bytes (round-trip fidelity: re-embedded on save, §7.5).
    Icc(std::sync::Arc<Vec<u8>>),
    /// The `doc.assign_profile{profile: null}` untagged state — presented
    /// as sRGB interim, nothing embedded on export (§5.3 note).
    Untagged,
    /// The CIELAB connection-space "profile" (`doc.mode_convert` to
    /// `lab_a`, §6.2/§5.3): built by the CMM (`Cms::builtin_lab`), no
    /// bytes to embed; seeded deterministically like the §5.1 builtins.
    BuiltinLab,
}

impl Document {
    /// Construct a document shell: validated geometry, a fresh undecorated
    /// root group, an inactive selection. `pub(crate)`: `doc.new` (slice 2)
    /// is the public door and passes engine-derived ids for replay
    /// determinism (ES §9.6).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn new(
        id: DocumentId,
        root_id: LayerId,
        geometry: DocumentGeometry,
        color: ColorState,
    ) -> Result<Document, GraphError> {
        geometry.validate()?;
        if !color.depth.is_storage_backed() {
            return Err(GraphError::DepthNotStorageBacked(color.depth));
        }
        let mut layers = Arena::new();
        layers
            .insert(LayerNode::new_group(root_id, "root", false))
            .map_err(GraphError::from)?;
        Ok(Document {
            id,
            geometry,
            color,
            root: root_id,
            layers,
            masks: Arena::new(),
            ops: Arena::new(),
            paths: Arena::new(),
            channels: Arena::new(),
            channel_order: Vec::new(),
            slices: Vec::new(),
            selection: Selection::new(),
            guides: Guides::default(),
            profiles: std::collections::BTreeMap::new(),
            meta: serde_json::Map::new(),
            selection_surface: Uuid::new_v5(
                &crate::engine::doc_new::APPNAME_NS,
                format!("surface/selection/{}", id.as_uuid()).as_bytes(),
            ),
        })
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn geometry(&self) -> &DocumentGeometry {
        &self.geometry
    }

    pub fn color(&self) -> &ColorState {
        &self.color
    }

    pub fn root(&self) -> LayerId {
        self.root
    }

    pub fn layer(&self, id: LayerId) -> Option<&LayerNode> {
        self.layers.get(id)
    }

    /// Node count including the root group.
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Bottom-to-top z-order of a group's direct children (ES §2.6: index 0
    /// composites first; the Layers panel displays the reverse).
    pub fn children_of(&self, id: LayerId) -> Option<&[LayerId]> {
        self.layers.get(id).map(|n| n.children())
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// §11.1/§40.1 effective lock set, THE one place (door-slice riding
    /// item, lock-consolidation): "Locks on a Group apply transitively to
    /// its subtree; the strictest lock along the ancestry wins" — read as
    /// the UNION of every flag from the node to the root (all flags
    /// transitive; strictest-wins = any ancestor's flag binds).
    pub fn effective_locks(&self, layer: LayerId) -> crate::graph::node::LockFlags {
        let mut flags = crate::graph::node::LockFlags::empty();
        let mut cur = Some(layer);
        while let Some(id) = cur {
            if let Some(n) = self.layer(id) {
                flags = flags.union(n.locks());
            }
            cur = self.parent_of(id);
        }
        flags
    }

    /// §9.6 global creation-id discipline (QA-F11 closure): a creation
    /// UUID must be absent from EVERY identity arena, not just the one it
    /// is destined for ("`newLayerId`/`newOpId` must be UUIDs not already
    /// present in any arena", §9.6 worked example; §40.1 stage-3
    /// `error.schema.duplicate_id`). Returns the arena that already holds
    /// `id`, if any — the caller builds the typed error.
    pub fn arena_holding(&self, id: Uuid) -> Option<&'static str> {
        if self.layers.contains(id) {
            return Some("layers");
        }
        if self.masks.contains(id) {
            return Some("masks");
        }
        if self.ops.contains(id) {
            return Some("ops");
        }
        if self.paths.contains(id) {
            return Some("paths");
        }
        if self.channels.contains(id) {
            return Some("channels");
        }
        None
    }

    pub fn mask(&self, id: crate::graph::id::MaskId) -> Option<&crate::graph::mask_node::MaskNode> {
        self.masks.get(id)
    }

    pub fn mask_count(&self) -> usize {
        self.masks.len()
    }

    /// Depth-first listing from `root` in composite order: children visited
    /// index-ascending (bottom first), each node listed before its own
    /// children. Deterministic; used by tests, dumps, and the slice-3
    /// writer.
    pub fn z_order_walk(&self) -> Vec<LayerId> {
        let mut out = Vec::with_capacity(self.layers.len().saturating_sub(1));
        let mut stack: Vec<LayerId> = Vec::new();
        if let Some(root) = self.layers.get(self.root) {
            stack.extend(root.children().iter().rev());
        }
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(n) = self.layers.get(id) {
                stack.extend(n.children().iter().rev());
            }
        }
        out
    }

    /// §4.6 step 1 EFFECTIVE visibility: a node contributes to the composite
    /// only if it is own-visible AND every ENCLOSING GROUP up to the root is
    /// visible. A hidden group hides its whole subtree (§4.7: a group
    /// composites its children as a unit — a hidden unit is not composited),
    /// so an own-visible leaf inside a hidden group does NOT composite. The
    /// render compositor enforces this STRUCTURALLY (it never descends into a
    /// hidden group's subtree); this predicate is the CPU-reference export's
    /// matching gate, so `z_order_walk` (which lists the whole tree) can be
    /// filtered to the exact set render composites — never a superset. The
    /// ROOT's own flag is not a gate (the composite walk seeds from the
    /// root's children, exactly as `z_order_walk` and the render compositor
    /// do), matching their structural behavior.
    pub fn effective_visible(&self, id: LayerId) -> bool {
        let Some(node) = self.layer(id) else {
            return false;
        };
        if !node.visible() {
            return false;
        }
        let mut cur = self.parent_of(id);
        while let Some(p) = cur {
            if p == self.root {
                break; // the root container's flag is never a gate
            }
            match self.layer(p) {
                Some(n) if n.visible() => {}
                _ => return false, // a hidden ancestor group hides the subtree
            }
            cur = self.parent_of(p);
        }
        true
    }

    /// Parent lookup, derived by scanning `children` lists — parent links are
    /// never stored (§2.7 single source of truth).
    pub fn parent_of(&self, id: LayerId) -> Option<LayerId> {
        self.layers
            .iter()
            .find(|n| n.children().contains(&id))
            .map(|n| n.id())
    }

    /// ES §2.7 `validate_tree`: (1) root exists, is a Group, undecorated;
    /// (2) iterative DFS with a visited set — revisit → `SharedChild` or
    /// `Cycle`, unresolved → `DanglingChild`; (3) non-Group with children;
    /// (4) mask references resolve (v1: any mask id is dangling — the arena
    /// arrives at CP-0.5); (5) visited count vs population → unreachable
    /// nodes.
    pub fn validate_tree(&self) -> Result<(), TreeError> {
        self.validate_tree_impl(true)
    }

    /// Door-assert variant: skips the ORPHAN checks (unreachable ops/masks
    /// are "a leak, not corruption" — §2.7), because multi-step
    /// construction (loader passes, multi-door kinds) is legitimately
    /// transient between doors. Full validation runs at load-end and at
    /// engine commit.
    pub(crate) fn validate_tree_doors(&self) -> Result<(), TreeError> {
        self.validate_tree_impl(false)
    }

    fn validate_tree_impl(&self, check_orphans: bool) -> Result<(), TreeError> {
        // 1. Root checks.
        let root = self.layers.get(self.root).ok_or(TreeError::MissingRoot)?;
        if !root.is_group() {
            return Err(TreeError::RootNotGroup);
        }
        let undecorated = root.masks().is_empty()
            && root.blend() == BlendMode::Normal
            && root.opacity() == 1.0
            && root.fill_opacity() == 1.0
            && root.locks() == LockFlags::empty()
            && !root.clip_to_below()
            && root.transform().is_identity();
        if !undecorated {
            return Err(TreeError::RootDecorated);
        }

        // 2. Iterative DFS (no recursion: depth is user-controlled, §2.7).
        // Two-phase stack entries mark enter/exit so "on the current path"
        // distinguishes Cycle from SharedChild.
        use std::collections::HashSet;
        let mut visited: HashSet<LayerId> = HashSet::new();
        let mut on_path: HashSet<LayerId> = HashSet::new();
        let mut mask_owner: HashSet<crate::graph::id::MaskId> = HashSet::new();
        let mut op_owner: HashSet<crate::graph::id::OpId> = HashSet::new();
        enum Step {
            Enter(LayerId, LayerId), // (parent, node)
            Exit(LayerId),
        }
        let mut stack = vec![Step::Enter(self.root, self.root)];
        while let Some(step) = stack.pop() {
            match step {
                Step::Enter(parent, id) => {
                    let node = match self.layers.get(id) {
                        Some(n) => n,
                        None => return Err(TreeError::DanglingChild { parent, child: id }),
                    };
                    if on_path.contains(&id) {
                        return Err(TreeError::Cycle { at: id });
                    }
                    if !visited.insert(id) {
                        return Err(TreeError::SharedChild { child: id });
                    }
                    on_path.insert(id);
                    // 3. Only groups have children.
                    if !node.is_group() && !node.children().is_empty() {
                        return Err(TreeError::NonGroupWithChildren { id });
                    }
                    // 4c. Op references resolve; single owner (§2.6 inv 3/7).
                    let op_refs: Vec<crate::graph::id::OpId> = node
                        .adjustment_op()
                        .into_iter()
                        .chain(
                            node.smart_object()
                                .into_iter()
                                .flat_map(|d| d.smart_filters().iter().copied()),
                        )
                        .collect();
                    for op_id in op_refs {
                        let Some(op) = self.ops.get(op_id) else {
                            return Err(TreeError::DanglingOp { owner: id });
                        };
                        if !op_owner.insert(op_id) {
                            return Err(TreeError::SharedOp { op: op_id });
                        }
                        if let Some(m) = op.mask() {
                            if !self.masks.contains(m) {
                                return Err(TreeError::DanglingOpMask { op: op_id });
                            }
                            if !mask_owner.insert(m) {
                                return Err(TreeError::SharedMask { mask: m });
                            }
                        }
                    }
                    // 4. Mask references resolve; §2.7 rule 6 single owner.
                    for m in node.masks() {
                        if !self.masks.contains(*m) {
                            return Err(TreeError::DanglingMask { owner: id });
                        }
                        if !mask_owner.insert(*m) {
                            return Err(TreeError::SharedMask { mask: *m });
                        }
                    }
                    stack.push(Step::Exit(id));
                    for child in node.children().iter().rev() {
                        stack.push(Step::Enter(id, *child));
                    }
                }
                Step::Exit(id) => {
                    on_path.remove(&id);
                }
            }
        }

        // 4b. Orphan masks (in the arena, referenced by no owner).
        if check_orphans && mask_owner.len() != self.masks.len() {
            let mut orphans: Vec<crate::graph::id::MaskId> = self
                .masks
                .iter()
                .map(|m| m.id())
                .filter(|m| !mask_owner.contains(m))
                .collect();
            orphans.sort_by_key(|m| m.as_uuid());
            return Err(TreeError::UnreachableMasks(orphans));
        }

        // 4d. Orphan ops (in the arena, referenced by no owner).
        if check_orphans && op_owner.len() != self.ops.len() {
            let mut orphans: Vec<crate::graph::id::OpId> = self
                .ops
                .iter()
                .map(|o| o.id())
                .filter(|o| !op_owner.contains(o))
                .collect();
            orphans.sort_by_key(|o| o.as_uuid());
            return Err(TreeError::UnreachableOps(orphans));
        }

        // 5. Unreachable nodes (a leak, not corruption — §2.7).
        if visited.len() != self.layers.len() {
            let mut orphans: Vec<LayerId> = self
                .layers
                .iter()
                .map(|n| n.id())
                .filter(|id| !visited.contains(id))
                .collect();
            orphans.sort_by_key(|l| l.as_uuid());
            return Err(TreeError::UnreachableNodes(orphans));
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Crate-internal mutation doors (plan rule 5 / D-48).
    // -----------------------------------------------------------------

    /// Insert `node` as a child of `parent` at z-index `index` (0 = bottom of
    /// the stack, ES §2.6). Returns the new id.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn insert_child(
        &mut self,
        parent: LayerId,
        index: usize,
        node: LayerNode,
    ) -> Result<LayerId, GraphError> {
        let id = node.id();
        {
            let p = self
                .layers
                .get(parent)
                .ok_or(GraphError::UnknownLayer(parent))?;
            if !p.is_group() {
                return Err(GraphError::NotAGroup(parent));
            }
            let len = p.children().len();
            if index > len {
                return Err(GraphError::IndexOutOfRange { len, index });
            }
        }
        self.layers.insert(node)?;
        let p = self.layers.get_mut(parent).expect("checked above");
        p.children.insert(index, id);
        debug_assert!(
            self.validate_tree_doors().is_ok(),
            "structural door corrupted the tree (§2.7): {:?}",
            self.validate_tree_doors()
        );
        Ok(id)
    }

    /// Remove a childless non-root node, returning it for undo re-insertion
    /// with its original UUID (ES §2.7). Subtree removal is orchestrated by
    /// the owning command (slice 2+), which walks leaves-first.
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn remove_layer(&mut self, id: LayerId) -> Result<LayerNode, GraphError> {
        if id == self.root {
            return Err(GraphError::RootImmutable);
        }
        let node = self.layers.get(id).ok_or(GraphError::UnknownLayer(id))?;
        if !node.children().is_empty() {
            return Err(GraphError::HasChildren(id));
        }
        let parent = self.parent_of(id);
        if let Some(pid) = parent {
            let p = self.layers.get_mut(pid).expect("parent resolves");
            p.children.retain(|c| *c != id);
        }
        let removed = self.layers.remove(id).expect("checked above");
        debug_assert!(
            self.validate_tree_doors().is_ok(),
            "structural door corrupted the tree (§2.7)"
        );
        Ok(removed)
    }

    /// Property/pixel mutation access to one node. Root is refused —
    /// commands MUST NOT target the root (§2.6 invariant 2).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Result<&mut LayerNode, GraphError> {
        if id == self.root {
            return Err(GraphError::RootImmutable);
        }
        self.layers.get_mut(id).ok_or(GraphError::UnknownLayer(id))
    }

    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn selection_mut(&mut self) -> &mut Selection {
        &mut self.selection
    }

    /// Attach a mask to a non-root layer (§2.6 stacking order = vec order;
    /// §2.7 rule 6 single ownership enforced by construction + validated).
    pub(crate) fn attach_mask(
        &mut self,
        layer: LayerId,
        mask: crate::graph::mask_node::MaskNode,
    ) -> Result<crate::graph::id::MaskId, GraphError> {
        let at = self.layers.get(layer).map(|n| n.masks().len()).unwrap_or(0);
        self.attach_mask_at(layer, at, mask)
    }

    /// Attach at an explicit §2.6 stacking index (undo re-attachment of a
    /// detached mask restores its recorded position).
    pub(crate) fn attach_mask_at(
        &mut self,
        layer: LayerId,
        index: usize,
        mask: crate::graph::mask_node::MaskNode,
    ) -> Result<crate::graph::id::MaskId, GraphError> {
        if layer == self.root {
            return Err(GraphError::RootImmutable);
        }
        let mask_id = mask.id();
        if !self.layers.contains(layer) {
            return Err(GraphError::UnknownLayer(layer));
        }
        self.masks.insert(mask).map_err(GraphError::from)?;
        let node = self.layers.get_mut(layer).expect("checked");
        let index = index.min(node.masks.len());
        node.masks.insert(index, mask_id);
        debug_assert!(
            self.validate_tree_doors().is_ok(),
            "attach_mask corrupted the tree (§2.7)"
        );
        Ok(mask_id)
    }

    /// The document's SELECTION surface id (§10.1: surfaces are disjoint
    /// UUID spaces — the selection mask buffer gets one deterministic id
    /// per document, derived, so replay agrees by construction).
    pub fn selection_surface(&self) -> Uuid {
        self.selection_surface
    }

    /// Re-derive `Selection::{bounds, active}` from the mask buffer after a
    /// delta restore touched the selection surface (§13.1 bounds
    /// maintenance; the buffer's recorded bounds are the restore's truth).
    pub(crate) fn sync_selection_bounds(&mut self) {
        let b = self.selection.mask().bounds();
        self.selection.set_bounds(b);
    }

    /// The §7.5 document profile store (read side).
    pub fn profile(&self, id: ProfileId) -> Option<&DocProfile> {
        self.profiles.get(&id)
    }

    pub fn profiles(&self) -> impl Iterator<Item = (&ProfileId, &DocProfile)> {
        self.profiles.iter()
    }

    /// True when any embedded (non-builtin) profile exists — the
    /// conditional-folding guard (mask precedent): builtin-only stores
    /// keep every pinned hash and document.json byte-identical.
    pub fn has_embedded_profiles(&self) -> bool {
        self.profiles
            .values()
            .any(|p| matches!(p.source, DocProfileSource::Icc(_)))
    }

    /// Register a profile entry (engine ingestion door + loader).
    pub(crate) fn register_profile(&mut self, id: ProfileId, entry: DocProfile) {
        self.profiles.insert(id, entry);
    }

    /// Retag the working profile (§5.3 assign — pure reinterpretation;
    /// the SetWorkingProfile op is the recorded inverse).
    pub(crate) fn set_working_profile(&mut self, id: ProfileId) {
        self.color.working = id;
    }

    /// Geometry swap for the doc.* kinds (O-TOOLS seam). The §19.1
    /// image==canvas construction invariant is RELAXED here for the
    /// crop-rect discipline (crop hides, never destroys — the caller owns
    /// semantic validity); canvas floor + finite-ppi still hold.
    pub(crate) fn set_geometry_recorded(&mut self, g: DocumentGeometry) {
        debug_assert!(
            g.canvas_size.x >= 1 && g.canvas_size.y >= 1,
            "§19.1 canvas floor"
        );
        debug_assert!(
            g.resolution_ppi.is_finite() && g.resolution_ppi > 0.0,
            "§19.1 ppi"
        );
        self.geometry = g;
    }

    #[doc(hidden)]
    pub fn _subtree_doc_anchor(&self) {}
}

/// §24.2 document furniture: sorted guide vectors + grid settings.
#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Guides {
    /// Horizontal guides: y positions, doc px, sub-pixel f64, SORTED.
    #[serde(rename = "horizontal", default, skip_serializing_if = "Vec::is_empty")]
    pub h: Vec<f64>,
    /// Vertical guides: x positions, SORTED.
    #[serde(rename = "vertical", default, skip_serializing_if = "Vec::is_empty")]
    pub v: Vec<f64>,
    #[serde(rename = "locked", default, skip_serializing_if = "std::ops::Not::not")]
    pub lock: bool,
    /// §24.2 GridSettings (persists inside the guides object — the §7.3
    /// $def carries no grid field yet; schema-gap flagged for fold-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridSettings>,
}

impl Guides {
    /// Anything to persist/fold? (Defaults keep pinned hashes + bytes.)
    pub fn is_default(&self) -> bool {
        self.h.is_empty() && self.v.is_empty() && !self.lock && self.grid.is_none()
    }
}

/// §29.2 export slice row (O-IO door 1 — the exact asked shape): name +
/// integer-px bounds + the §7.3 OPEN preset object (appname-io parses it
/// at export via `ExportPreset::from_wire`; core carries it verbatim).
/// Wire (`document.json` root "slices", schema-gap flagged — the paths
/// precedent): `{name, bounds: {x, y, w, h}, preset?}`.
#[derive(Clone, Debug, PartialEq)]
pub struct SliceRow {
    pub name: String,
    pub bounds: IRect,
    pub preset: Option<serde_json::Value>,
}

impl Serialize for SliceRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("name", &self.name)?;
        m.serialize_entry(
            "bounds",
            &serde_json::json!({
                "x": self.bounds.min.x,
                "y": self.bounds.min.y,
                "w": self.bounds.width(),
                "h": self.bounds.height(),
            }),
        )?;
        if let Some(p) = &self.preset {
            m.serialize_entry("preset", p)?;
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for SliceRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<SliceRow, D::Error> {
        use serde::de::Error;
        let v = serde_json::Value::deserialize(d)?;
        let name = v
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| D::Error::custom("slice row: `name` string required"))?
            .to_string();
        let b = v
            .get("bounds")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| D::Error::custom("slice row: `bounds` object required"))?;
        let geti = |k: &str| -> Result<i64, D::Error> {
            b.get(k)
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| D::Error::custom(format!("slice bounds: `{k}` integer required")))
        };
        let (x, y, w, h) = (geti("x")?, geti("y")?, geti("w")?, geti("h")?);
        if w < 1 || h < 1 {
            return Err(D::Error::custom("slice bounds: w/h must be >= 1 (§40.8)"));
        }
        Ok(SliceRow {
            name,
            bounds: IRect::new(x as i32, y as i32, x as i32 + w as i32, y as i32 + h as i32),
            preset: v.get("preset").cloned(),
        })
    }
}

/// §24.2 grid block (full-state on the wire, §10.3 grid.set).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GridSettings {
    pub spacing: f64,
    pub subdivisions: u32,
    pub show: bool,
    /// §22.1 color object, carried verbatim (display-only; drawn by the
    /// shell through the display transform).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<serde_json::Value>,
}

/// A detached subtree: the node, its masks, and its children in z-order —
/// re-attachable losslessly (§2.7 same-UUID rule; tiles ride the Arcs,
/// zero pixel copies).
#[derive(Clone)]
pub struct SubtreeFragment {
    pub(crate) node: LayerNode,
    pub(crate) masks: Vec<crate::graph::mask_node::MaskNode>,
    pub(crate) children: Vec<SubtreeFragment>,
}

impl Document {
    /// Detach a whole subtree leaves-first (`layer.delete`; root refused).
    pub(crate) fn detach_subtree(&mut self, id: LayerId) -> Result<SubtreeFragment, GraphError> {
        if id == self.root {
            return Err(GraphError::RootImmutable);
        }
        let child_ids: Vec<LayerId> = self.children_of(id).map(|c| c.to_vec()).unwrap_or_default();
        let mut children = Vec::with_capacity(child_ids.len());
        for c in child_ids {
            children.push(self.detach_subtree(c)?);
        }
        let mask_ids: Vec<_> = self
            .layer(id)
            .map(|n| n.masks().to_vec())
            .unwrap_or_default();
        let mut masks = Vec::with_capacity(mask_ids.len());
        for m in mask_ids {
            masks.push(self.detach_mask(id, m)?);
        }
        let node = self.remove_layer(id)?;
        Ok(SubtreeFragment {
            node,
            masks,
            children,
        })
    }

    /// Re-attach a detached fragment at `(parent, index)` (§2.7: the SAME
    /// nodes re-insert with their recorded ids).
    pub(crate) fn attach_subtree(
        &mut self,
        parent: LayerId,
        index: usize,
        frag: SubtreeFragment,
    ) -> Result<LayerId, GraphError> {
        let id = frag.node.id();
        self.insert_child(parent, index, frag.node)?;
        for m in frag.masks {
            self.attach_mask(id, m)?;
        }
        for (i, c) in frag.children.into_iter().enumerate() {
            self.attach_subtree(id, i, c)?;
        }
        Ok(id)
    }

    /// Move a node to `(parent, index)` (layer.reorder / group membership).
    /// Refuses the root, unknown endpoints, non-group parents, and moves
    /// into the moved node's own subtree (cycle).
    pub(crate) fn move_child(
        &mut self,
        layer: LayerId,
        to_parent: LayerId,
        index: usize,
    ) -> Result<(), GraphError> {
        if layer == self.root {
            return Err(GraphError::RootImmutable);
        }
        if !self.layers.contains(layer) {
            return Err(GraphError::UnknownLayer(layer));
        }
        // Cycle check: destination must not be within the moved subtree.
        let mut probe = Some(to_parent);
        while let Some(p) = probe {
            if p == layer {
                return Err(GraphError::UnknownLayer(to_parent)); // destination inside subtree
            }
            probe = self.parent_of(p);
        }
        {
            let p = self
                .layers
                .get(to_parent)
                .ok_or(GraphError::UnknownLayer(to_parent))?;
            if !p.is_group() {
                return Err(GraphError::NotAGroup(to_parent));
            }
        }
        let from_parent = self
            .parent_of(layer)
            .ok_or(GraphError::UnknownLayer(layer))?;
        let fp = self.layers.get_mut(from_parent).expect("parent resolves");
        fp.children.retain(|c| *c != layer);
        let tp = self.layers.get_mut(to_parent).expect("checked above");
        let index = index.min(tp.children.len());
        tp.children.insert(index, layer);
        debug_assert!(
            self.validate_tree_doors().is_ok(),
            "move_child corrupted the tree (§2.7)"
        );
        Ok(())
    }

    /// Read the §2.6 shared prop block.
    pub fn props_block(&self, id: LayerId) -> Option<crate::engine::delta::LayerPropsBlock> {
        let n = self.layer(id)?;
        Some(crate::engine::delta::LayerPropsBlock {
            name: n.name().to_string(),
            blend: n.blend(),
            opacity: n.opacity(),
            fill_opacity: n.fill_opacity(),
            visible: n.visible(),
            locks: n.locks(),
            clip_to_below: n.clip_to_below(),
            color_label: n.color_label(),
            transform: *n.transform(),
            isolated: n.isolated(),
        })
    }

    /// Apply a full §2.6 shared prop block (SetLayerProps restore + kinds).
    pub(crate) fn apply_props_block(
        &mut self,
        id: LayerId,
        b: &crate::engine::delta::LayerPropsBlock,
    ) -> Result<(), GraphError> {
        let n = self.layer_mut(id)?;
        n.apply_props_block(b);
        Ok(())
    }

    pub fn op(&self, id: crate::graph::id::OpId) -> Option<&crate::graph::op_node::OpNode> {
        self.ops.get(id)
    }

    // --- §20 doc-path arena (TEXTVEC Door 2) -------------------------------

    pub fn path(&self, id: crate::graph::id::PathId) -> Option<&crate::graph::path_node::PathNode> {
        self.paths.get(id)
    }

    pub fn path_count(&self) -> usize {
        self.paths.len()
    }

    /// Deterministic enumeration (sorted by uuid — the arena itself is
    /// unordered; persistence and hashing both need a stable order).
    pub fn path_ids_sorted(&self) -> Vec<crate::graph::id::PathId> {
        let mut ids: Vec<crate::graph::id::PathId> = self.paths.iter().map(|p| p.id()).collect();
        ids.sort_by_key(|p| p.as_uuid());
        ids
    }

    /// Every layer whose VECTOR CONTENT references `id` — shape layers by
    /// base path or `bool_ops` operand (vector masks join when the mask
    /// arm lands). Feeds `vector.path_delete`'s target_in_use check and
    /// `vector.path_edit`'s dirty rule (§40.7).
    pub fn path_referencing_layers(&self, id: crate::graph::id::PathId) -> Vec<LayerId> {
        self.z_order_walk()
            .into_iter()
            .filter(|lid| {
                self.layer(*lid)
                    .and_then(|n| n.shape_data())
                    .map(|s| s.path() == id || s.bool_ops().iter().any(|(_, p)| *p == id))
                    .unwrap_or(false)
            })
            .collect()
    }

    pub(crate) fn path_mut(
        &mut self,
        id: crate::graph::id::PathId,
    ) -> Option<&mut crate::graph::path_node::PathNode> {
        self.paths.get_mut(id)
    }

    pub(crate) fn insert_path(
        &mut self,
        path: crate::graph::path_node::PathNode,
    ) -> Result<crate::graph::id::PathId, GraphError> {
        let id = path.id();
        self.paths.insert(path).map_err(GraphError::from)?;
        Ok(id)
    }

    pub(crate) fn remove_path(
        &mut self,
        id: crate::graph::id::PathId,
    ) -> Option<crate::graph::path_node::PathNode> {
        self.paths.remove(id)
    }

    /// Apply a full §20.3 shape-payload value block (SetShapeData delta).
    pub(crate) fn apply_shape_data_block(
        &mut self,
        layer: LayerId,
        b: &crate::engine::delta::ShapeDataBlock,
    ) -> Result<(), GraphError> {
        let node = self.layer_mut(layer)?;
        let sd = node
            .shape_data_mut()
            .ok_or(GraphError::UnknownLayer(layer))?;
        sd.fill = b.fill.clone();
        sd.stroke = b.stroke.clone();
        sd.bool_ops = b.bool_ops.clone();
        Ok(())
    }

    /// Apply a full §23.1 text-payload value block (SetTextData delta —
    /// TEXTVEC Door 7).
    pub(crate) fn apply_text_data_block(
        &mut self,
        layer: LayerId,
        b: &crate::graph::node::TextLayerData,
    ) -> Result<(), GraphError> {
        let node = self.layer_mut(layer)?;
        let td = node
            .text_data_mut()
            .ok_or(GraphError::UnknownLayer(layer))?;
        *td = b.clone();
        Ok(())
    }

    /// Apply one side of a `text.to_shape` kind swap (SetContentKind
    /// delta): the layer's content kind replaces wholesale, value-complete
    /// (§40.7 — undo restores the Text node with editability intact).
    pub(crate) fn apply_content_kind_block(
        &mut self,
        layer: LayerId,
        b: &crate::engine::delta::ContentKindBlock,
    ) -> Result<(), GraphError> {
        let kind = match b {
            crate::engine::delta::ContentKindBlock::Text(d) => {
                crate::graph::node::LayerKind::Text(d.clone())
            }
            crate::engine::delta::ContentKindBlock::Shape(s) => {
                crate::graph::node::LayerKind::Shape(crate::graph::node::ShapeLayerData {
                    path: s.path,
                    fill: s.fill.clone(),
                    stroke: s.stroke.clone(),
                    bool_ops: s.bool_ops.clone(),
                })
            }
        };
        let node = self.layer_mut(layer)?;
        node.set_content_kind(kind);
        Ok(())
    }

    pub fn op_count(&self) -> usize {
        self.ops.len()
    }

    pub(crate) fn op_mut(
        &mut self,
        id: crate::graph::id::OpId,
    ) -> Option<&mut crate::graph::op_node::OpNode> {
        self.ops.get_mut(id)
    }

    pub(crate) fn insert_op(
        &mut self,
        op: crate::graph::op_node::OpNode,
    ) -> Result<crate::graph::id::OpId, GraphError> {
        let id = op.id();
        self.ops.insert(op).map_err(GraphError::from)?;
        Ok(id)
    }

    /// Insert a mask owned by an OP (§12.2 smart-filter mask; the op's
    /// `mask` field is the owner reference validate_tree checks).
    pub(crate) fn attach_op_mask(
        &mut self,
        _op: crate::graph::id::OpId,
        mask: crate::graph::mask_node::MaskNode,
    ) -> Result<crate::graph::id::MaskId, GraphError> {
        let id = mask.id();
        self.masks.insert(mask).map_err(GraphError::from)?;
        Ok(id)
    }

    /// Detach + remove an op's mask node (op.set_mask from:'none').
    pub(crate) fn detach_op_mask(
        &mut self,
        id: crate::graph::id::MaskId,
    ) -> Option<crate::graph::mask_node::MaskNode> {
        self.masks.remove(id)
    }

    /// Move an op within its smart-filter chain (§12.2 op.reorder).
    pub(crate) fn reorder_smart_filter(
        &mut self,
        layer: LayerId,
        op: crate::graph::id::OpId,
        index: usize,
    ) -> Result<(), GraphError> {
        let node = self.layer_mut(layer)?;
        let so = node
            .smart_object_mut()
            .ok_or(GraphError::UnknownLayer(layer))?;
        let cur = so
            .smart_filters
            .iter()
            .position(|o| *o == op)
            .ok_or(GraphError::UnknownLayer(layer))?;
        let o = so.smart_filters.remove(cur);
        let index = index.min(so.smart_filters.len());
        so.smart_filters.insert(index, o);
        Ok(())
    }

    pub(crate) fn insert_smart_filter_ref(
        &mut self,
        layer: LayerId,
        op: crate::graph::id::OpId,
        index: usize,
    ) -> Result<(), GraphError> {
        let node = self.layer_mut(layer)?;
        let so = node
            .smart_object_mut()
            .ok_or(GraphError::UnknownLayer(layer))?;
        let index = index.min(so.smart_filters.len());
        so.smart_filters.insert(index, op);
        Ok(())
    }

    pub(crate) fn remove_smart_filter_ref(
        &mut self,
        layer: LayerId,
        op: crate::graph::id::OpId,
    ) -> Result<(), GraphError> {
        let node = self.layer_mut(layer)?;
        let so = node
            .smart_object_mut()
            .ok_or(GraphError::UnknownLayer(layer))?;
        so.smart_filters.retain(|o| *o != op);
        Ok(())
    }

    /// Swap a smart object's embedded source (§11.5 edit_contents/replace).
    pub(crate) fn set_smart_source(
        &mut self,
        layer: LayerId,
        asset: Uuid,
    ) -> Result<(), GraphError> {
        let node = self.layer_mut(layer)?;
        let so = node
            .smart_object_mut()
            .ok_or(GraphError::UnknownLayer(layer))?;
        so.source = crate::graph::node::SmartSource::Embedded(asset);
        Ok(())
    }

    pub(crate) fn remove_op(
        &mut self,
        id: crate::graph::id::OpId,
    ) -> Option<crate::graph::op_node::OpNode> {
        self.ops.remove(id)
    }

    pub fn guides(&self) -> &Guides {
        &self.guides
    }

    /// Value-delta door (SetGuides restore + the guide/grid kinds).
    pub(crate) fn set_guides(&mut self, g: Guides) {
        self.guides = g;
    }

    /// The §7.3 open document-meta object (read surface — the badge/
    /// report consumers of §42.4 read `meta["compatReport"]` here).
    pub fn meta(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.meta
    }

    /// Loader/provenance door (package `meta` object + the engine's
    /// [`crate::engine::Engine::set_import_meta`]).
    pub(crate) fn meta_mut(&mut self) -> &mut serde_json::Map<String, serde_json::Value> {
        &mut self.meta
    }

    /// Every raster surface UUID in the document (§40.2 carried-selection
    /// rule: resize touches layer pixels, mask coverages, AND the
    /// selection with the same kernel) — z-order layers, then masks in
    /// attachment order, then named channels in §14.1 list order (they
    /// are document-dimensioned coverage planes exactly like the
    /// selection; door-slice-3 addition, flagged to O-TOOLS in the lane
    /// log), then the selection surface.
    pub fn raster_surfaces(&self) -> Vec<Uuid> {
        let mut out = Vec::new();
        for id in self.z_order_walk() {
            if let Some(n) = self.layer(id) {
                if n.pixels().is_some() {
                    out.push(id.as_uuid());
                }
                for m in n.masks() {
                    out.push(m.as_uuid());
                }
            }
        }
        for cid in &self.channel_order {
            out.push(cid.as_uuid());
        }
        out.push(self.selection_surface);
        out
    }

    // --- §14.1 named-channel arena (door C1) ---------------------------

    pub fn channel(
        &self,
        id: crate::graph::id::ChannelId,
    ) -> Option<&crate::graph::channel_node::NamedChannel> {
        self.channels.get(id)
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// §14.1 list order (creation order = list order; no reorder kind in
    /// v1.0). Persistence, hashing, and the panel all read THIS order.
    pub fn channel_ids_ordered(&self) -> &[crate::graph::id::ChannelId] {
        &self.channel_order
    }

    pub(crate) fn channel_mut(
        &mut self,
        id: crate::graph::id::ChannelId,
    ) -> Option<&mut crate::graph::channel_node::NamedChannel> {
        self.channels.get_mut(id)
    }

    /// Append a named channel (creation order = list order, §14.1).
    pub(crate) fn insert_channel(
        &mut self,
        channel: crate::graph::channel_node::NamedChannel,
    ) -> Result<crate::graph::id::ChannelId, GraphError> {
        let id = channel.id();
        self.channels.insert(channel).map_err(GraphError::from)?;
        self.channel_order.push(id);
        Ok(id)
    }

    /// Re-insert a removed channel at its recorded list index (undo of
    /// channel.delete — the SAME node, tiles riding, §2.7 analogue).
    pub(crate) fn insert_channel_at(
        &mut self,
        index: usize,
        channel: crate::graph::channel_node::NamedChannel,
    ) -> Result<crate::graph::id::ChannelId, GraphError> {
        let id = channel.id();
        self.channels.insert(channel).map_err(GraphError::from)?;
        let index = index.min(self.channel_order.len());
        self.channel_order.insert(index, id);
        Ok(id)
    }

    pub(crate) fn remove_channel(
        &mut self,
        id: crate::graph::id::ChannelId,
    ) -> Option<(usize, crate::graph::channel_node::NamedChannel)> {
        let index = self.channel_order.iter().position(|c| *c == id)?;
        let node = self.channels.remove(id)?;
        self.channel_order.remove(index);
        Some((index, node))
    }

    // --- §29.2 slice storage (export-subsystem door 1) -----------------

    /// The document's export slices, §14.1-list-style creation order,
    /// index-addressed (§10.3).
    pub fn slices(&self) -> &[SliceRow] {
        &self.slices
    }

    /// Command door (slice.* kinds; the guides full-state pattern).
    pub(crate) fn slices_mut(&mut self) -> &mut Vec<SliceRow> {
        &mut self.slices
    }

    /// §6.2 mode re-tag (door C5; CheckpointOnly — the class-C snapshot
    /// is the inverse, so this is a plain field write; the converted
    /// buffers land through the checkpoint layer-pixels door).
    pub(crate) fn set_color_mode(&mut self, mode: ColorModelKind) {
        self.color.mode = mode;
    }

    /// §6.3 depth conversion over every raster layer (CheckpointOnly —
    /// snapshot inverse; masks/selection stay f32 regardless, §13.1).
    /// Integer↔integer converts numerically (lossless promotion per the
    /// §6.3 formulas through the storage-normalized domain); integer↔f32
    /// crosses the TRC storage boundary under the CORRECTED §6.4 posture
    /// (the fix-round ruling): storage is PREMULTIPLIED-normalized at
    /// EVERY depth, so the f32→integer arm unpremultiplies, encodes the
    /// straight component, and re-premultiplies (`enc(c/a)·a` — c ≤ a by
    /// construction; the first cut's bare `enc(c)` against untouched
    /// linear alpha was the same §6.4 mint the mode-convert/apply-image
    /// fix closed, found by review). The integer→f32 decode arm keeps
    /// its per-component `dec(c)` shape: door-safe (dec contracts on
    /// [0, 1], so c ≤ a survives) and byte-exact on the opaque lattice
    /// (§34.4 property-2, which is stated there); for fractional-alpha
    /// pixels the pair is knowingly asymmetric — recorded, with the §5
    /// linear-unification wave as the retirement point.
    pub(crate) fn convert_depth(&mut self, to: ChannelKind) {
        use crate::io::srgb::{srgb_decode, srgb_encode};
        let from = self.color.depth;
        if from == to {
            return;
        }
        let int_pair = from != ChannelKind::F32 && to != ChannelKind::F32;
        let ids: Vec<LayerId> = self
            .z_order_walk()
            .into_iter()
            .filter(|id| {
                self.layer(*id)
                    .map(|n| n.pixels().is_some())
                    .unwrap_or(false)
            })
            .collect();
        for id in ids {
            let node = self.layers.get_mut(id).expect("walked");
            let src = node.pixels().expect("raster").clone();
            let mut dst = crate::buffer::Buffer::new(to, src.model(), src.profile());
            let b = src.bounds();
            for y in b.min.y..b.max.y {
                for x in b.min.x..b.max.x {
                    let p = IVec2::new(x, y);
                    let v = src.read_pixel_f32(p);
                    let out = if int_pair {
                        v // storage-normalized passthrough (§6.3 numeric)
                    } else if to == ChannelKind::F32 {
                        [
                            srgb_decode(v[0]),
                            srgb_decode(v[1]),
                            srgb_decode(v[2]),
                            v[3],
                        ]
                    } else {
                        // f32 → integer: premul-normalized marshalling
                        // (the §6.4 fix-round ruling; see the doc
                        // comment). a = 0 stays canonical all-zero.
                        let a = v[3];
                        if a > 0.0 {
                            let inv = 1.0 / a;
                            [
                                (srgb_encode((v[0] * inv).clamp(0.0, 1.0)) * a).clamp(0.0, a),
                                (srgb_encode((v[1] * inv).clamp(0.0, 1.0)) * a).clamp(0.0, a),
                                (srgb_encode((v[2] * inv).clamp(0.0, 1.0)) * a).clamp(0.0, a),
                                a,
                            ]
                        } else {
                            [0.0; 4]
                        }
                    };
                    if out.iter().any(|c| *c != 0.0) {
                        dst.write_pixel_f32(p, out);
                    }
                }
            }
            dst.set_bounds_recorded(b);
            node.set_pixels(dst);
        }
        self.color.depth = to;
    }

    /// Detach + remove a mask (undo of attach re-inserts the same node).
    pub(crate) fn detach_mask(
        &mut self,
        layer: LayerId,
        mask: crate::graph::id::MaskId,
    ) -> Result<crate::graph::mask_node::MaskNode, GraphError> {
        let node = self
            .layers
            .get_mut(layer)
            .ok_or(GraphError::UnknownLayer(layer))?;
        node.masks.retain(|m| *m != mask);
        let removed = self
            .masks
            .remove(mask)
            .ok_or(GraphError::UnknownLayer(layer))?;
        debug_assert!(
            self.validate_tree_doors().is_ok(),
            "detach_mask corrupted the tree (§2.7)"
        );
        Ok(removed)
    }

    pub(crate) fn mask_mut(
        &mut self,
        id: crate::graph::id::MaskId,
    ) -> Option<&mut crate::graph::mask_node::MaskNode> {
        self.masks.get_mut(id)
    }
}

/// Summary Debug (ES §1.6 discipline; nodes summarize themselves).
impl core::fmt::Debug for Document {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Document")
            .field("id", &self.id)
            .field("root", &self.root)
            .field("layers", &self.layers.len())
            .field("geometry", &self.geometry)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::mask::Selection;

    fn color_state() -> ColorState {
        ColorState {
            working: ProfileId::nil(),
            mode: ColorModelKind::Rgba,
            depth: ChannelKind::U8,
            rendering_intent: None,
            black_point_compensation: None,
        }
    }

    fn new_doc() -> Document {
        Document::new(
            DocumentId::new(),
            LayerId::new(),
            DocumentGeometry::new(640, 480),
            color_state(),
        )
        .unwrap()
    }

    fn raster(doc: &Document, name: &str) -> LayerNode {
        LayerNode::new_raster(
            LayerId::new(),
            name,
            Buffer::new(doc.color().depth, doc.color().mode, doc.color().working),
        )
    }

    #[test]
    fn fresh_document_validates_and_is_empty() {
        let doc = new_doc();
        assert!(doc.validate_tree().is_ok());
        assert_eq!(doc.layer_count(), 1); // the root group
        assert!(doc.children_of(doc.root()).unwrap().is_empty());
        assert!(!doc.selection().active());
        assert_eq!(doc.z_order_walk(), Vec::<LayerId>::new());
    }

    #[test]
    fn geometry_invariants() {
        assert!(DocumentGeometry::new(1, 1).validate().is_ok());
        assert!(DocumentGeometry::new(0, 100).validate().is_err());
        let mut g = DocumentGeometry::new(10, 10);
        g.resolution_ppi = 0.0;
        assert!(g.validate().is_err());
        let mut g2 = DocumentGeometry::new(10, 10);
        g2.image_size = IVec2::new(5, 5);
        assert!(g2.validate().is_err());
    }

    #[test]
    fn geometry_and_color_serialize_to_schema_shape() {
        let g = DocumentGeometry::new(1920, 1080);
        let v = serde_json::to_value(g).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "imageSize": [1920, 1080],
                "canvasSize": [1920, 1080],
                "resolutionPPI": 72.0
            })
        );
        let c = ColorState {
            rendering_intent: Some(RenderingIntent::RelativeColorimetric),
            ..color_state()
        };
        let cv = serde_json::to_value(c).unwrap();
        assert_eq!(cv["mode"], "rgba");
        assert_eq!(cv["depth"], "u8");
        assert_eq!(cv["renderingIntent"], "relativeColorimetric");
        assert_eq!(cv["working"], "00000000-0000-0000-0000-000000000000");
        assert!(
            cv.get("blackPointCompensation").is_none(),
            "None options stay off the wire"
        );
    }

    #[test]
    fn resolution_ppi_wire_spelling_is_the_schema_spelling_both_ways() {
        // document.v1.json geometry requires exactly "resolutionPPI"
        // (schema authority per the §2.8 precedence note); the camelCase
        // default would wrongly emit "resolutionPpi".
        let g = DocumentGeometry::new(64, 64);
        let text = serde_json::to_string(&g).unwrap();
        assert!(
            text.contains("\"resolutionPPI\":"),
            "serialized form: {text}"
        );
        assert!(!text.contains("resolutionPpi"), "serialized form: {text}");

        // Deserialization accepts the schema spelling…
        let back: DocumentGeometry = serde_json::from_str(
            "{\"imageSize\":[64,64],\"canvasSize\":[64,64],\"resolutionPPI\":300.0}",
        )
        .unwrap();
        assert_eq!(back.resolution_ppi, 300.0);
        // …and the non-schema spelling does not satisfy the required field.
        let wrong: Result<DocumentGeometry, _> = serde_json::from_str(
            "{\"imageSize\":[64,64],\"canvasSize\":[64,64],\"resolutionPpi\":300.0}",
        );
        assert!(wrong.is_err());
    }

    #[test]
    fn children_order_is_bottom_to_top_z_order() {
        let mut doc = new_doc();
        let root = doc.root();
        let bottom = doc.insert_child(root, 0, raster(&doc, "bottom")).unwrap();
        let top = doc.insert_child(root, 1, raster(&doc, "top")).unwrap();
        // Insert between them.
        let middle = doc.insert_child(root, 1, raster(&doc, "middle")).unwrap();
        assert_eq!(doc.children_of(root).unwrap(), &[bottom, middle, top]);
        assert_eq!(doc.z_order_walk(), vec![bottom, middle, top]);
        // Panel order is the reverse of composite order.
        let panel: Vec<LayerId> = doc.z_order_walk().into_iter().rev().collect();
        assert_eq!(panel, vec![top, middle, bottom]);
    }

    #[test]
    fn z_order_walk_descends_groups_depth_first() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        let g = doc
            .insert_child(root, 1, LayerNode::new_group(LayerId::new(), "g", false))
            .unwrap();
        let g1 = doc.insert_child(g, 0, raster(&doc, "g1")).unwrap();
        let g2 = doc.insert_child(g, 1, raster(&doc, "g2")).unwrap();
        let b = doc.insert_child(root, 2, raster(&doc, "b")).unwrap();
        assert_eq!(doc.z_order_walk(), vec![a, g, g1, g2, b]);
        assert_eq!(doc.parent_of(g1), Some(g));
        assert_eq!(doc.parent_of(a), Some(root));
        assert_eq!(doc.parent_of(root), None);
    }

    #[test]
    fn insert_rejects_bad_targets() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        // Raster is not a group.
        let e = doc.insert_child(a, 0, raster(&doc, "x")).unwrap_err();
        assert_eq!(e, GraphError::NotAGroup(a));
        // Index past the end.
        let e = doc.insert_child(root, 5, raster(&doc, "x")).unwrap_err();
        assert_eq!(e, GraphError::IndexOutOfRange { len: 1, index: 5 });
        // Unknown parent.
        let ghost = LayerId::new();
        let e = doc.insert_child(ghost, 0, raster(&doc, "x")).unwrap_err();
        assert_eq!(e, GraphError::UnknownLayer(ghost));
        assert!(doc.validate_tree().is_ok());
    }

    #[test]
    fn remove_and_undo_reinsertion_keep_uuid_identity() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        let b = doc.insert_child(root, 1, raster(&doc, "b")).unwrap();

        let removed = doc.remove_layer(a).unwrap();
        assert_eq!(removed.id(), a);
        assert!(doc.layer(a).is_none());
        assert_eq!(doc.children_of(root).unwrap(), &[b]);

        // Undo re-inserts the *same* node id at its old position (§2.7).
        let back = doc.insert_child(root, 0, removed).unwrap();
        assert_eq!(back, a);
        assert_eq!(doc.children_of(root).unwrap(), &[a, b]);
        assert!(doc.validate_tree().is_ok());
    }

    #[test]
    fn root_is_immutable_and_undeletable() {
        let mut doc = new_doc();
        let root = doc.root();
        assert_eq!(
            doc.remove_layer(root).unwrap_err(),
            GraphError::RootImmutable
        );
        assert_eq!(doc.layer_mut(root).unwrap_err(), GraphError::RootImmutable);
    }

    #[test]
    fn remove_refuses_non_empty_groups() {
        let mut doc = new_doc();
        let root = doc.root();
        let g = doc
            .insert_child(root, 0, LayerNode::new_group(LayerId::new(), "g", false))
            .unwrap();
        doc.insert_child(g, 0, raster(&doc, "inner")).unwrap();
        assert_eq!(doc.remove_layer(g).unwrap_err(), GraphError::HasChildren(g));
    }

    #[test]
    fn duplicate_node_id_is_refused() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        let dup = LayerNode::new_raster(
            a,
            "clone",
            Buffer::new(ChannelKind::U8, ColorModelKind::Rgba, ProfileId::nil()),
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            doc.insert_child(root, 0, dup)
        }));
        match result {
            Err(_) => {} // debug_assert in Arena::insert fired (debug builds)
            Ok(r) => assert_eq!(r.unwrap_err(), GraphError::DuplicateId(a)),
        }
        assert!(
            doc.validate_tree().is_ok(),
            "failed insert must not corrupt the tree"
        );
        assert_eq!(doc.children_of(root).unwrap(), &[a]);
    }

    // ---- corrupt-tree detection (fields are module-visible to tests) ----

    #[test]
    fn validate_detects_dangling_child() {
        let mut doc = new_doc();
        let root = doc.root();
        let ghost = LayerId::new();
        doc.layers.get_mut(root).unwrap().children.push(ghost);
        assert_eq!(
            doc.validate_tree().unwrap_err(),
            TreeError::DanglingChild {
                parent: root,
                child: ghost
            }
        );
    }

    #[test]
    fn validate_detects_shared_child() {
        let mut doc = new_doc();
        let root = doc.root();
        let g = doc
            .insert_child(root, 0, LayerNode::new_group(LayerId::new(), "g", false))
            .unwrap();
        let a = doc.insert_child(root, 1, raster(&doc, "a")).unwrap();
        // Corrupt: a appears in two children lists.
        doc.layers.get_mut(g).unwrap().children.push(a);
        assert_eq!(
            doc.validate_tree().unwrap_err(),
            TreeError::SharedChild { child: a }
        );
    }

    #[test]
    fn validate_detects_cycle() {
        let mut doc = new_doc();
        let root = doc.root();
        let g1 = doc
            .insert_child(root, 0, LayerNode::new_group(LayerId::new(), "g1", false))
            .unwrap();
        let g2 = doc
            .insert_child(g1, 0, LayerNode::new_group(LayerId::new(), "g2", false))
            .unwrap();
        // Corrupt: g2 points back at g1.
        doc.layers.get_mut(g2).unwrap().children.push(g1);
        assert_eq!(
            doc.validate_tree().unwrap_err(),
            TreeError::Cycle { at: g1 }
        );
    }

    #[test]
    fn validate_detects_non_group_with_children() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        let b = doc.insert_child(root, 1, raster(&doc, "b")).unwrap();
        doc.layers.get_mut(a).unwrap().children.push(b);
        // The DFS hits NonGroupWithChildren before SharedChild for `a` (a is
        // visited first at index 0).
        assert_eq!(
            doc.validate_tree().unwrap_err(),
            TreeError::NonGroupWithChildren { id: a }
        );
    }

    #[test]
    fn validate_detects_decorated_root_and_unreachable_nodes() {
        let mut doc = new_doc();
        let root = doc.root();
        doc.layers.get_mut(root).unwrap().blend = BlendMode::Multiply;
        assert_eq!(doc.validate_tree().unwrap_err(), TreeError::RootDecorated);
        doc.layers.get_mut(root).unwrap().blend = BlendMode::Normal;

        // An island: node in the arena, in no children list.
        let orphan = raster(&doc, "orphan");
        let orphan_id = orphan.id();
        doc.layers.insert(orphan).unwrap();
        assert_eq!(
            doc.validate_tree().unwrap_err(),
            TreeError::UnreachableNodes(vec![orphan_id])
        );
    }

    #[test]
    fn validate_detects_dangling_mask_reference() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        doc.layers
            .get_mut(a)
            .unwrap()
            .masks
            .push(crate::graph::id::MaskId::new());
        assert_eq!(
            doc.validate_tree().unwrap_err(),
            TreeError::DanglingMask { owner: a }
        );
    }

    #[test]
    fn selection_slot_is_part_of_the_document() {
        let mut doc = new_doc();
        assert!(!doc.selection().active());
        let sel: &mut Selection = doc.selection_mut();
        sel.mask_mut()
            .write_pixel_f32(IVec2::new(3, 3), [1.0, 0.0, 0.0, 0.0]);
        let b = sel.mask().bounds();
        sel.set_bounds(b);
        assert!(doc.selection().active());
        assert_eq!(doc.selection().bounds(), IRect::new(3, 3, 4, 4));
    }

    #[test]
    fn layer_pixels_flow_through_the_document_doors() {
        let mut doc = new_doc();
        let root = doc.root();
        let a = doc.insert_child(root, 0, raster(&doc, "a")).unwrap();
        let h0 = doc.layer(a).unwrap().pixels().unwrap().region_hash();
        doc.layer_mut(a)
            .unwrap()
            .pixels_mut()
            .unwrap()
            .fill_rect(IRect::new(0, 0, 64, 64), [0.25, 0.25, 0.25, 1.0]);
        let h1 = doc.layer(a).unwrap().pixels().unwrap().region_hash();
        assert_ne!(h0, h1);
        assert_eq!(
            doc.layer(a).unwrap().pixels().unwrap().bounds(),
            IRect::new(0, 0, 64, 64)
        );
    }

    #[test]
    fn document_refuses_paused_depths() {
        let bad = ColorState {
            depth: ChannelKind::F16,
            ..color_state()
        };
        let e = Document::new(
            DocumentId::new(),
            LayerId::new(),
            DocumentGeometry::new(8, 8),
            bad,
        )
        .unwrap_err();
        assert_eq!(e, GraphError::DepthNotStorageBacked(ChannelKind::F16));
    }

    #[test]
    fn crop_rect_validation_follows_the_derived_image_size_rule() {
        // §19.1: image_size == crop_rect.size while active, else canvas.
        let mut g = DocumentGeometry::new(200, 100);
        assert!(g.validate().is_ok());
        g.crop_rect = Some(crate::geom::IRect::new(10, 10, 110, 60));
        assert!(g.validate().is_err(), "stale derived image_size refuses");
        g.image_size = IVec2::new(100, 50);
        assert!(g.validate().is_ok(), "recomputed derived state validates");
        g.crop_rect = Some(crate::geom::IRect::new(5, 5, 5, 40));
        assert!(g.validate().is_err(), "empty crop refuses");
        g.crop_rect = None;
        assert!(g.validate().is_err(), "cropless image != canvas refuses");
        g.image_size = g.canvas_size;
        assert!(g.validate().is_ok());
    }

    #[test]
    fn crop_rect_serde_is_absent_when_none_and_round_trips_when_some() {
        let mut g = DocumentGeometry::new(64, 64);
        let v = serde_json::to_value(g).unwrap();
        assert!(v.get("cropRect").is_none(), "byte-compat: absent when None");
        g.crop_rect = Some(crate::geom::IRect::new(4, 4, 36, 36));
        g.image_size = IVec2::new(32, 32);
        let v = serde_json::to_value(g).unwrap();
        assert!(v.get("cropRect").is_some());
        let back: DocumentGeometry = serde_json::from_value(v).unwrap();
        assert_eq!(back.crop_rect, g.crop_rect);
        assert!(back.validate().is_ok());
    }
}
