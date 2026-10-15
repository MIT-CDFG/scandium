// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.4 (compositing model), §4.6 (evaluation order — post-order
//! walk, visibility skip; step 4b isolated-group composite), §4.7
//! (pass-through vs isolated groups — the decision table below), §4.8
//! (CPU reference driver shape), §3.7 (dirty consumption), §2.6 (z-order:
//! index 0 composites first), §2.5/§4.3 (absent tiles read transparent
//! black); DD ⑤⑧; plan §2.2 "CPU compositor (reference)" + §2.3
//! "pass-through vs isolated groups".
//!
//! The reference document compositor: flatten the layer tree through
//! appname-core's PUBLIC read surface ([`Document::z_order_walk`]-order
//! walk over [`Document::children_of`], [`appname_core::LayerNode`]
//! getters, [`appname_core::Buffer`] tile reads), then composite per tile,
//! bottom-to-top, in linear premultiplied f32 ([`TileRgba`]).
//!
//! CP-0.5 slice A: all 27 §4.5 blend modes route through the flatten walk
//! (separable / whole-pixel / non-separable / Dissolve — Dissolve layers
//! resolve their §4.5 blue-noise offsets from the document + layer uuids
//! at flatten time and thresholds in document space at composite time).
//!
//! **Groups (CP-0.5 slice B — plan §2.3 "pass-through vs isolated")**:
//! the ES §4.7 [NORMATIVE] decision table, first row that holds decides:
//!
//! | # | condition            | result       |
//! |---|----------------------|--------------|
//! | 1 | `isolated == true`   | isolate      |
//! | 2 | `blend != Normal`    | isolate      |
//! | 3 | `opacity < 1.0`      | isolate      |
//! | 4 | any enabled mask     | isolate      |
//! | 5 | any enabled effect   | isolate      |
//! | 6 | otherwise            | pass-through |
//!
//! Pass-through groups flatten inline (children blend against the
//! accumulated backdrop below the group — the CP-0.3 behavior, unchanged).
//! Isolated groups composite their children onto TRANSPARENCY, then blend
//! that single intermediate against the backdrop with the group's blend
//! and opacity (§4.6 compile step 4b). Notes, each pinned by test:
//!
//! * Row 4 still refuses ([`CompositeError::MasksNotImplemented`]) —
//!   isolation would be decided, but mask APPLICATION has no
//!   implementation until core's mask surface lands (lane-log note).
//!   Row 5 has no field until CP-0.7 (effects).
//! * Row 1 is compiled in but unreachable through public doors today:
//!   the CP-0.1 loader pins `isolated = false` on read (lane-log
//!   coordination note; rows 2–3 exercise the same isolate arm).
//! * Group `fill_opacity` is IGNORED for group compositing: §4.6 step 4b
//!   composites groups with `(blend, opacity)` — `fill_opacity` appears
//!   only in step 7's leaf composite. The CP-0.3 refusal is retired;
//!   §11.3 revisits when effects land (CP-0.7).
//! * A group whose `opacity == 0` skips its whole subtree — αs = 0 is
//!   the source-over identity (§4.4), same rule as leaves.
//! * An isolated group none of whose descendants contributed a present
//!   tile at a coordinate is SKIPPED there (bitwise identity), matching
//!   the absent-tile leaf convention — a dense-but-transparent child
//!   still counts as a contribution (kernel ran).
//!
//! **Skips that are exact, not approximations** (each from the §4.4
//! algebra): hidden nodes (§4.6 step 1 — a hidden group hides its whole
//! subtree); `opacity == 0` layers (αs = 0 makes source-over the identity
//! for every blend mode); absent tiles (§2.4 absent ≡ transparent, αs = 0).
//!
//! **Parametric content (reopened-CP-0.7 §12.1 slice — plan §2.4 rows via
//! the O-RENDER integration handoff)**: the compositor now SERVES what
//! core's export door serves, through the same landed evaluation:
//!
//! * **Smart-object layers** (ES §11.5) composite their value-captured
//!   proxy (`cached_composite`, DD ⑨/D-48 — baked at `smartobject.convert`
//!   / `edit_contents` / `replace`, never re-composited here) exactly like
//!   a raster surface — full §4.5 blend/opacity semantics, isolated-group
//!   membership included. Non-identity placement refuses typed (§18.2 is
//!   the tools row — mirrors core's arm).
//! * **Enabled smart-filter chains** (§12.2) evaluate over the proxy per
//!   output tile through core's exported [`ChainEvaluator`] — the §12.1
//!   chain formula, per-op masks honored, nested-ROI apron discipline
//!   (every kept pixel exact). A chain with NO enabled entries is the
//!   §12.4 structural identity: the layer flattens as a plain raster
//!   leaf over the proxy (bit-true, and DAG-cacheable for free).
//! * **Adjustment layers** (§17.1/§11.4) apply as the n = 1 chain over
//!   the accumulated backdrop at ROOT scope (pass-through groups splice
//!   inline, so §11.4 pass-through-over-backdrop reach is the whole
//!   backdrop — same scope core implemented). Inside an ISOLATING group
//!   (§4.7 rows) they refuse typed with the row named; non-default
//!   compositing props on the adjustment layer itself refuse typed (the
//!   §11.4 layer-level fade is not yet defined by the §12.1 slice).
//! * **Typed-refusal matrix** for what §11/§12.1 do not define is core's,
//!   inherited verbatim from [`ChainEvaluator`]: enabled op with an
//!   unregistered kind (bare assemblies, crafted packages), non-Normal
//!   per-op fade blend, feathered op mask — mapped to
//!   [`CompositeError::ParametricNotImplemented`], never omitted (QA-F8).
//! * **Parity (§34.3)**: the parametric CPU path is byte-compatible with
//!   core's export arm on the shared serve set (the render-vs-core
//!   cross-check suite); parametric documents are FORCED-CPU (W24
//!   first-class) — op kernels are CPU-only at this slice, and the DAG
//!   delegates parametric documents here uncached (recorded deferral).
//!
//! **Subtree-composite door ([`composite_subtree_with`] /
//! [`flatten_subtree_with`] — O-SCRIPT ABI-1.4 handoff, the GROUP arm of
//! `appname_doc_read_layer`)**: composites ONE group's subtree in
//! isolation onto transparency. The content-not-contribution election
//! extends to groups: the target's own blend/opacity/visibility are NOT
//! applied, while §4.4/§4.7 semantics apply in full WITHIN the subtree.
//! Root-as-target is the whole document by structural delegation (the
//! identity cross-check pin), and the refusal matrix inherits by shared
//! code — plus one deliberate strictness: adjustment layers anywhere in a
//! group-target subtree refuse typed (their §17.1 result over the door's
//! transparency backdrop is not their document content).
//!
//! Output: [`Composite`] — the tile-grid result over the requested ROI,
//! readable per pixel, per tile (future ABI readback, ES §9.2 composite
//! readback), or as a flat RGBA f32 region (CLI export consumption; the
//! CP-0.3→wave-2 handoff makes core's export delegate here).

use std::collections::BTreeMap;
use std::sync::Arc;

use appname_core::engine::op_impl::OpAssembly;
use appname_core::{
    apply_coverage, fold_adjustment_node_weight, resolve_parametric_page, BlendMode, Buffer,
    ChainEvaluator, ChannelKind, ColorModelKind, Document, EngineAssembly, EngineError, ErrorCode,
    IRect, IVec2, LayerId, LayerKind, MaskNode, OpId, OpPlane, ResolvedMasks, TextPageData,
    TileCoord, TileStorage, TILE, TILE_I32,
};

use crate::blend::{
    composite_tile_dissolve, composite_tile_triple, composite_tile_uniform_src,
    composite_tile_uniform_src_dissolve, composite_tile_uniform_src_triple, composite_tile_with,
    routing, BlendRouting, DissolveCtx,
};
use crate::bn64::bn_offsets;
use crate::dirty::DirtyMap;
use crate::error::CompositeError;
use crate::tile_rgba::{convert_one, TileRgba};

/// Working-tile memo cache capacity (MAJOR-3b), in entries. Bound
/// reasoning: one working tile is 1 MiB (256²·4·f32) and each entry also
/// keeps its source storage payload alive (≤ 1 MiB at f32, 256 KiB at
/// u8), so 128 entries ≤ 128 MiB working + ≤ 128 MiB retained storage
/// worst case — inside the ES §4.6 render-cache floor (256 MiB). The
/// prefs-driven budget arrives with the CP-0.5 DAG cache; this constant
/// is the documented CP-0.3 stand-in.
const WORKING_TILE_CACHE_CAP: usize = 128;

/// Memoization of storage→working conversion, keyed on **Arc identity +
/// conversion params** (MAJOR-3b: `recomposite_dirty` used to re-convert
/// every layer's storage tile for every dirty tile on every recomposite —
/// the CP-0.3 brush-echo path).
///
/// Soundness of pointer-identity keying (ES §3.3 COW):
///
/// * Every entry holds a **keeper** `Arc<TileStorage>` (a strong ref).
///   While the entry lives, the allocation cannot be freed, so its
///   address cannot be reused — no stale hit via address recycling (ABA).
/// * The keeper also forces the COW-clone path for any mutation: with the
///   cache holding a second strong ref, `TileMap::make_mut` always sees a
///   shared tile and clones into a NEW `Arc` (§3.3 — in-place mutation
///   happens only at refcount 1). A mutated tile therefore ALWAYS
///   arrives under a new pointer → cache miss → fresh conversion. There
///   is no way to observe a stale working tile.
/// * The key includes `(ChannelKind, premultiplied)` because content-hash
///   dedup (§3.8) may share one payload `Arc` between buffers whose
///   conversion params differ (notably the premultiplied flag) — same
///   pointer, different working bytes, distinct entries.
///
/// Determinism: memoization of a pure function — a hit returns bytes
/// bit-identical to a fresh conversion (proven by the cached-vs-uncached
/// equality tests), so composite OUTPUT never depends on cache state.
/// Eviction (least-recently-touched, O(cap) scan over the ordered map)
/// picks by pointer value when touch counters tie; pointer values vary
/// across runs, so the EVICTION CHOICE is not run-stable — only cost,
/// never bytes, depends on it.
pub(crate) struct WorkingTileCache {
    /// Key: (Arc address, ChannelKind wire byte, premultiplied).
    map: BTreeMap<(usize, u8, bool), CacheEntry>,
    tick: u64,
}

struct CacheEntry {
    /// Strong ref pinning the keyed allocation (soundness note above).
    keeper: Arc<TileStorage>,
    touched: u64,
    tile: TileRgba,
}

impl WorkingTileCache {
    pub(crate) fn new() -> WorkingTileCache {
        WorkingTileCache {
            map: BTreeMap::new(),
            tick: 0,
        }
    }

    /// Return the working conversion of `storage`, converting at most
    /// once per (Arc identity, params) while the entry stays resident.
    pub(crate) fn get_or_convert(
        &mut self,
        storage: &Arc<TileStorage>,
        kind: ChannelKind,
        premultiplied: bool,
    ) -> &TileRgba {
        let key = (Arc::as_ptr(storage) as usize, kind.as_u8(), premultiplied);
        self.tick += 1;
        let tick = self.tick;
        if !self.map.contains_key(&key) {
            if self.map.len() >= WORKING_TILE_CACHE_CAP {
                if let Some(evict) = self
                    .map
                    .iter()
                    .min_by_key(|(_, e)| e.touched)
                    .map(|(k, _)| *k)
                {
                    self.map.remove(&evict);
                }
            }
            let tile = TileRgba::from_storage(storage, kind, premultiplied);
            self.map.insert(
                key,
                CacheEntry {
                    keeper: Arc::clone(storage),
                    touched: tick,
                    tile,
                },
            );
        }
        let entry = self
            .map
            .get_mut(&key)
            .expect("present: just checked/inserted");
        entry.touched = tick;
        debug_assert!(
            Arc::ptr_eq(&entry.keeper, storage),
            "keeper pins the keyed allocation"
        );
        &entry.tile
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

/// One composited leaf source: a visible raster layer with its resolved
/// blend routing and effective opacity, in composite order (bottom first)
/// within its scope of the render tree ([`RenderItem`]).
pub struct FlatLayer<'d> {
    /// The originating layer (receipt correlation, §3.7 marks, reports).
    pub layer: LayerId,
    buf: &'d Buffer,
    /// Blend mode (informational; `routing` is the resolved dispatch).
    pub mode: BlendMode,
    /// Effective source-alpha scale: `opacity · fill_opacity` (§4.4 note:
    /// with no layer effects at CP-0.3 both reduce to scaling αs).
    pub opacity: f32,
    routing: BlendRouting,
    /// §4.5 per-layer blue-noise offsets — resolved for Dissolve layers
    /// (zero otherwise; only the Dissolve kernel reads them).
    bn_off: (i32, i32),
    /// §11.2 resolved coverage: the layer's ENABLED raster masks, each
    /// already carrying its canonical feathered plane (Sub-slice B).
    /// Empty ⇒ full coverage (unmasked fast path). The premultiplied
    /// source is multiplied by the shared per-pixel `E`
    /// ([`ResolvedMasks::coverage_at`]) before compositing.
    coverage: ResolvedMasks<'d>,
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads; the
/// buffer's own Debug is already a summary).
impl core::fmt::Debug for FlatLayer<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FlatLayer")
            .field("layer", &self.layer)
            .field("mode", &self.mode)
            .field("opacity", &self.opacity)
            .field("masks", &self.coverage.len())
            .field("buf", &self.buf)
            .finish()
    }
}

/// One item in a render scope, in composite order (ES §2.6 z-order):
/// a leaf raster source, an isolated-group subtree (§4.7), a smart-object
/// leaf with an enabled §12.2 filter chain, or a root-scope adjustment
/// (§17.1/§11.4). Pass-through groups never appear — they flatten inline
/// at build time; smart objects whose chains have no enabled entries
/// appear as plain [`RenderItem::Layer`]s over their proxy (§12.4
/// structural identity).
pub enum RenderItem<'d> {
    /// A raster leaf (§4.6 compile step 7 at CP-0.5 scope).
    Layer(FlatLayer<'d>),
    /// An isolated group (§4.6 compile step 4b): children composite onto
    /// transparency, the result blends once against the backdrop.
    Isolated(IsolatedGroup<'d>),
    /// A smart-object leaf whose §12.2 chain has ≥ 1 enabled entry: the
    /// chain evaluates over the value-captured proxy per output tile
    /// (§12.1), then the result composites like a raster source.
    Smart(SmartLeaf<'d>),
    /// A root-scope adjustment layer: the §17.1 n = 1 chain over the
    /// accumulated backdrop (§11.4 pass-through stacking). Only the
    /// parametric segment driver consumes these — they never appear
    /// inside isolated scopes (flatten refuses typed there).
    Adjustment(AdjustmentItem<'d>),
    /// A resolved ComposedPage leaf (§20.2 shape fill / §23.2 text page):
    /// the layer's OWN premultiplied-linear page (core's
    /// [`resolve_parametric_page`] — Door 6 text / Door 1 coverage ×
    /// FillConvert, one-math-library) blended like a raster source.
    Page(PageLeaf<'d>),
}

/// A resolved ComposedPage render leaf (module doc): the shape/text layer's
/// premultiplied-linear page over its rect (core's parametric resolver
/// output — render reuses core's Door-6/Door-1×FillConvert math, never
/// linking appname-text/appname-vector), with the layer's resolved §4.5
/// compositing. A page is raster-like: it blends through render's existing
/// §4.4–§4.7 kernels exactly like a dense source.
pub struct PageLeaf<'d> {
    /// The originating shape/text layer (receipt correlation, reports).
    pub layer: LayerId,
    /// The resolved premultiplied-linear page over its rect.
    page: TextPageData,
    /// Blend mode (informational; `routing` is the resolved dispatch).
    pub mode: BlendMode,
    /// Effective source-alpha scale: `opacity · fill_opacity` (§4.4).
    pub opacity: f32,
    routing: BlendRouting,
    /// §4.5 blue-noise offsets (Dissolve layers; zero otherwise).
    bn_off: (i32, i32),
    /// §11.2 resolved coverage (enabled raster masks, feathered or not);
    /// empty ⇒ unmasked. Applied to the page's overlap tile before
    /// compositing.
    coverage: ResolvedMasks<'d>,
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for PageLeaf<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PageLeaf")
            .field("layer", &self.layer)
            .field("rect", &self.page.rect)
            .field("mode", &self.mode)
            .field("opacity", &self.opacity)
            .finish()
    }
}

/// A smart-object render leaf (module doc): the value-captured proxy plus
/// the §12.2 chain (enabled entries evaluate; disabled are §12.4-skipped
/// by the evaluator), with the layer's resolved §4.5 compositing.
pub struct SmartLeaf<'d> {
    /// The originating smart-object layer.
    pub layer: LayerId,
    /// The value-captured proxy (`cached_composite`, DD ⑨/D-48).
    proxy: &'d Buffer,
    /// The full §12.2 chain in order (evaluator applies §12.4 skips).
    chain: &'d [OpId],
    /// Blend mode (informational; `routing` is the resolved dispatch).
    pub mode: BlendMode,
    /// Effective source-alpha scale: `opacity · fill_opacity` (§4.4).
    pub opacity: f32,
    routing: BlendRouting,
    /// §4.5 blue-noise offsets (Dissolve layers; zero otherwise).
    bn_off: (i32, i32),
    /// §11.2 resolved coverage (enabled raster masks, feathered or not);
    /// empty ⇒ unmasked. Applied to the served chain output tile before
    /// compositing.
    coverage: ResolvedMasks<'d>,
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for SmartLeaf<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SmartLeaf")
            .field("layer", &self.layer)
            .field("chain_len", &self.chain.len())
            .field("mode", &self.mode)
            .field("opacity", &self.opacity)
            .finish()
    }
}

/// A root-scope adjustment item (§17.1: the n = 1 chain over the
/// backdrop).
///
/// FI-13 [NORMATIVE] (ENGINEERING-SPEC :5956-5962, frozen at
/// `v5.6-freeze`): an Adjustment node's OWN `masks` and `opacity` fold
/// into the weight of §12.1's lerp rather than into a source alpha —
///
/// ```text
/// w = node.opacity · Π coverage(node.masks) · opᵢ.opacity · mᵢ
/// ```
///
/// `eval_chain` already applies the `opᵢ.opacity · mᵢ` factor, so this
/// item carries the two NODE factors and the executor folds them in as a
/// second lerp against the same backdrop. That is exact, not an
/// approximation: `lerp(a, lerp(a, b, x), y) == lerp(a, b, x·y)` for the
/// `a + (b - a)·t` form both sites use.
#[derive(Clone, Debug)]
pub struct AdjustmentItem<'d> {
    /// The originating adjustment layer.
    pub layer: LayerId,
    /// The referenced op (the layer's whole §17.1 chain).
    pub op: OpId,
    /// `node.opacity()` — the FI-13 weight's node-opacity factor.
    pub opacity: f32,
    /// `node.blend()` — ruling-2 stage 2a. `Normal` is the collapse case and
    /// is NOT a special branch: a Normal blend returns the source unchanged,
    /// so the stage-2a plane is byte-identical to `net` and stage 2b receives
    /// exactly what FI-13 gave it.
    pub blend: BlendMode,
    /// The node's own enabled LAYER masks (`LayerNode::masks`, §10.3);
    /// their §11.2 product is the FI-13 weight's coverage factor.
    coverage: Vec<&'d MaskNode>,
}

impl<'d> AdjustmentItem<'d> {
    /// The node's own layer masks (FI-13 coverage factor).
    pub(crate) fn coverage(&self) -> &[&'d MaskNode] {
        &self.coverage
    }
}

/// An isolated group's render node (§4.7 rows 1–5; module-doc table).
pub struct IsolatedGroup<'d> {
    /// The group layer (receipt correlation, reports).
    pub layer: LayerId,
    /// The group's blend mode for the final intermediate→backdrop
    /// composite (informational; `routing` is the resolved dispatch).
    pub mode: BlendMode,
    /// Group opacity — scales the composited intermediate (§4.6 step 4b:
    /// group `fill_opacity` deliberately does NOT participate).
    pub opacity: f32,
    routing: BlendRouting,
    /// §4.5 blue-noise offsets from the GROUP's own uuid (a group can
    /// carry Dissolve; zero for other modes).
    bn_off: (i32, i32),
    /// §11.2 resolved coverage (enabled raster masks, feathered or not)
    /// on the GROUP (§4.7 row 4 isolates); empty ⇒ unmasked. Applied to the
    /// composited group intermediate before it blends against the backdrop
    /// (§4.6 step 4b).
    coverage: ResolvedMasks<'d>,
    children: Vec<RenderItem<'d>>,
}

impl<'d> IsolatedGroup<'d> {
    /// The group's children, composite order (bottom first).
    pub fn children(&self) -> &[RenderItem<'d>] {
        &self.children
    }
}

impl<'d> FlatLayer<'d> {
    /// The leaf's source buffer (crate-internal: the DAG hashes and
    /// composites through the same reads as the oracle).
    pub(crate) fn buf(&self) -> &'d Buffer {
        self.buf
    }

    /// The resolved routing (crate-internal, DAG dispatch).
    pub(crate) fn routing(&self) -> BlendRouting {
        self.routing
    }

    /// The §4.5 blue-noise offsets (crate-internal, DAG hashing).
    pub(crate) fn bn_off(&self) -> (i32, i32) {
        self.bn_off
    }

    /// The resolved §11.2 coverage masks (crate-internal, DAG hashing —
    /// the leaf hash MUST fold mask identity or a warm cache serves STALE
    /// tiles on a mask edit; requirement 2).
    pub(crate) fn coverage(&self) -> &ResolvedMasks<'d> {
        &self.coverage
    }
}

impl<'d> IsolatedGroup<'d> {
    /// The resolved routing (crate-internal, DAG dispatch).
    pub(crate) fn routing(&self) -> BlendRouting {
        self.routing
    }

    /// The §4.5 blue-noise offsets (crate-internal, DAG hashing).
    pub(crate) fn bn_off(&self) -> (i32, i32) {
        self.bn_off
    }

    /// The resolved §11.2 coverage masks on the group (crate-internal, DAG
    /// hashing — folded into the group tile hash).
    pub(crate) fn coverage(&self) -> &ResolvedMasks<'d> {
        &self.coverage
    }
}

impl<'d> RenderItem<'d> {
    /// The leaf source, if this item is one.
    pub fn as_layer(&self) -> Option<&FlatLayer<'d>> {
        match self {
            RenderItem::Layer(l) => Some(l),
            _ => None,
        }
    }

    /// The isolated-group node, if this item is one.
    pub fn as_isolated(&self) -> Option<&IsolatedGroup<'d>> {
        match self {
            RenderItem::Isolated(g) => Some(g),
            _ => None,
        }
    }

    /// The originating layer id.
    pub fn layer_id(&self) -> LayerId {
        match self {
            RenderItem::Layer(l) => l.layer,
            RenderItem::Isolated(g) => g.layer,
            RenderItem::Smart(s) => s.layer,
            RenderItem::Adjustment(a) => a.layer,
            RenderItem::Page(p) => p.layer,
        }
    }
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for RenderItem<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RenderItem::Layer(l) => f.debug_tuple("Layer").field(l).finish(),
            RenderItem::Isolated(g) => f
                .debug_struct("Isolated")
                .field("layer", &g.layer)
                .field("mode", &g.mode)
                .field("opacity", &g.opacity)
                .field("children", &g.children.len())
                .finish(),
            RenderItem::Smart(s) => f.debug_tuple("Smart").field(s).finish(),
            RenderItem::Adjustment(a) => f.debug_tuple("Adjustment").field(a).finish(),
            RenderItem::Page(p) => f.debug_tuple("Page").field(p).finish(),
        }
    }
}

/// Header of an isolated group being filled during the tree build:
/// its resolved §4.5 compositing plus §11.2 coverage (row-4 mask, if any).
struct GroupHeader<'d> {
    layer: LayerId,
    mode: BlendMode,
    routing: BlendRouting,
    opacity: f32,
    bn_off: (i32, i32),
    coverage: ResolvedMasks<'d>,
}

/// How a build scope isolates — decides the adjustment-item arm (§11.4:
/// pass-through-over-backdrop reach is defined at the DOCUMENT root scope
/// only at the §12.1 slice).
#[derive(Clone, Copy)]
enum ScopeIso {
    /// The document root scope: adjustment items serve (segment pipeline).
    Root,
    /// An isolated group scope; the §4.7 row that decided it, named in
    /// adjustment refusals.
    Row(&'static str),
    /// The subtree-composite door's target scope
    /// ([`flatten_subtree_with`]): children composite onto TRANSPARENCY,
    /// so an adjustment's §17.1 result here is not its document content —
    /// adjustment items refuse typed.
    Subtree,
}

/// One in-flight scope of the iterative tree build (§2.7: no recursion —
/// depth is user-controlled).
struct BuildFrame<'d> {
    /// `None` = the scope under build's own item list; `Some` = an
    /// isolated group being filled.
    group: Option<GroupHeader<'d>>,
    /// The scope's isolation posture (adjustment-arm dispatch).
    iso: ScopeIso,
    items: Vec<RenderItem<'d>>,
    /// Children still to visit, reversed (pop order = z-order).
    pending: Vec<LayerId>,
}

/// [`flatten_document_with`] under a BARE op registry: raster/group
/// semantics unchanged; smart-object proxies with no ENABLED chain
/// entries serve (§12.4 identity needs no kernels); any ENABLED op
/// refuses typed (its kernel is not in the empty registry — the same
/// honest refusal core's bare-assembly door makes).
pub fn flatten_document(doc: &Document) -> Result<Vec<RenderItem<'_>>, CompositeError> {
    flatten_document_with(doc, &EngineAssembly::new())
}

/// Build the render tree of `doc` (ES §2.6 z-order: index 0 first)
/// through core's public read surface, honoring visibility, flattening
/// pass-through groups inline, materializing isolated groups as subtrees
/// (§4.7 module-doc table), serving parametric content through the §12.1
/// registry `ops` carries (module doc), and refusing — typed — every
/// construct outside the implemented semantics. Chain validation happens
/// HERE (the evaluator's own refusal checks probed per §12.2 chain /
/// §17.1 op), so an unserveable document refuses before any pixel work —
/// the same door-level discipline as core's export arm.
pub fn flatten_document_with<'d>(
    doc: &'d Document,
    asm: &EngineAssembly,
) -> Result<Vec<RenderItem<'d>>, CompositeError> {
    flatten_scope_with(doc, asm, doc.root(), ScopeIso::Root)
}

/// [`flatten_subtree_with`] under a BARE op registry (see
/// [`flatten_document`] for the bare-registry serve/refuse posture).
pub fn flatten_subtree(
    doc: &Document,
    target: LayerId,
) -> Result<Vec<RenderItem<'_>>, CompositeError> {
    flatten_subtree_with(doc, &EngineAssembly::new(), target)
}

/// Build the render tree of ONE subtree of `doc` in isolation — the
/// subtree-composite door's flatten arm (O-SCRIPT ABI-1.4 handoff: the
/// GROUP arm of `appname_doc_read_layer`).
///
/// **Target semantics (content, not contribution — the per-layer readback
/// election, extended to groups):**
///
/// * `target == doc.root()`: EXACTLY [`flatten_document_with`] (structural
///   delegation — the identity cross-check contract).
/// * `target` a GROUP layer: the group's children flatten as a top-level
///   scope onto TRANSPARENCY — the §4.6 step-4b intermediate, regardless
///   of the group's own §4.7 election (a pass-through group's isolated
///   content is a DEFINITION of "content", not a decomposition of the
///   document composite — its children blend against the document
///   backdrop in situ). The group's own blend / opacity / fill_opacity /
///   visibility are contribution and are NOT applied (a hidden or faded
///   group still serves its content; pinned by test). WITHIN the subtree
///   every §4.4/§4.5/§4.6/§4.7 rule applies unchanged: child visibility,
///   nested pass-through vs isolated groups, smart leaves, the works —
///   the walk is THE document flatten walk ([`flatten_scope_with`]), so
///   the full compositor's typed-refusal matrix inherits by construction
///   (anything it refuses typed, this door refuses typed).
/// * The target's own transform / clipToBelow / masks refuse typed,
///   mirroring the flatten arms verbatim (content under those constructs
///   is undefined at this slice — the cabi per-layer readback's exact
///   node-level matrix).
/// * Non-group leaf targets refuse typed: leaf content serves through the
///   per-layer readback's §4.4 lone-source identity, not this door.
/// * Adjustment layers ANYWHERE in a group-target subtree refuse typed
///   ([`ScopeIso::Subtree`] and the nested §4.7 rows): their §17.1 result
///   over the door's transparency backdrop is not their §11.4 document
///   content — refused, never approximated (QA-F8). Note this is
///   deliberately STRICTER than the full compositor, which serves
///   root-scope adjustments: the inheritance contract is one-directional.
pub fn flatten_subtree_with<'d>(
    doc: &'d Document,
    asm: &EngineAssembly,
    target: LayerId,
) -> Result<Vec<RenderItem<'d>>, CompositeError> {
    if target == doc.root() {
        // The identity arm: the whole document IS the root's subtree.
        return flatten_document_with(doc, asm);
    }
    let node = doc
        .layer(target)
        .ok_or_else(|| CompositeError::UnsupportedSource {
            layer: target,
            detail: "subtree target does not resolve".into(),
        })?;
    // Node-level refusal matrix, mirroring the flatten arms verbatim
    // (doc comment above; visibility/blend/opacity deliberately NOT
    // consulted — contribution, not content).
    if !node.transform().is_identity() {
        return Err(CompositeError::TransformNotImplemented { layer: target });
    }
    if node.clip_to_below() {
        return Err(CompositeError::ClippingNotImplemented { layer: target });
    }
    if node
        .masks()
        .iter()
        .any(|m| doc.mask(*m).is_some_and(|n| n.enabled()))
    {
        // The subtree-composite door's masked-GROUP target is a recorded
        // thumbnail deferral (its isolated content under the mask is
        // undefined at this slice); a DISABLED mask is a §11.2 no-op and
        // passes through.
        return Err(CompositeError::MasksNotImplemented { layer: target });
    }
    if !matches!(node.kind(), LayerKind::Group { .. }) {
        return Err(CompositeError::UnsupportedSource {
            layer: target,
            detail: format!(
                "the subtree-composite door serves the document root or GROUP layers; \
                 a \"{}\" leaf's content serves through the per-layer readback \
                 (§4.4 lone-source identity)",
                node.kind().kind_str()
            ),
        });
    }
    flatten_scope_with(doc, asm, target, ScopeIso::Subtree)
}

/// The one flatten walk (module doc): build the render tree of the scope
/// rooted at `scope_root`'s children, with `scope_iso` deciding the
/// adjustment-item arm. Both public flatten doors land here — refusal
/// inheritance between them is by construction, not by parallel code.
fn flatten_scope_with<'d>(
    doc: &'d Document,
    asm: &EngineAssembly,
    scope_root: LayerId,
    scope_iso: ScopeIso,
) -> Result<Vec<RenderItem<'d>>, CompositeError> {
    if doc.color().mode != ColorModelKind::Rgba {
        return Err(CompositeError::UnsupportedDocumentMode {
            mode: doc.color().mode,
        });
    }
    let ev = ChainEvaluator::new(asm.ops(), doc);
    // Any rect works as the refusal probe: the evaluator's typed checks
    // (registration, fade blend, mask feather, resolution) are
    // rect-independent; the canvas is the natural choice.
    let probe = doc.geometry().canvas_rect();
    let mut frames: Vec<BuildFrame<'d>> = vec![BuildFrame {
        group: None,
        iso: scope_iso,
        items: Vec::new(),
        pending: doc
            .children_of(scope_root)
            .map(|c| c.iter().rev().copied().collect())
            .unwrap_or_default(),
    }];
    loop {
        let frame = frames.last_mut().expect("root frame lives until return");
        let Some(id) = frame.pending.pop() else {
            // Scope complete: fold it into its parent, or return the root.
            let done = frames.pop().expect("scope");
            match done.group {
                Some(h) => {
                    frames
                        .last_mut()
                        .expect("isolated scopes always have a parent")
                        .items
                        .push(RenderItem::Isolated(IsolatedGroup {
                            layer: h.layer,
                            mode: h.mode,
                            opacity: h.opacity,
                            routing: h.routing,
                            bn_off: h.bn_off,
                            coverage: h.coverage,
                            children: done.items,
                        }));
                    continue;
                }
                None => return Ok(done.items),
            }
        };
        let node = doc
            .layer(id)
            .ok_or_else(|| CompositeError::UnsupportedSource {
                layer: id,
                detail: "layer id in tree does not resolve".into(),
            })?;
        // §4.6 step 1: !visible skips the node — and, for a group, its
        // whole subtree (children are simply never pushed).
        if !node.visible() {
            continue;
        }
        if !node.transform().is_identity() {
            return Err(CompositeError::TransformNotImplemented { layer: id });
        }
        if node.clip_to_below() {
            return Err(CompositeError::ClippingNotImplemented { layer: id });
        }
        match node.kind() {
            LayerKind::Group { isolated } => {
                // §11.2: an enabled mask on a GROUP is §4.7 row 4
                // (isolate + mask the intermediate), but isolated-group
                // compositing has NO CPU REFERENCE path (the reference door
                // refuses every isolated group), so a masked group refuses
                // UNIFORMLY on both paths — never a serve↔refuse split —
                // until the isolated-group reference row lands. Sub-slice A
                // serves unfeathered raster masks on LEAVES (the ≥2-layer
                // direct target). Disabled masks are a no-op (empty here).
                let coverage = resolve_layer_coverage(doc, node.masks());
                if !coverage.is_empty() {
                    return Err(CompositeError::MasksNotImplemented { layer: id });
                }
                // ES §4.7 decision table, rows 1–3 (row 4 refused above,
                // row 5 has no CP-0.5 field), else row 6 pass-through.
                // First row that holds decides — and is REMEMBERED, so an
                // adjustment refusal inside the scope can name it.
                let iso_row = if *isolated {
                    Some("row 1 (isolated == true)")
                } else if node.blend() != BlendMode::Normal {
                    Some("row 2 (blend != Normal)")
                } else if node.opacity() < 1.0 {
                    Some("row 3 (opacity < 1.0)")
                } else {
                    None
                };
                if iso_row.is_none() {
                    // Row 6: flatten through — children composite against
                    // the accumulated backdrop below the group. (Group
                    // fill_opacity ignored per §4.6 step 4b, module doc.)
                    if let Some(children) = doc.children_of(id) {
                        frame.pending.extend(children.iter().rev());
                    }
                    continue;
                }
                if node.opacity() == 0.0 {
                    // αs = 0 ⇒ identity for the whole subtree (§4.4).
                    continue;
                }
                let mode = node.blend();
                let r = routing(mode);
                let bn_off = if matches!(r, BlendRouting::Dissolve) {
                    bn_offsets(doc.id(), id) // §4.5 seed rule, group uuid
                } else {
                    (0, 0)
                };
                frames.push(BuildFrame {
                    group: Some(GroupHeader {
                        layer: id,
                        mode,
                        routing: r,
                        opacity: node.opacity(),
                        bn_off,
                        coverage,
                    }),
                    iso: ScopeIso::Row(iso_row.expect("checked Some above")),
                    items: Vec::new(),
                    pending: doc
                        .children_of(id)
                        .map(|c| c.iter().rev().copied().collect())
                        .unwrap_or_default(),
                });
            }
            LayerKind::Raster(buf) => {
                // §11.2: serve the layer's ENABLED raster masks,
                // FEATHERED OR NOT (Sub-slice B). The
                // non-identity-transform refusal already fired above.
                let coverage = resolve_layer_coverage(doc, node.masks());
                if buf.model() != ColorModelKind::Rgba {
                    return Err(CompositeError::UnsupportedSource {
                        layer: id,
                        detail: format!(
                            "layer buffer model {:?} needs the §5/§6 pipeline (CP-0.5)",
                            buf.model()
                        ),
                    });
                }
                let opacity = node.opacity() * node.fill_opacity();
                if opacity == 0.0 {
                    // αs = 0 ⇒ source-over identity for every mode (§4.4)
                    // — and Dissolve's α_eff = 0 never passes a threshold
                    // (thresholds are strictly positive, §4.5 domain).
                    continue;
                }
                let mode = node.blend();
                let r = routing(mode);
                let bn_off = if matches!(r, BlendRouting::Dissolve) {
                    bn_offsets(doc.id(), id) // §4.5 seed rule
                } else {
                    (0, 0)
                };
                frame.items.push(RenderItem::Layer(FlatLayer {
                    layer: id,
                    buf,
                    mode,
                    opacity,
                    routing: r,
                    bn_off,
                    coverage,
                }));
            }
            LayerKind::SmartObject(so) => {
                // §11.2: serve raster masks on the smart leaf, feathered
                // or not (Sub-slice B).
                let coverage = resolve_layer_coverage(doc, node.masks());
                let proxy = so.cached_composite();
                if proxy.model() != ColorModelKind::Rgba {
                    return Err(CompositeError::UnsupportedSource {
                        layer: id,
                        detail: format!(
                            "smart proxy model {:?} needs the §5/§6 pipeline (CP-0.5)",
                            proxy.model()
                        ),
                    });
                }
                let opacity = node.opacity() * node.fill_opacity();
                if opacity == 0.0 {
                    // αs = 0 ⇒ source-over identity (§4.4) — exact skip,
                    // BEFORE the parametric checks (mirrors core's arm).
                    continue;
                }
                if !so.place_transform().is_identity() {
                    return Err(CompositeError::ParametricNotImplemented {
                        layer: id,
                        detail: "non-identity smart-object placement composites through \
                                 the tools resample engines (§18.2) — refused typed, \
                                 never omitted (mirrors core's export arm)"
                            .into(),
                    });
                }
                // §12.1 refusal matrix, probed through the evaluator's own
                // checks (unregistered enabled kind / non-Normal fade
                // blend / feathered op mask / unresolvable op) — core's
                // matrix verbatim, never re-derived here.
                ev.chain_input_rect(so.smart_filters(), probe)
                    .map_err(|e| map_op_err(id, e))?;
                let mode = node.blend();
                let r = routing(mode);
                let bn_off = if matches!(r, BlendRouting::Dissolve) {
                    bn_offsets(doc.id(), id) // §4.5 seed rule
                } else {
                    (0, 0)
                };
                let enabled = so
                    .smart_filters()
                    .iter()
                    .any(|o| doc.op(*o).map(|n| n.enabled()).unwrap_or(false));
                if enabled {
                    frame.items.push(RenderItem::Smart(SmartLeaf {
                        layer: id,
                        proxy,
                        chain: so.smart_filters(),
                        mode,
                        opacity,
                        routing: r,
                        bn_off,
                        coverage,
                    }));
                } else {
                    // §12.4 structural identity: no enabled entries ⇒ the
                    // proxy IS the layer's pixels — a plain raster leaf
                    // (bit-true, and §4.6-cacheable in the DAG for free).
                    frame.items.push(RenderItem::Layer(FlatLayer {
                        layer: id,
                        buf: proxy,
                        mode,
                        opacity,
                        routing: r,
                        bn_off,
                        coverage,
                    }));
                }
            }
            LayerKind::Adjustment(op) => {
                if node.opacity() == 0.0 {
                    // Exact identity skip, mirroring core's parametric arm
                    // (αs = 0 before any parametric check).
                    continue;
                }
                // FI-13 [NORMATIVE] SERVED (ENGINEERING-SPEC :5956-5962,
                // frozen at v5.6-freeze). The layer-mask refusal that stood
                // here is LIFTED, and the reasoning it rested on is now
                // answered by the letter itself rather than by this desk.
                //
                // The old comment argued: §12.1's tail defers the node's own
                // masks to "§11.2, §4.7"; §11.2 multiplies coverage into a
                // SOURCE's composited alpha before a §4.4 source-over; an
                // adjustment written back IN PLACE (§4.6 step 6) has no
                // separate source, and that route does not reduce to identity
                // at full coverage — so it could not be the rule, and no rule
                // was left. THE FROZEN TEXT NOW SUPPLIES ONE, AND IT AGREES
                // WITH THE DIAGNOSIS: "Adjustment nodes take the §12.1 weight,
                // not the §11.2 route … the node's own masks, blend and
                // opacity fold into the weight of the lerp above rather than
                // into a source alpha", and the §11.2 mechanism "MUST NOT be
                // used for an Adjustment node, because such a node has no
                // separate source to composite: it *is* the backdrop,
                // modified."
                //
                //     w = node.opacity · Π coverage(node.masks) · opᵢ.opacity · mᵢ
                //
                // `eval_chain` already applies `opᵢ.opacity · mᵢ`. The two
                // NODE factors ride on AdjustmentItem and the executor folds
                // them in as a second lerp against the SAME backdrop, which is
                // exact rather than approximate: for the `a + (b - a)·t` form
                // both sites use, lerp(a, lerp(a, b, x), y) == lerp(a, b, x·y).
                // The deviation is at FLOAT-EPSILON SCALE. One unseeded 100k-draw
                // sweep measured 4.441e-16, BUT THAT SAMPLE IS NOT A PIN — the
                // draws were unseeded, so a re-run differs and a reader cannot
                // tell a changed implementation from a different probe. THE
                // PROPERTY IS THE CLAIM, NOT THE SAMPLE.
                // eval_chain is therefore NOT modified.
                //
                // Sub-slice B: a FEATHERED layer mask is now SERVED here —
                // this is no longer a refusal site at all.
                //
                // The RAW stack is carried, not a ResolvedMasks: the FI-13 fold
                // resolves once per adjustment internally (op_eval), so
                // resolving here too would build the canonical Gaussian plane
                // twice for the same node. Feather is still honoured — it is
                // honoured INSIDE the fold.
                let coverage: Vec<&MaskNode> = node
                    .masks()
                    .iter()
                    .filter_map(|mid| doc.mask(*mid))
                    .collect();
                // node.fill_opacity IS STILL REFUSED, on its own terms:
                //
                //  · node.fill_opacity: FI-13's weight does not mention it and
                //    §4.6 step 6 does not carry it. Unlike blend this is not
                //    under-determined — THERE IS NO PATH FOR IT AT ALL, which
                //    is a different finding and refuses on its own terms.
                // RULING 2 (operator, 2026-08-28) SERVES node.blend as an
                // ENGINE-BINDING INTERPRETATION; the frozen spec is untouched.
                // FI-13's prose folds masks+blend+opacity into the weight while
                // its FORMULA carries only the two scalars -- because A BLEND
                // MODE CANNOT FOLD INTO A SCALAR. The letter has a real gap and
                // the refusal that stood here was right to name it.
                //
                // The interpretation evaluates the node unit as a second stage,
                // in the same "as one unit" order the paragraph prescribes for
                // source nodes, realized in place:
                //   stage 1   net   = the §12.1 per-op chain VERBATIM
                //   stage 2a  blend(in0, net, node.blend)   COLOUR ONLY, alpha = a0
                //   stage 2b  lerp(in0, that, node.opacity * PI coverage)
                // At node.blend = Normal a Normal blend RETURNS THE SOURCE, so
                // stage 2a is byte-identical to `net` and the whole thing
                // collapses to FI-13's published w WITHOUT A BRANCH.
                //
                // DISSOLVE STILL REFUSES: it routes to neither the separable nor
                // the triple kernel (blend.rs routing partition, 20 + 6 + Dissolve)
                // and its §4.5 semantics are a per-pixel stochastic threshold on
                // COVERAGE, which an in-place colour-only stage cannot express.
                if matches!(routing(node.blend()), BlendRouting::Dissolve) {
                    return Err(CompositeError::ParametricNotImplemented {
                        layer: id,
                        detail: "adjustment layer carries blend Dissolve; ruling 2's stage 2a is a \
                                 COLOUR-ONLY blend and Dissolve is a stochastic COVERAGE threshold \
                                 (§4.5) routing to neither kernel arm — refused typed, never omitted"
                            .into(),
                    });
                }
                if node.fill_opacity() != 1.0 {
                    return Err(CompositeError::ParametricNotImplemented {
                        layer: id,
                        detail: "adjustment layer carries non-default fill_opacity; FI-13's \
                                 weight does not mention it and §4.6 step 6 does not carry \
                                 it — no path in the frozen letter (distinct from \
                                 node.blend, which is under-determined rather than absent) \
                                 — refused typed, never omitted"
                            .into(),
                    });
                }
                match frames.last().expect("root frame lives").iso {
                    ScopeIso::Row(row) => {
                        return Err(CompositeError::ParametricNotImplemented {
                            layer: id,
                            detail: format!(
                                "adjustment layer sits inside an isolating group — §4.7 {row}; \
                                 the §12.1 slice implements §11.4 pass-through-over-backdrop \
                                 at ROOT scope only (isolated-scope adjustment reach is \
                                 undefined) — refused typed, never omitted"
                            ),
                        });
                    }
                    ScopeIso::Subtree => {
                        return Err(CompositeError::ParametricNotImplemented {
                            layer: id,
                            detail: "adjustment layer sits inside the subtree-composite \
                                     door's target scope; the door composites the target's \
                                     children onto TRANSPARENCY (content, not contribution), \
                                     and an adjustment's §17.1 result over that isolated \
                                     backdrop is not its §11.4 document content — refused \
                                     typed, never omitted (read the document composite)"
                                .into(),
                        });
                    }
                    ScopeIso::Root => {}
                }
                // §12.1 refusal matrix for the n = 1 chain (§17.1).
                ev.chain_input_rect(std::slice::from_ref(op), probe)
                    .map_err(|e| map_op_err(id, e))?;
                frames
                    .last_mut()
                    .expect("root frame lives")
                    .items
                    .push(RenderItem::Adjustment(AdjustmentItem {
                        layer: id,
                        op: *op,
                        opacity: node.opacity(),
                        blend: node.blend(),
                        coverage,
                    }));
            }
            // §22 FILL layers join this arm rather than getting one of
            // their own, and that is the point: a fill layer's page comes
            // from the SAME `resolve_parametric_page` call the shape and
            // text legs make, so this door and the reference export door
            // cannot disagree about fill pixels — there is one function
            // and both call it. `pattern` content and `noise` gradients
            // refuse typed out of that shared call, identically at both
            // doors (QA-F8: never a serve↔refuse split, which is the
            // defect class this lane already hit once).
            LayerKind::Shape(_) | LayerKind::Text(_) | LayerKind::Fill(_) => {
                // Parametric vector/text/fill content (§20.2/§23.2/§22): resolve the
                // layer's OWN premultiplied-linear page through core's
                // SINGLE resolver (Door 6 text / Door 1 coverage ×
                // FillConvert — one-math-library; the seams are the
                // assembly's TEXTVEC services, injected at assembly time —
                // render holds handles, never links appname-text/
                // appname-vector), then blend the page (raster-like) through
                // render's own §4.4–§4.7 kernels. Non-default blend/opacity
                // therefore SERVE here (render owns those kernels), the same
                // asymmetry as smart-object leaves; core's source-over-only
                // reference arm refuses them (its recorded limitation).
                // §11.2: serve raster masks on the page leaf, feathered
                // or not (Sub-slice B).
                let coverage = resolve_layer_coverage(doc, node.masks());
                let opacity = node.opacity() * node.fill_opacity();
                if opacity == 0.0 {
                    // αs = 0 ⇒ source-over identity for every mode (§4.4).
                    continue;
                }
                let page = resolve_parametric_page(
                    doc,
                    node.kind(),
                    asm.text_layout(),
                    asm.vector_geometry(),
                    asm.fill_convert_routing(),
                )
                .map_err(|e| map_op_err(id, e))?
                .expect("shape/text/fill kinds resolve to a ComposedPage");
                let mode = node.blend();
                let r = routing(mode);
                let bn_off = if matches!(r, BlendRouting::Dissolve) {
                    bn_offsets(doc.id(), id) // §4.5 seed rule
                } else {
                    (0, 0)
                };
                frame.items.push(RenderItem::Page(PageLeaf {
                    layer: id,
                    page,
                    mode,
                    opacity,
                    routing: r,
                    bn_off,
                    coverage,
                }));
            }
            // LayerKind is #[non_exhaustive]; future kinds (Fill, …) land
            // with their own composite paths.
            other => {
                return Err(CompositeError::UnsupportedSource {
                    layer: id,
                    detail: format!(
                        "layer kind \"{}\" has no CP-0.5 composite path",
                        other.kind_str()
                    ),
                });
            }
        }
    }
}

/// The composited result over one ROI: whole working tiles covering the
/// ROI's tile grid (lod 0). Tiles straddling the ROI edge are composited
/// in full (tile-granular recomposition contract); readers crop.
pub struct Composite {
    roi: IRect,
    /// Keyed `(ty, tx)` — deterministic row-major iteration.
    tiles: BTreeMap<(i32, i32), TileRgba>,
    /// Working-tile memo (MAJOR-3b): survives across
    /// [`recomposite_dirty`] calls on this composite so unchanged layers'
    /// tiles convert once. Pure memoization — never observable in output
    /// bytes (see [`WorkingTileCache`]).
    cache: WorkingTileCache,
}

/// What a dirty recomposition did (test/CI accounting; the §3.7 cache
/// hit-rate metric's CP-0.3 ancestor).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecompositeStats {
    pub tiles_recomposited: usize,
    pub tiles_in_composite: usize,
}

/// [`composite_document_with`] under a BARE op registry (see
/// [`flatten_document`]): smart proxies with no enabled chain entries
/// serve; any ENABLED op refuses typed — never omits (QA-F8).
pub fn composite_document(doc: &Document, roi: IRect) -> Result<Composite, CompositeError> {
    composite_document_with(doc, &EngineAssembly::new(), roi)
}

/// Composite the document over `roi` (document space, lod 0) — the full
/// pass, serving parametric content through the §12.1 registry `ops`
/// carries (module doc). Deterministic: fixed walk order, row-major tile
/// order, scalar math, and §12.1 evaluation pure in (params, inputs).
///
/// Documents whose root scope carries adjustment items composite through
/// the SEGMENT pipeline ([`composite_parametric_tiles`]): the whole-tile
/// output contract is preserved (evaluation runs over the requested ROI's
/// tile-aligned cover, so every emitted tile is exact in full).
pub fn composite_document_with(
    doc: &Document,
    asm: &EngineAssembly,
    roi: IRect,
) -> Result<Composite, CompositeError> {
    let flat = flatten_document_with(doc, asm)?;
    composite_flat(doc, asm.ops(), &flat, roi)
}

/// [`composite_subtree_with`] under a BARE op registry (see
/// [`flatten_document`] for the bare-registry serve/refuse posture).
pub fn composite_subtree(
    doc: &Document,
    target: LayerId,
    roi: IRect,
) -> Result<Composite, CompositeError> {
    composite_subtree_with(doc, &EngineAssembly::new(), target, roi)
}

/// Composite ONE subtree of `doc` in isolation over `roi` (document
/// space, lod 0) — the public subtree-composite door (O-SCRIPT ABI-1.4
/// handoff: the GROUP arm of `appname_doc_read_layer` consumes this,
/// additively under that symbol).
///
/// Semantics are [`flatten_subtree_with`]'s (content, not contribution:
/// the target group's children composite onto transparency with full
/// §4.4/§4.5/§4.6/§4.7 semantics WITHIN the subtree; the group's own
/// blend/opacity/visibility are never applied), driven by the SAME
/// per-tile driver as [`composite_document_with`] — one compositing
/// truth, so `composite_subtree_with(doc, ops, doc.root(), roi)` is
/// [`composite_document_with`] by construction (the identity cross-check
/// pin), and the typed-refusal matrix inherits: anything the full
/// compositor refuses typed, this door refuses typed.
///
/// Reference-path (CPU) service, deterministic like the document door
/// (fixed walk order, row-major tiles, scalar math). DAG caching for
/// subtree readback is a recorded deferral (thumbnail traffic is
/// ROI-small; the oracle path is the §34.3 truth).
pub fn composite_subtree_with(
    doc: &Document,
    asm: &EngineAssembly,
    target: LayerId,
    roi: IRect,
) -> Result<Composite, CompositeError> {
    let flat = flatten_subtree_with(doc, asm, target)?;
    composite_flat(doc, asm.ops(), &flat, roi)
}

/// The one composite driver behind both public doors: per-tile evaluation
/// of a flattened scope, or the §11.4 segment pipeline when the scope
/// carries adjustment items (only reachable from the document/root arm —
/// subtree-target scopes refuse adjustments at flatten).
fn composite_flat(
    doc: &Document,
    ops: &OpAssembly,
    flat: &[RenderItem<'_>],
    roi: IRect,
) -> Result<Composite, CompositeError> {
    let ev = ChainEvaluator::new(ops, doc);
    let mut cache = WorkingTileCache::new();
    let mut tiles = BTreeMap::new();
    if flat.iter().any(|i| matches!(i, RenderItem::Adjustment(_))) {
        tiles = composite_parametric_tiles(&ev, flat, roi, &mut cache)?;
    } else {
        for (tx, ty) in roi.tiles(TILE_I32) {
            let mut acc = TileRgba::transparent();
            let _touched = composite_into(&mut acc, flat, tx, ty, &mut cache, &ev)?;
            tiles.insert((ty, tx), acc);
        }
    }
    Ok(Composite { roi, tiles, cache })
}

/// The tile-aligned cover of `r` ([`IRect::EMPTY`] for empty input).
fn tile_cover(r: IRect) -> IRect {
    if r.is_empty() {
        return IRect::EMPTY;
    }
    IRect::new(
        r.min.x.div_euclid(TILE_I32) * TILE_I32,
        r.min.y.div_euclid(TILE_I32) * TILE_I32,
        ((r.max.x - 1).div_euclid(TILE_I32) + 1) * TILE_I32,
        ((r.max.y - 1).div_euclid(TILE_I32) + 1) * TILE_I32,
    )
}

/// Resolve a layer/group node's §11.2 coverage: its ENABLED raster masks
/// (disabled skipped — the §11.2 no-op), each carrying its canonical
/// feathered plane. Empty ⇒ unmasked. A non-identity layer transform on a
/// masked layer refuses UPSTREAM ([`CompositeError::TransformNotImplemented`],
/// checked before every kind arm), so Sub-slice C is moot here.
///
/// **FEATHER IS SERVED HERE (Sub-slice B)** — through the same
/// [`ResolvedMasks`] the CPU reference door uses, so the two compositing
/// surfaces cannot drift into a serve↔refuse split, and cannot drift on
/// the coverage value either.
///
/// **Called ONCE PER LAYER at flatten time, never per tile.** That is
/// load-bearing, not incidental: `feather_px > 96` puts the §11.2 Gaussian
/// on the kernel's Kovesi box path, whose running sum makes it
/// rect-DEPENDENT, so re-resolving per tile would produce different low
/// bits per tile and show as seams (`appname_core::graph::mask_eval`
/// module doc). `apply_coverage_tile` SAMPLES the resolved stack.
fn resolve_layer_coverage<'d>(
    doc: &'d Document,
    mask_ids: &[appname_core::MaskId],
) -> ResolvedMasks<'d> {
    let masks: Vec<&'d MaskNode> = mask_ids.iter().filter_map(|mid| doc.mask(*mid)).collect();
    ResolvedMasks::resolve(&masks)
}

/// Apply §11.2 coverage to a working source tile in place: multiply each
/// premultiplied pixel by the shared per-pixel `E`
/// ([`ResolvedMasks::coverage_at`]). No-op where `E == 1`
/// (outside the mask / full coverage), so the common unmasked interior is
/// untouched. The identical `E` the GPU multiplies (gpu.rs determinism
/// note), so CPU/GPU are bit-equal at this multiply.
fn apply_coverage_tile(src: &mut TileRgba, masks: &ResolvedMasks<'_>, tx: i32, ty: i32) {
    if masks.is_empty() {
        return;
    }
    let (ox, oy) = (tx * TILE_I32, ty * TILE_I32);
    for ly in 0..TILE {
        for lx in 0..TILE {
            let p = IVec2::new(ox + lx as i32, oy + ly as i32);
            let e = masks.coverage_at(p);
            if e != 1.0 {
                src.set(lx, ly, apply_coverage(src.get(lx, ly), e));
            }
        }
    }
}

/// Composite a working source tile over `acc` through the resolved §4.5
/// routing at `opacity` — the shared dispatch tail every leaf/intermediate
/// composite uses (bit-identical across the raster/page/smart/group arms).
fn composite_working_over(
    acc: &mut TileRgba,
    src: &TileRgba,
    routing: BlendRouting,
    opacity: f32,
    ctx: DissolveCtx,
) {
    match routing {
        BlendRouting::Separable(f) => composite_tile_with(acc, src, f, opacity),
        BlendRouting::WholePixel(f) | BlendRouting::NonSeparable(f) => {
            composite_tile_triple(acc, src, f, opacity)
        }
        BlendRouting::Dissolve => composite_tile_dissolve(acc, src, opacity, ctx),
    }
}

/// Map a §12.1 evaluator/kernel error into the compositor's typed set:
/// `NotImplemented` refusals stay refusals ([QA-F8] refuse-never-omit);
/// anything else is an evaluation failure, surfaced, never swallowed.
fn map_op_err(layer: LayerId, e: EngineError) -> CompositeError {
    if e.code == ErrorCode::NotImplemented {
        CompositeError::ParametricNotImplemented {
            layer,
            detail: e.message,
        }
    } else {
        CompositeError::OpEvaluationFailed {
            layer,
            detail: format!("{:?}: {}", e.code, e.message),
        }
    }
}

/// The §11.4 SEGMENT pipeline for root scopes carrying adjustment items:
/// composite runs of non-adjustment items tile-wise (the ordinary §4.4/
/// §4.5 kernels, bit-identical dispatch to the plain driver), and at each
/// adjustment boundary extract the accumulated backdrop as an [`OpPlane`],
/// evaluate the §17.1 n = 1 chain through core's evaluator, and continue
/// over the served result.
///
/// ROI discipline (§12.1 nested rects, §4.1 pull): the accumulator starts
/// over the widest rect any adjustment's chain needs — rects composed
/// BACKWARD from the requested ROI's tile-aligned cover — and shrinks
/// through each evaluation, so every kernel tap inside a kept region
/// reads real accumulated backdrop. Because the final rect is the
/// tile-aligned cover, every emitted tile is exact in full (the
/// compositor's whole-tile contract). Pixels a partially-covered
/// intermediate tile holds OUTSIDE its stage's kept rect are dead: no
/// extraction ever reads them (each extraction rect is the stage's exact
/// kept rect, a superset of every later rect).
fn composite_parametric_tiles<'d>(
    ev: &ChainEvaluator<'_>,
    flat: &[RenderItem<'d>],
    roi: IRect,
    cache: &mut WorkingTileCache,
) -> Result<BTreeMap<(i32, i32), TileRgba>, CompositeError> {
    let mut out_tiles = BTreeMap::new();
    if roi.is_empty() {
        return Ok(out_tiles);
    }
    let out = tile_cover(roi);

    // Segment the root scope at adjustment items: n adjustments split the
    // list into n + 1 runs (possibly empty) of ordinary items.
    let mut segments: Vec<&[RenderItem<'_>]> = Vec::new();
    let mut adjs: Vec<AdjustmentItem<'d>> = Vec::new();
    let mut seg_start = 0usize;
    for (i, item) in flat.iter().enumerate() {
        if let RenderItem::Adjustment(a) = item {
            segments.push(&flat[seg_start..i]);
            adjs.push(a.clone());
            seg_start = i + 1;
        }
    }
    segments.push(&flat[seg_start..]);

    // Backward pass (§12.1): the rect each stage must span so the LAST
    // stage lands exactly on the tile cover.
    let n = adjs.len();
    let mut rects = vec![out; n + 1];
    for k in (0..n).rev() {
        rects[k] = ev
            .chain_input_rect(std::slice::from_ref(&adjs[k].op), rects[k + 1])
            .map_err(|e| map_op_err(adjs[k].layer, e))?;
    }

    // Forward pass: composite segment k over cover(rects[k]) tile-wise,
    // then evaluate adjustment k over the exact kept rect.
    let mut grid: BTreeMap<(i32, i32), TileRgba> = BTreeMap::new();
    for (k, seg) in segments.iter().enumerate() {
        for (tx, ty) in rects[k].tiles(TILE_I32) {
            let acc = grid.entry((ty, tx)).or_insert_with(TileRgba::transparent);
            let _touched = composite_into(acc, seg, tx, ty, cache, ev)?;
        }
        if k < n {
            let plane = plane_from_grid(&grid, rects[k]);
            let served = ev
                .eval_chain(std::slice::from_ref(&adjs[k].op), plane, rects[k + 1])
                .map_err(|e| map_op_err(adjs[k].layer, e))?;
            // FI-13 [NORMATIVE] (:5956-5962): fold the NODE's own opacity and
            // layer-mask coverage into the SAME lerp weight. `eval_chain` has
            // applied `opᵢ.opacity · mᵢ`; a second lerp against the SAME
            // backdrop by `node.opacity · Π coverage(node.masks)` composes to
            // exactly `w`, because lerp(a, lerp(a, b, x), y) == lerp(a, b, x·y)
            // for the `a + (b - a)·t` form. The backdrop is re-read over the
            // OUTPUT rect (`rects[k + 1]`), not the expanded input rect, and
            // `grid` still holds the pre-adjustment tiles at this point.
            //
            // §11.2 raises alpha above αb at every non-zero coverage, which an
            // in-place adjustment MUST NEVER do (:5964's worked example). This
            // route cannot: a lerp between the backdrop and a result computed
            // FROM that backdrop returns exactly the backdrop at w = 0 and
            // never introduces alpha the two endpoints do not already carry.
            // RULING 2 stage 2a, before the stage-2b fold. At Normal this is a
            // no-op returning `served` untouched, so the FI-13 path is unchanged
            // byte-for-byte and needs no branch of its own.
            let served = blend_node_unit(&grid, &adjs[k], served, rects[k + 1]);
            let served = fold_node_weight(&grid, &adjs[k], served, rects[k + 1]);
            grid = grid_from_plane(&served);
        }
    }

    // rects[n] == out: every tile of the requested cover is exact in full.
    for (tx, ty) in roi.tiles(TILE_I32) {
        let t = grid.remove(&(ty, tx)).unwrap_or_else(TileRgba::transparent);
        out_tiles.insert((ty, tx), t);
    }
    Ok(out_tiles)
}

/// RULING 2 (operator, 2026-08-28) STAGE 2a — apply the Adjustment node's own
/// `blend` between the backdrop and the chain's result, COLOUR ONLY.
///
/// ```text
/// stage 2a   blended = blend(in0, net, node.blend)      alpha stays a0
/// stage 2b   final   = lerp(in0, blended, node.opacity * PI coverage)
/// ```
///
/// ★ AT `Normal` THIS RETURNS `net` UNTOUCHED AND ALLOCATES NOTHING. A Normal
/// blend of a source over a backdrop IS the source, so the FI-13 path is
/// byte-identical and the collapse to the published `w` is STRUCTURAL rather
/// than algebraic — it holds before any floating-point reasoning, which is
/// what the bit-identity goldens pin.
///
/// COLOUR ONLY, and alpha from the BACKDROP: :5964's invariant is that an
/// in-place adjustment never raises alpha above `ab`. §16.1's evaluation-domain
/// discipline applies — un-premultiply, apply the kernel, re-premultiply — and
/// since the §17 kernels preserve alpha bitwise, `a_net == a0`, so one alpha
/// serves both ends of the un-premultiply/re-premultiply pair exactly.
///
/// Dissolve cannot reach here: it refuses typed at the flatten arm, because it
/// routes to neither kernel and is a coverage threshold rather than a colour op.
fn blend_node_unit(
    grid: &BTreeMap<(i32, i32), TileRgba>,
    adj: &AdjustmentItem<'_>,
    mut served: OpPlane,
    rect: IRect,
) -> OpPlane {
    if adj.blend == BlendMode::Normal {
        return served;
    }
    let sep = crate::blend::render_separable_kernel(adj.blend);
    let tri = crate::blend::render_triple_kernel(adj.blend);
    let backdrop = plane_from_grid(grid, rect);
    for (b, t) in backdrop.px.iter().zip(served.px.iter_mut()) {
        let a = b[3];
        if a <= 0.0 {
            *t = *b;
            continue;
        }
        let inv = 1.0 / a;
        let bs = [b[0] * inv, b[1] * inv, b[2] * inv];
        let ts = [t[0] * inv, t[1] * inv, t[2] * inv];
        let out = if let Some(f) = sep {
            [f(bs[0], ts[0]), f(bs[1], ts[1]), f(bs[2], ts[2])]
        } else if let Some(f) = tri {
            f(bs, ts)
        } else {
            ts
        };
        *t = [out[0] * a, out[1] * a, out[2] * a, a];
    }
    served
}

/// FI-13 [NORMATIVE]: fold an Adjustment node's OWN `opacity` and
/// layer-mask coverage into §12.1's lerp weight.
///
/// THE MATH IS NOT HERE. It is [`appname_core::fold_adjustment_node_weight`],
/// the single implementation both doors call — the same one-math-library
/// discipline as [`layer_effective_coverage`], and the reason this arm and
/// the reference export door cannot drift apart. This wrapper only lifts
/// the backdrop out of the working tile grid over the OUTPUT rect
/// (`grid` still holds the pre-adjustment tiles at the call site).
fn fold_node_weight(
    grid: &BTreeMap<(i32, i32), TileRgba>,
    adj: &AdjustmentItem<'_>,
    mut served: OpPlane,
    rect: IRect,
) -> OpPlane {
    let masks = adj.coverage();
    if masks.is_empty() && adj.opacity == 1.0 {
        return served;
    }
    let backdrop = plane_from_grid(grid, rect);
    fold_adjustment_node_weight(&backdrop, &mut served, adj.opacity, masks);
    served
}

/// Extract `rect` from a tile grid as an [`OpPlane`] (exact copies; tiles
/// absent from the grid read transparent — they were never composited, so
/// transparent IS their §2.4 content).
fn plane_from_grid(grid: &BTreeMap<(i32, i32), TileRgba>, rect: IRect) -> OpPlane {
    let mut plane = OpPlane::transparent(rect);
    if rect.is_empty() {
        return plane;
    }
    let w = rect.width() as usize;
    for (tx, ty) in rect.tiles(TILE_I32) {
        let Some(t) = grid.get(&(ty, tx)) else {
            continue;
        };
        let tr = TileCoord::new(0, tx, ty).pixel_rect();
        let ov = rect.intersect(&tr);
        for y in ov.min.y..ov.max.y {
            let ly = (y - tr.min.y) as usize;
            let row = t.row_span(
                ly,
                (ov.min.x - tr.min.x) as usize,
                (ov.max.x - tr.min.x) as usize,
            );
            let base = (y - rect.min.y) as usize * w + (ov.min.x - rect.min.x) as usize;
            for (i, px) in row.chunks_exact(4).enumerate() {
                plane.px[base + i] = [px[0], px[1], px[2], px[3]];
            }
        }
    }
    plane
}

/// Splat a plane back into a tile grid over its tile cover (pixels the
/// plane does not span read transparent — DEAD pixels by the segment
/// pipeline's extraction discipline, never read back).
fn grid_from_plane(plane: &OpPlane) -> BTreeMap<(i32, i32), TileRgba> {
    let mut grid = BTreeMap::new();
    let rect = plane.rect;
    if rect.is_empty() {
        return grid;
    }
    let w = rect.width() as usize;
    for (tx, ty) in rect.tiles(TILE_I32) {
        let mut t = TileRgba::transparent();
        let tr = TileCoord::new(0, tx, ty).pixel_rect();
        let ov = rect.intersect(&tr);
        {
            let s = t.as_mut_slice();
            for y in ov.min.y..ov.max.y {
                let ly = (y - tr.min.y) as usize;
                let src_base = (y - rect.min.y) as usize * w + (ov.min.x - rect.min.x) as usize;
                let dst_base = (ly * TILE + (ov.min.x - tr.min.x) as usize) * 4;
                for (i, px) in plane.px[src_base..src_base + ov.width() as usize]
                    .iter()
                    .enumerate()
                {
                    s[dst_base + i * 4..dst_base + i * 4 + 4].copy_from_slice(px);
                }
            }
        }
        grid.insert((ty, tx), t);
    }
    grid
}

/// [`recomposite_dirty_with`] under a BARE op registry (see
/// [`flatten_document`]).
pub fn recomposite_dirty(
    comp: &mut Composite,
    doc: &Document,
    dirty: &DirtyMap,
) -> Result<RecompositeStats, CompositeError> {
    recomposite_dirty_with(comp, doc, &EngineAssembly::new(), dirty)
}

/// Recomposite ONLY the tiles of `comp` that intersect `dirty` (union
/// across layers — flat consumption per the [`crate::dirty`] module doc),
/// against the document's current state. Tiles outside the dirty set are
/// left untouched; the ROI never changes (grow = new full composite).
///
/// MINOR-5: recomposition happens **in place** into the composite's
/// existing tile allocations (reset to transparent, then recomposited) —
/// no per-tile accumulator allocation; MAJOR-3b: the composite's working
/// cache serves unchanged layers' tiles without re-conversion.
///
/// §12.1 slice: the door-level dirty rects core's chain-edit kinds now
/// emit (R1 ruling: FULL CANVAS for unmasked adjustments; ROI-inflated
/// instance bounds for chain members / below-stack extent for masked
/// adjustments) feed this door unchanged through [`DirtyMap`]. Documents
/// whose root scope carries adjustments recompute each dirty tile through
/// the segment pipeline (out = that tile — exact by the pipeline's
/// kept-rect discipline, pinned equal to a fresh full composite by the
/// integration suite).
pub fn recomposite_dirty_with(
    comp: &mut Composite,
    doc: &Document,
    asm: &EngineAssembly,
    dirty: &DirtyMap,
) -> Result<RecompositeStats, CompositeError> {
    let flat = flatten_document_with(doc, asm)?;
    let ev = ChainEvaluator::new(asm.ops(), doc);
    let has_adjustments = flat.iter().any(|i| matches!(i, RenderItem::Adjustment(_)));
    let union = dirty.union();
    let mut n = 0;
    if !union.is_empty() {
        // Split borrows: tiles and cache are disjoint fields.
        let Composite { tiles, cache, .. } = comp;
        for ((ty, tx), tile) in tiles.iter_mut() {
            let tile_rect = TileCoord::new(0, *tx, *ty).pixel_rect();
            if union.intersects(&tile_rect) {
                if has_adjustments {
                    let mut sub = composite_parametric_tiles(&ev, &flat, tile_rect, cache)?;
                    *tile = sub
                        .remove(&(*ty, *tx))
                        .unwrap_or_else(TileRgba::transparent);
                } else {
                    tile.as_mut_slice().fill(0.0); // in-place transparent reset
                    let _touched = composite_into(tile, &flat, *tx, *ty, cache, &ev)?;
                }
                n += 1;
            }
        }
    }
    Ok(RecompositeStats {
        tiles_recomposited: n,
        tiles_in_composite: comp.tiles.len(),
    })
}

/// Composite one leaf source into `acc` at tile (tx, ty); returns whether
/// a kernel ran (the scope-touched signal — module-doc skip rule).
///
/// Source routing (MINOR-5/MAJOR-3b): `Uniform` tiles composite through
/// the constant-source kernel — one [`convert_one`] call, zero
/// materialization (bit-equal to the materialized path, proven in
/// `blend::tests`); dense tiles convert through the working cache (at
/// most once per Arc identity while resident).
pub(crate) fn composite_leaf_into(
    acc: &mut TileRgba,
    l: &FlatLayer<'_>,
    tx: i32,
    ty: i32,
    cache: &mut WorkingTileCache,
) -> bool {
    let c = TileCoord::new(0, tx, ty);
    // Absent tile ⇒ transparent ⇒ αs = 0 ⇒ exact no-op (§2.4/§4.4). This
    // holds even under a mask: masking scales a transparent source, still
    // transparent (a mask never CREATES source content — §11.2).
    let Some(storage) = l.buf.get_tile(c) else {
        return false;
    };
    // Dissolve context: document-space tile origin (tile seams
    // invisible, §4.5) + the layer's resolved offsets.
    let ctx = DissolveCtx {
        bn_off: l.bn_off,
        tile_origin: (tx * TILE_I32, ty * TILE_I32),
    };
    if l.coverage.is_empty() {
        // Unmasked fast path (unchanged — the goldens bar): Uniform tiles
        // composite through the constant-source kernel (zero materialize).
        match storage.as_ref() {
            TileStorage::Uniform { value } => apply_uniform_source(
                acc,
                *value,
                l.buf.kind(),
                l.buf.premultiplied(),
                l.routing,
                l.opacity,
                ctx,
            ),
            _ => {
                let src = cache.get_or_convert(storage, l.buf.kind(), l.buf.premultiplied());
                composite_working_over(acc, src, l.routing, l.opacity, ctx);
                true
            }
        }
    } else {
        // §11.2 masked: materialize a working source tile and multiply it
        // by the per-pixel `E` (a masked Uniform is no longer uniform — `E`
        // varies across the tile — so it must materialize). Then composite
        // through the identical routing. The multiply is the SAME `E` the
        // GPU applies (`src *= coverage`), so CPU/GPU stay bit-equal.
        let mut src = match storage.as_ref() {
            TileStorage::Uniform { value } => {
                TileRgba::splat(convert_one(*value, l.buf.kind(), l.buf.premultiplied()))
            }
            _ => cache
                .get_or_convert(storage, l.buf.kind(), l.buf.premultiplied())
                .clone(),
        };
        apply_coverage_tile(&mut src, &l.coverage, tx, ty);
        composite_working_over(acc, &src, l.routing, l.opacity, ctx);
        true
    }
}

/// Materialize `rect` of a buffer as an [`OpPlane`] through the working
/// cache (the SAME storage→working conversions the leaf composite path
/// uses — [`TileRgba::from_storage`] per tile, memoized — so the §12.1
/// evaluation input is bit-identical to what a plain composite of the
/// same buffer would read). Absent tiles read transparent (§2.4; buffers
/// are unbounded §19.1, so out-of-canvas reads are real content).
fn materialize_buffer_plane(buf: &Buffer, rect: IRect, cache: &mut WorkingTileCache) -> OpPlane {
    let mut plane = OpPlane::transparent(rect);
    if rect.is_empty() {
        return plane;
    }
    let w = rect.width() as usize;
    for (tx, ty) in rect.tiles(TILE_I32) {
        let c = TileCoord::new(0, tx, ty);
        let Some(storage) = buf.get_tile(c) else {
            continue;
        };
        let t = cache.get_or_convert(storage, buf.kind(), buf.premultiplied());
        let tr = c.pixel_rect();
        let ov = rect.intersect(&tr);
        for y in ov.min.y..ov.max.y {
            let ly = (y - tr.min.y) as usize;
            let row = t.row_span(
                ly,
                (ov.min.x - tr.min.x) as usize,
                (ov.max.x - tr.min.x) as usize,
            );
            let base = (y - rect.min.y) as usize * w + (ov.min.x - rect.min.x) as usize;
            for (i, px) in row.chunks_exact(4).enumerate() {
                plane.px[base + i] = [px[0], px[1], px[2], px[3]];
            }
        }
    }
    plane
}

/// Repack a served plane spanning EXACTLY one tile rect as a working tile.
fn tile_from_plane(plane: &OpPlane) -> TileRgba {
    debug_assert_eq!(
        (plane.rect.width(), plane.rect.height()),
        (TILE_I32, TILE_I32),
        "served smart planes span exactly the output tile"
    );
    let mut t = TileRgba::transparent();
    let s = t.as_mut_slice();
    for (i, px) in plane.px.iter().enumerate() {
        s[i * 4..i * 4 + 4].copy_from_slice(px);
    }
    t
}

/// Composite one smart-object leaf into `acc` at tile (tx, ty): evaluate
/// the §12.2 chain over the value-captured proxy for exactly this tile
/// (§12.1 nested-ROI discipline: the input plane spans
/// `chain_input_rect(chain, tile)`, so every kept pixel is exact), then
/// composite the served result with the layer's §4.5 routing — the same
/// kernels, bit-identical dispatch, as any dense raster source.
///
/// Always returns `Ok(true)`: the chain kernels RAN (a served-transparent
/// result still counts as a contribution — the module-doc scope rule; and
/// a chain may lawfully create content where the proxy has none).
fn composite_smart_into(
    acc: &mut TileRgba,
    s: &SmartLeaf<'_>,
    tx: i32,
    ty: i32,
    cache: &mut WorkingTileCache,
    ev: &ChainEvaluator<'_>,
) -> Result<bool, CompositeError> {
    let out = TileCoord::new(0, tx, ty).pixel_rect();
    let in_rect = ev
        .chain_input_rect(s.chain, out)
        .map_err(|e| map_op_err(s.layer, e))?;
    let plane = materialize_buffer_plane(s.proxy, in_rect, cache);
    let served = ev
        .eval_chain(s.chain, plane, out)
        .map_err(|e| map_op_err(s.layer, e))?;
    let mut src = tile_from_plane(&served);
    // §11.2 masked smart leaf: multiply the served chain output by `E`.
    apply_coverage_tile(&mut src, &s.coverage, tx, ty);
    let ctx = DissolveCtx {
        bn_off: s.bn_off,
        tile_origin: (tx * TILE_I32, ty * TILE_I32),
    };
    composite_working_over(acc, &src, s.routing, s.opacity, ctx);
    Ok(true)
}

/// Composite one ComposedPage leaf (§20.2 shape / §23.2 text — a resolved
/// premultiplied-linear page over its rect) into `acc` at tile (tx, ty):
/// the page's overlap with the output tile materializes as a working
/// source tile (pixels the page does not span read transparent, §2.4) and
/// blends through the layer's §4.5 routing — the SAME kernels, bit-identical
/// dispatch, as any dense raster source (a page IS raster-like). Returns
/// whether a kernel ran: an empty overlap is αs = 0 over the whole tile,
/// the source-over identity (§4.4), skipped bitwise.
fn composite_page_into(acc: &mut TileRgba, p: &PageLeaf, tx: i32, ty: i32) -> bool {
    let tile_rect = TileCoord::new(0, tx, ty).pixel_rect();
    let span = tile_rect.intersect(&p.page.rect);
    if span.is_empty() {
        return false; // exact no-op (§2.4 absent ≡ transparent ⇒ §4.4 identity)
    }
    let mut src = TileRgba::transparent();
    let pw = p.page.rect.width() as usize;
    {
        let s = src.as_mut_slice();
        for y in span.min.y..span.max.y {
            let ly = (y - tile_rect.min.y) as usize;
            let prow = (y - p.page.rect.min.y) as usize * pw;
            for x in span.min.x..span.max.x {
                let lx = (x - tile_rect.min.x) as usize;
                let px = p.page.pixels[prow + (x - p.page.rect.min.x) as usize];
                let d = (ly * TILE + lx) * 4;
                s[d..d + 4].copy_from_slice(&px);
            }
        }
    }
    // §11.2 masked page leaf: multiply the page overlap tile by `E`.
    apply_coverage_tile(&mut src, &p.coverage, tx, ty);
    let ctx = DissolveCtx {
        bn_off: p.bn_off,
        tile_origin: (tx * TILE_I32, ty * TILE_I32),
    };
    composite_working_over(acc, &src, p.routing, p.opacity, ctx);
    true
}

/// One in-flight isolated scope of the iterative per-tile evaluation
/// (§2.7: no recursion — isolation nesting is user-controlled).
struct EvalFrame<'a, 'd> {
    group: &'a IsolatedGroup<'d>,
    idx: usize,
    acc: TileRgba,
    touched: bool,
}

/// Composite one output tile into `acc` (caller provides it transparent):
/// every item of the render tree bottom-to-top within its scope (§4.8
/// driver: parallel across tiles LATER — strictly ordered within a tile,
/// Porter-Duff is order-dependent). Isolated groups evaluate onto a
/// transparent intermediate, then blend once against their backdrop with
/// the group routing at group opacity (§4.6 step 4b); scopes none of
/// whose descendants ran a kernel are skipped bitwise (module-doc rule);
/// smart leaves evaluate their chain per [`composite_smart_into`].
/// Returns whether any kernel wrote into `acc`. Adjustment items never
/// reach this driver — the segment pipeline consumes them before per-tile
/// evaluation, and flatten confines them to root scope.
fn composite_into(
    acc: &mut TileRgba,
    items: &[RenderItem<'_>],
    tx: i32,
    ty: i32,
    cache: &mut WorkingTileCache,
    ev: &ChainEvaluator<'_>,
) -> Result<bool, CompositeError> {
    let mut root_idx = 0usize;
    let mut root_touched = false;
    let mut stack: Vec<EvalFrame<'_, '_>> = Vec::new();
    loop {
        // Resolve the current scope's item list and cursor.
        let (list, idx) = match stack.last() {
            Some(f) => (f.group.children(), f.idx),
            None => (items, root_idx),
        };
        if let Some(item) = list.get(idx) {
            match item {
                RenderItem::Layer(l) => {
                    // Split the borrow: leaf compositing writes the CURRENT
                    // scope's accumulator.
                    let ran = match stack.last_mut() {
                        Some(f) => composite_leaf_into(&mut f.acc, l, tx, ty, cache),
                        None => composite_leaf_into(acc, l, tx, ty, cache),
                    };
                    match stack.last_mut() {
                        Some(f) => {
                            f.touched |= ran;
                            f.idx += 1;
                        }
                        None => {
                            root_touched |= ran;
                            root_idx += 1;
                        }
                    }
                }
                RenderItem::Smart(s) => {
                    let ran = match stack.last_mut() {
                        Some(f) => composite_smart_into(&mut f.acc, s, tx, ty, cache, ev)?,
                        None => composite_smart_into(acc, s, tx, ty, cache, ev)?,
                    };
                    match stack.last_mut() {
                        Some(f) => {
                            f.touched |= ran;
                            f.idx += 1;
                        }
                        None => {
                            root_touched |= ran;
                            root_idx += 1;
                        }
                    }
                }
                RenderItem::Isolated(g) => {
                    match stack.last_mut() {
                        Some(f) => f.idx += 1,
                        None => root_idx += 1,
                    }
                    stack.push(EvalFrame {
                        group: g,
                        idx: 0,
                        acc: TileRgba::transparent(),
                        touched: false,
                    });
                }
                RenderItem::Page(p) => {
                    let ran = match stack.last_mut() {
                        Some(f) => composite_page_into(&mut f.acc, p, tx, ty),
                        None => composite_page_into(acc, p, tx, ty),
                    };
                    match stack.last_mut() {
                        Some(f) => {
                            f.touched |= ran;
                            f.idx += 1;
                        }
                        None => {
                            root_touched |= ran;
                            root_idx += 1;
                        }
                    }
                }
                RenderItem::Adjustment(_) => {
                    unreachable!(
                        "adjustment items are confined to root scope (flatten) and \
                         consumed by the segment pipeline before per-tile evaluation"
                    );
                }
            }
            continue;
        }
        // Scope exhausted.
        let Some(done) = stack.pop() else {
            return Ok(root_touched);
        };
        if !done.touched {
            continue; // bitwise skip (module-doc rule)
        }
        let g = done.group;
        let parent: &mut TileRgba = match stack.last_mut() {
            Some(f) => &mut f.acc,
            None => acc,
        };
        composite_group_intermediate(parent, &done.acc, g, tx, ty);
        match stack.last_mut() {
            Some(f) => f.touched = true,
            None => root_touched = true,
        }
    }
}

/// §4.6 step-4b composite of an isolated group's intermediate onto its
/// backdrop — the ONE routine both the oracle ([`composite_into`]) and
/// the cached engine (`crate::dag::Renderer`) call, so masked-group output
/// is identical cross-engine. Applies the group's §11.2 coverage (§4.7
/// row-4 mask) to the intermediate FIRST (multiply by the per-pixel `E`),
/// then blends through the group routing at group opacity.
pub(crate) fn composite_group_intermediate(
    parent: &mut TileRgba,
    intermediate: &TileRgba,
    g: &IsolatedGroup<'_>,
    tx: i32,
    ty: i32,
) {
    let ctx = DissolveCtx {
        bn_off: g.bn_off,
        tile_origin: (tx * TILE_I32, ty * TILE_I32),
    };
    if g.coverage.is_empty() {
        composite_working_over(parent, intermediate, g.routing, g.opacity, ctx);
    } else {
        let mut masked = intermediate.clone();
        apply_coverage_tile(&mut masked, &g.coverage, tx, ty);
        composite_working_over(parent, &masked, g.routing, g.opacity, ctx);
    }
}

/// Apply one Uniform source value to an accumulator tile — the
/// `composite_into` uniform arm, extracted so the normative zero-alpha
/// behavior is directly testable.
///
/// [NORMATIVE — O-INT ruling, perf-round review minor] **Zero-alpha
/// uniform sources are skipped, and the skip is the §4.4-EXACT
/// semantics**: with αs = 0 the source-over equation is the identity
/// (`Co = αb·Cb`, `αo = αb`), so the output IS the backdrop, bitwise.
/// This skip does not merely avoid work — it REPLACES a computing path
/// that was not exact: running either kernel with αs = 0 evaluates
/// `αb·(p/αb)` per channel (un-premultiply → re-premultiply), which
/// perturbs ~11% of f32 (p, αb) pairs by 1 ulp. That perturbation was
/// the latent wrong; identity is the law. The materialized/uniform
/// kernels MUST NOT be handed zero-alpha uniform sources, and the
/// CP-0.5 GPU path MUST implement this same exact skip (parity-harness
/// handoff — lane log ruling record). Pinned by
/// `equality_suite_zero_alpha_uniform_pins_skip_semantics`.
///
/// The skip is routing-independent: αs = 0 makes source-over the
/// identity for every §4.5 routing (separable, whole-pixel,
/// non-separable — all share the Porter-Duff wrapper), and a zero-alpha
/// Dissolve source has `α_eff = 0`, which never exceeds a threshold
/// (thresholds are strictly positive, §4.5 domain) — identity again.
///
/// Returns whether a kernel actually ran (`false` = the normative skip
/// fired) — the scope-touched signal for the §4.7 evaluator.
pub(crate) fn apply_uniform_source(
    acc: &mut TileRgba,
    value: [f32; 4],
    kind: ChannelKind,
    premultiplied: bool,
    r: BlendRouting,
    opacity: f32,
    ctx: DissolveCtx,
) -> bool {
    let w = convert_one(value, kind, premultiplied);
    if w[3] == 0.0 {
        return false; // NORMATIVE zero-alpha skip (doc comment above)
    }
    match r {
        BlendRouting::Separable(f) => composite_tile_uniform_src(acc, w, f, opacity),
        BlendRouting::WholePixel(f) | BlendRouting::NonSeparable(f) => {
            composite_tile_uniform_src_triple(acc, w, f, opacity)
        }
        BlendRouting::Dissolve => composite_tile_uniform_src_dissolve(acc, w, opacity, ctx),
    }
    true
}

impl Composite {
    /// Assemble a composite from computed tiles (crate-internal: the DAG
    /// renderer's output door; the working cache starts fresh — it only
    /// serves this composite's own [`recomposite_dirty`] calls).
    pub(crate) fn from_parts(roi: IRect, tiles: BTreeMap<(i32, i32), TileRgba>) -> Composite {
        Composite {
            roi,
            tiles,
            cache: WorkingTileCache::new(),
        }
    }

    /// The requested region (document space).
    pub fn roi(&self) -> IRect {
        self.roi
    }

    /// One whole composited tile of the ROI's grid cover.
    pub fn tile(&self, tx: i32, ty: i32) -> Option<&TileRgba> {
        self.tiles.get(&(ty, tx))
    }

    /// All tiles, deterministic row-major — the tile-iterator output shape
    /// (plan §2.2: "composite Buffer (or tile iterator)"; ABI readback
    /// consumes this per §9.2 at CP-0.3's C-ABI row).
    pub fn tiles(&self) -> impl Iterator<Item = ((i32, i32), &TileRgba)> {
        self.tiles.iter().map(|(&(ty, tx), t)| ((tx, ty), t))
    }

    /// Read one pixel (document space): linear premultiplied RGBA.
    /// Outside the covered tile grid reads transparent black (§4.3).
    pub fn pixel(&self, p: IVec2) -> [f32; 4] {
        let (tx, ty) = (p.x.div_euclid(TILE_I32), p.y.div_euclid(TILE_I32));
        match self.tiles.get(&(ty, tx)) {
            Some(t) => t.get(
                p.x.rem_euclid(TILE_I32) as usize,
                p.y.rem_euclid(TILE_I32) as usize,
            ),
            None => [0.0; 4],
        }
    }

    /// Flat row-major RGBA f32 readback of `r` (linear premultiplied;
    /// `r.area() · 4` scalars) — the export-consumption surface (plan
    /// §2.2 output note (a)). Pixels outside the covered grid read
    /// transparent black. Span-copied per tile row, no per-pixel dispatch.
    pub fn region_rgba_f32(&self, r: IRect) -> Vec<f32> {
        if r.is_empty() {
            return Vec::new();
        }
        let w = r.width() as usize;
        let mut out = vec![0.0f32; w * r.height() as usize * 4];
        for y in r.min.y..r.max.y {
            let ty = y.div_euclid(TILE_I32);
            let ly = y.rem_euclid(TILE_I32) as usize;
            let row_base = (y - r.min.y) as usize * w * 4;
            let mut x = r.min.x;
            while x < r.max.x {
                let tx = x.div_euclid(TILE_I32);
                let lx0 = x.rem_euclid(TILE_I32) as usize;
                let span = ((TILE - lx0) as i32).min(r.max.x - x) as usize;
                if let Some(t) = self.tiles.get(&(ty, tx)) {
                    let src = t.row_span(ly, lx0, lx0 + span);
                    let dst_off = row_base + (x - r.min.x) as usize * 4;
                    out[dst_off..dst_off + span * 4].copy_from_slice(src);
                }
                x += span as i32;
            }
        }
        out
    }
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for Composite {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Composite")
            .field("roi", &self.roi)
            .field("tiles", &self.tiles.len())
            .field("cached_working_tiles", &self.cache.map.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Composite type mechanics on hand-built tile grids (document-level
    /// behavior is exercised by the integration suite through core's
    /// public doors).
    fn comp_with(tiles: Vec<((i32, i32), TileRgba)>, roi: IRect) -> Composite {
        Composite {
            roi,
            tiles: tiles
                .into_iter()
                .map(|((tx, ty), t)| ((ty, tx), t))
                .collect(),
            cache: WorkingTileCache::new(),
        }
    }

    fn uniform_arc(value: [f32; 4]) -> Arc<TileStorage> {
        Arc::new(TileStorage::Uniform { value })
    }

    /// MAJOR-3b: a cache hit returns bytes bit-identical to a fresh
    /// conversion, and the entry count reflects memoization.
    #[test]
    fn cache_hit_is_bitidentical_to_fresh_conversion() {
        let mut cache = WorkingTileCache::new();
        let a = uniform_arc([0.5, 0.25, 0.125, 0.5]);
        let fresh = TileRgba::from_storage(&a, ChannelKind::F32, true);
        let first = cache.get_or_convert(&a, ChannelKind::F32, true).clone();
        let hit = cache.get_or_convert(&a, ChannelKind::F32, true).clone();
        assert!(
            first == fresh && hit == fresh,
            "hit bytes == fresh conversion bytes"
        );
        assert_eq!(cache.len(), 1, "one Arc, one params set — one entry");
    }

    /// The key carries conversion params: one shared payload Arc under
    /// premult=true vs =false (the §3.8 dedup sharing case) yields two
    /// entries with different working bytes.
    #[test]
    fn cache_key_separates_conversion_params() {
        let mut cache = WorkingTileCache::new();
        let shared = uniform_arc([1.0, 0.5, 0.25, 0.5]);
        let premult = cache
            .get_or_convert(&shared, ChannelKind::F32, true)
            .clone();
        let straight = cache
            .get_or_convert(&shared, ChannelKind::F32, false)
            .clone();
        assert_eq!(cache.len(), 2);
        assert_eq!(premult.get(0, 0), [1.0, 0.5, 0.25, 0.5]);
        assert_eq!(straight.get(0, 0), [0.5, 0.25, 0.125, 0.5]);
    }

    /// Eviction: the map never exceeds the cap; evicted entries reconvert
    /// on next use with identical bytes (memoization invariant).
    #[test]
    fn cache_caps_and_evicted_entries_reconvert_identically() {
        let mut cache = WorkingTileCache::new();
        let arcs: Vec<Arc<TileStorage>> = (0..WORKING_TILE_CACHE_CAP + 8)
            .map(|i| uniform_arc([i as f32 / 512.0, 0.0, 0.0, 1.0]))
            .collect();
        for a in &arcs {
            cache.get_or_convert(a, ChannelKind::F32, true);
        }
        assert_eq!(cache.len(), WORKING_TILE_CACHE_CAP, "cap enforced");
        // Whether arcs[0] is resident or was evicted, its conversion is
        // byte-stable.
        let again = cache
            .get_or_convert(&arcs[0], ChannelKind::F32, true)
            .clone();
        assert!(again == TileRgba::from_storage(&arcs[0], ChannelKind::F32, true));
        assert!(cache.len() <= WORKING_TILE_CACHE_CAP);
    }

    /// The keeper holds a strong ref while cached (the ABA/COW-forcing
    /// soundness argument depends on it).
    #[test]
    fn cache_keeper_pins_the_allocation() {
        let mut cache = WorkingTileCache::new();
        let a = uniform_arc([0.25, 0.25, 0.25, 1.0]);
        assert_eq!(Arc::strong_count(&a), 1);
        cache.get_or_convert(&a, ChannelKind::F32, true);
        assert_eq!(
            Arc::strong_count(&a),
            2,
            "entry keeps the keyed allocation alive"
        );
    }

    /// Naive-reference equality suite, zero-alpha-uniform case
    /// [NORMATIVE pin — O-INT ruling, perf-round review minor]: the
    /// EXPECTED bytes for a zero-alpha uniform source are the SKIP
    /// semantics — output ≡ backdrop, bitwise (§4.4 with αs = 0 is the
    /// identity). The old materialized path is deliberately NOT the
    /// reference here: its αs = 0 un-premultiply→re-premultiply round
    /// trip (`αb·(p/αb)`) perturbs backdrop channels by 1 ulp on ~11% of
    /// f32 pairs — the latent wrong the ruling retired. This pin exists
    /// BEFORE zero-alpha uniforms are reachable through public doors;
    /// CP-0.5 `fill` makes them reachable (tripwire in the integration
    /// suite), and its author extends the document-level equality tests
    /// with reachable fixtures against exactly this expectation.
    #[test]
    fn equality_suite_zero_alpha_uniform_pins_skip_semantics() {
        let f = crate::blend::separable_fn(BlendMode::Normal).unwrap();

        // Deterministic scan for a premult (p, αb) pair whose
        // un-premultiply→re-premultiply round trip is NOT exact in f32 —
        // the reviewer measured ~11% of pairs perturb, so a small fixed
        // grid must contain one (hand-picked probes can all land in the
        // exact 89%, as the first draft of this test proved).
        // Independent p/αb grids (deriving p from a product with αb
        // biases the round trip toward exactness — first draft's lesson);
        // p ≤ αb keeps the probe §6.4-conventional, though f32 premult
        // does not require it.
        let mut perturbing: Option<(f32, f32)> = None;
        'scan: for j in 1..500u32 {
            let ab = j as f32 / 500.0;
            for k in 1..997u32 {
                let p = k as f32 / 997.0;
                if p <= ab && (p / ab) * ab != p {
                    perturbing = Some((p, ab));
                    break 'scan;
                }
            }
        }
        let (pp, pab) = perturbing.expect(
            "no perturbing (p, αb) pair in a ~250k-pair grid — the ruling's ~11% premise \
             changed; re-raise to O-INT",
        );

        // Backdrop probes: fixed values plus the FOUND perturbing pair.
        let mut backdrop = TileRgba::transparent();
        let vals: [[f32; 4]; 5] = [
            [0.1, 0.3, 0.7, 0.3],
            [0.2, 0.9, 0.05, 0.95],
            [0.30, 0.30, 0.30, 0.50],
            [0.0, 0.0, 0.0, 1.0],
            [pp, pp, pp, pab],
        ];
        for (i, v) in vals.iter().enumerate() {
            backdrop.set(i, 0, *v);
        }

        // Zero-alpha uniform sources through every conversion arm:
        // premult f32 (all-zero), straight f32 with §6.4-legit hidden
        // RGB, canonical u8 with hidden color.
        let cases: [([f32; 4], ChannelKind, bool); 3] = [
            ([0.0, 0.0, 0.0, 0.0], ChannelKind::F32, true),
            ([0.5, 0.25, 0.125, 0.0], ChannelKind::F32, false),
            ([128.0 / 255.0, 0.0, 0.0, 0.0], ChannelKind::U8, true),
        ];
        for (value, kind, premult) in cases {
            // The skip condition fires: converted working alpha is 0.
            assert_eq!(convert_one(value, kind, premult)[3], 0.0);
            // Pinned expectation: bitwise backdrop identity — for EVERY
            // routing (the skip is routing-independent; doc comment).
            for r in [
                BlendRouting::Separable(f),
                routing(BlendMode::Hue),
                routing(BlendMode::DarkerColor),
                routing(BlendMode::Dissolve),
            ] {
                let mut acc = backdrop.clone();
                apply_uniform_source(
                    &mut acc,
                    value,
                    kind,
                    premult,
                    r,
                    1.0,
                    DissolveCtx::default(),
                );
                assert!(
                    acc == backdrop,
                    "zero-alpha uniform {value:?}/{kind:?}/{r:?} must be the exact identity"
                );
            }
        }

        // Counterfactual, documenting WHY the skip is load-bearing: the
        // retired computing path (materialize + kernel) is NOT the
        // identity — its round trip perturbs at least one of the probe
        // backdrops by 1 ulp.
        let zero_src = TileRgba::from_storage(
            &TileStorage::Uniform { value: [0.0; 4] },
            ChannelKind::F32,
            true,
        );
        let mut via_retired_path = backdrop.clone();
        composite_tile_with(&mut via_retired_path, &zero_src, f, 1.0);
        assert!(
            via_retired_path != backdrop,
            "expected the retired αs=0 computing path to perturb ≥1 probe pixel"
        );
        // The perturbation shows exactly at the scanned pair …
        assert_ne!(
            via_retired_path.get(4, 0)[0].to_bits(),
            pp.to_bits(),
            "the found perturbing pair must diverge under the retired path"
        );
        // … and is bounded to the round trip: alphas are exact even on
        // the retired path (αo = 0 + αb·1).
        for (i, v) in vals.iter().enumerate() {
            assert_eq!(via_retired_path.get(i, 0)[3].to_bits(), v[3].to_bits());
        }
    }

    #[test]
    fn pixel_reads_cross_tile_and_outside_reads_zero() {
        let mut a = TileRgba::transparent();
        a.set(255, 0, [0.1, 0.2, 0.3, 1.0]);
        let mut b = TileRgba::transparent();
        b.set(0, 0, [0.4, 0.5, 0.6, 1.0]);
        let c = comp_with(vec![((0, 0), a), ((1, 0), b)], IRect::new(0, 0, 512, 256));
        assert_eq!(c.pixel(IVec2::new(255, 0)), [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(c.pixel(IVec2::new(256, 0)), [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(c.pixel(IVec2::new(-1, -1)), [0.0; 4]);
    }

    #[test]
    fn tiles_iterate_row_major() {
        let t = TileRgba::transparent;
        let c = comp_with(
            vec![((1, 1), t()), ((0, 0), t()), ((1, 0), t()), ((0, 1), t())],
            IRect::new(0, 0, 512, 512),
        );
        let order: Vec<(i32, i32)> = c.tiles().map(|(k, _)| k).collect();
        assert_eq!(order, vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
    }

    #[test]
    fn region_readback_crops_and_spans_tiles() {
        let mut a = TileRgba::transparent();
        a.set(254, 10, [0.1, 0.1, 0.1, 1.0]);
        a.set(255, 10, [0.2, 0.2, 0.2, 1.0]);
        let mut b = TileRgba::transparent();
        b.set(0, 10, [0.3, 0.3, 0.3, 1.0]);
        let c = comp_with(vec![((0, 0), a), ((1, 0), b)], IRect::new(0, 0, 512, 256));
        // A 3×1 window straddling the tile seam at x = 256.
        let px = c.region_rgba_f32(IRect::new(254, 10, 257, 11));
        assert_eq!(px.len(), 12);
        assert_eq!(&px[0..4], &[0.1, 0.1, 0.1, 1.0]);
        assert_eq!(&px[4..8], &[0.2, 0.2, 0.2, 1.0]);
        assert_eq!(&px[8..12], &[0.3, 0.3, 0.3, 1.0]);
        // A window over uncovered grid reads zeros.
        let px = c.region_rgba_f32(IRect::new(-4, -4, -2, -3));
        assert_eq!(px, vec![0.0; 8]);
    }

    #[test]
    fn empty_region_readback_is_empty() {
        let c = comp_with(vec![], IRect::EMPTY);
        assert!(c.region_rgba_f32(IRect::EMPTY).is_empty());
    }
}
