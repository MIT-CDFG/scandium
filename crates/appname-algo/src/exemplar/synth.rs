// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §21.4 SCHEDULE / CANDIDATES / COMMIT / REFINE stages; ES §45.3
//! constant table rows "Priority schedule", "Coherence candidates",
//! "Index", "Refine passes R"; ALGORITHMS.md §3.B identity contract.
//! Clean-room from criminisi04 (priority front), ashikhmin01 (coherence
//! candidates), hertzmann01 (ANN patch search); census key
//! `caf_primary_cleanroom` (verdict none-found by design — no vendorable
//! primary exists).
//!
//! STRUCTURAL RIGHTS INVARIANTS (contract `content_aware_fill` rows,
//! CI-enforced per ES §45.3 [LEGAL] / Appendix Q §Q.3), maintained here
//! by construction:
//!
//! * NO per-pixel best-match store of any kind exists ("no NN-field" in
//!   the contract's words): each query's candidate set is assembled
//!   fresh, used, and dropped — the only per-pixel record this module
//!   keeps is [`SrcRef`], the COMMITTED synthesis provenance (which
//!   source pixel was copied where), written exactly once at commit
//!   time. That record is sanctioned expressly by §45.3's "Coherence
//!   candidates | committed 8-neighbors' sources, offset-advanced" row
//!   and §21.4 candidate class (a). It is not a neighbor-field in the
//!   restricted sense: it is never updated by search, never iterated
//!   over, never carried between pyramid levels or refine iterations as
//!   a search state — it is output bookkeeping (also consumed by the
//!   seam pass), not search state.
//! * NOTHING computed by one query seeds, biases, or warm-starts a later
//!   query ("no propagation between iterations"): candidate sets are
//!   closed-form functions of (immutable index, committed pixels, query
//!   position) — §21.4: "candidate sets are closed-form functions of
//!   committed state — no field survives between pixels".
//! * NO random or perturbed search window exists: candidate enumeration
//!   is exhaustive (coherence set + full best-first k-d walk); the §45.3
//!   seeded streams decide only heap TIE-BREAKS among exactly-equal
//!   priorities and the PCA sample draw — never which candidates are
//!   examined ("no random probe is ever taken", §21.4).
//!
//! Determinism (D-48): single-threaded, fixed-order scalar code; every
//! comparison that could tie carries a total deterministic key; the two
//! seeded draws are stateless counter-mode reads (§45.3), so the fill is
//! a pure function of (image, masks, params incl. seed) — byte-exact.

use crate::exemplar::descriptor::{extract_patch, valid_centers, Pca, PATCH_RADIUS, PATCH_SIDE};
use crate::exemplar::kdtree::KdTree;
use crate::exemplar::stream::{draw_u64, STREAM_TIEBREAK};
use crate::plane::Plane;
use std::collections::BinaryHeap;

/// Sentinel: pixel not owned by any committed patch (known pixels, or
/// degenerate diffusion-kept pixels). Its "label color" is the buffer.
pub(crate) const NO_OWNER: u32 = u32::MAX;
/// Sentinel library id inside [`SrcRef`]: no recorded source.
pub(crate) const NO_SRC: u16 = u16::MAX;

/// Committed synthesis provenance of one filled pixel: which library
/// pixel was copied here (§45.3 coherence-candidate row; written once at
/// commit, read by neighboring queries within the SAME fill, never
/// mutated by search).
#[derive(Clone, Copy)]
pub(crate) struct SrcRef {
    pub lib: u16,
    pub x: u16,
    pub y: u16,
}

impl SrcRef {
    pub(crate) const NONE: SrcRef = SrcRef {
        lib: NO_SRC,
        x: 0,
        y: 0,
    };
    pub(crate) fn is_some(&self) -> bool {
        self.lib != NO_SRC
    }
}

/// One committed patch (target center + chosen source center).
#[derive(Clone, Copy)]
pub(crate) struct PatchRec {
    pub lib: u16,
    pub sx: u16,
    pub sy: u16,
    pub tx: u16,
    pub ty: u16,
}

/// One exemplar library: a materialized (identity or transform-augmented)
/// rendering with its valid patch centers and immutable descriptor index.
pub(crate) struct Library {
    pub planes: Vec<Plane>,
    pub centers: Vec<(u32, u32)>,
    /// O(1) center-validity lookup (w*h bitmap), for coherence candidates.
    pub center_ok: Vec<bool>,
    pub tree: KdTree,
}

/// Per-level library set sharing one PCA basis (§45.3 descriptor row).
pub(crate) struct LevelLibs {
    pub pca: Pca,
    pub libs: Vec<Library>,
    /// Libraries dropped by the 2×-base materialization cap (§45.3
    /// transform-allowance rule; surfaced for the caller's job log).
    pub dropped: usize,
}

/// Internal engine knobs. The public API always uses `Knobs::default()`;
/// tests flip these to demonstrate order-invariance and to run the
/// ablation oracles (onion-peel vs priority, seam vs naive paste).
#[derive(Clone, Copy, Default)]
pub(crate) struct Knobs {
    /// Evaluate each query's candidate list in reversed order (winner
    /// selection is a total-order argmin, so output must not change).
    pub reverse_candidates: bool,
    /// Force D(p) = 1 (pure confidence onion-peel; ablation oracle).
    pub onion_peel: bool,
    /// Skip the graph-cut seam pass (naive-paste ablation oracle).
    pub no_seam: bool,
}

const FOUR: [(isize, isize); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
/// Fixed 8-neighbor scan order (raster order) for coherence candidates.
const EIGHT: [(isize, isize); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Build the per-level library set: identity + the §45.3 transform
/// augmentation (rotation / scale / mirror), all indexed against the
/// identity-fit PCA basis. `s_valid` is the level's sample-region mask
/// (already excludes hole, guard band, and transparent pixels).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_libraries(
    planes: &[Plane],
    s_valid: &[bool],
    w: usize,
    h: usize,
    rotations: &[f64],
    scales: &[f64],
    mirror: bool,
    key: &[u8; 32],
) -> LevelLibs {
    let identity_centers = valid_centers(s_valid, w, h);
    let pca = crate::exemplar::descriptor::fit_pca(planes, &identity_centers, key);

    let base_count = identity_centers.len();
    let mut libs = vec![make_library(planes.to_vec(), s_valid.to_vec(), w, h, &pca)];

    // Enumeration order — which also fixes the library ids of the KEPT
    // entries: mirror(identity), rotations (table order), scales (table
    // order), then mirrored copies of the rotation/scale set.
    let mut transforms: Vec<(f64, f64, bool)> = Vec::new(); // (theta_deg, scale, mirror)
    if mirror {
        transforms.push((0.0, 1.0, true));
    }
    for &r in rotations {
        transforms.push((r, 1.0, false));
    }
    for &s in scales {
        transforms.push((0.0, s, false));
    }
    if mirror {
        for &r in rotations {
            transforms.push((r, 1.0, true));
        }
        for &s in scales {
            transforms.push((0.0, s, true));
        }
    }

    // §45.3 materialization cap: "libraries materialize per level, capped
    // at 2× base-library memory; beyond the cap the coarsest-scale
    // libraries drop first, documented in the job log." Two phases:
    // (1) a validity-only pass sizes every candidate library (no planes
    // are materialized yet); (2) while the candidate total exceeds the
    // 2×-base budget, the coarsest-scale candidate drops — coarsest =
    // smallest scale factor s, the most minified re-rendering, exactly
    // as pyramid levels are "coarse" — then the next-coarsest, until the
    // kept set fits. Only kept libraries are materialized. RESIDUAL
    // INTERPRETATION (the spec sentence orders drops by scale only):
    // within one scale value (the s = 1 rotation/mirror group, or a
    // scale and its mirrored copy) drops proceed in REVERSE enumeration
    // order, so earlier-enumerated libraries survive longest; s > 1
    // libraries are the finest and drop last. The identity library is
    // never dropped and never counts against the budget. A transform
    // with no valid centers is dropped outright (as before).
    struct Pending {
        theta: f64,
        s: f64,
        m: bool,
        count: usize,
        keep: bool,
    }
    let mut dropped = 0usize;
    let mut pending: Vec<Pending> = Vec::new();
    for (theta, s, m) in transforms {
        let map = InvMap::new(w, h, theta, s, m);
        let count = valid_centers(&transform_validity(s_valid, w, h, &map), w, h).len();
        if count == 0 {
            dropped += 1;
            continue;
        }
        pending.push(Pending {
            theta,
            s,
            m,
            count,
            keep: true,
        });
    }
    let budget = 2 * base_count;
    let mut total: usize = pending.iter().map(|p| p.count).sum();
    while total > budget {
        let mut victim: Option<usize> = None;
        for (i, p) in pending.iter().enumerate() {
            if !p.keep {
                continue;
            }
            victim = Some(match victim {
                None => i,
                // Coarsest scale drops first; within an equal scale the
                // later-enumerated entry drops first (i > v here because
                // the scan ascends).
                Some(v) => match p.s.total_cmp(&pending[v].s) {
                    std::cmp::Ordering::Greater => v,
                    _ => i,
                },
            });
        }
        let Some(v) = victim else { break };
        pending[v].keep = false;
        dropped += 1;
        total -= pending[v].count;
    }
    for p in &pending {
        if !p.keep {
            continue;
        }
        let map = InvMap::new(w, h, p.theta, p.s, p.m);
        let tv = transform_validity(s_valid, w, h, &map);
        let tp = materialize_planes(planes, w, h, &map);
        libs.push(make_library(tp, tv, w, h, &pca));
    }
    LevelLibs { pca, libs, dropped }
}

fn make_library(planes: Vec<Plane>, valid: Vec<bool>, w: usize, h: usize, pca: &Pca) -> Library {
    let centers = valid_centers(&valid, w, h);
    let mut center_ok = vec![false; w * h];
    for &(x, y) in &centers {
        center_ok[y as usize * w + x as usize] = true;
    }
    let mut coords = Vec::with_capacity(centers.len() * pca.k);
    let mut raw = Vec::new();
    let mut proj = Vec::new();
    for &(x, y) in &centers {
        extract_patch(&planes, x as usize, y as usize, &mut raw);
        pca.project(&raw, &mut proj);
        coords.extend_from_slice(&proj);
    }
    let tree = KdTree::build(coords, pca.k.max(1));
    Library {
        planes,
        centers,
        center_ok,
        tree,
    }
}

/// Inverse affine map of one §45.3 transform (§45.3: "S is re-rendered
/// at a discrete transform set … Patch distance samples the transformed
/// library with bilinear taps at build time"). The affine maps about the
/// canvas center: mirror (horizontal) first, then scale, then rotation.
/// Shared by [`transform_validity`] and [`materialize_planes`] so the
/// cap-sizing pass and the materialization pass see the exact same taps.
pub(crate) struct InvMap {
    cx: f64,
    cy: f64,
    sin: f64,
    cos: f64,
    inv_s: f64,
    mirror: bool,
}

impl InvMap {
    pub(crate) fn new(w: usize, h: usize, theta_deg: f64, scale: f64, mirror: bool) -> InvMap {
        let (sin, cos) = theta_deg.to_radians().sin_cos();
        InvMap {
            cx: (w as f64 - 1.0) * 0.5,
            cy: (h as f64 - 1.0) * 0.5,
            sin,
            cos,
            inv_s: 1.0 / scale,
            mirror,
        }
    }

    /// Bilinear tap for output pixel (x, y): top-left integer tap plus
    /// the fractional weights. Inverse map: undo rotation, undo scale,
    /// undo mirror.
    #[inline]
    fn tap(&self, x: usize, y: usize) -> (isize, isize, f64, f64) {
        let dx = x as f64 - self.cx;
        let dy = y as f64 - self.cy;
        let rx = (self.cos * dx + self.sin * dy) * self.inv_s;
        let ry = (-self.sin * dx + self.cos * dy) * self.inv_s;
        let sxf = if self.mirror {
            self.cx - rx
        } else {
            self.cx + rx
        };
        let syf = self.cy + ry;
        let x0 = sxf.floor();
        let y0 = syf.floor();
        (x0 as isize, y0 as isize, sxf - x0, syf - y0)
    }
}

/// Validity of one transform-augmented rendering: a pixel is valid iff
/// every bilinear tap falls on valid source pixels. Computed WITHOUT
/// materializing planes, so the §45.3 cap can size every candidate
/// library before deciding which to materialize.
pub(crate) fn transform_validity(s_valid: &[bool], w: usize, h: usize, map: &InvMap) -> Vec<bool> {
    let mut valid = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (x0i, y0i, _, _) = map.tap(x, y);
            let mut ok = true;
            for (tx, ty) in [
                (x0i, y0i),
                (x0i + 1, y0i),
                (x0i, y0i + 1),
                (x0i + 1, y0i + 1),
            ] {
                if tx < 0
                    || ty < 0
                    || tx >= w as isize
                    || ty >= h as isize
                    || !s_valid[ty as usize * w + tx as usize]
                {
                    ok = false;
                }
            }
            valid[y * w + x] = ok;
        }
    }
    valid
}

/// Materialize the planes of one transform-augmented rendering (bilinear
/// taps, edge-clamped — identical arithmetic to the pre-split
/// implementation; validity is [`transform_validity`]'s concern).
fn materialize_planes(planes: &[Plane], w: usize, h: usize, map: &InvMap) -> Vec<Plane> {
    let mut out: Vec<Plane> = (0..planes.len()).map(|_| Plane::new(w, h)).collect();
    for y in 0..h {
        for x in 0..w {
            let (x0i, y0i, fx, fy) = map.tap(x, y);
            let i = y * w + x;
            for (c, plane) in planes.iter().enumerate() {
                let v00 = plane.at_clamped(x0i, y0i) as f64;
                let v10 = plane.at_clamped(x0i + 1, y0i) as f64;
                let v01 = plane.at_clamped(x0i, y0i + 1) as f64;
                let v11 = plane.at_clamped(x0i + 1, y0i + 1) as f64;
                let v = v00 * (1.0 - fx) * (1.0 - fy)
                    + v10 * fx * (1.0 - fy)
                    + v01 * (1.0 - fx) * fy
                    + v11 * fx * fy;
                out[c].data[i] = v as f32;
            }
        }
    }
    out
}

/// Per-level synthesis result.
pub(crate) struct LevelSynth {
    pub owner: Vec<u32>,
    pub src: Vec<SrcRef>,
    pub patches: Vec<PatchRec>,
}

struct FrontEntry {
    p_bits: u64,
    tie: u64,
    y: u32,
    x: u32,
}

impl PartialEq for FrontEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for FrontEntry {}
impl Ord for FrontEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Max-heap: higher priority pops first; exact ties fall to the
        // §45.3 seeded draw (higher draw first), then (y, x) ascending —
        // a TOTAL order.
        self.p_bits
            .cmp(&other.p_bits)
            .then(self.tie.cmp(&other.tie))
            .then(other.y.cmp(&self.y))
            .then(other.x.cmp(&self.x))
    }
}
impl PartialOrd for FrontEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// State shared by the priority computation.
struct Fill<'a> {
    planes: &'a mut [Plane],
    hole: &'a [bool],
    w: usize,
    h: usize,
    filled: Vec<bool>,
    conf: Vec<f32>,
    src: Vec<SrcRef>,
    owner: Vec<u32>,
    patches: Vec<PatchRec>,
}

/// Read-only view of exactly the synthesis state a [`query`] consults:
/// buffers, masks, and committed provenance. [`synthesize_level`] derives
/// it from the live [`Fill`]; [`refine`] borrows it directly from the
/// live level buffers (post-synthesis every hole pixel is committed, so
/// `filled` IS the hole mask) — no per-patch full-canvas clones exist
/// anywhere on the refine path ([PERF §45.3] fix round).
struct QueryView<'a> {
    planes: &'a [Plane],
    hole: &'a [bool],
    filled: &'a [bool],
    src: &'a [SrcRef],
    w: usize,
    h: usize,
}

impl QueryView<'_> {
    #[inline]
    fn known_or_filled(&self, x: usize, y: usize) -> bool {
        let i = y * self.w + x;
        !self.hole[i] || self.filled[i]
    }
}

/// Caller-owned scratch buffers threaded through [`query`] so the hot
/// front loop performs no per-query heap allocation ([PERF §45.3] fix
/// round: capacity reuse only). RIGHTS-INVARIANT NOTE (module header
/// invariants 1–2): every field is emptied or fully overwritten at the
/// start of the query that uses it (`extract_patch`/`Pca::project`/
/// `KdTree::query_leaves` all begin with `clear()`; `valid_off` and
/// `cands` are emptied below before any push), so NO information written
/// by one query survives into — or can bias — a later query. Candidate
/// sets remain closed-form functions of (immutable index, committed
/// pixels, query position); only allocation capacity is carried, never
/// contents. The cfg(test) fresh-candidate instrumentation counts are
/// unaffected: every query still builds its set from scratch.
#[derive(Default)]
struct QueryScratch {
    raw: Vec<f64>,
    valid_off: Vec<(usize, isize, isize)>,
    proj: Vec<f64>,
    leaf_pts: Vec<u32>,
    cands: Vec<Cand>,
}

impl Fill<'_> {
    fn view(&self) -> QueryView<'_> {
        QueryView {
            planes: self.planes,
            hole: self.hole,
            filled: &self.filled,
            src: &self.src,
            w: self.w,
            h: self.h,
        }
    }

    #[inline]
    fn known_or_filled(&self, x: usize, y: usize) -> bool {
        let i = y * self.w + x;
        !self.hole[i] || self.filled[i]
    }

    fn luma(&self, x: isize, y: isize) -> f64 {
        let mut acc = 0f64;
        for p in self.planes.iter() {
            acc += p.at_clamped(x, y) as f64;
        }
        acc / self.planes.len() as f64
    }

    /// §45.3 priority P = C·D with confidence decay handled at commit,
    /// isophote data term D = |∇I⊥·n| (the constant-table `/255`
    /// normalization is identity here: gradients are taken on the 0..1
    /// working-range buffer, which equals the 8-bit formula's
    /// `∇(255·I)/255`), floor ε = 1e-3.
    fn priority(&self, x: usize, y: usize, onion_peel: bool) -> f64 {
        let r = PATCH_RADIUS as isize;
        let (xi, yi) = (x as isize, y as isize);
        // C(p): patch-mean confidence over known/filled member pixels
        // (unknown pixels contribute 0), normalized by patch area.
        let mut csum = 0f64;
        for dy in -r..=r {
            for dx in -r..=r {
                let (xx, yy) = (xi + dx, yi + dy);
                if xx < 0 || yy < 0 || xx >= self.w as isize || yy >= self.h as isize {
                    continue;
                }
                let j = yy as usize * self.w + xx as usize;
                if !self.hole[j] || self.filled[j] {
                    csum += self.conf[j] as f64;
                }
            }
        }
        let c = csum / (PATCH_SIDE * PATCH_SIDE) as f64;
        if onion_peel {
            return c;
        }
        // D(p): isophote strength across the front normal.
        let gx = (self.luma(xi + 1, yi) - self.luma(xi - 1, yi)) * 0.5;
        let gy = (self.luma(xi, yi + 1) - self.luma(xi, yi - 1)) * 0.5;
        let unk = |xx: isize, yy: isize| -> f64 {
            if xx < 0 || yy < 0 || xx >= self.w as isize || yy >= self.h as isize {
                return 0.0;
            }
            let j = yy as usize * self.w + xx as usize;
            if self.hole[j] && !self.filled[j] {
                1.0
            } else {
                0.0
            }
        };
        let nx = (unk(xi + 1, yi) - unk(xi - 1, yi)) * 0.5;
        let ny = (unk(xi, yi + 1) - unk(xi, yi - 1)) * 0.5;
        let nlen = (nx * nx + ny * ny).sqrt();
        let d = if nlen > 1e-12 {
            // Isophote ⊥ gradient: (−gy, gx) · n̂.
            ((-gy) * (nx / nlen) + gx * (ny / nlen)).abs()
        } else {
            0.0
        };
        c * d.max(1e-3)
    }
}

/// One candidate source position with its adjusted match distance.
struct Cand {
    lib: u16,
    sx: u16,
    sy: u16,
    adj: f64,
}

/// Query the candidate set for the patch at (x, y) and return the winner.
/// The set is assembled FRESH on every call (see module invariants):
/// coherence candidates from committed neighbors' recorded sources
/// (distance × κ, §45.3), plus the exact-SSD re-rank of the top leaves of
/// every library index. Winner = total-order argmin. `scratch` is
/// capacity-only reuse (see [`QueryScratch`] — emptied before use, no
/// cross-query information flow).
#[allow(clippy::too_many_arguments)]
fn query(
    view: &QueryView<'_>,
    libs: &LevelLibs,
    x: usize,
    y: usize,
    kappa: f64,
    top_l: usize,
    all_valid: bool,
    knobs: Knobs,
    scratch: &mut QueryScratch,
) -> Option<(u16, u16, u16)> {
    #[cfg(test)]
    trace::QUERIES.with(|c| c.set(c.get() + 1));
    let QueryScratch {
        raw,
        valid_off,
        proj,
        leaf_pts,
        cands,
    } = scratch;
    let (w, h) = (view.w, view.h);
    let r = PATCH_RADIUS as isize;
    // Target patch raw values (provisional values included) and the valid
    // offset list (known/filled pixels only, §45.3 "SSD over valid patch
    // pixels").
    extract_patch(view.planes, x, y, raw);
    valid_off.clear();
    let mut oi = 0usize;
    for dy in -r..=r {
        for dx in -r..=r {
            let (xx, yy) = (x as isize + dx, y as isize + dy);
            let inside = xx >= 0 && yy >= 0 && xx < w as isize && yy < h as isize;
            if inside && (all_valid || view.known_or_filled(xx as usize, yy as usize)) {
                valid_off.push((oi, dx, dy));
            }
            oi += 1;
        }
    }
    let area = PATCH_SIDE * PATCH_SIDE;
    let ssd = |lib: &Library, sx: usize, sy: usize| -> f64 {
        let mut acc = 0f64;
        for &(off, dx, dy) in valid_off.iter() {
            let lx = (sx as isize + dx) as usize;
            let ly = (sy as isize + dy) as usize;
            for (c, plane) in lib.planes.iter().enumerate() {
                let dv = raw[c * area + off] - plane.data[ly * w + lx] as f64;
                acc += dv * dv;
            }
        }
        acc
    };

    cands.clear();
    // (a) COHERENCE: committed 8-neighbors' sources, offset-advanced.
    for (dx, dy) in EIGHT {
        let (qx, qy) = (x as isize + dx, y as isize + dy);
        if qx < 0 || qy < 0 || qx >= w as isize || qy >= h as isize {
            continue;
        }
        let j = qy as usize * w + qx as usize;
        if !(view.hole[j] && view.filled[j]) {
            continue;
        }
        let s = view.src[j];
        if !s.is_some() {
            continue;
        }
        let cx = s.x as isize - dx;
        let cy = s.y as isize - dy;
        if cx < 0 || cy < 0 || cx >= w as isize || cy >= h as isize {
            continue;
        }
        let lib = &libs.libs[s.lib as usize];
        if !lib.center_ok[cy as usize * w + cx as usize] {
            continue;
        }
        cands.push(Cand {
            lib: s.lib,
            sx: cx as u16,
            sy: cy as u16,
            adj: ssd(lib, cx as usize, cy as usize) * kappa,
        });
    }
    // (b)+(c) INDEX + TRANSFORM LIBRARIES: top leaves, exact re-rank.
    libs.pca.project(raw, proj);
    for (li, lib) in libs.libs.iter().enumerate() {
        if lib.tree.is_empty() {
            continue;
        }
        lib.tree.query_leaves(proj, top_l, leaf_pts);
        for &pi in leaf_pts.iter() {
            let (sx, sy) = lib.centers[pi as usize];
            cands.push(Cand {
                lib: li as u16,
                sx: sx as u16,
                sy: sy as u16,
                adj: ssd(lib, sx as usize, sy as usize),
            });
        }
    }
    #[cfg(test)]
    trace::CANDIDATE_SETS_BUILT.with(|c| c.set(c.get() + 1));
    if knobs.reverse_candidates {
        cands.reverse();
    }
    // Total-order argmin: (distance bits, library, sy, sx). Distances are
    // ≥ 0 so f64 bits order monotonically.
    cands
        .iter()
        .min_by_key(|c| (c.adj.to_bits(), c.lib, c.sy, c.sx))
        .map(|c| (c.lib, c.sx, c.sy))
}

/// Run the §21.4 priority-front synthesis over one level. `planes` holds
/// provisional values everywhere in the hole (diffusion or upsample) and
/// is committed in place.
#[allow(clippy::too_many_arguments)]
pub(crate) fn synthesize_level(
    planes: &mut [Plane],
    hole: &[bool],
    w: usize,
    h: usize,
    libs: &LevelLibs,
    kappa: f64,
    top_l: usize,
    key: &[u8; 32],
    knobs: Knobs,
) -> LevelSynth {
    let mut fill = Fill {
        planes,
        hole,
        w,
        h,
        filled: vec![false; w * h],
        conf: hole
            .iter()
            .map(|&in_h| if in_h { 0.0 } else { 1.0 })
            .collect(),
        src: vec![SrcRef::NONE; w * h],
        owner: vec![NO_OWNER; w * h],
        patches: Vec::new(),
    };

    let tie = |i: usize| draw_u64(key, STREAM_TIEBREAK, i as u64);
    let mut scratch = QueryScratch::default();
    let mut heap: BinaryHeap<FrontEntry> = BinaryHeap::new();
    let is_front = |fill: &Fill<'_>, x: usize, y: usize| -> bool {
        let i = y * w + x;
        if !fill.hole[i] || fill.filled[i] {
            return false;
        }
        FOUR.iter().any(|&(dx, dy)| {
            let (xx, yy) = (x as isize + dx, y as isize + dy);
            xx >= 0
                && yy >= 0
                && xx < w as isize
                && yy < h as isize
                && fill.known_or_filled(xx as usize, yy as usize)
        })
    };
    for y in 0..h {
        for x in 0..w {
            if is_front(&fill, x, y) {
                let p = fill.priority(x, y, knobs.onion_peel);
                heap.push(FrontEntry {
                    p_bits: p.to_bits(),
                    tie: tie(y * w + x),
                    y: y as u32,
                    x: x as u32,
                });
            }
        }
    }

    let r = PATCH_RADIUS as isize;
    while let Some(e) = heap.pop() {
        let (x, y) = (e.x as usize, e.y as usize);
        let i = y * w + x;
        if fill.filled[i] || !fill.hole[i] {
            continue; // committed by an earlier patch
        }
        // Lazy-heap verification: recompute; stale entries re-enter with
        // their current priority instead of committing at a stale one.
        let p_now = fill.priority(x, y, knobs.onion_peel);
        if p_now.to_bits() != e.p_bits {
            heap.push(FrontEntry {
                p_bits: p_now.to_bits(),
                tie: e.tie,
                y: e.y,
                x: e.x,
            });
            continue;
        }
        let Some((lib, sx, sy)) = query(
            &fill.view(),
            libs,
            x,
            y,
            kappa,
            top_l,
            false,
            knobs,
            &mut scratch,
        ) else {
            // No candidate anywhere (degenerate S): keep the diffused
            // value; mark filled so the front advances.
            fill.filled[i] = true;
            fill.conf[i] = 1e-3;
            continue;
        };
        // COMMIT: copy the winner over the target's unknown pixels;
        // §45.3 confidence carry: mean of contributing confidences ×0.95.
        let mut csum = 0f64;
        let mut ccnt = 0usize;
        for dy in -r..=r {
            for dx in -r..=r {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                if fill.known_or_filled(xx as usize, yy as usize) {
                    csum += fill.conf[yy as usize * w + xx as usize] as f64;
                    ccnt += 1;
                }
            }
        }
        let new_conf = if ccnt > 0 {
            ((csum / ccnt as f64) * 0.95) as f32
        } else {
            1e-3
        };
        let patch_id = fill.patches.len() as u32;
        fill.patches.push(PatchRec {
            lib,
            sx,
            sy,
            tx: x as u16,
            ty: y as u16,
        });
        let libref = &libs.libs[lib as usize];
        for dy in -r..=r {
            for dx in -r..=r {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if !fill.hole[j] || fill.filled[j] {
                    continue;
                }
                let (lx, ly) = ((sx as isize + dx) as usize, (sy as isize + dy) as usize);
                for (c, plane) in libref.planes.iter().enumerate() {
                    fill.planes[c].data[j] = plane.data[ly * w + lx];
                }
                fill.filled[j] = true;
                fill.conf[j] = new_conf;
                fill.src[j] = SrcRef {
                    lib,
                    x: lx as u16,
                    y: ly as u16,
                };
                fill.owner[j] = patch_id;
            }
        }
        // Newly exposed front pixels enter the heap; nearby front pixels
        // re-enter with refreshed priorities (their C/D changed).
        let reach = 2 * r + 1;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (xx, yy) = (x as isize + dx, y as isize + dy);
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let (ux, uy) = (xx as usize, yy as usize);
                if is_front(&fill, ux, uy) {
                    let p = fill.priority(ux, uy, knobs.onion_peel);
                    heap.push(FrontEntry {
                        p_bits: p.to_bits(),
                        tie: tie(uy * w + ux),
                        y: uy as u32,
                        x: ux as u32,
                    });
                }
            }
        }
    }

    LevelSynth {
        owner: fill.owner,
        src: fill.src,
        patches: fill.patches,
    }
}

/// §21.4 REFINE: `passes` re-search-and-commit rounds over the patches
/// whose post-seam SSD residual (buffer vs their source, over the full
/// target footprint) is in the worst decile.
#[allow(clippy::too_many_arguments)]
pub(crate) fn refine(
    planes: &mut [Plane],
    hole: &[bool],
    w: usize,
    h: usize,
    libs: &LevelLibs,
    kappa: f64,
    top_l: usize,
    knobs: Knobs,
    state: &mut LevelSynth,
    passes: usize,
) {
    let r = PATCH_RADIUS as isize;
    let mut scratch = QueryScratch::default();
    for _pass in 0..passes {
        // Post-seam residual per patch.
        let mut residual = vec![0f64; state.patches.len()];
        for (j, pr) in state.patches.iter().enumerate() {
            let lib = &libs.libs[pr.lib as usize];
            let mut acc = 0f64;
            for dy in -r..=r {
                for dx in -r..=r {
                    let (xx, yy) = (pr.tx as isize + dx, pr.ty as isize + dy);
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let q = yy as usize * w + xx as usize;
                    let (lx, ly) = (
                        (pr.sx as isize + dx) as usize,
                        (pr.sy as isize + dy) as usize,
                    );
                    for (c, plane) in lib.planes.iter().enumerate() {
                        let dv = planes[c].data[q] as f64 - plane.data[ly * w + lx] as f64;
                        acc += dv * dv;
                    }
                }
            }
            residual[j] = acc;
        }
        let mut order: Vec<usize> = (0..state.patches.len()).collect();
        order.sort_by(|&a, &b| residual[b].total_cmp(&residual[a]).then(a.cmp(&b)));
        let take = state.patches.len().div_ceil(10).max(1).min(order.len());
        for &j in &order[..take] {
            let pr = state.patches[j];
            // Fresh re-search at the patch center; every pixel now has a
            // committed value, so the whole footprint is valid and the
            // hole mask doubles as the filled mask. The view BORROWS the
            // live level buffers — no per-patch canvas clones ([PERF
            // §45.3] fix round; the pre-fix code cloned four full-canvas
            // buffers per re-searched patch).
            let view = QueryView {
                planes,
                hole,
                filled: hole,
                src: &state.src,
                w,
                h,
            };
            let won = query(
                &view,
                libs,
                pr.tx as usize,
                pr.ty as usize,
                kappa,
                top_l,
                true,
                knobs,
                &mut scratch,
            );
            let Some((lib, sx, sy)) = won else { continue };
            state.patches[j] = PatchRec {
                lib,
                sx,
                sy,
                tx: pr.tx,
                ty: pr.ty,
            };
            let libref = &libs.libs[lib as usize];
            for dy in -r..=r {
                for dx in -r..=r {
                    let (xx, yy) = (pr.tx as isize + dx, pr.ty as isize + dy);
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let q = yy as usize * w + xx as usize;
                    if state.owner[q] != j as u32 {
                        continue;
                    }
                    let (lx, ly) = ((sx as isize + dx) as usize, (sy as isize + dy) as usize);
                    for (c, plane) in libref.planes.iter().enumerate() {
                        planes[c].data[q] = plane.data[ly * w + lx];
                    }
                    state.src[q] = SrcRef {
                        lib,
                        x: lx as u16,
                        y: ly as u16,
                    };
                }
            }
        }
    }
}

/// cfg(test) instrumentation: proves every query rebuilds its candidate
/// set fresh (queries == candidate-set builds; nothing is ever served
/// from a cache because no cache exists to serve from).
#[cfg(test)]
pub(crate) mod trace {
    use std::cell::Cell;
    thread_local! {
        pub static QUERIES: Cell<u64> = const { Cell::new(0) };
        pub static CANDIDATE_SETS_BUILT: Cell<u64> = const { Cell::new(0) };
    }
    pub fn reset() {
        QUERIES.with(|c| c.set(0));
        CANDIDATE_SETS_BUILT.with(|c| c.set(0));
    }
    pub fn snapshot() -> (u64, u64) {
        (
            QUERIES.with(|c| c.get()),
            CANDIDATE_SETS_BUILT.with(|c| c.get()),
        )
    }
}
