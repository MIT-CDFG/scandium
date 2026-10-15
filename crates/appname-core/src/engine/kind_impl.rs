// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §10.1 (commands route mutations through the recorder), §10.3
//! (catalog), D-48 (determinism spine); integration-log wave-2 binding
//! arbitration 1 (stroke-kind assembly seam).
//!
//! **The assembly seam.** Command-kind implementations can live OUTSIDE
//! `appname-core` (the stroke kinds call `appname-tools`, which itself
//! depends on core — so core cannot depend back). A [`KindImpl`] is
//! registered at engine CONSTRUCTION only ([`EngineAssembly`]); there is no
//! runtime registration door, so D-48 stays intact: every mutation still
//! flows through `Engine::apply*`, and an assembled engine's behavior is a
//! pure function of (assembly, journal).
//!
//! A [`KindImpl`] is **pure** in the D-48 sense: `(effective params,
//! envelope, document state, assets, recorder doors) → deltas + outcome`.
//! It receives a [`KindContext`] — constructible only by the engine inside
//! `apply` — whose mutation methods wrap core's `pub(crate)` doors and
//! couple every write to the §10.1 recorder. Replay therefore runs the
//! recorded envelope through the SAME impl with the same captured values
//! and reproduces the same deltas byte-for-byte.
//!
//! Bare-core engines (no assembly) keep answering
//! `error.command.not_implemented` for every non-`doc.new` kind; `doc.new`
//! stays engine-built-in (it manipulates the document SLOT, which no
//! [`KindImpl`] can reach).

use std::sync::Arc;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::buffer::Buffer;
use crate::engine::delta::{DeltaRecorder, TreeOp};
use crate::engine::envelope::Envelope;
use crate::engine::error::EngineError;
use crate::geom::IRect;
use crate::graph::document::Document;
use crate::graph::id::LayerId;
use crate::tile::map::TileCoord;
use crate::tile::TILE;

/// The §14.4 kernel-injection seam (door C4, the D1-fix addendum shape):
/// render's §4.5 routing table, injected at assembly construction — core
/// re-derives NO blend math (the one-math-library clause). The exact
/// relayed shape: `render_separable_kernel` (20 modes) feeds
/// `appname_color::apply_blend_plane`; `render_triple_kernel` (exactly
/// hue/saturation/color/luminosity/darkerColor/lighterColor) feeds
/// `apply_blend_planes3`; Dissolve returns `None` from BOTH
/// (`dissolve_reserved()` owns it); `None` ⟺ the-other-arm.
/// A §4.5 separable kernel `B(b, s)` (render's fn-pointer type).
pub type SeparableKernel = fn(f32, f32) -> f32;
/// A §4.5 triple kernel `B(T_rgb, S_rgb)` (render's fn-pointer type).
pub type TripleKernel = fn([f32; 3], [f32; 3]) -> [f32; 3];

#[derive(Clone, Copy)]
pub struct BlendKernelRouting {
    pub separable: fn(crate::graph::node::BlendMode) -> Option<SeparableKernel>,
    pub triple: fn(crate::graph::node::BlendMode) -> Option<TripleKernel>,
}

/// The fill-math seam (composite-leg slice, the C4/kernel-seam
/// precedent): TEXTVEC's exported fill conversion, injected at assembly
/// construction — core re-derives NO §4.4/EOTF constants. The exact
/// relayed export shapes (appname-text crate root):
/// `fill_to_linear_rgba(Option<&Value>) -> [f32; 4]` (FillContent JSON →
/// STRAIGHT linear RGBA; v1 posture: absent = opaque black, none =
/// clear, solid gray/grayA/rgb/rgba via the EOTF, cmyk/lab/gradient/
/// pattern flagged black) and
/// `premultiply_coverage([f32; 4], f32) -> [f32; 4]` (straight ×
/// coverage → premultiplied). TEXTVEC's own layout_and_compose routes
/// through the same export — the single source is load-bearing.
/// FillContent JSON → straight linear RGBA (appname-text's fn-pointer
/// type; aliased so the assembly-side identity pin names it without a
/// serde_json dep — the SeparableKernel/TripleKernel precedent).
pub type FillToLinearFn = fn(Option<&Value>) -> [f32; 4];
/// Straight RGBA × coverage → premultiplied (appname-text's fn-pointer
/// type).
pub type PremultiplyCoverageFn = fn([f32; 4], f32) -> [f32; 4];

#[derive(Clone, Copy)]
pub struct FillConvertRouting {
    pub fill_to_linear_rgba: FillToLinearFn,
    pub premultiply_coverage: PremultiplyCoverageFn,
}

/// One command-kind implementation, registered at assembly time.
pub trait KindImpl: Send + Sync {
    /// The registered §10.3 kind this implements (must exist in the
    /// generated registry and must not be `doc.new`).
    fn kind(&self) -> &'static str;

    /// Stages 6–7 for one envelope: live-state validation (target/locks/
    /// asset resolution — stage-5 schema validation already ran) and
    /// execution through the context's recorder-coupled doors.
    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError>;
}

/// What an implementation reports back (the engine builds the receipt,
/// deltas, and dirty rects itself from the recorder).
#[derive(Default)]
pub struct KindOutcome {
    /// Ids of objects the command created (deterministic across replay,
    /// §9.6).
    pub created_ids: Vec<Uuid>,
}

/// §9.6 global creation-id discipline (QA-F11 closure), enforced at the
/// structural doors: a creation UUID entering ANY arena must be absent
/// from EVERY arena → `error.schema.duplicate_id` (§40.1 stage-3 row).
/// Door-level enforcement means every creation kind — core-side and
/// assembled — inherits the rule without per-arm repetition.
fn require_globally_fresh(
    doc: &crate::graph::document::Document,
    id: Uuid,
    field: &str,
) -> Result<(), EngineError> {
    if let Some(arena) = doc.arena_holding(id) {
        return Err(EngineError::with_field(
            crate::engine::error::ErrorCode::DuplicateId,
            field,
            format!("creation id {id} is already present in the `{arena}` arena (§9.6)"),
        ));
    }
    Ok(())
}

/// The per-application context handed to a [`KindImpl`] — the ONLY path to
/// document mutation outside core (constructed exclusively by the engine's
/// apply stage; all doors record through §10.1 before touching state).
pub struct KindContext<'a> {
    pub(crate) env: &'a Envelope,
    pub(crate) params: &'a Map<String, Value>,
    pub(crate) doc: &'a mut Document,
    pub(crate) rec: &'a mut DeltaRecorder,
    pub(crate) assets: &'a mut crate::engine::assets::AssetStore,
    /// Per-engine CMM handle (§5.2; the color kinds build transforms
    /// through it — cached, content-keyed).
    pub(crate) cms: &'a mut appname_color::Cms,
    /// §12.1 op registry reference (assembly-owned).
    pub(crate) ops: &'a crate::engine::op_impl::OpAssembly,
    /// §20 vector-geometry service (assembly-owned; TEXTVEC Door 1).
    /// `None` on bare assemblies — geometry-requiring arms answer the
    /// typed NI mirror.
    pub(crate) vector: Option<&'a Arc<dyn crate::engine::vector_ops::VectorGeometry>>,
    /// §23 text-layout service (assembly-owned; TEXTVEC Door 6). `None`
    /// on bare assemblies — text-requiring arms answer the typed NI
    /// mirror.
    pub(crate) text: Option<&'a Arc<dyn crate::engine::text_ops::TextLayout>>,
    /// §14.4 kernel seam (door C4) — `None` on bare assemblies; the
    /// §4.5-kernel apply-image arms answer the typed NI mirror
    /// (add/subtract legacy arithmetic needs no kernel and always runs).
    pub(crate) blend_kernels: Option<BlendKernelRouting>,
    /// Fill-math seam — `None` on bare assemblies (shape-serving
    /// composite arms answer typed NI).
    pub(crate) fill_convert: Option<FillConvertRouting>,
    /// §6.2 warning accumulator (door C5): MessageIds the arm surfaces
    /// with the command result — the engine copies them onto
    /// `ApplyReceipt.warnings`. Context-side (NOT a `KindOutcome`
    /// field) so out-of-crate `KindImpl`s' literal outcome constructors
    /// stay source-compatible.
    pub(crate) warnings: &'a mut Vec<String>,
    /// D-48 vector value-capture, inbound: a capture previously recorded
    /// for THIS envelope id (journal replay) — the arm consumes it instead
    /// of re-running geometry.
    pub(crate) capture_in: Option<Arc<crate::engine::vector_ops::GeometryResult>>,
    /// D-48 vector value-capture, outbound: the arm records its generated
    /// geometry here; the engine persists it keyed by envelope id.
    pub(crate) capture_out: &'a mut Option<Arc<crate::engine::vector_ops::GeometryResult>>,
    /// True for §10.3 class-C kinds: the engine holds a pre-command
    /// snapshot as the inverse, unlocking the wholesale-replacement doors.
    pub(crate) checkpoint: bool,
}

impl KindContext<'_> {
    /// The validated envelope (id, target block, samples reference…).
    pub fn envelope(&self) -> &Envelope {
        self.env
    }

    /// Stage-5 effective params (schema-validated, defaults filled).
    pub fn params(&self) -> &Map<String, Value> {
        self.params
    }

    /// Read access to the document.
    pub fn document(&self) -> &Document {
        self.doc
    }

    /// Bytes of a package/session asset (e.g. an `appname.samples.v1`
    /// blob), by id. Assets are opaque values to core (D-48 captured
    /// values); the impl owns decoding.
    pub fn asset_bytes(&self, id: Uuid) -> Option<&[u8]> {
        self.assets.bytes(id)
    }

    /// Install one full-tile pixel plane (storage-normalized f32, row-major
    /// `TILE × TILE`, zero-padded components) onto a raster layer through
    /// the recorder-coupled door: `will_edit` captures the before Arc, then
    /// the plane writes through the single COW door. The §2.4 prune and
    /// after-capture run at engine commit.
    pub fn install_tile_plane(
        &mut self,
        layer: LayerId,
        coord: TileCoord,
        pixels: &[[f32; 4]],
    ) -> Result<(), EngineError> {
        self.install_surface_plane(layer.as_uuid(), coord, pixels)
    }

    /// Surface-generic plane install (§10.1 disjoint surface spaces): the
    /// UUID resolves to a layer's pixels, a mask's coverage, or the
    /// document's selection mask ([`KindContext::selection_surface`]) —
    /// the CP-0.5 generalization the mask/selection kinds write through.
    pub fn install_surface_plane(
        &mut self,
        surface: Uuid,
        coord: TileCoord,
        pixels: &[[f32; 4]],
    ) -> Result<(), EngineError> {
        if pixels.len() != TILE * TILE {
            return Err(EngineError::bad_target(format!(
                "tile plane must carry {} pixels, got {}",
                TILE * TILE,
                pixels.len()
            )));
        }
        let Self { doc, rec, .. } = self;
        let buf = surface_buffer_of(doc, surface)?;
        // First-touch capture, then write the plane through the COW door.
        rec.will_edit_bounds(surface, buf);
        rec.will_edit(surface, buf.tiles(), coord);
        let tile = buf.make_mut_tile(coord);
        let mut i = 0;
        for y in 0..TILE {
            for x in 0..TILE {
                tile.write_pixel_f32(x, y, pixels[i]);
                i += 1;
            }
        }
        Ok(())
    }

    /// Grow a layer's content bounds (eager growth, §2.4) — callers pass
    /// the stroke/commit bounds.
    pub fn grow_layer_bounds(&mut self, layer: LayerId, rect: IRect) -> Result<(), EngineError> {
        self.grow_surface_bounds(layer.as_uuid(), rect)
    }

    /// Surface-generic eager bounds growth (§2.4).
    pub fn grow_surface_bounds(&mut self, surface: Uuid, rect: IRect) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let buf = surface_buffer_of(doc, surface)?;
        rec.will_edit_bounds(surface, buf);
        buf.grow_bounds_recorded(rect);
        Ok(())
    }

    /// Tile-granular bounds re-tighten after erasing edits (§2.4 lazy
    /// shrink). Runs after installs; the engine's commit prune still runs
    /// per touched coord.
    pub fn recompute_layer_bounds(&mut self, layer: LayerId) -> Result<(), EngineError> {
        self.recompute_surface_bounds(layer.as_uuid())
    }

    /// Surface-generic §2.4 lazy shrink.
    pub fn recompute_surface_bounds(&mut self, surface: Uuid) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let buf = surface_buffer_of(doc, surface)?;
        rec.will_edit_bounds(surface, buf);
        buf.recompute_bounds();
        Ok(())
    }

    /// PIXEL-EXACT bounds re-tighten after a whole-plane bake that MOVES
    /// content (transform.free/perspective/warp). The tile-granular lazy
    /// shrink ([`Self::recompute_layer_bounds`]) cannot tighten below a
    /// tile that is still occupied, so when a translate/rotate vacates a
    /// sub-tile region the stale extent lingers in `bounds` — and since
    /// the package loader recomputes bounds PIXEL-EXACT on reload, the
    /// live and reloaded `region_hash` diverge, killing cross-boundary
    /// undo (the D-48 replay-as-recovery gate; O-QA W12 finding 1). This
    /// door matches the loader's canonicalization so live bounds equal
    /// persisted bounds for every content-moving bake. Runs the SAME
    /// per-pixel scan the bake already performed — no new asymptotic cost.
    pub fn recompute_layer_bounds_pixel_exact(
        &mut self,
        layer: LayerId,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let surface = layer.as_uuid();
        let buf = surface_buffer_of(doc, surface)?;
        rec.will_edit_bounds(surface, buf);
        buf.recompute_bounds_pixel_exact();
        Ok(())
    }

    /// PIXEL-EXACT eager growth SCOPED to a just-written region (Q-1
    /// determinism class). The loader-agreeing sibling of
    /// [`Self::grow_layer_bounds`] for alpha-PRESERVING stroke commits: it
    /// grows `bounds` by the tight (post-quantization) extent of the written
    /// tiles instead of the caller's pre-quantization coverage rect, so the
    /// soft anti-aliased fringe that counts in `changed`/`commit.bounds` but
    /// stores as exactly transparent cannot leave the live extent proud of
    /// the persisted plane. Same identity as
    /// [`Self::recompute_layer_bounds_pixel_exact`] (the loader/D-48
    /// canonicalization) but scoped to the stroke footprint — no full-layer
    /// rescan, so the paint hot path keeps its cost. For a coverage-REDUCING
    /// commit use `recompute_layer_bounds_pixel_exact`, which shrinks
    /// sub-tile.
    pub fn grow_layer_bounds_pixel_exact(
        &mut self,
        layer: LayerId,
        written: IRect,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let surface = layer.as_uuid();
        let buf = surface_buffer_of(doc, surface)?;
        rec.will_edit_bounds(surface, buf);
        buf.grow_bounds_pixel_exact_within(written);
        Ok(())
    }

    /// Set a surface's bounds to an exact recorded rect (the §13.1
    /// selection-bounds discipline: combines compute the tight nonzero
    /// rect in-pass).
    pub fn set_surface_bounds(&mut self, surface: Uuid, rect: IRect) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let buf = surface_buffer_of(doc, surface)?;
        rec.will_edit_bounds(surface, buf);
        buf.set_bounds_recorded(rect);
        Ok(())
    }

    /// The document's deterministic selection-surface id (§10.1); planes
    /// installed on it edit the §13.1 selection mask under the recorder.
    /// Kinds that finish editing it call
    /// [`KindContext::sync_selection_bounds`].
    pub fn selection_surface(&self) -> Uuid {
        self.doc.selection_surface()
    }

    /// Re-derive `Selection::{bounds, active}` from the mask buffer after
    /// selection-surface edits (§13.1 bounds maintenance).
    pub fn sync_selection_bounds(&mut self) {
        self.doc.sync_selection_bounds();
    }

    /// Record a structural op (tree-mutating kinds; §10.1).
    pub fn record_tree_op(&mut self, op: TreeOp) {
        self.rec.tree(op);
    }

    /// The kind tag of a registered asset, if any (stage-4-analog asset
    /// resolution: `edit.paste` accepts only ingested raster assets).
    pub fn asset_kind(&self, id: Uuid) -> Option<crate::engine::assets::AssetKind> {
        self.assets.kind_of(id)
    }

    /// Structural door: insert a new node under `parent` at z-index
    /// `index`, recording `TreeOp::AddNode` (§10.1) so undo removes it and
    /// redo re-inserts the SAME node (§2.7). Duplicate-id and tree-shape
    /// violations surface as typed errors.
    pub fn insert_layer(
        &mut self,
        parent: LayerId,
        index: usize,
        node: crate::graph::node::LayerNode,
    ) -> Result<LayerId, EngineError> {
        require_globally_fresh(self.doc, node.id().into(), "newLayerId")?;
        // (QA-F11: the check above is the §9.6 GLOBAL discipline — every
        // arena, not just the destination — enforced once at the doors so
        // every creation kind inherits it without per-arm repetition.)
        let Self { doc, rec, .. } = self;
        let id = doc
            .insert_child(parent, index, node)
            .map_err(|e| EngineError::bad_target(format!("layer insertion refused: {e:?}")))?;
        rec.tree(TreeOp::AddNode {
            parent: Some(parent),
            index,
            id,
        });
        Ok(id)
    }

    /// Structural door: attach a mask to `layer`, recording
    /// `TreeOp::AttachMask` — undo detaches into the stash, redo
    /// re-attaches the SAME node (§2.7). Reverse-order undo makes this
    /// compose with a same-command [`KindContext::insert_layer`]
    /// (`edit.paste_into`): the mask detaches before the node removes.
    pub fn attach_mask(
        &mut self,
        layer: LayerId,
        mask: crate::graph::mask_node::MaskNode,
    ) -> Result<crate::graph::id::MaskId, EngineError> {
        require_globally_fresh(self.doc, mask.id().into(), "newMaskId")?;
        let Self { doc, rec, .. } = self;
        let id = doc
            .attach_mask(layer, mask)
            .map_err(|e| EngineError::bad_target(format!("mask attach refused: {e:?}")))?;
        rec.tree(TreeOp::AttachMask { layer, mask: id });
        Ok(id)
    }

    /// Structural door: detach + remove a mask (`mask.delete`,
    /// `mask.apply`), recording `TreeOp::DetachMask` with the removed node
    /// riding the delta from execution time (§10.1).
    pub fn detach_mask(
        &mut self,
        layer: LayerId,
        mask: crate::graph::id::MaskId,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let index = doc
            .layer(layer)
            .and_then(|n| n.masks().iter().position(|m| *m == mask))
            .ok_or_else(|| {
                EngineError::bad_target(format!("mask {mask} is not attached to layer {layer}"))
            })?;
        let node = doc
            .detach_mask(layer, mask)
            .map_err(|e| EngineError::bad_target(format!("mask detach refused: {e:?}")))?;
        rec.tree_with_stash(
            TreeOp::DetachMask { layer, mask, index },
            crate::engine::delta::TreeUndoStash::Mask(node),
        );
        Ok(())
    }

    /// O-TOOLS seam (selection kinds): install one single-channel coverage
    /// plane on the SELECTION surface (`SelectionEdit.patches` shape —
    /// `values` is row-major `TILE × TILE` f32 coverage).
    pub fn install_selection_tile_plane(
        &mut self,
        coord: TileCoord,
        values: &[f32],
    ) -> Result<(), EngineError> {
        if values.len() != TILE * TILE {
            return Err(EngineError::bad_target(format!(
                "coverage plane must carry {} values, got {}",
                TILE * TILE,
                values.len()
            )));
        }
        let surface = self.selection_surface();
        let plane: Vec<[f32; 4]> = values.iter().map(|v| [*v, 0.0, 0.0, 0.0]).collect();
        self.install_surface_plane(surface, coord, &plane)
    }

    /// §13.1 split, grow side (`SelectionEdit.shrinking == false`): O(1)
    /// union with the written rect, then bounds re-sync.
    pub fn grow_selection_bounds(&mut self, rect: IRect) -> Result<(), EngineError> {
        let surface = self.selection_surface();
        self.grow_surface_bounds(surface, rect)?;
        self.sync_selection_bounds();
        Ok(())
    }

    /// §13.1 split, shrink side (`shrinking == true`): tile-granular
    /// re-tighten, then bounds re-sync.
    pub fn recompute_selection_bounds(&mut self) -> Result<(), EngineError> {
        let surface = self.selection_surface();
        self.recompute_surface_bounds(surface)?;
        self.sync_selection_bounds();
        Ok(())
    }

    /// Exact-bounds side (combines that track the tight nonzero rect
    /// in-pass), then re-sync.
    pub fn set_selection_bounds(&mut self, rect: IRect) -> Result<(), EngineError> {
        let surface = self.selection_surface();
        self.set_surface_bounds(surface, rect)?;
        self.sync_selection_bounds();
        Ok(())
    }

    /// O-TOOLS seam (doc.* kinds): swap the document geometry, recording
    /// the §19.1 value delta (crop-hide's SetProp-only inverse).
    pub fn set_document_geometry(
        &mut self,
        after: crate::graph::document::DocumentGeometry,
    ) -> Result<(), EngineError> {
        if after.canvas_size.x < 1 || after.canvas_size.y < 1 {
            return Err(EngineError::range(
                "geometry",
                "canvas floor is 1x1 (§19.1)",
            ));
        }
        if !(after.resolution_ppi.is_finite() && after.resolution_ppi > 0.0) {
            return Err(EngineError::range(
                "geometry",
                "resolution must be finite and > 0",
            ));
        }
        let Self { doc, rec, .. } = self;
        let before = *doc.geometry();
        doc.set_geometry_recorded(after);
        rec.tree(TreeOp::SetGeometry { before, after });
        Ok(())
    }

    /// O-TOOLS seam: every raster surface (layers, mask coverages, the
    /// selection) — the §40.2 all-surface enumeration resize walks.
    pub fn raster_surfaces(&self) -> Vec<Uuid> {
        self.doc.raster_surfaces()
    }

    /// CheckpointOnly door: replace a surface's buffer wholesale. Guarded:
    /// only §10.3 class-C kinds (image.flatten, doc.depth_convert — snap
    /// inverse) hold the snapshot this depends on. NOT the resize path:
    /// doc.resize is class R (the § class table governs — wave-4 finding
    /// b) and rebuilds surfaces delta-style through the plane-install
    /// doors, exactly as O-TOOLS wired it.
    pub fn checkpoint_replace_surface(
        &mut self,
        surface: Uuid,
        buffer: Buffer,
    ) -> Result<(), EngineError> {
        if !self.checkpoint {
            return Err(EngineError::bad_target(
                "surface replacement is CheckpointOnly (§10.3 class C) — no snapshot inverse held",
            ));
        }
        if surface == self.doc.selection_surface() {
            *self.doc.selection_mut().mask_mut() = buffer;
            self.doc.sync_selection_bounds();
            return Ok(());
        }
        let layer = LayerId::from_uuid(surface);
        if self
            .doc
            .layer(layer)
            .map(|n| n.pixels().is_some())
            .unwrap_or(false)
        {
            self.doc
                .layer_mut(layer)
                .map_err(|e| EngineError::bad_target(format!("{e:?}")))?
                .set_pixels(buffer);
            return Ok(());
        }
        if let Some(m) = self
            .doc
            .mask_mut(crate::graph::id::MaskId::from_uuid(surface))
        {
            *m.coverage_mut() = buffer;
            return Ok(());
        }
        Err(EngineError::bad_target(format!(
            "surface {surface} does not resolve"
        )))
    }

    /// Structural door: detach a whole subtree, recording
    /// `TreeOp::RemoveSubtree` with the fragment riding the delta from
    /// execution time (`layer.delete`, ungroup's group disposal).
    pub fn remove_subtree(&mut self, layer: LayerId) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let parent = doc.parent_of(layer).ok_or_else(|| {
            EngineError::bad_target(format!("layer {layer} does not resolve (or is the root)"))
        })?;
        let index = doc
            .children_of(parent)
            .and_then(|c| c.iter().position(|k| *k == layer))
            .expect("parent lists the child");
        let frag = doc
            .detach_subtree(layer)
            .map_err(|e| EngineError::bad_target(format!("subtree removal refused: {e:?}")))?;
        rec.tree_with_stash(
            TreeOp::RemoveSubtree {
                parent,
                index,
                root: layer,
            },
            crate::engine::delta::TreeUndoStash::Subtree(frag),
        );
        Ok(())
    }

    /// Structural door: detach a whole subtree TOGETHER WITH its op-stack
    /// entries — every adjustment op and smart-filter chain member in it,
    /// each op's own mask with it — in one DeltaSet. Subtree first, then
    /// the ops, so undo re-inserts the ops before the nodes that reference
    /// them: the order `op.delete`'s coupled delete already uses.
    /// `remove_subtree` alone leaves those ops in the arena unreferenced —
    /// debug refuses that commit (validate_tree's orphan check) and release
    /// commits a document whose journal cannot replay (q-tutorial/Q-12).
    pub fn remove_subtree_and_ops(&mut self, layer: LayerId) -> Result<(), EngineError> {
        let ops: Vec<crate::graph::id::OpId> = {
            let doc = self.document();
            let mut out = Vec::new();
            let mut stack = vec![layer];
            while let Some(id) = stack.pop() {
                if let Some(n) = doc.layer(id) {
                    out.extend(n.adjustment_op());
                    if let Some(so) = n.smart_object() {
                        out.extend(so.smart_filters().iter().copied());
                    }
                    stack.extend(n.children().iter().copied());
                }
            }
            out
        };
        self.remove_subtree(layer)?;
        for op in ops {
            self.remove_op_node(op)?;
        }
        Ok(())
    }

    /// Structural door: move a node to `(parent, index)`, recording the
    /// param-inverse `TreeOp::MoveNode`.
    pub fn move_layer(
        &mut self,
        layer: LayerId,
        to_parent: LayerId,
        to_index: usize,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let from_parent = doc
            .parent_of(layer)
            .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?;
        let from_index = doc
            .children_of(from_parent)
            .and_then(|c| c.iter().position(|k| *k == layer))
            .expect("parent lists the child");
        doc.move_child(layer, to_parent, to_index).map_err(|e| {
            EngineError::bad_target(format!(
                "move refused ({e:?}; cycles and non-groups refuse)"
            ))
        })?;
        let landed = doc
            .children_of(to_parent)
            .and_then(|c| c.iter().position(|k| *k == layer))
            .expect("moved child present");
        rec.tree(TreeOp::MoveNode {
            layer,
            from: (from_parent, from_index),
            to: (to_parent, landed),
        });
        Ok(())
    }

    /// Value door: apply a full §2.6 shared prop block, recording the
    /// before/after delta (`layer.rename` / `layer.set_props` /
    /// align-translate).
    pub fn set_layer_props(
        &mut self,
        layer: LayerId,
        after: crate::engine::delta::LayerPropsBlock,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let before = doc
            .props_block(layer)
            .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?;
        if before == after {
            return Ok(()); // no-op: nothing recorded (§40.1 empty push)
        }
        doc.apply_props_block(layer, &after)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?;
        rec.tree(TreeOp::SetLayerProps {
            layer,
            before: Box::new(before),
            after: Box::new(after),
        });
        Ok(())
    }

    /// Value door: swap the §24.2 guides/grid block, recording the
    /// full-state delta (guide.* / grid.set).
    pub fn set_guides(&mut self, after: crate::graph::document::Guides) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let before = doc.guides().clone();
        if before == after {
            return Ok(()); // §40.1 no-op push
        }
        doc.set_guides(after.clone());
        rec.tree(TreeOp::SetGuides {
            before: Box::new(before),
            after: Box::new(after),
        });
        Ok(())
    }

    /// §10.3 D-emission for chain-edit kinds (the reopened-§12.1 slice):
    /// an op's params/mask/order/enabled flip changes composited pixels
    /// only at re-render, so no surface delta carries the region — this
    /// door computes it per the contract rows and pushes a receipt-side
    /// hint. SMART-FILTER chain members dirty the owning INSTANCE's
    /// bounds inflated through the chain's registered ROIs (§12.2:
    /// reorder dirties bounds inflated by the ops' ROIs; NOTE the
    /// inflation uses `roi_out_to_in` as a stand-in for the outward
    /// spread — exact for symmetric kernels, a recorded approximation
    /// otherwise, conservative for the shipped corpus). ADJUSTMENT ops
    /// (R1 ruling, 2026-08-16): UNMASKED → FULL CANVAS (the
    /// `adjustment.add.yaml` contract letter; over-inclusive is the
    /// safe direction); MASKED → below-stack extent, ROI-inflated — a
    /// RECORDED DIVERGENCE from the mask-bounded contract wording (a
    /// conservative superset; fold-in candidate).
    /// Attribution: the owning layer. An op that is disabled and stays
    /// disabled emits nothing (§12.4: identity before and after — callers
    /// gate on the enabled flags). Unregistered kinds contribute identity
    /// inflation (their kernels cannot evaluate — the serving door
    /// refuses typed, so no served composite can be stale).
    pub fn note_op_composite_dirty(&mut self, op: crate::graph::id::OpId) {
        let Some(owner) = self.doc.z_order_walk().into_iter().find(|lid| {
            let n = self.doc.layer(*lid).expect("walk resolves");
            n.adjustment_op() == Some(op)
                || n.smart_object()
                    .map(|so| so.smart_filters().contains(&op))
                    .unwrap_or(false)
        }) else {
            return; // detached op: nothing composites it
        };
        let node = self.doc.layer(owner).expect("resolved");
        let rect = if let Some(so) = node.smart_object() {
            let mut rect = so.cached_composite().bounds();
            for o in so.smart_filters() {
                if let Some(n) = self.doc.op(*o) {
                    if n.enabled() {
                        if let Some(imp) = self.ops.get(n.op()) {
                            rect = imp.roi_out_to_in(n.params(), rect);
                        }
                    }
                }
            }
            rect
        } else {
            // R1 RULING (O-INT, 2026-08-16 — reviewer escalation on the
            // adjustment D row): two arms, split on the op's mask.
            //
            // UNMASKED (no mask, or the mask node disabled/dangling —
            // coverage ≡ 1): FULL CANVAS, the `adjustment.add.yaml`
            // contract letter. The previous below-stack-extent emission
            // was under-inclusive here: it leaned on the unpinned
            // alpha-preservation property of the §17 kernels, and a
            // bottom-stack adjustment emitted EMPTY. Over-inclusive is
            // the only safe direction for dirty.
            let masked = self
                .doc
                .op(op)
                .and_then(|n| n.mask())
                .and_then(|m| self.doc.mask(m))
                .map(|m| m.enabled())
                .unwrap_or(false);
            if !masked {
                self.doc.geometry().canvas_rect()
            } else {
                // MASKED: below-stack extent (content bounds of every
                // effectively visible pixel-bearing layer strictly below
                // the adjustment in the §2.6 composite order),
                // ROI-inflated. RECORDED DIVERGENCE (R1 arm 2, fold-in
                // candidate): the contract row says mask-BOUNDED; this
                // emits a conservative superset of the true changed
                // region instead — the change is confined to below-stack
                // content by the alpha-preservation property of the §17
                // kernels, which the conformance suite PINS per
                // registered adjustment op (R1 arm 3) so a future
                // alpha-writing op turns a test red rather than silently
                // invalidating this region math.
                let walk = self.doc.z_order_walk();
                let pos = walk
                    .iter()
                    .position(|l| *l == owner)
                    .expect("owner is in the walk");
                let mut extent: Option<IRect> = None;
                for id in &walk[..pos] {
                    let Some(n) = self.doc.layer(*id) else {
                        continue;
                    };
                    if !self.layer_effectively_visible(*id) {
                        continue;
                    }
                    let b = n
                        .pixels()
                        .map(Buffer::bounds)
                        .or_else(|| n.smart_object().map(|so| so.cached_composite().bounds()));
                    if let Some(b) = b {
                        if !b.is_empty() {
                            extent = Some(extent.map(|e| e.union(&b)).unwrap_or(b));
                        }
                    }
                }
                let mut rect = match extent {
                    Some(r) => r,
                    None => return, // nothing below: the masked op recolors nothing
                };
                if let Some(n) = self.doc.op(op) {
                    if let Some(imp) = self.ops.get(n.op()) {
                        rect = imp.roi_out_to_in(n.params(), rect);
                    }
                }
                rect
            }
        };
        self.rec
            .dirty_hint(crate::engine::DirtyRect { layer: owner, rect });
    }

    /// §4.6 step 1: own flag + every ancestor group visible.
    fn layer_effectively_visible(&self, layer: LayerId) -> bool {
        let mut probe = Some(layer);
        while let Some(id) = probe {
            if let Some(n) = self.doc.layer(id) {
                if !n.visible() {
                    return false;
                }
            }
            probe = self.doc.parent_of(id);
        }
        true
    }

    /// Structural door: insert an op node, recording `TreeOp::AddOp`.
    pub fn insert_op_node(
        &mut self,
        op: crate::graph::op_node::OpNode,
    ) -> Result<crate::graph::id::OpId, EngineError> {
        require_globally_fresh(self.doc, op.id().into(), "newOpId")?;
        let Self { doc, rec, .. } = self;
        let id = doc
            .insert_op(op)
            .map_err(|e| EngineError::bad_target(format!("op insertion refused: {e:?}")))?;
        rec.tree(TreeOp::AddOp { op: id });
        Ok(id)
    }

    /// Structural door: remove an op node (`op.delete`), the node riding
    /// the delta from execution time.
    pub fn remove_op_node(&mut self, op: crate::graph::id::OpId) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        // Remove any op-owned mask with the op (single DeltaSet).
        if let Some(mid) = doc.op(op).and_then(|o| o.mask()) {
            if let Some(node) = doc.detach_op_mask(mid) {
                rec.tree_with_stash(
                    TreeOp::SetOpMask {
                        op,
                        before: Some(mid),
                        after: None,
                    },
                    crate::engine::delta::TreeUndoStash::OpMaskSwap {
                        created: None,
                        removed: Some(node),
                    },
                );
                doc.op_mut(op).expect("resolved").set_mask(None);
            }
        }
        let node = doc
            .remove_op(op)
            .ok_or_else(|| EngineError::bad_target(format!("op {op} does not resolve")))?;
        rec.tree_with_stash(
            TreeOp::RemoveOp { op },
            crate::engine::delta::TreeUndoStash::Op(node),
        );
        Ok(())
    }

    /// Value door: the §2.6 mutable-op block (`op.set_params`/`op.toggle`).
    pub fn set_op_value(
        &mut self,
        op: crate::graph::id::OpId,
        after: crate::graph::op_node::OpValueBlock,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .op_mut(op)
            .ok_or_else(|| EngineError::bad_target(format!("op {op} does not resolve")))?;
        let before = node.value_block();
        if before == after {
            return Ok(()); // §40.1 no-op push
        }
        node.apply_value_block(&after);
        let composite_changed = before.enabled || after.enabled;
        rec.tree(TreeOp::SetOpValue {
            op,
            before: Box::new(before),
            after: Box::new(after),
        });
        // §10.3 D: a disabled-to-disabled edit composites nothing (§12.4).
        if composite_changed {
            self.note_op_composite_dirty(op);
        }
        Ok(())
    }

    /// `op.set_mask from:'none'` — clear the slot (removed node stashed).
    pub fn set_op_mask(
        &mut self,
        op: crate::graph::id::OpId,
        to: Option<crate::graph::id::MaskId>,
    ) -> Result<(), EngineError> {
        debug_assert!(
            to.is_none(),
            "assignment goes through set_op_mask_with_node"
        );
        let Self { doc, rec, .. } = self;
        let before = doc
            .op(op)
            .ok_or_else(|| EngineError::bad_target(format!("op {op} does not resolve")))?
            .mask();
        if before.is_none() {
            return Ok(()); // already clear: §40.1 no-op push
        }
        let removed = before.and_then(|m| doc.detach_op_mask(m));
        doc.op_mut(op).expect("resolved").set_mask(None);
        rec.tree_with_stash(
            TreeOp::SetOpMask {
                op,
                before,
                after: None,
            },
            crate::engine::delta::TreeUndoStash::OpMaskSwap {
                created: None,
                removed,
            },
        );
        if self.doc.op(op).map(|n| n.enabled()).unwrap_or(false) {
            self.note_op_composite_dirty(op); // §10.3 D: the gate region changed
        }
        Ok(())
    }

    /// `op.set_mask from:'selection'` — install a NEW op mask node.
    pub fn set_op_mask_with_node(
        &mut self,
        op: crate::graph::id::OpId,
        mask: crate::graph::mask_node::MaskNode,
    ) -> Result<(), EngineError> {
        require_globally_fresh(self.doc, mask.id().into(), "newMaskId")?;
        let Self { doc, rec, .. } = self;
        let before = doc
            .op(op)
            .ok_or_else(|| EngineError::bad_target(format!("op {op} does not resolve")))?
            .mask();
        let removed = before.and_then(|m| doc.detach_op_mask(m));
        let mid = mask.id();
        doc.attach_op_mask(op, mask)
            .map_err(|e| EngineError::bad_target(format!("op mask refused: {e:?}")))?;
        doc.op_mut(op).expect("resolved").set_mask(Some(mid));
        rec.tree_with_stash(
            TreeOp::SetOpMask {
                op,
                before,
                after: Some(mid),
            },
            crate::engine::delta::TreeUndoStash::OpMaskSwap {
                created: None,
                removed,
            },
        );
        if self.doc.op(op).map(|n| n.enabled()).unwrap_or(false) {
            self.note_op_composite_dirty(op); // §10.3 D: the gate region changed
        }
        Ok(())
    }

    /// `op.reorder` within a smart-filter chain (param inverse).
    pub fn reorder_op(
        &mut self,
        layer: LayerId,
        op: crate::graph::id::OpId,
        index: usize,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let from = doc
            .layer(layer)
            .and_then(|n| n.smart_object())
            .and_then(|so| so.smart_filters().iter().position(|o| *o == op))
            .ok_or_else(|| {
                EngineError::bad_target(format!("op {op} is not in layer {layer}'s chain"))
            })?;
        doc.reorder_smart_filter(layer, op, index)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?;
        let to = doc
            .layer(layer)
            .and_then(|n| n.smart_object())
            .and_then(|so| so.smart_filters().iter().position(|o| *o == op))
            .expect("just moved");
        rec.tree(TreeOp::ReorderOp {
            layer,
            op,
            from,
            to,
        });
        if from != to && self.doc.op(op).map(|n| n.enabled()).unwrap_or(false) {
            self.note_op_composite_dirty(op); // §12.2: reorder is a composite change
        }
        Ok(())
    }

    /// Detach a filter from a chain (op.delete on a smart filter): the
    /// chain edit is a ReorderOp-free splice recorded as a SetOpValue-less
    /// structural pair — v1 models it as remove+re-insert via the chain
    /// vector inside RemoveOp's stash… simpler: record the chain position
    /// through a MoveNode-style param op.
    pub fn remove_smart_filter(
        &mut self,
        layer: LayerId,
        op: crate::graph::id::OpId,
    ) -> Result<(), EngineError> {
        // §10.3 D, noted BEFORE the splice (the op must still resolve to
        // its owner; extent conservative with the member still in-chain).
        if self.doc.op(op).map(|n| n.enabled()).unwrap_or(false) {
            self.note_op_composite_dirty(op);
        }
        let Self { doc, rec, .. } = self;
        let from = doc
            .layer(layer)
            .and_then(|n| n.smart_object())
            .and_then(|so| so.smart_filters().iter().position(|o| *o == op))
            .ok_or_else(|| {
                EngineError::bad_target(format!("op {op} is not in layer {layer}'s chain"))
            })?;
        let node = doc
            .layer_mut(layer)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?;
        let so = node.smart_object_mut().expect("checked");
        so.smart_filters.remove(from);
        rec.tree(TreeOp::RemoveFilterRef {
            layer,
            op,
            index: from,
        });
        Ok(())
    }

    /// Recorded INSERT door for smart-filter chain entries
    /// (`filter.smart_add`'s execution tail — the O-TOOLS integration
    /// ask-2 door, exact mirror of [`Self::remove_smart_filter`]): arena
    /// insert (records `TreeOp::AddOp`) + chain splice (records
    /// `TreeOp::AddFilterRef`), ONE DeltaSet. `index` CLAMPS to
    /// `0..=len(chain)` (§40.6 — never an error); the recorded index is
    /// the post-clamp position, so undo/redo is exact-inverse. Nothing
    /// mutates unless the target resolves as a smart object first.
    pub fn insert_smart_filter(
        &mut self,
        layer: LayerId,
        index: usize,
        node: crate::graph::op_node::OpNode,
    ) -> Result<crate::graph::id::OpId, EngineError> {
        if self
            .doc
            .layer(layer)
            .and_then(|n| n.smart_object())
            .is_none()
        {
            return Err(EngineError::new(
                crate::engine::error::ErrorCode::TargetKindMismatch,
                "insert_smart_filter targets a smart object's chain (§12.2) — run \
                 smartobject.convert first (§40.6)",
            ));
        }
        let id = self.insert_op_node(node)?;
        let Self { doc, rec, .. } = self;
        doc.insert_smart_filter_ref(layer, id, index)
            .map_err(|e| EngineError::bad_target(format!("chain splice refused: {e:?}")))?;
        let at = doc
            .layer(layer)
            .and_then(|n| n.smart_object())
            .and_then(|so| so.smart_filters().iter().position(|o| *o == id))
            .expect("just spliced");
        rec.tree(TreeOp::AddFilterRef {
            layer,
            op: id,
            index: at,
        });
        if self.doc.op(id).map(|n| n.enabled()).unwrap_or(false) {
            self.note_op_composite_dirty(id); // §10.3 D over instance bounds
        }
        Ok(id)
    }

    // --- D-ML-1 — the mask-asset ingress doors (O-ML filing) ---------------

    /// Resolve a MaskPlane asset (stage-4-analog, the `edit.paste`
    /// pattern): must resolve AND carry [`AssetKind::MaskPlane`] →
    /// `error.command.bad_target` otherwise; decode is re-validated
    /// (ingestion already checked — belt-and-braces, bytes are immutable).
    fn resolve_mask_plane(&self, asset: Uuid) -> Result<(IRect, Vec<f32>), EngineError> {
        use crate::engine::assets::AssetKind;
        match self.assets.kind_of(asset) {
            Some(AssetKind::MaskPlane) => {}
            Some(_) => {
                return Err(EngineError::bad_target(format!(
                    "asset {asset} is not an appname.mask.v1 coverage plane (D-ML-1)"
                )))
            }
            None => {
                return Err(EngineError::bad_target(format!(
                    "mask asset {asset} does not resolve (§9.2 ingestion first)"
                )))
            }
        }
        crate::engine::assets::decode_mask_plane(
            self.assets.bytes(asset).expect("kind_of resolved"),
        )
    }

    /// Single-channel read of any coverage-bearing surface (selection
    /// mask, layer-mask coverage, or a §14.1 named channel), 0.0 outside
    /// stored tiles.
    fn read_coverage(&self, surface: Uuid, x: i32, y: i32) -> f32 {
        let p = crate::geom::IVec2::new(x, y);
        if surface == self.doc.selection_surface() {
            return self.doc.selection().mask().read_pixel_f32(p)[0];
        }
        if let Some(m) = self.doc.mask(crate::graph::id::MaskId::from_uuid(surface)) {
            return m.coverage().read_pixel_f32(p)[0];
        }
        if let Some(c) = self
            .doc
            .channel(crate::graph::id::ChannelId::from_uuid(surface))
        {
            return c.buffer().read_pixel_f32(p)[0];
        }
        0.0
    }

    /// §13.2 combine of a decoded plane onto a coverage surface — the
    /// op-aware tile walk of the RULED `selection.from_path` shape
    /// (door-slice review, MAJOR fix): Replace zeroes previously-covered
    /// space outside the plane; Intersect visits the UNION of plane-cover
    /// and existing tiles (outside the plane b = 0 must CLEAR); Add and
    /// Subtract are identity outside the plane — plane-cover tiles
    /// suffice.
    fn combine_plane_onto_surface(
        &mut self,
        surface: Uuid,
        rect: IRect,
        values: &[f32],
        op: crate::engine::mask_ops::SelectOp,
    ) -> Result<(), EngineError> {
        use crate::engine::mask_ops::{combine, SelectOp};
        let sample = |x: i32, y: i32| -> f32 {
            if x < rect.min.x || x >= rect.max.x || y < rect.min.y || y >= rect.max.y {
                return 0.0;
            }
            values[(y - rect.min.y) as usize * rect.width() as usize + (x - rect.min.x) as usize]
        };
        let existing_coords: Vec<TileCoord> = {
            let buf: &Buffer = if surface == self.doc.selection_surface() {
                self.doc.selection().mask()
            } else if let Some(m) = self.doc.mask(crate::graph::id::MaskId::from_uuid(surface)) {
                m.coverage()
            } else if let Some(c) = self
                .doc
                .channel(crate::graph::id::ChannelId::from_uuid(surface))
            {
                c.buffer()
            } else {
                return Err(EngineError::bad_target(format!(
                    "surface {surface} is not a coverage surface (selection, layer mask, or \
                     channel)"
                )));
            };
            buf.tiles().iter().map(|(c, _)| c).collect()
        };
        let t = TILE as i32;
        let mut coords: Vec<TileCoord> = Vec::new();
        {
            let x0 = rect.min.x.div_euclid(t);
            let y0 = rect.min.y.div_euclid(t);
            let x1 = (rect.max.x - 1).div_euclid(t);
            let y1 = (rect.max.y - 1).div_euclid(t);
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    coords.push(TileCoord::new(0, tx, ty));
                }
            }
        }
        if matches!(op, SelectOp::Intersect) {
            for c in &existing_coords {
                if !coords.contains(c) {
                    coords.push(*c);
                }
            }
        }
        if matches!(op, SelectOp::Replace) {
            for c in &existing_coords {
                if !coords.contains(c) {
                    let zeros = vec![[0.0f32; 4]; TILE * TILE];
                    self.install_surface_plane(surface, *c, &zeros)?;
                }
            }
        }
        for c in coords {
            let base_x = c.x * t;
            let base_y = c.y * t;
            let mut plane = vec![[0.0f32; 4]; TILE * TILE];
            let mut any = false;
            for yy in 0..t {
                for xx in 0..t {
                    let (px, py) = (base_x + xx, base_y + yy);
                    let b = sample(px, py);
                    let a = self.read_coverage(surface, px, py);
                    let v = combine(op, a, b);
                    plane[(yy * t + xx) as usize] = [v, 0.0, 0.0, 0.0];
                    any = any || v != 0.0 || a != 0.0;
                }
            }
            if any || matches!(op, SelectOp::Replace) {
                self.install_surface_plane(surface, c, &plane)?;
            }
        }
        Ok(())
    }

    /// D-ML-1 door (a): apply an ingested `appname.mask.v1` coverage
    /// plane to the SELECTION surface with the §13.2 op set — the O-ML
    /// filing's surface (a). Same recorder doors + bounds split the
    /// selection kinds use; typed staging precedes mutation.
    ///
    /// WIRE STATUS (recorded, not invented): NO v5.5 kind reaches this
    /// door — `selection.set`'s source enum and `mask.add`'s from enum
    /// carry no raster/asset arm (contracts read first-hand; stage 5
    /// refuses those spellings already). The door serves the sanctioned
    /// future surfacings the filing names: a v5.6 fold-in param arm, or
    /// Part 53 `origin:ai` ordinary commands post-1.0.
    pub fn apply_mask_asset_to_selection(
        &mut self,
        asset: Uuid,
        op: crate::engine::mask_ops::SelectOp,
    ) -> Result<(), EngineError> {
        use crate::engine::mask_ops::SelectOp;
        let (rect, values) = self.resolve_mask_plane(asset)?;
        let surface = self.selection_surface();
        self.combine_plane_onto_surface(surface, rect, &values, op)?;
        match op {
            SelectOp::Replace => self.set_selection_bounds(rect)?,
            SelectOp::Add => self.grow_selection_bounds(rect)?,
            SelectOp::Subtract | SelectOp::Intersect => self.recompute_selection_bounds()?,
        }
        self.sync_selection_bounds();
        Ok(())
    }

    /// D-ML-1 door (b): apply an ingested coverage plane to a LAYER
    /// RASTER MASK's coverage with the §13.2 op set — the filing's
    /// surface (b). The mask must resolve (stage-4 → `bad_target`).
    pub fn apply_mask_asset_to_layer_mask(
        &mut self,
        mask: crate::graph::id::MaskId,
        asset: Uuid,
        op: crate::engine::mask_ops::SelectOp,
    ) -> Result<(), EngineError> {
        use crate::engine::mask_ops::SelectOp;
        if self.doc.mask(mask).is_none() {
            return Err(EngineError::bad_target(format!(
                "mask {mask} does not resolve (§2.7 arenas)"
            )));
        }
        let (rect, values) = self.resolve_mask_plane(asset)?;
        let surface = mask.as_uuid();
        self.combine_plane_onto_surface(surface, rect, &values, op)?;
        match op {
            SelectOp::Replace => self.set_surface_bounds(surface, rect)?,
            SelectOp::Add => self.grow_surface_bounds(surface, rect)?,
            SelectOp::Subtract | SelectOp::Intersect => self.recompute_surface_bounds(surface)?,
        }
        Ok(())
    }

    // --- §14 channel doors (door slice 3: C1–C3 plumbing) -----------------

    /// Structural door: append a NEW empty §14.1 named channel (Gray/f32
    /// coverage, the §13.1 one-representation rule), recording
    /// `TreeOp::AddChannel` — undo removes it into the stash, redo
    /// re-appends the SAME node. §9.6 global creation-id discipline is
    /// door-inherited (`duplicate_id`).
    pub fn insert_named_channel(
        &mut self,
        id: crate::graph::id::ChannelId,
        name: impl Into<String>,
    ) -> Result<crate::graph::id::ChannelId, EngineError> {
        require_globally_fresh(self.doc, id.as_uuid(), "newChannelId")?;
        let Self { doc, rec, .. } = self;
        let node = crate::graph::channel_node::NamedChannel::new(
            id,
            name,
            crate::mask::Selection::new_mask_buffer(),
        );
        let id = doc
            .insert_channel(node)
            .map_err(|e| EngineError::bad_target(format!("channel insertion refused: {e:?}")))?;
        rec.tree(TreeOp::AddChannel { channel: id });
        Ok(id)
    }

    /// Structural door: remove a named channel (`channel.delete`), the
    /// node + its §14.1 list index riding the delta from execution time.
    pub fn remove_named_channel(
        &mut self,
        id: crate::graph::id::ChannelId,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let (index, node) = doc
            .remove_channel(id)
            .ok_or_else(|| EngineError::bad_target(format!("channel {id} does not resolve")))?;
        rec.tree_with_stash(
            TreeOp::RemoveChannel { channel: id, index },
            crate::engine::delta::TreeUndoStash::Channel(node),
        );
        Ok(())
    }

    /// Value door: rename a named channel (`channel.edit{name}`, §40.4).
    pub fn rename_channel(
        &mut self,
        id: crate::graph::id::ChannelId,
        name: String,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .channel_mut(id)
            .ok_or_else(|| EngineError::bad_target(format!("channel {id} does not resolve")))?;
        let before = node.name().to_string();
        if before == name {
            return Ok(()); // §40.1 no-op push
        }
        node.set_name(name.clone());
        rec.tree(TreeOp::SetChannelName {
            channel: id,
            before,
            after: name,
        });
        Ok(())
    }

    /// §14.2 REPLACE-form coverage copy, zero pixel copies: every stored
    /// tile of `from` lands on `to` as a COW `Arc` clone (the §34 test
    /// asserts per-tile `Arc::ptr_eq` post-round-trip); destination tiles
    /// outside the source zero out (replace semantics); bounds set to the
    /// source's recorded bounds. Both surfaces must be coverage surfaces
    /// (selection, mask, or channel — Gray/f32 by construction).
    pub fn copy_coverage_tiles_cow(
        &mut self,
        from: Uuid,
        to: Uuid,
        bounds: IRect,
    ) -> Result<(), EngineError> {
        let src: Vec<(TileCoord, Arc<crate::tile::storage::TileStorage>)> = {
            let buf = self.coverage_buffer_of(from)?;
            buf.tiles()
                .iter()
                .map(|(c, arc)| (c, Arc::clone(arc)))
                .collect()
        };
        // Zero destination tiles the source does not cover (replace).
        let dst_coords: Vec<TileCoord> = {
            let buf = self.coverage_buffer_of(to)?;
            buf.tiles().iter().map(|(c, _)| c).collect()
        };
        for c in dst_coords {
            if !src.iter().any(|(sc, _)| *sc == c) {
                let zeros = vec![[0.0f32; 4]; TILE * TILE];
                self.install_surface_plane(to, c, &zeros)?;
            }
        }
        // COW installs through the recorder (will_edit captures before).
        {
            let Self { doc, rec, .. } = self;
            let buf = surface_buffer_of(doc, to)?;
            rec.will_edit_bounds(to, buf);
            for (c, arc) in src {
                rec.will_edit(to, buf.tiles(), c);
                buf.install_tile(c, arc);
            }
            buf.set_bounds_recorded(bounds);
        }
        Ok(())
    }

    /// Read-side resolver for coverage surfaces (typed refusal otherwise).
    fn coverage_buffer_of(&self, surface: Uuid) -> Result<&Buffer, EngineError> {
        if surface == self.doc.selection_surface() {
            return Ok(self.doc.selection().mask());
        }
        if let Some(m) = self.doc.mask(crate::graph::id::MaskId::from_uuid(surface)) {
            return Ok(m.coverage());
        }
        if let Some(c) = self
            .doc
            .channel(crate::graph::id::ChannelId::from_uuid(surface))
        {
            return Ok(c.buffer());
        }
        Err(EngineError::bad_target(format!(
            "surface {surface} is not a coverage surface (selection, layer mask, or channel)"
        )))
    }

    /// §13.2 combine door over an arbitrary coverage source rect/plane
    /// onto a coverage SURFACE (selection or channel) — the selection.save
    /// non-replace arms and selection.load reuse core's EXISTING combine
    /// machinery (§14.2: "no new mechanism").
    pub fn combine_coverage_plane(
        &mut self,
        surface: Uuid,
        rect: IRect,
        values: &[f32],
        op: crate::engine::mask_ops::SelectOp,
    ) -> Result<(), EngineError> {
        self.combine_plane_onto_surface(surface, rect, values, op)
    }

    /// Value door: swap the §29.2 slice vector (slice.* kinds — the
    /// guides full-state pattern), recording `TreeOp::SetSlices`.
    pub fn set_slices(
        &mut self,
        after: Vec<crate::graph::document::SliceRow>,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let before = doc.slices().to_vec();
        if before == after {
            return Ok(()); // §40.1 no-op push
        }
        *doc.slices_mut() = after.clone();
        rec.tree(TreeOp::SetSlices {
            before: Box::new(before),
            after: Box::new(after),
        });
        Ok(())
    }

    /// Value door: set one REGISTERED document-meta key (`meta.set`,
    /// §10.3/§40.8) — inverse = previous value; `None` removes.
    pub fn set_meta_key(
        &mut self,
        key: &str,
        value: Option<serde_json::Value>,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let before = doc.meta().get(key).cloned();
        if before == value {
            return Ok(()); // §40.1 no-op push
        }
        match &value {
            Some(v) => {
                doc.meta_mut().insert(key.to_string(), v.clone());
            }
            None => {
                doc.meta_mut().remove(key);
            }
        }
        rec.tree(TreeOp::SetMetaKey {
            key: key.to_string(),
            before: Box::new(before),
            after: Box::new(value),
        });
        Ok(())
    }

    /// The raster-node CONSTRUCTION door (O-TOOLS escalation 3, door
    /// slice 2): recorded raster-layer creation for ASSEMBLED kinds —
    /// `content_aware_fill` `output:"new_layer"`/`"duplicate_layer"`
    /// (§40.5 "writes a NEW node above the target"), §13.4 stage-9
    /// output-to-new-layer at workspace commit, W14's shape-composite
    /// path. Mirror of the chain-insert door
    /// ([`KindContext::insert_smart_filter`]): typed staging BEFORE any
    /// mutation — the parent must resolve and be a group or the root
    /// (`target_kind_mismatch`), the creation id must be §9.6 GLOBALLY
    /// fresh (door-inherited `duplicate_id`) — then one recorded
    /// `TreeOp::AddNode` (§2.7: undo removes, redo re-inserts the SAME
    /// node, tiles riding). The node lands EMPTY in the document's depth/
    /// working space; the caller writes computed content through the
    /// EXISTING recorder-coupled plane doors
    /// ([`KindContext::install_tile_plane`] /
    /// [`KindContext::grow_layer_bounds`] /
    /// [`KindContext::set_surface_bounds`]) inside the SAME apply — ONE
    /// DeltaSet, replay-deterministic by construction.
    pub fn insert_raster_layer(
        &mut self,
        parent: LayerId,
        index: usize,
        id: LayerId,
        name: impl Into<String>,
    ) -> Result<LayerId, EngineError> {
        let Some(parent_node) = self.doc.layer(parent) else {
            return Err(EngineError::bad_target(format!(
                "parent {parent} does not resolve"
            )));
        };
        if !parent_node.is_group() && parent != self.doc.root() {
            return Err(EngineError::new(
                crate::engine::error::ErrorCode::TargetKindMismatch,
                "insert_raster_layer parents must be a group or the root (§2.6)",
            ));
        }
        let index = index.min(self.doc.children_of(parent).map(|c| c.len()).unwrap_or(0));
        let working = self.doc.color().working;
        let depth = self.doc.color().depth;
        let node = crate::graph::node::LayerNode::new_raster(
            id,
            name,
            Buffer::new(depth, crate::tile::model::ColorModelKind::Rgba, working),
        );
        self.insert_layer(parent, index, node)
    }

    /// Convenience over [`KindContext::insert_raster_layer`]: insert AND
    /// install computed content planes + the exact §2.4 bounds — the
    /// one-call shape for kinds whose output is already tiled
    /// (`values` per plane are storage-normalized row-major
    /// `TILE × TILE` RGBA, exactly [`KindContext::install_tile_plane`]).
    pub fn insert_raster_layer_with_content(
        &mut self,
        parent: LayerId,
        index: usize,
        id: LayerId,
        name: impl Into<String>,
        bounds: IRect,
        planes: &[(TileCoord, Vec<[f32; 4]>)],
    ) -> Result<LayerId, EngineError> {
        let lid = self.insert_raster_layer(parent, index, id, name)?;
        for (coord, pixels) in planes {
            self.install_tile_plane(lid, *coord, pixels)?;
        }
        self.set_surface_bounds(lid.as_uuid(), bounds)?;
        Ok(lid)
    }

    /// `smartobject.edit_contents`/`replace`: per-instance source swap.
    pub fn set_smart_source(
        &mut self,
        layer: LayerId,
        before: Uuid,
        after: Uuid,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        doc.set_smart_source(layer, after)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?;
        rec.tree(TreeOp::SetSmartSource {
            layer,
            before,
            after,
        });
        Ok(())
    }

    /// Re-decode the cached composite from the (new) embedded source
    /// (§11.5 invalidation) through the recorder tile doors.
    pub fn refresh_smart_cache(&mut self, layer: LayerId) -> Result<(), EngineError> {
        let asset = {
            let so = self
                .doc
                .layer(layer)
                .and_then(|n| n.smart_object())
                .ok_or_else(|| EngineError::bad_target("not a smart object"))?;
            let crate::graph::node::SmartSource::Embedded(a) = so.source();
            *a
        };
        let bytes = self
            .asset_bytes(asset)
            .ok_or_else(|| EngineError::bad_target(format!("asset {asset} unresolved")))?
            .to_vec();
        let working = self.doc.color().working;
        let (decoded, w, h) = crate::io::png_io::decode_png_to_buffer(&bytes, working)
            .map_err(|e| EngineError::bad_target(format!("embedded source failed decode: {e}")))?;
        // Cache surface: install through the surface doors (the smart
        // object's cache is its tile surface — resolver reads pixels() OR
        // the cache; v1 replaces wholesale via the doc door + records the
        // tile deltas by hand over old ∪ new).
        let old = {
            let so = self
                .doc
                .layer(layer)
                .and_then(|n| n.smart_object())
                .expect("checked");
            so.cached_composite().clone()
        };
        let dest = crate::geom::IRect::new(0, 0, w as i32, h as i32);
        let union = old.bounds().union(&dest);
        let coords: Vec<(i32, i32)> = union.tiles(crate::tile::TILE_I32).collect();
        for (tx, ty) in coords {
            let coord = TileCoord::new(0, tx, ty);
            let rect = coord.pixel_rect();
            if rect.intersect(&union).is_empty() {
                continue;
            }
            let mut plane = vec![[0.0f32; 4]; TILE * TILE];
            let mut changed = false;
            let mut i = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = crate::geom::IVec2::new(x, y);
                    let v = if dest.contains(p) {
                        decoded.read_pixel_f32(p)
                    } else {
                        [0.0; 4]
                    };
                    if v != old.read_pixel_f32(p) {
                        changed = true;
                    }
                    plane[i] = v;
                    i += 1;
                }
            }
            if changed {
                self.install_surface_plane(layer.as_uuid(), coord, &plane)?;
            }
        }
        self.set_surface_bounds(layer.as_uuid(), dest)?;
        Ok(())
    }

    /// CheckpointOnly door: bake a smart object to a plain raster layer
    /// (§12.4 — bit-identical to the composite at commit: v1's cache IS
    /// the composite; snapshot inverse held by the engine).
    pub fn checkpoint_rasterize(&mut self, layer: LayerId) -> Result<KindOutcome, EngineError> {
        if !self.checkpoint {
            return Err(EngineError::bad_target(
                "rasterize is CheckpointOnly (§10.3 class C) — no snapshot inverse held",
            ));
        }
        let (cache, place_identity, filters, chain_enabled) = {
            let so = self
                .doc
                .layer(layer)
                .and_then(|n| n.smart_object())
                .ok_or_else(|| EngineError::bad_target("not a smart object"))?;
            let m = so.place_transform();
            let ident = m.is_identity();
            (
                so.cached_composite().clone(),
                ident,
                so.smart_filters().to_vec(),
                so.smart_filters()
                    .iter()
                    .any(|o| self.doc.op(*o).map(|n| n.enabled()).unwrap_or(false)),
            )
        };
        if !place_identity {
            return Err(EngineError::new(
                crate::engine::error::ErrorCode::NotImplemented,
                "non-identity placement bakes through the tools resample engines (§18.2)",
            ));
        }
        if chain_enabled {
            return Err(EngineError::new(
                crate::engine::error::ErrorCode::NotImplemented,
                "baking an ENABLED smart-filter chain needs the §12.1 kernel evaluation                  (the compositor/O-TOOLS row); disable or delete the filters first",
            ));
        }
        // Swap the node kind wholesale (snapshot inverse) + drop chain ops.
        for o in filters {
            self.doc.remove_op(o);
            // (disabled ops contribute identity — §12.4 bit-identity holds)
        }
        let node = self
            .doc
            .layer_mut(layer)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?;
        node.replace_kind_with_raster(cache);
        Ok(KindOutcome::default())
    }

    /// CheckpointOnly door (the composite-leg slice): swap a TEXT layer's
    /// kind to Raster with the Door-6-composed page baked into `pixels`
    /// (class-C snapshot inverse — undo restores the Text node wholesale,
    /// editability intact, exactly the smart-object rasterize shape).
    pub fn checkpoint_rasterize_text(
        &mut self,
        layer: LayerId,
        pixels: Buffer,
    ) -> Result<KindOutcome, EngineError> {
        if !self.checkpoint {
            return Err(EngineError::bad_target(
                "rasterize is CheckpointOnly (§10.3 class C) — no snapshot inverse held",
            ));
        }
        let node = self
            .doc
            .layer_mut(layer)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?;
        if node.text_data().is_none() {
            return Err(EngineError::bad_target(
                "checkpoint_rasterize_text needs a Text layer",
            ));
        }
        node.replace_kind_with_raster(pixels);
        Ok(KindOutcome::default())
    }

    /// Encode a raster layer's content as PNG bytes (smartobject.convert's
    /// asset ingestion; deterministic — the §8.2 export encoder).
    pub fn encode_layer_png(&self, layer: LayerId) -> Result<Vec<u8>, EngineError> {
        crate::io::png_io::encode_layer_png(self.doc, layer)
    }

    /// Ingest PNG bytes as a raster asset mid-command (convert). The asset
    /// store is engine state, not doc state; recording is unnecessary —
    /// content addressing makes re-execution idempotent (D-48 captured
    /// value; the envelope's journal entry re-derives the same id).
    pub fn ingest_raster_asset(&mut self, bytes: Vec<u8>) -> Result<Uuid, EngineError> {
        crate::io::png_io::decode_png_to_buffer(&bytes, self.doc.color().working)
            .map_err(|e| EngineError::bad_target(format!("not an ingestible raster: {e}")))?;
        Ok(self
            .assets
            .insert_kind(bytes, crate::engine::assets::AssetKind::RasterImage))
    }

    /// The assembly's §12.1 op registry (adjustment.add / op.set_params
    /// stage-3 nested validation; bake sites call the kernels).
    pub fn op_registry(&self) -> &crate::engine::op_impl::OpAssembly {
        self.ops
    }

    /// The per-engine CMM (color kinds; §5.4 transform cache lives inside).
    pub fn cms(&mut self) -> &mut appname_color::Cms {
        self.cms
    }

    /// The assembly's §20 vector-geometry service (TEXTVEC Door 1), if
    /// one is registered.
    pub fn vector_geometry(&self) -> Option<&Arc<dyn crate::engine::vector_ops::VectorGeometry>> {
        self.vector
    }

    /// The geometry service or the typed NI refusal (bare-core mirror) —
    /// the standard opener for every geometry-requiring vector arm.
    pub fn require_vector_geometry(
        &self,
    ) -> Result<Arc<dyn crate::engine::vector_ops::VectorGeometry>, EngineError> {
        self.vector
            .map(Arc::clone)
            .ok_or_else(|| crate::engine::vector_ops::geometry_unregistered(&self.env.kind))
    }

    /// The assembly's §23 text-layout service (TEXTVEC Door 6), if one is
    /// registered.
    pub fn text_layout(&self) -> Option<&Arc<dyn crate::engine::text_ops::TextLayout>> {
        self.text
    }

    /// The text service or the typed NI refusal (bare-core mirror) — the
    /// standard opener for every text-requiring arm.
    pub fn require_text_layout(
        &self,
    ) -> Result<Arc<dyn crate::engine::text_ops::TextLayout>, EngineError> {
        self.text
            .map(Arc::clone)
            .ok_or_else(|| crate::engine::text_ops::text_unregistered(&self.env.kind))
    }

    /// Surface a warning MessageId with this command's result (§6.2
    /// warnings-before-execute; rides `ApplyReceipt.warnings`).
    pub fn push_warning(&mut self, id: impl Into<String>) {
        self.warnings.push(id.into());
    }

    /// The §14.4 kernel routing (door C4) or the typed NI mirror.
    pub fn require_blend_kernels(&self) -> Result<BlendKernelRouting, EngineError> {
        self.blend_kernels.ok_or_else(|| {
            EngineError::new(
                crate::engine::error::ErrorCode::NotImplemented,
                "the §4.5 kernel seam is unregistered on this assembly — the assembly point \
                 injects render's routing (render_separable_kernel/render_triple_kernel, \
                 door C4); add/subtract legacy arithmetic runs without it",
            )
        })
    }

    /// Value door: full §23.1 text-payload delta (`text.edit`/`set_style`/
    /// `set_paragraph` — TEXTVEC Door 7; §40.1 no-op push when nothing
    /// changes). Sidecars are validated/normalized by the TextLayout
    /// service BEFORE they reach this door — core carries them opaque.
    pub fn set_text_data(
        &mut self,
        layer: LayerId,
        after: crate::graph::node::TextLayerData,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .layer(layer)
            .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?;
        let before = node
            .text_data()
            .ok_or_else(|| {
                EngineError::new(
                    crate::engine::error::ErrorCode::TargetKindMismatch,
                    "the target is not a Text layer (§40.7 family rule)",
                )
            })?
            .clone();
        if before == after {
            return Ok(()); // §40.1 no-op push
        }
        doc.apply_text_data_block(layer, &after)
            .map_err(|e| EngineError::bad_target(format!("text data refused: {e:?}")))?;
        rec.tree(crate::engine::delta::TreeOp::SetTextData {
            layer,
            before: Box::new(before),
            after: Box::new(after),
        });
        Ok(())
    }

    /// Value door: `text.to_shape`'s recorded kind swap (§40.7 "SetProp
    /// kind + geometry payloads" — delta-invertible; undo restores the
    /// Text node with editability intact). The current kind MUST be Text;
    /// the shape's path node is inserted by the caller through
    /// [`KindContext::insert_path`] in the SAME DeltaSet.
    pub fn convert_text_to_shape(
        &mut self,
        layer: LayerId,
        shape: crate::engine::delta::ShapeContentBlock,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let before = doc
            .layer(layer)
            .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?
            .text_data()
            .ok_or_else(|| {
                EngineError::new(
                    crate::engine::error::ErrorCode::TargetKindMismatch,
                    "the target is not a Text layer (§40.7 family rule)",
                )
            })?
            .clone();
        let after = crate::engine::delta::ContentKindBlock::Shape(shape);
        doc.apply_content_kind_block(layer, &after)
            .map_err(|e| EngineError::bad_target(format!("kind swap refused: {e:?}")))?;
        rec.tree(crate::engine::delta::TreeOp::SetContentKind {
            layer,
            before: Box::new(crate::engine::delta::ContentKindBlock::Text(before)),
            after: Box::new(after),
        });
        Ok(())
    }

    /// D-48 inbound capture for THIS envelope (journal replay: consume
    /// instead of re-running geometry).
    pub fn vector_capture(&self) -> Option<&Arc<crate::engine::vector_ops::GeometryResult>> {
        self.capture_in.as_ref()
    }

    /// D-48 outbound capture: record generated geometry for this envelope
    /// (persisted with the journal; replay consumes it).
    pub fn record_vector_capture(&mut self, g: Arc<crate::engine::vector_ops::GeometryResult>) {
        *self.capture_out = Some(g);
    }

    /// Receipt-side dirty hint for a layer whose PIXELS the deltas cannot
    /// name (§10.3 rows over parametric content — vector kinds' shape
    /// bounds; the op-edit analogue is
    /// [`KindContext::note_op_composite_dirty`]). Empty rects are dropped.
    pub fn note_layer_dirty(&mut self, layer: LayerId, rect: IRect) {
        if rect.is_empty() {
            return;
        }
        self.rec
            .dirty_hint(crate::engine::DirtyRect { layer, rect });
    }

    // --- §20 doc-path arena doors (TEXTVEC Door 2) -------------------------

    /// Structural door: add a doc path, recording `TreeOp::AddPath` —
    /// undo removes into the stash, redo re-inserts the SAME node (§2.7).
    /// The node's sidecar is value-captured: neither history direction
    /// re-runs geometry (D-48).
    pub fn insert_path(
        &mut self,
        node: crate::graph::path_node::PathNode,
    ) -> Result<crate::graph::id::PathId, EngineError> {
        require_globally_fresh(self.doc, node.id().into(), "newPathId")?;
        let Self { doc, rec, .. } = self;
        let id = doc
            .insert_path(node)
            .map_err(|e| EngineError::bad_target(format!("path insertion refused: {e:?}")))?;
        rec.tree(TreeOp::AddPath { path: id });
        Ok(id)
    }

    /// Structural door: remove a doc path (`vector.path_delete`,
    /// `path_bool` flatten-consumption), the node riding the delta from
    /// execution time.
    pub fn remove_path(&mut self, id: crate::graph::id::PathId) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .remove_path(id)
            .ok_or_else(|| EngineError::bad_target(format!("path {id} does not resolve")))?;
        rec.tree_with_stash(
            TreeOp::RemovePath { path: id },
            crate::engine::delta::TreeUndoStash::Path(node),
        );
        Ok(())
    }

    /// Value door: full path-state snapshot delta (`vector.path_edit`,
    /// `vector.edit`, `path_bool` doc-path bake) — §40.1 no-op push when
    /// nothing changes.
    pub fn set_path_value(
        &mut self,
        id: crate::graph::id::PathId,
        after: crate::graph::path_node::PathValueBlock,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .path_mut(id)
            .ok_or_else(|| EngineError::bad_target(format!("path {id} does not resolve")))?;
        let before = node.value_block();
        if before == after {
            return Ok(()); // §40.1 no-op push
        }
        node.apply_value_block(&after);
        rec.tree(TreeOp::SetPathValue {
            path: id,
            before: Box::new(before),
            after: Box::new(after),
        });
        Ok(())
    }

    /// Value door: full §20.3 shape-payload delta
    /// (`vector.shape_set_style`, `path_bool` shape-mode append).
    pub fn set_shape_data(
        &mut self,
        layer: LayerId,
        after: crate::engine::delta::ShapeDataBlock,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .layer(layer)
            .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?;
        let sd = node.shape_data().ok_or_else(|| {
            EngineError::new(
                crate::engine::error::ErrorCode::TargetKindMismatch,
                "the target is not a Shape layer (§20.2)",
            )
        })?;
        let before = crate::engine::delta::ShapeDataBlock {
            fill: sd.fill().clone(),
            stroke: sd.stroke().clone(),
            bool_ops: sd.bool_ops().to_vec(),
        };
        if before == after {
            return Ok(()); // §40.1 no-op push
        }
        doc.apply_shape_data_block(layer, &after)
            .map_err(|e| EngineError::bad_target(format!("shape data refused: {e:?}")))?;
        rec.tree(TreeOp::SetShapeData {
            layer,
            before: Box::new(before),
            after: Box::new(after),
        });
        Ok(())
    }

    /// Retag the working profile, recording the §40.2 param inverse
    /// (`doc.assign_profile` + the retag half of `doc.convert_profile`).
    pub fn set_working_profile(&mut self, after: crate::graph::id::ProfileId) {
        let Self { doc, rec, .. } = self;
        let before = doc.color().working;
        doc.set_working_profile(after);
        rec.tree(TreeOp::SetWorkingProfile { before, after });
    }

    /// CheckpointOnly door: §6.3 depth conversion over every raster layer
    /// (guarded — only class-C kinds hold a snapshot inverse, §10.4).
    pub fn checkpoint_convert_depth(
        &mut self,
        to: crate::tile::channel::ChannelKind,
    ) -> Result<(), EngineError> {
        if !self.checkpoint {
            return Err(EngineError::bad_target(
                "depth conversion is CheckpointOnly (§10.3 class C) — no snapshot inverse is held",
            ));
        }
        self.doc.convert_depth(to);
        Ok(())
    }

    /// CheckpointOnly door (door C5): replace one raster layer's pixel
    /// buffer wholesale (a mode-converted buffer of a DIFFERENT color
    /// model). Guarded — only class-C kinds hold a snapshot inverse
    /// (§10.4); the §10.1 recorder cannot express a model swap.
    pub fn checkpoint_set_layer_pixels(
        &mut self,
        layer: LayerId,
        buffer: Buffer,
    ) -> Result<(), EngineError> {
        if !self.checkpoint {
            return Err(EngineError::bad_target(
                "layer-buffer replacement is CheckpointOnly (§10.3 class C) — no snapshot \
                 inverse is held",
            ));
        }
        let node = self.doc.layer_mut(layer).map_err(|e| {
            EngineError::bad_target(format!("layer {layer} does not resolve: {e:?}"))
        })?;
        if node.pixels().is_none() {
            return Err(EngineError::bad_target(format!(
                "layer {layer} is not a raster surface"
            )));
        }
        node.set_pixels(buffer);
        Ok(())
    }

    /// CheckpointOnly door (door C5): re-tag the document color mode +
    /// working profile after a §6.2 mode conversion.
    pub fn checkpoint_set_color_mode(
        &mut self,
        mode: crate::tile::model::ColorModelKind,
        working: crate::graph::id::ProfileId,
    ) -> Result<(), EngineError> {
        if !self.checkpoint {
            return Err(EngineError::bad_target(
                "mode re-tagging is CheckpointOnly (§10.3 class C) — no snapshot inverse is \
                 held",
            ));
        }
        self.doc.set_color_mode(mode);
        self.doc.set_working_profile(working);
        Ok(())
    }

    /// Property door: apply a full §2.6 mask-property block, recording the
    /// before/after SetProp delta (`mask.set_props`).
    pub fn set_mask_props(
        &mut self,
        mask: crate::graph::id::MaskId,
        after: crate::engine::delta::MaskProps,
    ) -> Result<(), EngineError> {
        let Self { doc, rec, .. } = self;
        let node = doc
            .mask_mut(mask)
            .ok_or_else(|| EngineError::bad_target(format!("mask {mask} does not resolve")))?;
        let before = node.props();
        node.set_props(after);
        rec.tree(TreeOp::SetMaskProps {
            mask,
            before,
            after,
        });
        Ok(())
    }
}

/// §10.1 surface resolution at the door: selection mask, layer pixels, or
/// mask coverage (disjoint UUID spaces — the selection id is derived, layer
/// and mask ids are creation UUIDs).
fn surface_buffer_of(doc: &mut Document, surface: Uuid) -> Result<&mut Buffer, EngineError> {
    if surface == doc.selection_surface() {
        return Ok(doc.selection_mut().mask_mut());
    }
    let layer = LayerId::from_uuid(surface);
    if doc
        .layer(layer)
        .map(|n| n.pixels().is_some())
        .unwrap_or(false)
    {
        return Ok(doc
            .layer_mut(layer)
            .map_err(|e| EngineError::bad_target(format!("layer {layer} does not resolve: {e:?}")))?
            .pixels_mut()
            .expect("checked above"));
    }
    if doc
        .layer(layer)
        .map(|n| n.smart_object().is_some())
        .unwrap_or(false)
    {
        return Ok(doc
            .layer_mut(layer)
            .map_err(|e| EngineError::bad_target(format!("{e:?}")))?
            .smart_object_mut()
            .map(|so| &mut so.cached_composite)
            .expect("checked above"));
    }
    if doc
        .mask(crate::graph::id::MaskId::from_uuid(surface))
        .is_some()
    {
        return Ok(doc
            .mask_mut(crate::graph::id::MaskId::from_uuid(surface))
            .expect("checked above")
            .coverage_mut());
    }
    // §14.1 named channels are surfaces too (door C1: channel edits are
    // tile deltas through the recorder — delta.rs' "channels/selection
    // join" note discharges here).
    if doc
        .channel(crate::graph::id::ChannelId::from_uuid(surface))
        .is_some()
    {
        return Ok(doc
            .channel_mut(crate::graph::id::ChannelId::from_uuid(surface))
            .expect("checked above")
            .buffer_mut());
    }
    Err(EngineError::bad_target(format!(
        "surface {surface} does not resolve to a layer, mask, channel, or the selection"
    )))
}

/// Construction-time kind registration (binding arbitration 1): a clonable
/// assembly of [`KindImpl`]s. `Engine::new()` is the bare assembly.
#[derive(Clone, Default)]
pub struct EngineAssembly {
    pub(crate) impls: Vec<Arc<dyn KindImpl>>,
    /// §12.1 op registry (DD ⑯ seam) — construction-only like the kinds.
    pub(crate) ops: crate::engine::op_impl::OpAssembly,
    /// §20 vector-geometry service (TEXTVEC Door 1) — construction-only,
    /// at most one; `None` = bare-core mirror (geometry arms answer typed
    /// NI).
    pub(crate) vector: Option<Arc<dyn crate::engine::vector_ops::VectorGeometry>>,
    /// §23 text-layout service (TEXTVEC Door 6) — construction-only, at
    /// most one; `None` = bare-core mirror (text arms answer typed NI).
    pub(crate) text: Option<Arc<dyn crate::engine::text_ops::TextLayout>>,
    /// §14.4 kernel seam (door C4) — construction-only; `None` =
    /// bare-core mirror (kernel-requiring apply-image arms answer typed
    /// NI).
    pub(crate) blend_kernels: Option<BlendKernelRouting>,
    /// Fill-math seam (composite-leg slice) — construction-only; `None`
    /// = bare mirror (shape-serving composite arms answer typed NI).
    pub(crate) fill_convert: Option<FillConvertRouting>,
}

impl EngineAssembly {
    pub fn new() -> EngineAssembly {
        EngineAssembly::default()
    }

    /// Register one kind implementation. Panics on a kind the generated
    /// registry does not carry, on `doc.new` (engine-built-in), and on
    /// duplicates — assembly is programmer-controlled construction, not
    /// runtime input.
    pub fn with_kind(mut self, imp: Arc<dyn KindImpl>) -> EngineAssembly {
        let kind = imp.kind();
        assert!(
            crate::engine::registry::lookup(kind).is_some(),
            "EngineAssembly: kind `{kind}` is not in the §10.3 registry"
        );
        assert!(
            kind != "doc.new",
            "EngineAssembly: doc.new is engine-built-in"
        );
        assert!(
            !self.impls.iter().any(|i| i.kind() == kind),
            "EngineAssembly: kind `{kind}` registered twice"
        );
        self.impls.push(imp);
        self
    }

    /// Register the §14.4 kernel routing (door C4) — at most one; the
    /// assembly point passes render's exported table
    /// (`render_separable_kernel` / `render_triple_kernel`).
    pub fn with_blend_kernels(mut self, routing: BlendKernelRouting) -> EngineAssembly {
        assert!(
            self.blend_kernels.is_none(),
            "EngineAssembly: blend kernels registered twice"
        );
        self.blend_kernels = Some(routing);
        self
    }

    /// The registered §14.4 kernel routing, if any (door C4 — the
    /// production-parity guard pins its presence + fn-pointer identity
    /// assembly-side).
    pub fn blend_kernel_routing(&self) -> Option<BlendKernelRouting> {
        self.blend_kernels
    }

    /// The registered fill-math routing, if any (composite-leg slice —
    /// the production-parity guard pins its presence + fn-pointer
    /// identity assembly-side).
    pub fn fill_convert_routing(&self) -> Option<FillConvertRouting> {
        self.fill_convert
    }

    /// Register the fill-math routing (composite-leg slice) — at most
    /// one; the assembly point passes appname-text's exports.
    pub fn with_fill_convert(mut self, routing: FillConvertRouting) -> EngineAssembly {
        assert!(
            self.fill_convert.is_none(),
            "EngineAssembly: fill convert registered twice"
        );
        self.fill_convert = Some(routing);
        self
    }

    /// Build an engine over this assembly.
    pub fn build(&self) -> crate::engine::Engine {
        crate::engine::Engine::with_assembly(self.clone())
    }

    pub fn kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.impls.iter().map(|i| i.kind())
    }

    /// Register a parametric op implementation (§12.1 registry; O-TOOLS'
    /// compute half plugs in here — DD ⑯: "a new filter is addable with
    /// no edits to core").
    pub fn with_op(mut self, op: Arc<dyn crate::engine::op_impl::OpImpl>) -> EngineAssembly {
        self.ops = self.ops.with_op(op);
        self
    }

    pub fn ops(&self) -> &crate::engine::op_impl::OpAssembly {
        &self.ops
    }

    /// Register THE vector-geometry service (TEXTVEC Door 1;
    /// `appname_vector::register` wraps this at the production assembly
    /// points). Panics on a wrong schema stamp or double registration —
    /// the seam is version-stamped (`appname.vector.v1`) and singular.
    pub fn with_vector_geometry(
        mut self,
        svc: Arc<dyn crate::engine::vector_ops::VectorGeometry>,
    ) -> EngineAssembly {
        assert_eq!(
            svc.schema(),
            crate::engine::vector_ops::VECTOR_SIDECAR_SCHEMA,
            "EngineAssembly: VectorGeometry must speak the version-stamped seam"
        );
        assert!(
            self.vector.is_none(),
            "EngineAssembly: VectorGeometry registered twice"
        );
        self.vector = Some(svc);
        self
    }

    pub fn vector_geometry(&self) -> Option<&Arc<dyn crate::engine::vector_ops::VectorGeometry>> {
        self.vector.as_ref()
    }

    /// Register THE text-layout service (TEXTVEC Door 6; `appname_text`'s
    /// register wraps this at the assembly composition point). Panics on a
    /// wrong schema stamp or double registration — the seam is
    /// version-stamped (`appname.text.v1`) and singular, exactly the
    /// Door-1 pattern.
    pub fn with_text_layout(
        mut self,
        svc: Arc<dyn crate::engine::text_ops::TextLayout>,
    ) -> EngineAssembly {
        assert_eq!(
            svc.schema(),
            crate::engine::text_ops::TEXT_SIDECAR_SCHEMA,
            "EngineAssembly: TextLayout must speak the version-stamped seam"
        );
        assert!(
            self.text.is_none(),
            "EngineAssembly: TextLayout registered twice"
        );
        self.text = Some(svc);
        self
    }

    pub fn text_layout(&self) -> Option<&Arc<dyn crate::engine::text_ops::TextLayout>> {
        self.text.as_ref()
    }
}

#[cfg(test)]
mod seam_tests {
    use super::*;

    use crate::geom::IVec2;
    use serde_json::json;

    /// The O-TOOLS seam doors work end-to-end through a probe KindImpl:
    /// selection plane install + bounds split + geometry SetProp round-trip
    /// under undo/redo (the wiring their selection.set/doc.* kinds will use
    /// assembly-only).
    struct SeamProbe;
    impl KindImpl for SeamProbe {
        fn kind(&self) -> &'static str {
            "selection.all" // registered kind carried by the probe assembly
        }
        fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
            // A 0.75-coverage plane on tile (0,0) + exact bounds.
            let mut values = vec![0.0f32; TILE * TILE];
            for y in 0..64 {
                for x in 0..64 {
                    values[y * TILE + x] = 0.75;
                }
            }
            let coord = TileCoord::new(0, 0, 0);
            cx.install_selection_tile_plane(coord, &values)?;
            cx.set_selection_bounds(IRect::new(0, 0, 64, 64))?;
            // Geometry SetProp: bump the ppi (value-delta inverse).
            let mut g = *cx.document().geometry();
            g.resolution_ppi = 300.0;
            cx.set_document_geometry(g)?;
            Ok(KindOutcome::default())
        }
    }

    #[test]
    fn seam_geometry_only_round_trip() {
        struct GeoProbe;
        impl KindImpl for GeoProbe {
            fn kind(&self) -> &'static str {
                "selection.none"
            }
            fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
                let mut g = *cx.document().geometry();
                g.resolution_ppi = 144.0;
                cx.set_document_geometry(g)?;
                Ok(KindOutcome::default())
            }
        }
        let mut e = EngineAssembly::new().with_kind(Arc::new(GeoProbe)).build();
        e.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000011","kind":"doc.new","params":{"width":64,"height":64}}).to_string()).unwrap();
        let base_ppi = e.document().unwrap().geometry().resolution_ppi;
        e.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000012","kind":"selection.none","params":{}}).to_string()).unwrap();
        assert_eq!(
            e.document().unwrap().geometry().resolution_ppi,
            144.0,
            "applied"
        );
        e.undo().unwrap();
        assert_eq!(
            e.document().unwrap().geometry().resolution_ppi,
            base_ppi,
            "undone"
        );
    }

    /// The raster-node construction door (O-TOOLS escalation 3): an
    /// assembled kind creates a raster layer with COMPUTED content in one
    /// DeltaSet — typed staging, §2.7 undo/redo identity, §9.6 global
    /// duplicate inheritance, replay determinism.
    #[test]
    fn raster_node_construction_door_round_trips() {
        use crate::graph::id::LayerId as L;
        const NEW_ID: Uuid = Uuid::from_u128(0x5ea7_0000_0000_4000_8000_0000_0000_0777);

        struct RasterDoorProbe {
            parent_override: Option<Uuid>,
        }
        impl KindImpl for RasterDoorProbe {
            fn kind(&self) -> &'static str {
                "selection.all" // registered carrier kind for the probe
            }
            fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
                let parent = self
                    .parent_override
                    .map(L::from_uuid)
                    .unwrap_or_else(|| cx.document().root());
                let index = cx
                    .document()
                    .children_of(parent)
                    .map(|c| c.len())
                    .unwrap_or(0);
                // Computed content: a deterministic gradient plane.
                let mut pixels = vec![[0.0f32; 4]; TILE * TILE];
                for y in 0..64usize {
                    for x in 0..64usize {
                        let v = (x as f32) / 63.0;
                        pixels[y * TILE + x] = [v, 0.0, 0.0, 1.0];
                    }
                }
                let lid = cx.insert_raster_layer_with_content(
                    parent,
                    index,
                    L::from_uuid(NEW_ID),
                    "Computed",
                    IRect::new(0, 0, 64, 64),
                    &[(TileCoord::new(0, 0, 0), pixels)],
                )?;
                Ok(KindOutcome {
                    created_ids: vec![lid.as_uuid()],
                })
            }
        }

        let mut e = EngineAssembly::new()
            .with_kind(Arc::new(RasterDoorProbe {
                parent_override: None,
            }))
            .build();
        e.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000021","kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}).to_string()).unwrap();
        let pre = e.state_hash();
        let receipt = e.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000022","kind":"selection.all","params":{}}).to_string()).unwrap();
        assert_eq!(receipt.created_ids, vec![NEW_ID]);
        let d = e.document().unwrap();
        let node = d
            .layer(crate::graph::id::LayerId::from_uuid(NEW_ID))
            .unwrap();
        assert_eq!(node.kind().kind_str(), "raster");
        let buf = node.pixels().unwrap();
        assert_eq!(
            buf.bounds(),
            IRect::new(0, 0, 64, 64),
            "exact recorded bounds"
        );
        assert_eq!(
            buf.read_pixel_f32(IVec2::new(63, 10)),
            [1.0, 0.0, 0.0, 1.0],
            "computed content landed through the recorder doors"
        );
        let after = e.state_hash();

        // ONE DeltaSet: a single undo removes node + tiles + bounds.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "AddNode + planes + bounds, one step");
        assert!(e
            .document()
            .unwrap()
            .layer(crate::graph::id::LayerId::from_uuid(NEW_ID))
            .is_none());
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after, "the SAME node re-inserts (§2.7)");

        // §9.6 global duplicate: the same creation id refuses typed, and
        // the failed apply mutates NOTHING (stage-6 rollback).
        let err = e.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000023","kind":"selection.all","params":{}}).to_string()).unwrap_err();
        assert_eq!(err.code, crate::engine::error::ErrorCode::DuplicateId);
        assert_eq!(e.state_hash(), after, "no mutation on refusal");

        // Replay determinism over the whole journal.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());

        // Typed staging: a NON-GROUP parent answers target_kind_mismatch
        // BEFORE any mutation.
        let mut e2 = EngineAssembly::new()
            .with_kind(Arc::new(RasterDoorProbe {
                parent_override: None, // replaced below per-doc
            }))
            .build();
        e2.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000031","kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}).to_string()).unwrap();
        let bg = e2.document().unwrap().z_order_walk()[0].as_uuid();
        let mut e3 = EngineAssembly::new()
            .with_kind(Arc::new(RasterDoorProbe {
                parent_override: Some(bg),
            }))
            .build();
        e3.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000031","kind":"doc.new","params":{"width":64,"height":64,"background":"white"}}).to_string()).unwrap();
        let pre3 = e3.state_hash();
        let err = e3.apply_json(&json!({"schema":"appname.command.v1","id":"5ea70000-0000-4000-8000-000000000032","kind":"selection.all","params":{}}).to_string()).unwrap_err();
        assert_eq!(
            err.code,
            crate::engine::error::ErrorCode::TargetKindMismatch,
            "raster parents refuse (§2.6)"
        );
        assert_eq!(e3.state_hash(), pre3, "typed staging precedes mutation");
    }

    /// D-ML-1 — the mask-asset ingress: appname.mask.v1 decode gates,
    /// §13.2 application to the selection surface (incl. the RULED
    /// intersect-clears-outside semantics) and to a layer raster mask,
    /// undo/redo byte identity, and replay determinism (assets are
    /// captured values — fresh_like carries them).
    #[test]
    fn mask_asset_ingress_applies_through_the_combine_doors() {
        use crate::engine::assets::{asset_id_for, encode_mask_plane};
        use crate::engine::mask_ops::SelectOp;

        struct Step {
            asset: Uuid,
            op: SelectOp,
            mask: Option<crate::graph::id::MaskId>,
        }
        struct MlProbe {
            steps: std::collections::HashMap<Uuid, Step>,
        }
        impl KindImpl for MlProbe {
            fn kind(&self) -> &'static str {
                "selection.all" // registered carrier kind for the probe
            }
            fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
                // Keyed by envelope id — replay re-resolves identically.
                let step = self
                    .steps
                    .get(&cx.envelope().id)
                    .ok_or_else(|| EngineError::bad_target("probe: unscripted envelope"))?;
                match step.mask {
                    None => cx.apply_mask_asset_to_selection(step.asset, step.op)?,
                    Some(m) => cx.apply_mask_asset_to_layer_mask(m, step.asset, step.op)?,
                }
                Ok(KindOutcome::default())
            }
        }

        // Plane A: 64×64 at origin, 0.75 coverage. Plane B: 16×16 inset.
        let plane_a = encode_mask_plane(IRect::new(0, 0, 64, 64), &vec![0.75f32; 64 * 64]);
        let plane_b = encode_mask_plane(IRect::new(16, 16, 32, 32), &vec![1.0f32; 16 * 16]);
        let (id_a, id_b) = (asset_id_for(&plane_a), asset_id_for(&plane_b));
        let mid = crate::graph::id::MaskId::from_uuid(Uuid::from_u128(0x77));

        let e_id =
            |n: u32| Uuid::parse_str(&format!("5ea70000-0000-4000-8000-0004{n:08x}")).unwrap();
        let mut steps = std::collections::HashMap::new();
        steps.insert(
            e_id(3),
            Step {
                asset: id_a,
                op: SelectOp::Replace,
                mask: None,
            },
        );
        steps.insert(
            e_id(4),
            Step {
                asset: id_b,
                op: SelectOp::Intersect,
                mask: None,
            },
        );
        steps.insert(
            e_id(5),
            Step {
                asset: id_b,
                op: SelectOp::Replace,
                mask: Some(mid),
            },
        );
        steps.insert(
            e_id(6),
            Step {
                asset: Uuid::from_u128(0xdead),
                op: SelectOp::Replace,
                mask: None,
            },
        );

        let assembly = crate::engine::kinds::register(
            EngineAssembly::new().with_kind(Arc::new(MlProbe { steps })),
        );
        let mut e = assembly.build();
        let apply = |e: &mut crate::engine::Engine,
                     n: u32,
                     kind: &str,
                     params: serde_json::Value,
                     target: Option<Uuid>| {
            let mut v = json!({"schema": "appname.command.v1", "id": e_id(n).to_string(),
                               "kind": kind, "params": params});
            if let Some(t) = target {
                v["target"] = json!({"layerId": t.to_string()});
            }
            e.apply_json(&v.to_string())
        };
        apply(
            &mut e,
            1,
            "doc.new",
            json!({"width": 128, "height": 128, "background": "white"}),
            None,
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0].as_uuid();
        apply(
            &mut e,
            2,
            "mask.add",
            json!({"kind": "raster", "from": "hide_all", "newMaskId": mid.as_uuid().to_string()}),
            Some(bg),
        )
        .unwrap();

        // Ingest: malformed bytes answer typed at the ingress.
        assert_eq!(
            e.add_mask_asset(b"garbage".to_vec()).unwrap_err().code,
            crate::engine::error::ErrorCode::OutOfRange
        );
        let wrong = serde_json::to_vec(&json!({"schema": "appname.other"})).unwrap();
        let mut wrong_hdr = wrong.clone();
        wrong_hdr.push(b'\n');
        assert_eq!(
            e.add_mask_asset(wrong_hdr).unwrap_err().code,
            crate::engine::error::ErrorCode::WrongSchema
        );
        assert_eq!(
            e.add_mask_asset(plane_a.clone()).unwrap(),
            id_a,
            "content-derived id"
        );
        assert_eq!(e.add_mask_asset(plane_b.clone()).unwrap(), id_b);

        // (a) selection replace: §13.2 Replace = b everywhere.
        let pre = e.state_hash();
        apply(&mut e, 3, "selection.all", json!({}), None).unwrap();
        {
            let d = e.document().unwrap();
            assert!(d.selection().active());
            assert_eq!(d.selection().bounds(), IRect::new(0, 0, 64, 64));
            assert_eq!(
                d.selection().mask().read_pixel_f32(IVec2::new(40, 40))[0],
                0.75
            );
        }
        // (a) intersect with the inset plane: outside the plane b = 0
        // CLEARS (the RULED from_path semantics carried into this door).
        apply(&mut e, 4, "selection.all", json!({}), None).unwrap();
        {
            let d = e.document().unwrap();
            assert_eq!(
                d.selection().mask().read_pixel_f32(IVec2::new(40, 40))[0],
                0.0,
                "outside witness cleared (min(a, 0) = 0)"
            );
            assert_eq!(
                d.selection().mask().read_pixel_f32(IVec2::new(20, 20))[0],
                0.75,
                "inside min-combines (min(0.75, 1) = 0.75)"
            );
        }
        // (b) layer-mask replace: coverage = plane, outside zeroed,
        // bounds = the plane rect.
        apply(&mut e, 5, "selection.all", json!({}), None).unwrap();
        {
            let d = e.document().unwrap();
            let m = d.mask(mid).unwrap();
            assert_eq!(m.coverage().read_pixel_f32(IVec2::new(20, 20))[0], 1.0);
            assert_eq!(m.coverage().read_pixel_f32(IVec2::new(8, 8))[0], 0.0);
            assert_eq!(m.coverage().bounds(), IRect::new(16, 16, 32, 32));
        }
        let after = e.state_hash();

        // Undo the three applications in order; redo restores byte-exact.
        e.undo().unwrap();
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "three inverse DeltaSets");
        e.redo().unwrap();
        e.redo().unwrap();
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);

        // Unresolved asset answers bad_target, mutating NOTHING.
        let err = apply(&mut e, 6, "selection.all", json!({}), None).unwrap_err();
        assert_eq!(err.code, crate::engine::error::ErrorCode::BadTarget);
        assert_eq!(e.state_hash(), after, "typed staging precedes mutation");

        // Replay determinism (assets + steps re-resolve by envelope id).
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after);

        // A non-MaskPlane asset refuses typed at the door.
        let samples = e.add_samples_asset(b"not-a-plane".to_vec());
        let mut cx_steps = std::collections::HashMap::new();
        cx_steps.insert(
            e_id(7),
            Step {
                asset: samples,
                op: SelectOp::Add,
                mask: None,
            },
        );
        let assembly2 = crate::engine::kinds::register(
            EngineAssembly::new().with_kind(Arc::new(MlProbe { steps: cx_steps })),
        );
        let mut e2 = assembly2.build();
        apply(
            &mut e2,
            1,
            "doc.new",
            json!({"width": 32, "height": 32}),
            None,
        )
        .unwrap();
        e2.add_samples_asset(b"not-a-plane".to_vec());
        let err = apply(&mut e2, 7, "selection.all", json!({}), None).unwrap_err();
        assert_eq!(err.code, crate::engine::error::ErrorCode::BadTarget);
        assert!(
            err.message.contains("appname.mask.v1"),
            "refusal names the format"
        );
    }

    #[test]
    fn seam_doors_round_trip_under_undo_redo() {
        let mut e = EngineAssembly::new().with_kind(Arc::new(SeamProbe)).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "5ea70000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 128, "height": 128, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let before = e.state_hash();
        let base_ppi = e.document().unwrap().geometry().resolution_ppi;
        assert_eq!(
            e.document().unwrap().raster_surfaces().len(),
            2,
            "bg layer + selection"
        );
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "5ea70000-0000-4000-8000-000000000002",
                "kind": "selection.all",
                "params": {},
            })
            .to_string(),
        )
        .unwrap();
        let d = e.document().unwrap();
        assert!(d.selection().active());
        assert_eq!(d.selection().bounds(), IRect::new(0, 0, 64, 64));
        assert_eq!(
            d.selection().mask().read_pixel_f32(IVec2::new(10, 10))[0],
            0.75
        );
        assert_eq!(d.geometry().resolution_ppi, 300.0);
        let after = e.state_hash();

        e.undo().unwrap();
        let d = e.document().unwrap();
        assert!(
            !d.selection().active(),
            "selection-surface deltas + bounds restore"
        );
        assert_eq!(
            d.geometry().resolution_ppi,
            base_ppi,
            "SetGeometry value inverse"
        );
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
    }
}
