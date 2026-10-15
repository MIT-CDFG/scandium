// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §4.6 (layer-DAG evaluation and caching — post-order, the
//! render-tile cache keyed by content hash, generation sweep, [PERF]
//! ancestry-proportional re-render), §4.1 (render graph + pull-driven ROI;
//! at CP-0.5 scope every apron is zero: no effects/ops, identity
//! transforms, masks refused — the propagation shape is here, the
//! expansions arrive with their owners), §4.5 (Dissolve document-space
//! dependence folds into hashes), §2.7 (deduplicating shared sub-results),
//! §3.3 (COW — the Arc-identity memo soundness), §34.3 (the DAG output is
//! pinned bit-equal to the naive oracle); DD ⑤⑧; plan §2.3 "wgpu render
//! DAG v1 (post-order, content-hash dedup, ROI scheduling)" — CPU executor
//! (the wgpu executor rides slice D on the same plan/hash machinery).
//!
//! The production renderer: a [`Renderer`] holds the render-tile cache
//! across frames and evaluates documents through the same tree + kernels
//! as the oracle ([`crate::compositor`]), probing the cache at every node
//! before recomputing. **Caching is output-invisible**: a hit returns
//! bytes bit-identical to the recompute it replaced (memoization of pure
//! per-tile evaluation — pinned by the DAG≡oracle equality suite), so the
//! oracle stays the single behavior authority (§34.3) and the cache can
//! only change cost, never pixels. The equality contract is scoped to
//! the REQUESTED ROI ([`Renderer::render`] doc — outside it, §4.1
//! step-5 skips legitimately diverge from the oracle's full-edge-tile
//! contract).
//!
//! **Per-(node, tile) content hashing.** Each plan node's identity at a
//! tile folds everything that affects its output bytes there (§4.6
//! `recompute_hash`, made tile-granular):
//!
//! * leaf: a domain tag, blend mode, effective opacity, the buffer's
//!   conversion params (kind + premultiplied), and the STORAGE tile's
//!   content hash at that coordinate (absent ⇒ the transparent-tile
//!   hash, §2.4);
//! * isolated group: a domain tag, blend mode, group opacity, and the
//!   children's tile hashes in composite order (child edits invalidate
//!   ancestors — §4.6);
//! * root scope: a domain tag and the top-level items' tile hashes.
//!
//! Dissolve nodes additionally fold their §4.5 blue-noise offsets AND the
//! tile's document-space origin — the pattern is document-anchored, so
//! content-identical Dissolve nodes at different layers (different seed)
//! or different coordinates (different pattern window) must never share
//! entries. Conversely, identical content under identical params at
//! DIFFERENT coordinates hashes identically for every other mode — the
//! §2.7 dedup: one cache entry serves both places.
//!
//! **Costs.** Storage-tile hashing (blake3 over ≤1 MiB) is memoized by
//! Arc identity ([`TileHashCache`] — same keeper/COW soundness argument
//! as the CP-0.3 [`WorkingTileCache`]), so warm renders hash only edited
//! tiles. The render-tile cache is LRU under a byte budget with a
//! two-generation reachability sweep (§4.6's generation sweep, expressed
//! renderer-side: the renderer sees documents at render time, so "live"
//! means reachable in this or the previous render — keeps undo-toggle
//! ping-pong warm while dropping stale states).

use std::collections::BTreeMap;
use std::sync::Arc;

use appname_core::hash::ContentHash;
use appname_core::{
    Document, EngineAssembly, IRect, ResolvedMasks, TileCoord, TileStorage, TILE_I32,
};

use crate::blend::BlendRouting;
use crate::compositor::{
    composite_document_with, composite_group_intermediate, composite_leaf_into,
    flatten_document_with, Composite, FlatLayer, IsolatedGroup, RenderItem, WorkingTileCache,
};
use crate::error::CompositeError;
use crate::tile_rgba::{TileRgba, TILE_SCALARS};

/// Render-tile cache budget (bytes) — the ES §4.6 floor
/// (`clamp(15% · engine_ram_budget, 256 MiB, 4 GiB)`), documented CP-0.5
/// stand-in until the prefs plumbing lands (same pattern as the
/// working-tile cache cap).
pub const RENDER_TILE_CACHE_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// One cached tile is a full working tile plus key overhead.
const ENTRY_BYTES: usize = TILE_SCALARS * core::mem::size_of::<f32>() + 64;

// ---------------------------------------------------------------------
// Storage-tile content-hash memo (Arc identity, COW-sound).
// ---------------------------------------------------------------------

/// Memoizes `TileStorage::content_hash` per **(Arc identity, kind,
/// model)** — the WorkingTileCache soundness argument (ES §3.3 COW), all
/// three bullets: the keeper strong-ref pins the allocation (no ABA) and
/// forces the clone-on-write path, so mutated content always arrives
/// under a new pointer; and the key carries the hash's OTHER inputs —
/// `content_hash(kind, model)` encodes a `Uniform` value at the map's
/// depth (`hash_uniform::<u8>` ≠ `hash_uniform::<f32>` for one value),
/// and Uniform Arcs are exempt from core's cross-context tag guard, so
/// pointer-only keying could alias one Arc observed under two contexts
/// (slice C+E review MINOR-4 — unreachable at CP-0.5's one-depth
/// documents, keyed correctly anyway). A hit returns the identical
/// `ContentHash` a fresh hash would (pure fn memo).
struct TileHashCache {
    map: BTreeMap<(usize, u8, u8), (Arc<TileStorage>, ContentHash)>,
    tick_map: BTreeMap<(usize, u8, u8), u64>,
    tick: u64,
    cap: usize,
}

impl TileHashCache {
    fn new() -> TileHashCache {
        // Hash entries are ~50 bytes + a keeper ref; the keeper retains
        // storage payloads (≤1 MiB each), so cap like the working cache.
        TileHashCache {
            map: BTreeMap::new(),
            tick_map: BTreeMap::new(),
            tick: 0,
            cap: 1024,
        }
    }

    fn get_or_hash(
        &mut self,
        storage: &Arc<TileStorage>,
        map: &appname_core::TileMap,
    ) -> ContentHash {
        let key = (
            Arc::as_ptr(storage) as usize,
            map.kind().as_u8(),
            map.model() as u8,
        );
        self.tick += 1;
        if !self.map.contains_key(&key) {
            if self.map.len() >= self.cap {
                if let Some((&evict, _)) = self.tick_map.iter().min_by_key(|(_, &t)| t) {
                    self.map.remove(&evict);
                    self.tick_map.remove(&evict);
                }
            }
            let h = storage.content_hash(map.kind(), map.model());
            self.map.insert(key, (Arc::clone(storage), h));
        }
        self.tick_map.insert(key, self.tick);
        self.map.get(&key).expect("just inserted").1
    }
}

// ---------------------------------------------------------------------
// The render-tile cache (ES §4.6): LRU + byte budget + generation sweep.
// ---------------------------------------------------------------------

struct CacheEntry {
    tile: TileRgba,
    tick: u64,
    gen: u64,
}

struct RenderTileCache {
    map: BTreeMap<[u8; 32], CacheEntry>,
    budget: usize,
    tick: u64,
    evictions: u64,
}

impl RenderTileCache {
    fn new(budget: usize) -> RenderTileCache {
        RenderTileCache {
            map: BTreeMap::new(),
            budget,
            tick: 0,
            evictions: 0,
        }
    }

    fn get(&mut self, key: &[u8; 32], gen: u64) -> Option<&TileRgba> {
        self.tick += 1;
        let tick = self.tick;
        let e = self.map.get_mut(key)?;
        e.tick = tick;
        e.gen = gen;
        Some(&e.tile)
    }

    fn insert(&mut self, key: [u8; 32], tile: TileRgba, gen: u64) {
        self.tick += 1;
        while (self.map.len() + 1) * ENTRY_BYTES > self.budget && !self.map.is_empty() {
            // LRU eviction; ticks are unique, so the choice is
            // deterministic for a deterministic call sequence.
            if let Some((&evict, _)) = self.map.iter().min_by_key(|(_, e)| e.tick) {
                self.map.remove(&evict);
                self.evictions += 1;
            }
        }
        if ENTRY_BYTES <= self.budget {
            self.map.insert(
                key,
                CacheEntry {
                    tile,
                    tick: self.tick,
                    gen,
                },
            );
        }
    }

    /// The §4.6 generation sweep, renderer-side: drop entries not touched
    /// in this or the previous render generation.
    fn sweep(&mut self, gen: u64) -> usize {
        let before = self.map.len();
        self.map.retain(|_, e| e.gen + 1 >= gen);
        before - self.map.len()
    }
}

// ---------------------------------------------------------------------
// Plan arena (post-order; children before parents — §4.6 evaluation).
// ---------------------------------------------------------------------

enum PlanSrc<'a, 'd> {
    Leaf(&'a FlatLayer<'d>),
    Group(&'a IsolatedGroup<'d>),
}

struct PlanNode<'a, 'd> {
    src: PlanSrc<'a, 'd>,
    /// Child arena indices in composite order (empty for leaves).
    children: Vec<usize>,
}

struct Plan<'a, 'd> {
    /// Post-order arena: every child index precedes its parent's index.
    nodes: Vec<PlanNode<'a, 'd>>,
    /// Top-level items (the root scope), composite order.
    roots: Vec<usize>,
}

/// Build the arena iteratively (§2.7: no recursion — depth is
/// user-controlled). Children are appended before their parent, so a
/// simple forward pass evaluates hashes bottom-up.
fn build_plan<'a, 'd>(items: &'a [RenderItem<'d>]) -> Plan<'a, 'd> {
    struct Frame<'a, 'd> {
        items: &'a [RenderItem<'d>],
        idx: usize,
        group: Option<&'a IsolatedGroup<'d>>,
        child_ids: Vec<usize>,
    }
    let mut plan = Plan {
        nodes: Vec::new(),
        roots: Vec::new(),
    };
    let mut stack = vec![Frame {
        items,
        idx: 0,
        group: None,
        child_ids: Vec::new(),
    }];
    loop {
        let top = stack.last_mut().expect("root frame lives until return");
        if let Some(item) = top.items.get(top.idx) {
            top.idx += 1;
            match item {
                RenderItem::Layer(l) => {
                    plan.nodes.push(PlanNode {
                        src: PlanSrc::Leaf(l),
                        children: Vec::new(),
                    });
                    top.child_ids.push(plan.nodes.len() - 1);
                }
                RenderItem::Isolated(g) => stack.push(Frame {
                    items: g.children(),
                    idx: 0,
                    group: Some(g),
                    child_ids: Vec::new(),
                }),
                RenderItem::Smart(_) | RenderItem::Adjustment(_) | RenderItem::Page(_) => {
                    unreachable!(
                        "parametric items delegate to the oracle path before plan build \
                         (Renderer::render_with parametric routing)"
                    )
                }
            }
            continue;
        }
        let done = stack.pop().expect("frame");
        match done.group {
            Some(g) => {
                plan.nodes.push(PlanNode {
                    src: PlanSrc::Group(g),
                    children: done.child_ids,
                });
                let id = plan.nodes.len() - 1;
                stack
                    .last_mut()
                    .expect("isolated scopes always have a parent")
                    .child_ids
                    .push(id);
            }
            None => {
                plan.roots = done.child_ids;
                return plan;
            }
        }
    }
}

// ---------------------------------------------------------------------
// Per-(node, tile) hashing (module doc).
// ---------------------------------------------------------------------

/// Domain tags keep leaf/group/root key spaces disjoint even under
/// otherwise-identical folds.
const TAG_LEAF: u8 = 0x4C; // 'L'
const TAG_GROUP: u8 = 0x47; // 'G'
const TAG_ROOT: u8 = 0x52; // 'R'

/// §4.5: a Dissolve node's pattern is document-anchored — its hash folds
/// the seed offsets AND the tile's document-space origin (module doc).
/// Returned as bytes so hash sites stay closure-generic over the hasher.
fn dissolve_fold_bytes(
    routing: BlendRouting,
    bn_off: (i32, i32),
    tx: i32,
    ty: i32,
) -> Option<[u8; 16]> {
    if !matches!(routing, BlendRouting::Dissolve) {
        return None;
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&bn_off.0.to_le_bytes());
    out[4..8].copy_from_slice(&bn_off.1.to_le_bytes());
    out[8..12].copy_from_slice(&(tx * TILE_I32).to_le_bytes());
    out[12..16].copy_from_slice(&(ty * TILE_I32).to_le_bytes());
    Some(out)
}

/// Second load-bearing core invariant (slice C+E review MINOR-2,
/// alongside the §2.5 bounds ⊇ content dependency documented at the
/// need-rect computation): **no dense tile whose materialized bytes are
/// uniform-valued survives a public core door** — the loader, importer,
/// and command commit all run the §2.4 prune/collapse sweep (bitwise
/// zero → absent; uniform-valued dense → `Uniform`). The cache key
/// cannot distinguish a `Uniform` from its dense twin (their content
/// hashes are equal BY core's design), but evaluation CAN at zero
/// working alpha (normative uniform skip vs the dense kernel's 1-ulp
/// round trip) — the prune discipline is what makes that one key ↔ one
/// output. Tripwired by `prune_discipline_holds_through_public_doors`
/// in the DAG integration suite.
fn leaf_tile_hash(l: &FlatLayer<'_>, tx: i32, ty: i32, hashes: &mut TileHashCache) -> [u8; 32] {
    let c = TileCoord::new(0, tx, ty);
    let map = l.buf().tiles();
    let content = match l.buf().get_tile(c) {
        Some(storage) => hashes.get_or_hash(storage, map),
        // Absent ⇒ transparent (§2.4); cheap (uniform hash), no memo.
        None => map.content_hash_at(c),
    };
    // §11.2 coverage identity (requirement 2): fold each ENABLED mask's
    // density, inverted flag, and coverage-tile content hash. Without this
    // the warm cache returns STALE tiles on a mask edit (mask.edit fill /
    // mask.set_props) — silent-wrong, worse than the refusal. Precomputed
    // before the stream closure (the borrow shape `get_or_hash` needs).
    let cov = coverage_fold(l.coverage(), c, hashes);
    let h = ContentHash::from_stream(|h| {
        h.update(&[TAG_LEAF]);
        h.update(&(l.mode as u32).to_le_bytes());
        h.update(&l.opacity.to_le_bytes());
        h.update(&[l.buf().kind().as_u8(), l.buf().premultiplied() as u8]);
        if let Some(d) = dissolve_fold_bytes(l.routing(), l.bn_off(), tx, ty) {
            h.update(&d);
        }
        h.update(&(cov.len() as u32).to_le_bytes());
        for f in &cov {
            h.update(&f.density);
            h.update(&[f.inverted]);
            h.update(&f.feather);
            // Length-prefixed: the tile count varies with Rmask, and an
            // unprefixed concatenation could alias two different folds.
            h.update(&(f.tiles.len() as u32).to_le_bytes());
            for ch in &f.tiles {
                h.update(ch.as_bytes());
            }
        }
        h.update(content.as_bytes());
    });
    *h.as_bytes()
}

fn group_tile_hash(
    g: &IsolatedGroup<'_>,
    child_hashes: &[[u8; 32]],
    tx: i32,
    ty: i32,
    hashes: &mut TileHashCache,
) -> [u8; 32] {
    let c = TileCoord::new(0, tx, ty);
    // §11.2 row-4 group coverage identity — same rationale as the leaf.
    let cov = coverage_fold(g.coverage(), c, hashes);
    let h = ContentHash::from_stream(|h| {
        h.update(&[TAG_GROUP]);
        h.update(&(g.mode as u32).to_le_bytes());
        h.update(&g.opacity.to_le_bytes());
        if let Some(d) = dissolve_fold_bytes(g.routing(), g.bn_off(), tx, ty) {
            h.update(&d);
        }
        h.update(&(cov.len() as u32).to_le_bytes());
        for f in &cov {
            h.update(&f.density);
            h.update(&[f.inverted]);
            h.update(&f.feather);
            // Length-prefixed: the tile count varies with Rmask, and an
            // unprefixed concatenation could alias two different folds.
            h.update(&(f.tiles.len() as u32).to_le_bytes());
            for ch in &f.tiles {
                h.update(ch.as_bytes());
            }
        }
        for ch in child_hashes {
            h.update(ch);
        }
    });
    *h.as_bytes()
}

/// One enabled mask's per-tile hash inputs: density, inverted flag,
/// FEATHER, and the coverage-tile content hashes this output tile
/// actually depends on.
struct MaskFold {
    density: [u8; 4],
    inverted: u8,
    feather: [u8; 4],
    /// One hash for an unfeathered mask (its own tile); the whole
    /// `Rmask`-inflated tile neighbourhood for a feathered one. Row-major
    /// `(ty, tx)` order, so the fold is deterministic.
    tiles: Vec<ContentHash>,
}

/// Precompute the §11.2 coverage fold for a leaf/group at tile `c`. Done
/// outside the `from_stream` closure because `TileHashCache::get_or_hash`
/// needs `&mut` while the closure borrows the hasher.
///
/// **★ THE FEATHER WIDENS THE DEPENDENCY, SO IT WIDENS THE KEY.** Without
/// feather, tile `c` of the composite depends on exactly tile `c` of each
/// mask's coverage, and hashing that one tile is a complete key. With
/// feather it is NOT: a `mask.edit` at `q` changes the feathered coverage
/// throughout `q.inflate(Rmask)`, so an edit in a NEIGHBOURING coverage
/// tile changes this tile's output while leaving a single-tile key
/// unchanged — the warm cache would then serve a STALE tile, which is
/// silent-wrong and strictly worse than the refusal this slice replaced.
/// So a feathered mask folds every coverage tile intersecting
/// `c.pixel_rect().inflate(Rmask)`.
///
/// `Rmask = ⌈3σ⌉ = ⌈feather_px⌉` is a true upper bound on BOTH kernel
/// paths (the direct kernel's `⌈3σ⌉` support, and the 3-box path's total
/// support which is `≤ ⌈3σ⌉` by construction), so this neighbourhood is
/// sufficient, and it is TIGHT — a pixel at distance exactly `Rmask` does
/// change (`appname_algo::rgba_blur::tests::a_one_pixel_mask_edit_changes_exactly_the_rmask_neighbourhood`).
///
/// `feather` itself is folded unconditionally: a `mask.set_props` that
/// only changes feather alters every output pixel of the layer, and
/// before this slice nothing in the key would have noticed.
fn coverage_fold(
    masks: &ResolvedMasks<'_>,
    c: TileCoord,
    hashes: &mut TileHashCache,
) -> Vec<MaskFold> {
    masks
        .masks()
        .map(|m| {
            let mc = m.coverage();
            let cov_map = mc.tiles();
            let hash_at = |coord: TileCoord, hashes: &mut TileHashCache| match mc.get_tile(coord) {
                Some(storage) => hashes.get_or_hash(storage, cov_map),
                None => cov_map.content_hash_at(coord),
            };
            let reach = appname_core::feather_reach_px(m.feather());
            let tiles = if reach == 0 {
                vec![hash_at(c, hashes)]
            } else {
                c.pixel_rect()
                    .inflate(reach, reach)
                    .tiles(TILE_I32)
                    .map(|(tx, ty)| hash_at(TileCoord::new(c.lod, tx, ty), hashes))
                    .collect()
            };
            MaskFold {
                density: m.density().to_le_bytes(),
                inverted: m.inverted() as u8,
                feather: m.feather().to_le_bytes(),
                tiles,
            }
        })
        .collect()
}

fn root_tile_hash(root_hashes: &[[u8; 32]]) -> [u8; 32] {
    let h = ContentHash::from_stream(|h| {
        h.update(&[TAG_ROOT]);
        for rh in root_hashes {
            h.update(rh);
        }
    });
    *h.as_bytes()
}

// ---------------------------------------------------------------------
// The renderer.
// ---------------------------------------------------------------------

/// Per-render accounting (the §4.6 [PERF] hit-rate contracts, testable).
/// Reset at the start of every [`Renderer::render`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RenderStats {
    /// Whole-tile results served from cache (no evaluation at all).
    pub root_hits: u64,
    /// Whole-tile results evaluated this render.
    pub root_misses: u64,
    /// Isolated-group intermediates served from cache.
    pub group_hits: u64,
    /// Isolated-group intermediates evaluated this render.
    pub group_misses: u64,
    /// Leaf kernel invocations (composite of one source tile).
    pub leaf_kernel_runs: u64,
    /// Group→backdrop composite kernel invocations.
    pub group_composites: u64,
    /// Entries dropped by the LRU byte budget this render.
    pub evictions: u64,
    /// Entries dropped by the generation sweep this render.
    pub swept: u64,
    /// Entries resident after this render.
    pub entries: u64,
    /// (node, tile) visits skipped by the §4.1 step-5 empty-need rule.
    pub roi_skips: u64,
}

/// The stateful production renderer (module doc): §4.6 evaluation with
/// content-hash caching over the oracle's own tree and kernels. Behavior
/// authority stays [`crate::compositor::composite_document`] — the
/// equality suite pins `Renderer::render` bit-equal to it, cold and warm.
pub struct Renderer {
    cache: RenderTileCache,
    hashes: TileHashCache,
    working: WorkingTileCache,
    stats: RenderStats,
    gen: u64,
}

impl Default for Renderer {
    fn default() -> Renderer {
        Renderer::new()
    }
}

impl Renderer {
    /// A renderer with the default byte budget
    /// ([`RENDER_TILE_CACHE_BUDGET_BYTES`]).
    pub fn new() -> Renderer {
        Renderer::with_budget_bytes(RENDER_TILE_CACHE_BUDGET_BYTES)
    }

    /// A renderer with an explicit render-tile-cache byte budget
    /// (tests exercise eviction with small budgets).
    pub fn with_budget_bytes(budget: usize) -> Renderer {
        Renderer {
            cache: RenderTileCache::new(budget),
            hashes: TileHashCache::new(),
            working: WorkingTileCache::new(),
            stats: RenderStats::default(),
            gen: 0,
        }
    }

    /// Accounting for the most recent [`Renderer::render`] call.
    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    /// [`Renderer::render_with`] under a BARE op registry: raster/group
    /// documents render cached as ever; smart-object proxies with no
    /// enabled chain entries are plain cached leaves; any ENABLED op
    /// refuses typed (unregistered in the empty registry — QA-F8).
    pub fn render(&mut self, doc: &Document, roi: IRect) -> Result<Composite, CompositeError> {
        self.render_with(doc, &EngineAssembly::new(), roi)
    }

    /// Render `doc` over `roi` (document space, lod 0) through the cache.
    /// **Within the requested ROI**, output is bit-equal to
    /// [`composite_document_with`] of the same inputs, for any cache
    /// state (module doc invariant). Outside the ROI the two doors differ
    /// BY SPEC on edge tiles: the oracle composites ROI-straddling tiles
    /// in full (its documented tile-granular contract), while this engine
    /// applies the §4.1 step-5 empty-need skip — content whose need
    /// misses the ROI never composites, so out-of-ROI reads of covered
    /// edge tiles may legitimately diverge (slice C+E review MINOR-3).
    /// Readers that need pixels must include them in the ROI.
    ///
    /// **Parametric routing (§12.1 slice — RECORDED DEFERRAL)**: a tree
    /// carrying evaluated parametric items (enabled smart-filter chains,
    /// adjustment layers) renders through the ORACLE path directly —
    /// bit-equal by construction, FORCED-CPU (W24 first-class; the op
    /// kernels are CPU-only at this slice), and UNCACHED: §4.6
    /// content-hash caching for parametric nodes needs op-aware node keys
    /// (params/enabled/mask folds) and §4.1 apron-aware need rects, which
    /// arrive with the op-owning lanes' GPU pairs. Stats reset to zero
    /// for such renders (nothing probed, nothing cached). Smart layers
    /// whose chains have no enabled entries do NOT trigger the deferral —
    /// they flatten as plain proxy leaves and cache normally.
    ///
    /// [`composite_document_with`]: crate::compositor::composite_document_with
    pub fn render_with(
        &mut self,
        doc: &Document,
        asm: &EngineAssembly,
        roi: IRect,
    ) -> Result<Composite, CompositeError> {
        let tree = flatten_document_with(doc, asm)?;
        if tree_has_parametric(&tree) {
            self.stats = RenderStats::default();
            return composite_document_with(doc, asm, roi);
        }
        let plan = build_plan(&tree);
        self.gen += 1;
        self.stats = RenderStats::default();

        // §4.1 pull-driven ROI: per-node need rects, bottom-up (children
        // precede parents in the arena). At CP-0.5 every apron is zero
        // (no effects/ops, identity transforms), so a leaf's need is
        // roi ∩ its buffer bounds (§4.1 step 3: content bounds, NEVER
        // the canvas) and a group's is the union of its children's.
        // Nodes whose need misses a tile are skipped there entirely
        // (§4.1 step 5) — sound because an absent-by-bounds subtree
        // contributes exactly nothing (§2.5 bounds ⊇ content; the
        // DAG≡oracle equality suite is the standing tripwire for that
        // core invariant), so omitting it from fold and evaluation
        // changes neither bytes nor key semantics.
        let mut need: Vec<IRect> = Vec::with_capacity(plan.nodes.len());
        for node in &plan.nodes {
            let r = match &node.src {
                PlanSrc::Leaf(l) => roi.intersect(&l.buf().bounds()),
                PlanSrc::Group(_) => node
                    .children
                    .iter()
                    .fold(IRect::EMPTY, |acc, &c| acc.union(&need[c])),
            };
            need.push(r);
        }

        let mut tiles = BTreeMap::new();
        let mut node_hashes = vec![[0u8; 32]; plan.nodes.len()];
        let mut alive = vec![false; plan.nodes.len()];
        let mut child_buf: Vec<[u8; 32]> = Vec::new();
        for (tx, ty) in roi.tiles(TILE_I32) {
            let tile_rect = TileCoord::new(0, tx, ty).pixel_rect();
            // Pass 1 — needs + hashes, bottom-up over the post-order
            // arena; dead nodes are neither hashed nor folded.
            for i in 0..plan.nodes.len() {
                alive[i] = !need[i].intersect(&tile_rect).is_empty();
                if !alive[i] {
                    self.stats.roi_skips += 1;
                    continue;
                }
                node_hashes[i] = match &plan.nodes[i].src {
                    PlanSrc::Leaf(l) => leaf_tile_hash(l, tx, ty, &mut self.hashes),
                    PlanSrc::Group(g) => {
                        child_buf.clear();
                        child_buf.extend(
                            plan.nodes[i]
                                .children
                                .iter()
                                .filter(|&&c| alive[c])
                                .map(|&c| node_hashes[c]),
                        );
                        group_tile_hash(g, &child_buf, tx, ty, &mut self.hashes)
                    }
                };
            }
            child_buf.clear();
            child_buf.extend(
                plan.roots
                    .iter()
                    .filter(|&&r| alive[r])
                    .map(|&r| node_hashes[r]),
            );
            let root_key = root_tile_hash(&child_buf);

            // Pass 2 — evaluate with probes.
            if let Some(hit) = self.cache.get(&root_key, self.gen) {
                self.stats.root_hits += 1;
                tiles.insert((ty, tx), hit.clone());
                continue;
            }
            self.stats.root_misses += 1;
            let acc = self.evaluate_tile(&plan, &node_hashes, &alive, tx, ty);
            self.cache.insert(root_key, acc.clone(), self.gen);
            tiles.insert((ty, tx), acc);
        }
        self.stats.swept = self.cache.sweep(self.gen) as u64;
        self.stats.evictions = self.cache.evictions;
        self.cache.evictions = 0;
        self.stats.entries = self.cache.map.len() as u64;
        Ok(Composite::from_parts(roi, tiles))
    }

    /// Evaluate one tile of the root scope with per-group cache probes —
    /// the compositor's stack machine, cache-aware (kernels and order
    /// identical to the oracle; probes replace subtree evaluation with
    /// the bytes that evaluation produced).
    fn evaluate_tile(
        &mut self,
        plan: &Plan<'_, '_>,
        node_hashes: &[[u8; 32]],
        alive: &[bool],
        tx: i32,
        ty: i32,
    ) -> TileRgba {
        struct EvalFrame<'p> {
            node: usize,
            children: &'p [usize],
            idx: usize,
            acc: TileRgba,
            touched: bool,
        }
        let mut root_acc = TileRgba::transparent();
        let mut root_idx = 0usize;
        let mut stack: Vec<EvalFrame<'_>> = Vec::new();
        loop {
            let (list, idx) = match stack.last() {
                Some(f) => (f.children, f.idx),
                None => (&plan.roots[..], root_idx),
            };
            if let Some(&node_id) = list.get(idx) {
                match stack.last_mut() {
                    Some(f) => f.idx += 1,
                    None => root_idx += 1,
                }
                if !alive[node_id] {
                    continue; // §4.1 step-5 empty-need skip
                }
                match &plan.nodes[node_id].src {
                    PlanSrc::Leaf(l) => {
                        let acc = match stack.last_mut() {
                            Some(f) => &mut f.acc,
                            None => &mut root_acc,
                        };
                        let ran = composite_leaf_into(acc, l, tx, ty, &mut self.working);
                        if ran {
                            self.stats.leaf_kernel_runs += 1;
                            if let Some(f) = stack.last_mut() {
                                f.touched = true
                            }
                        }
                    }
                    PlanSrc::Group(g) => {
                        // Probe the intermediate before evaluating the
                        // subtree (§4.6 step 2).
                        if let Some(hit) = self.cache.get(&node_hashes[node_id], self.gen) {
                            self.stats.group_hits += 1;
                            let hit = hit.clone();
                            let acc = match stack.last_mut() {
                                Some(f) => &mut f.acc,
                                None => &mut root_acc,
                            };
                            composite_group_intermediate(acc, &hit, g, tx, ty);
                            self.stats.group_composites += 1;
                            if let Some(f) = stack.last_mut() {
                                f.touched = true;
                            }
                            continue;
                        }
                        self.stats.group_misses += 1;
                        stack.push(EvalFrame {
                            node: node_id,
                            children: &plan.nodes[node_id].children,
                            idx: 0,
                            acc: TileRgba::transparent(),
                            touched: false,
                        });
                    }
                }
                continue;
            }
            let Some(done) = stack.pop() else {
                return root_acc;
            };
            if !done.touched {
                continue; // untouched-scope bitwise skip (compositor rule)
            }
            let PlanSrc::Group(g) = &plan.nodes[done.node].src else {
                unreachable!("only groups push frames");
            };
            // Cache the pre-blend intermediate (§4.1 variant-1 entry).
            self.cache
                .insert(node_hashes[done.node], done.acc.clone(), self.gen);
            let acc = match stack.last_mut() {
                Some(f) => &mut f.acc,
                None => &mut root_acc,
            };
            composite_group_intermediate(acc, &done.acc, g, tx, ty);
            self.stats.group_composites += 1;
            if let Some(f) = stack.last_mut() {
                f.touched = true;
            }
        }
    }
}

/// Does the flattened tree carry parametric items the cached engine must
/// delegate (module-doc parametric routing)? Smart leaves can sit inside
/// isolated subtrees; adjustments are root-only (flatten's contract).
fn tree_has_parametric(items: &[RenderItem<'_>]) -> bool {
    let mut stack: Vec<&[RenderItem<'_>]> = vec![items];
    while let Some(list) = stack.pop() {
        for item in list {
            match item {
                RenderItem::Smart(_) | RenderItem::Adjustment(_) | RenderItem::Page(_) => {
                    // ComposedPage leaves (shape/text) route through the
                    // oracle at this slice — §4.6 content-hash caching for
                    // pages needs page-aware node keys (recorded deferral,
                    // module doc; the same FORCED-CPU/uncached posture as
                    // smart chains and adjustments).
                    return true;
                }
                RenderItem::Isolated(g) => stack.push(g.children()),
                RenderItem::Layer(_) => {}
            }
        }
    }
    false
}

#[cfg(test)]
mod coverage_fold_tests {
    use super::*;
    use appname_core::hash::ContentHash;
    use appname_core::{Engine, EngineAssembly, MaskId};
    use serde_json::json;

    const L: &str = "33c40000-0000-4000-8000-000000000001";
    const M: &str = "33c40000-0000-4000-8000-000000000002";

    /// A doc whose canvas is `n × n`, carrying a `reveal_all` raster mask
    /// on a pasted opaque layer, feathered by `feather` px. The mask's
    /// stored coverage is exactly the canvas rect, so the canvas size
    /// decides which coverage TILES are occupied and what they contain.
    fn masked_doc(n: u32, feather: f64) -> Engine {
        let mut e = appname_core::engine::kinds::register(EngineAssembly::new()).build();
        let cmd = |e: &mut Engine, id: u32, kind: &str, target, params| {
            let mut v = json!({
                "schema": "appname.command.v1",
                "id": format!("33c40000-0000-4000-8000-{id:012x}"),
                "kind": kind,
                "params": params,
            });
            if let Some(t) = target {
                v["target"] = t;
            }
            e.apply_json(&v.to_string())
                .unwrap_or_else(|err| panic!("{kind}: {} {}", err.code.as_str(), err.message));
        };
        cmd(
            &mut e,
            1,
            "doc.new",
            None,
            json!({"width": n, "height": n, "background": "white"}),
        );
        let plane: Vec<f32> = std::iter::repeat([0.0f32, 0.0, 1.0, 1.0])
            .take((n * n) as usize)
            .flatten()
            .collect();
        let png = appname_core::encode_composite_png(n, n, &plane).expect("encode");
        let asset = e.add_raster_asset(png).expect("asset");
        cmd(
            &mut e,
            2,
            "edit.paste",
            None,
            json!({"assetId": asset.to_string(), "at": [0, 0], "newLayerId": L}),
        );
        cmd(
            &mut e,
            3,
            "mask.add",
            Some(json!({"layerId": L})),
            json!({"kind": "raster", "from": "reveal_all", "newMaskId": M}),
        );
        if feather > 0.0 {
            cmd(
                &mut e,
                4,
                "mask.set_props",
                Some(json!({"layerId": L, "maskId": M})),
                json!({"feather": feather}),
            );
        }
        e
    }

    fn fold_of(e: &Engine, c: TileCoord) -> Vec<[u8; 32]> {
        let doc = e.document().expect("initialized");
        let lid = appname_core::LayerId::from_uuid(L.parse().expect("layer uuid"));
        let mid: MaskId = doc.layer(lid).expect("layer").masks()[0];
        let mask = doc.mask(mid).expect("mask");
        let resolved = ResolvedMasks::resolve(&[mask]);
        let mut hashes = TileHashCache::new();
        coverage_fold(&resolved, c, &mut hashes)
            .iter()
            .map(|f| {
                let h = ContentHash::from_stream(|h| {
                    h.update(&f.density);
                    h.update(&[f.inverted]);
                    h.update(&f.feather);
                    h.update(&(f.tiles.len() as u32).to_le_bytes());
                    for ch in &f.tiles {
                        h.update(ch.as_bytes());
                    }
                });
                *h.as_bytes()
            })
            .collect()
    }

    /// **THE STALE-TILE GUARD.** Two documents whose mask coverage is
    /// IDENTICAL inside tile (0,0) but DIFFERENT in the neighbouring tiles
    /// (canvas 300 vs 260 — both fully cover tile (0,0) with 1.0, and
    /// differ in how far the coverage extends into tiles (1,0)/(0,1)).
    ///
    /// * Unfeathered, tile (0,0) of the composite depends only on tile
    ///   (0,0) of the coverage, so the two folds MUST agree — the cache
    ///   key stays as tight as it was, no false invalidation.
    /// * Feathered, tile (0,0) also depends on coverage within `Rmask` of
    ///   it, which reaches into those neighbours, so the two folds MUST
    ///   DIFFER. If they agreed, a warm cache would serve a STALE tile
    ///   after a neighbouring coverage edit — silent-wrong, and strictly
    ///   worse than the refusal this slice replaced.
    #[test]
    fn feathered_fold_depends_on_the_rmask_neighbourhood_unfeathered_does_not() {
        let c = TileCoord::new(0, 0, 0);
        // Sanity: tile (0,0) really is identically covered in both docs.
        let plain_a = fold_of(&masked_doc(300, 0.0), c);
        let plain_b = fold_of(&masked_doc(260, 0.0), c);
        assert_eq!(
            plain_a, plain_b,
            "unfeathered: tile (0,0) coverage is identical, so the key must \
             not widen — a differing key here would mean needless cache misses"
        );

        // Feathered: Rmask = 40 reaches out of tile (0,0) (TILE = 256, and
        // 256 + 40 > 256), so the neighbours are now dependencies.
        let feathered_a = fold_of(&masked_doc(300, 40.0), c);
        let feathered_b = fold_of(&masked_doc(260, 40.0), c);
        assert_ne!(
            feathered_a, feathered_b,
            "feathered: tile (0,0) depends on coverage within Rmask, which \
             differs between these docs — an equal key is a STALE-TILE bug"
        );
    }

    /// FEATHER ITSELF must be in the key. Before this slice the fold
    /// carried density and inverted but NOT feather, so a `mask.set_props`
    /// that changed only feather would leave every cache key untouched
    /// while changing every output pixel.
    #[test]
    fn changing_only_feather_changes_the_cache_key() {
        let c = TileCoord::new(0, 0, 0);
        let none = fold_of(&masked_doc(300, 0.0), c);
        let some = fold_of(&masked_doc(300, 40.0), c);
        let more = fold_of(&masked_doc(300, 41.0), c);
        assert_ne!(none, some, "feather 0 vs 40 must key differently");
        assert_ne!(some, more, "feather 40 vs 41 must key differently");
    }
}
