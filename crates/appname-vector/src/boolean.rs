// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §20.3 Boolean path operations (CP-1.0 row: "Vatti/Greiner–Hormann
//! booleans").
//!
//! Spec: ES §20.3 [NORMATIVE]. The headline normative sanctions
//! "polygon clipping (**Vatti** or **Greiner–Hormann**) after
//! flattening"; this module elects the **Vatti-class winding-number
//! arm**: one planar-subdivision engine resolves self-intersections per
//! operand fill rule (step 2) AND clips (step 3) by classifying faces
//! with per-operand winding numbers. The election and its fidelity to
//! the step list are recorded in the lane log (the G-H-specific
//! ε-perturbation retry protocol of step 3 is inapplicable to this arm:
//! degenerate incidences — vertex-on-edge, collinear overlap, shared
//! edges — are EXACT cases here, resolved by ε-grid snapping and
//! multi-edge collapse, not perturbation).
//!
//! Numeric contracts, all honored exactly as written:
//! * flatten tolerance 0.05 px doc space ([`crate::flatten::TOL_BOOLEAN`], step 1);
//! * incidence/equality ε = **1e−9 doc px** (step 5) — realized as an
//!   integer grid of 1e−9 px units (i64), on which every orientation/
//!   incidence predicate is EXACT (i128 arithmetic): deterministic on
//!   every OS (DD ㉒), no epsilon tuning, no retries;
//! * intersection points computed in f64 (step 5) and snapped to the
//!   grid;
//! * orientation by the shoelace sum (step 5; i128 over grid coords —
//!   exact, agreeing with the f64 shoelace within its own contract);
//! * refit per step 4 lives in [`crate::fit`] (Schneider, 0.1 px, 30°
//!   corners);
//! * output: outers CCW (positive shoelace under the standard formula
//!   in doc coords), holes CW (step 2's orientation convention);
//! * `exclude` = symmetric difference (step 3), realized directly by
//!   the face predicate `insideA ⊕ insideB` (set-identical to
//!   `union − intersection`, property-tested).
//!
//! Non-destructive storage (`bool_ops` on shape layers) and the
//! `vector.path_bool` command kind are the CP-1.0 doc-model wiring —
//! coordinated with O-CORE; this module is the pure geometry engine.
//! Open subpaths participate as implicitly closed (fill semantics,
//! §20.2 — same convention as the rasterizer).

use crate::fit::{refit_closed_contour, TOL_REFIT};
use crate::flatten::{flatten_path, TOL_BOOLEAN};
use crate::geom::Vec2;
use crate::path::{FillRule, SubPath, VectorPath};
use appname_core::PathId;

/// §20.3 boolean operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Intersect,
    Subtract,
    Exclude,
}

/// Boolean-engine failures. All are INPUT failures — the engine itself
/// has no numeric failure modes (exact predicates, no retries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooleanError {
    /// A coordinate is non-finite or outside the ±9·10⁸ px domain the
    /// 1e−9 grid can address in i64.
    CoordinateOutOfRange,
}

impl std::fmt::Display for BooleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BooleanError::CoordinateOutOfRange => {
                write!(
                    f,
                    "path coordinate non-finite or outside the boolean grid domain"
                )
            }
        }
    }
}

impl std::error::Error for BooleanError {}

/// Grid units per document pixel (ε = 1e−9 px, §20.3 step 5).
const GRID: f64 = 1e9;
/// Domain guard: |coord| ≤ 9e8 px keeps grid values well inside i64.
const MAX_COORD_PX: f64 = 9e8;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct IVec {
    x: i64,
    y: i64,
}

fn quantize(v: Vec2) -> IVec {
    IVec {
        x: (v.x * GRID).round() as i64,
        y: (v.y * GRID).round() as i64,
    }
}

fn unquantize(p: IVec) -> Vec2 {
    Vec2::new(p.x as f64 / GRID, p.y as f64 / GRID)
}

/// Exact orientation: sign of cross(b−a, c−a) in i128.
fn orient(a: IVec, b: IVec, c: IVec) -> i128 {
    let abx = i128::from(b.x) - i128::from(a.x);
    let aby = i128::from(b.y) - i128::from(a.y);
    let acx = i128::from(c.x) - i128::from(a.x);
    let acy = i128::from(c.y) - i128::from(a.y);
    abx * acy - aby * acx
}

fn sign(v: i128) -> i32 {
    match v.cmp(&0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// One directed input edge with per-operand winding contribution
/// (+1 crossing right→left of p→q).
#[derive(Clone, Debug)]
struct RawEdge {
    p: IVec,
    q: IVec,
    da: i32,
    db: i32,
}

/// Flatten one operand into grid-space directed edges (implicit close).
fn operand_edges(
    path: &VectorPath,
    which: usize,
    out: &mut Vec<RawEdge>,
) -> Result<(), BooleanError> {
    for poly in flatten_path(path, TOL_BOOLEAN) {
        if poly.len() < 2 {
            continue;
        }
        for v in &poly {
            if !v.is_finite() || v.x.abs() > MAX_COORD_PX || v.y.abs() > MAX_COORD_PX {
                return Err(BooleanError::CoordinateOutOfRange);
            }
        }
        let pts: Vec<IVec> = poly.iter().map(|&v| quantize(v)).collect();
        let n = pts.len();
        for i in 0..n {
            let p = pts[i];
            let q = pts[(i + 1) % n]; // implicit close (fill semantics)
            if p == q {
                continue;
            }
            let (da, db) = if which == 0 { (1, 0) } else { (0, 1) };
            out.push(RawEdge { p, q, da, db });
        }
    }
    Ok(())
}

/// Split every edge at every pairwise incidence until fixpoint.
/// Sweep-and-prune pairing; EXACT predicates; intersection points in
/// f64 snapped to the grid (a snap can in principle create a new
/// microscopic crossing, hence the fixpoint loop — bounded, and in
/// practice 1–2 rounds).
fn split_all(mut edges: Vec<RawEdge>) -> Vec<RawEdge> {
    for _round in 0..4 {
        let n = edges.len();
        let mut cuts: Vec<Vec<IVec>> = vec![Vec::new(); n];
        // Sort indices by min-x for the sweep.
        let mut order: Vec<usize> = (0..n).collect();
        let key = |e: &RawEdge| (e.p.x.min(e.q.x), e.p.y.min(e.q.y));
        order.sort_by_key(|&i| key(&edges[i]));
        let max_x = |e: &RawEdge| e.p.x.max(e.q.x);
        for oi in 0..n {
            let i = order[oi];
            let (a, b) = (edges[i].p, edges[i].q);
            let ix1 = max_x(&edges[i]);
            for &j in order.iter().skip(oi + 1) {
                let (c, d) = (edges[j].p, edges[j].q);
                if c.x.min(d.x) > ix1 {
                    break; // swept past
                }
                if c.y.min(d.y) > a.y.max(b.y) || c.y.max(d.y) < a.y.min(b.y) {
                    continue;
                }
                let o1 = sign(orient(a, b, c));
                let o2 = sign(orient(a, b, d));
                let o3 = sign(orient(c, d, a));
                let o4 = sign(orient(c, d, b));
                if o1 != o2 && o3 != o4 && o1 * o2 < 0 && o3 * o4 < 0 {
                    // Proper crossing: point via f64, snapped.
                    let t = {
                        let oa = orient(c, d, a) as f64;
                        let ob = orient(c, d, b) as f64;
                        oa / (oa - ob)
                    };
                    let ax = a.x as f64 + (b.x as f64 - a.x as f64) * t;
                    let ay = a.y as f64 + (b.y as f64 - a.y as f64) * t;
                    let x = IVec {
                        x: ax.round() as i64,
                        y: ay.round() as i64,
                    };
                    if x != a && x != b {
                        cuts[i].push(x);
                    }
                    if x != c && x != d {
                        cuts[j].push(x);
                    }
                } else {
                    // Endpoint-on-edge and collinear-overlap incidences:
                    // split each edge at the other's interior endpoints.
                    if o1 == 0 && between(a, b, c) {
                        cuts[i].push(c);
                    }
                    if o2 == 0 && between(a, b, d) {
                        cuts[i].push(d);
                    }
                    if o3 == 0 && between(c, d, a) {
                        cuts[j].push(a);
                    }
                    if o4 == 0 && between(c, d, b) {
                        cuts[j].push(b);
                    }
                }
            }
        }
        if cuts.iter().all(Vec::is_empty) {
            return edges;
        }
        let mut next = Vec::with_capacity(edges.len() * 2);
        for (i, e) in edges.into_iter().enumerate() {
            if cuts[i].is_empty() {
                next.push(e);
                continue;
            }
            let mut pts = std::mem::take(&mut cuts[i]);
            // Order along the edge: exact projection onto p→q.
            let along = |v: IVec| -> i128 {
                let dx = i128::from(e.q.x) - i128::from(e.p.x);
                let dy = i128::from(e.q.y) - i128::from(e.p.y);
                (i128::from(v.x) - i128::from(e.p.x)) * dx
                    + (i128::from(v.y) - i128::from(e.p.y)) * dy
            };
            pts.sort_by_key(|&v| along(v));
            pts.dedup();
            let mut prev = e.p;
            for v in pts.into_iter().chain(std::iter::once(e.q)) {
                if v != prev {
                    next.push(RawEdge {
                        p: prev,
                        q: v,
                        da: e.da,
                        db: e.db,
                    });
                    prev = v;
                }
            }
        }
        edges = next;
    }
    debug_assert!(false, "boolean split did not reach fixpoint in 4 rounds");
    edges
}

/// Collinear betweenness, strict interior (assumes orient(a,b,c) == 0).
fn between(a: IVec, b: IVec, c: IVec) -> bool {
    if c == a || c == b {
        return false;
    }
    let inx = (a.x.min(b.x)..=a.x.max(b.x)).contains(&c.x);
    let iny = (a.y.min(b.y)..=a.y.max(b.y)).contains(&c.y);
    inx && iny
}

/// The planar subdivision: unique vertices, collapsed undirected edges
/// with net winding deltas, half-edge fans, faces, per-face windings.
struct Planar {
    verts: Vec<IVec>,
    /// (u, v, da, db): u < v lexicographically; deltas for u→v traversal.
    edges: Vec<(u32, u32, i32, i32)>,
    /// Per vertex: outgoing half-edge ids, CCW-sorted from east.
    fans: Vec<Vec<u32>>,
    /// half-edge id = edge*2 (u→v) or edge*2+1 (v→u).
    face_of: Vec<u32>,
    /// Per face: (windingA, windingB).
    winding: Vec<(i32, i32)>,
}

fn he_edge(h: u32) -> usize {
    (h >> 1) as usize
}

impl Planar {
    fn tail(&self, h: u32) -> u32 {
        let (u, v, _, _) = self.edges[he_edge(h)];
        if h & 1 == 0 {
            u
        } else {
            v
        }
    }
    fn head(&self, h: u32) -> u32 {
        let (u, v, _, _) = self.edges[he_edge(h)];
        if h & 1 == 0 {
            v
        } else {
            u
        }
    }
    /// Winding delta crossing onto the LEFT of h's direction.
    fn delta(&self, h: u32) -> (i32, i32) {
        let (_, _, da, db) = self.edges[he_edge(h)];
        if h & 1 == 0 {
            (da, db)
        } else {
            (-da, -db)
        }
    }
    /// Face-walk successor: at head(h), the CCW-predecessor of twin(h)
    /// in the outgoing fan (interior-on-left traversal).
    fn next(&self, h: u32) -> u32 {
        let w = self.head(h) as usize;
        let fan = &self.fans[w];
        let twin = h ^ 1;
        let idx = fan
            .iter()
            .position(|&g| g == twin)
            .expect("twin present in its tail fan");
        fan[(idx + fan.len() - 1) % fan.len()]
    }
}

/// CCW-from-east exact direction comparator.
fn dir_cmp(a: (i64, i64), b: (i64, i64)) -> std::cmp::Ordering {
    let half = |d: (i64, i64)| -> u8 {
        // [0, π) = 0; [π, 2π) = 1. East (positive x, y = 0) starts half 0.
        if d.1 > 0 || (d.1 == 0 && d.0 > 0) {
            0
        } else {
            1
        }
    };
    half(a).cmp(&half(b)).then_with(|| {
        let cross = i128::from(a.0) * i128::from(b.1) - i128::from(a.1) * i128::from(b.0);
        // Same half-plane: a precedes b iff b is strictly CCW of a.
        sign(cross)
            .cmp(&0)
            .reverse()
            .then(std::cmp::Ordering::Equal)
    })
}

fn build_planar(split: Vec<RawEdge>) -> Planar {
    // Unique vertices.
    let mut verts: Vec<IVec> = split.iter().flat_map(|e| [e.p, e.q]).collect();
    verts.sort();
    verts.dedup();
    let vid = |p: IVec| -> u32 { verts.binary_search(&p).unwrap() as u32 };

    // Collapse to undirected edges with net deltas; drop zero-delta.
    use std::collections::BTreeMap;
    let mut net: BTreeMap<(u32, u32), (i32, i32)> = BTreeMap::new();
    for e in &split {
        let (pu, pv) = (vid(e.p), vid(e.q));
        debug_assert_ne!(pu, pv);
        let (key, s) = if pu < pv {
            ((pu, pv), 1)
        } else {
            ((pv, pu), -1)
        };
        let ent = net.entry(key).or_insert((0, 0));
        ent.0 += s * e.da;
        ent.1 += s * e.db;
    }
    let edges: Vec<(u32, u32, i32, i32)> = net
        .into_iter()
        .filter(|&(_, (da, db))| da != 0 || db != 0)
        .map(|((u, v), (da, db))| (u, v, da, db))
        .collect();

    // Fans: outgoing half-edges per vertex, CCW-sorted.
    let mut fans: Vec<Vec<u32>> = vec![Vec::new(); verts.len()];
    for (ei, &(u, v, _, _)) in edges.iter().enumerate() {
        fans[u as usize].push((ei * 2) as u32);
        fans[v as usize].push((ei * 2 + 1) as u32);
    }
    let planar_dirs =
        |verts: &Vec<IVec>, edges: &Vec<(u32, u32, i32, i32)>, h: u32| -> (i64, i64) {
            let (u, v, _, _) = edges[he_edge(h)];
            let (t, hd) = if h & 1 == 0 { (u, v) } else { (v, u) };
            let tp = verts[t as usize];
            let hp = verts[hd as usize];
            (hp.x - tp.x, hp.y - tp.y)
        };
    for fan in fans.iter_mut() {
        fan.sort_by(|&a, &b| {
            dir_cmp(
                planar_dirs(&verts, &edges, a),
                planar_dirs(&verts, &edges, b),
            )
        });
    }

    let mut planar = Planar {
        verts,
        edges,
        fans,
        face_of: Vec::new(),
        winding: Vec::new(),
    };

    // Faces: walk next() cycles.
    let nh = planar.edges.len() * 2;
    let mut face_of = vec![u32::MAX; nh];
    let mut cycles: Vec<Vec<u32>> = Vec::new();
    for h0 in 0..nh as u32 {
        if face_of[h0 as usize] != u32::MAX {
            continue;
        }
        let f = cycles.len() as u32;
        let mut cyc = Vec::new();
        let mut h = h0;
        loop {
            face_of[h as usize] = f;
            cyc.push(h);
            h = planar.next(h);
            if h == h0 {
                break;
            }
            debug_assert!(cyc.len() <= nh, "face walk failed to close");
            if cyc.len() > nh {
                break;
            }
        }
        cycles.push(cyc);
    }
    planar.face_of = face_of;

    // Signed area per cycle (component-local origin). i128 terms are
    // exact (≤ ~6.5e36 ≪ i128); the ACCUMULATION uses checked adds —
    // ~27 near-maximal same-sign terms could overflow on an adversarial
    // domain-spanning spiral (review finding, slice-3 round). Overflow
    // degrades to f64 accumulation: only the outer-cycle SELECTION
    // reads these values, and at 1e37 scale f64's relative error cannot
    // reorder a comparison that i128 could not hold in the first place.
    let cycle_area = |cyc: &[u32]| -> f64 {
        let o = planar.verts[planar.tail(cyc[0]) as usize];
        let mut sum: i128 = 0;
        let mut f64_sum = 0.0f64;
        let mut overflowed = false;
        for &h in cyc {
            let a = planar.verts[planar.tail(h) as usize];
            let b = planar.verts[planar.head(h) as usize];
            let ax = i128::from(a.x - o.x);
            let ay = i128::from(a.y - o.y);
            let bx = i128::from(b.x - o.x);
            let by = i128::from(b.y - o.y);
            let term = ax * by - ay * bx;
            if !overflowed {
                match sum.checked_add(term) {
                    Some(s) => sum = s,
                    None => {
                        overflowed = true;
                        f64_sum = sum as f64;
                    }
                }
            }
            if overflowed {
                f64_sum += term as f64;
            }
        }
        if overflowed {
            f64_sum
        } else {
            sum as f64
        }
    };

    // Connected components over vertices (via edges).
    let nv = planar.verts.len();
    let mut comp = vec![u32::MAX; nv];
    let mut ncomp = 0u32;
    for s in 0..nv {
        if comp[s] != u32::MAX {
            continue;
        }
        let c = ncomp;
        ncomp += 1;
        let mut stack = vec![s as u32];
        comp[s] = c;
        while let Some(v) = stack.pop() {
            for &h in &planar.fans[v as usize] {
                let w = planar.head(h);
                if comp[w as usize] == u32::MAX {
                    comp[w as usize] = c;
                    stack.push(w);
                }
            }
        }
    }

    // Component ambient winding at its lexicographically-least vertex,
    // from all OTHER components' edges (exact point-winding formula;
    // cross-component vertex/edge incidence is impossible post-split).
    let mut comp_least: Vec<u32> = vec![u32::MAX; ncomp as usize];
    for (v, &cv) in comp.iter().enumerate() {
        let c = cv as usize;
        if comp_least[c] == u32::MAX {
            comp_least[c] = v as u32; // verts are sorted: first hit = least
        }
    }
    let point_winding = |p: IVec, exclude_comp: u32| -> (i32, i32) {
        let (mut wa, mut wb) = (0, 0);
        for &(u, v, da, db) in &planar.edges {
            if comp[u as usize] == exclude_comp {
                continue;
            }
            let a = planar.verts[u as usize];
            let b = planar.verts[v as usize];
            // Upward/downward crossing of the horizontal line through p,
            // half-open at the upper endpoint; exact side test.
            if a.y <= p.y && b.y > p.y {
                if orient(a, b, p) > 0 {
                    wa += da;
                    wb += db;
                }
            } else if a.y > p.y && b.y <= p.y && orient(a, b, p) < 0 {
                wa -= da;
                wb -= db;
            }
        }
        (wa, wb)
    };

    // Per component: locate the outer cycle (negative area), seed it
    // with the ambient winding, BFS windings across shared edges.
    let ncycles = cycles.len();
    let mut winding: Vec<Option<(i32, i32)>> = vec![None; ncycles];
    let mut comp_of_cycle = vec![u32::MAX; ncycles];
    for (ci, cyc) in cycles.iter().enumerate() {
        comp_of_cycle[ci] = comp[planar.tail(cyc[0]) as usize];
    }
    for c in 0..ncomp {
        let mut outer: Option<(usize, f64)> = None;
        for (ci, cyc) in cycles.iter().enumerate() {
            if comp_of_cycle[ci] != c {
                continue;
            }
            let area = cycle_area(cyc);
            if area < 0.0 {
                debug_assert!(
                    outer.is_none() || outer.unwrap().1 < area,
                    "one outer cycle per component expected"
                );
                if outer.is_none_or(|(_, a)| area < a) {
                    outer = Some((ci, area));
                }
            }
        }
        let Some((outer_ci, _)) = outer else {
            // A component whose every cycle has area ≥ 0 carries no
            // fill: a zero-area sliver minted by degenerate incidences
            // (coincident/collinear operand geometry — the §20.3
            // degenerate arm: handled deterministically, never a
            // crash; the O-TEXTVEC stroke fix round downgraded this
            // from a debug assert after the selection-boundary
            // Inside/Outside case reached it with per-cell collinear
            // contour vertices).
            continue;
        };
        let ambient = point_winding(planar.verts[comp_least[c as usize] as usize], c);
        winding[outer_ci] = Some(ambient);
        // BFS across edges of this component.
        let mut queue = std::collections::VecDeque::from([outer_ci]);
        while let Some(fi) = queue.pop_front() {
            let w = winding[fi].unwrap();
            for &h in &cycles[fi] {
                let g = h ^ 1;
                let gi = planar.face_of[g as usize] as usize;
                if winding[gi].is_none() {
                    // face(g) is left of g; crossing from face(h)
                    // (= left of h = right of g) onto left of g adds
                    // delta(g).
                    let (da, db) = planar.delta(g);
                    winding[gi] = Some((w.0 + da, w.1 + db));
                    queue.push_back(gi);
                }
            }
        }
    }
    planar.winding = winding.into_iter().map(|w| w.unwrap_or((0, 0))).collect();
    planar
}

fn rule_contains(rule: FillRule, w: i32) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// Extract the region boundary (inside on the left), walk contours,
/// refit, and assemble the result path.
fn extract(planar: &Planar, inside: impl Fn((i32, i32)) -> bool, id: PathId) -> VectorPath {
    let nh = planar.edges.len() * 2;
    let kept: Vec<bool> = (0..nh as u32)
        .map(|h| {
            let fi = planar.face_of[h as usize] as usize;
            let gi = planar.face_of[(h ^ 1) as usize] as usize;
            inside(planar.winding[fi]) && !inside(planar.winding[gi])
        })
        .collect();

    let mut used = vec![false; nh];
    let mut path = VectorPath::new(id, FillRule::NonZero);
    for h0 in 0..nh as u32 {
        if !kept[h0 as usize] || used[h0 as usize] {
            continue;
        }
        // Walk: continue at head with the first KEPT outgoing edge
        // CCW-before twin(h) (region boundary may merge faces).
        let mut contour: Vec<IVec> = Vec::new();
        let mut h = h0;
        loop {
            used[h as usize] = true;
            contour.push(planar.verts[planar.tail(h) as usize]);
            let w = planar.head(h) as usize;
            let fan = &planar.fans[w];
            let twin = h ^ 1;
            let idx = fan
                .iter()
                .position(|&g| g == twin)
                .expect("twin present in its tail fan");
            let mut k = (idx + fan.len() - 1) % fan.len();
            let mut found = None;
            for _ in 0..fan.len() {
                let cand = fan[k];
                if kept[cand as usize] {
                    found = Some(cand);
                    break;
                }
                k = (k + fan.len() - 1) % fan.len();
            }
            let Some(nh_) = found else {
                debug_assert!(false, "boundary walk dead-ends");
                break;
            };
            h = nh_;
            if h == h0 {
                break;
            }
            debug_assert!(contour.len() <= nh, "boundary walk failed to close");
            if contour.len() > nh {
                break;
            }
        }
        if contour.len() < 3 {
            continue;
        }
        let pts: Vec<Vec2> = contour.into_iter().map(unquantize).collect();
        let segments = refit_closed_contour(&pts, TOL_REFIT);
        if !segments.is_empty() {
            path.subpaths.push(SubPath {
                segments,
                closed: true,
            });
        }
    }
    let normalized = path.normalize();
    debug_assert!(normalized.is_ok(), "boolean output failed to normalize");
    path
}

/// §20.3 boolean of two paths. Fill rules are the operands' OWN
/// (step 2); the result is a normalized NonZero path (simple polygons
/// refit to Béziers; outers CCW, holes CW).
pub fn path_boolean(
    a: &VectorPath,
    b: &VectorPath,
    op: BoolOp,
    out_id: PathId,
) -> Result<VectorPath, BooleanError> {
    let mut edges = Vec::new();
    operand_edges(a, 0, &mut edges)?;
    operand_edges(b, 1, &mut edges)?;
    let planar = build_planar(split_all(edges));
    let (ra, rb) = (a.fill_rule, b.fill_rule);
    let inside = move |(wa, wb): (i32, i32)| -> bool {
        let ia = rule_contains(ra, wa);
        let ib = rule_contains(rb, wb);
        match op {
            BoolOp::Union => ia || ib,
            BoolOp::Intersect => ia && ib,
            BoolOp::Subtract => ia && !ib,
            BoolOp::Exclude => ia != ib,
        }
    };
    Ok(extract(&planar, inside, out_id))
}

/// §20.3 step 2 standalone: resolve one path's self-intersections by
/// its OWN fill rule into simple, consistently-oriented contours
/// (outers CCW / holes CW), refit. The fill-region is exactly
/// preserved; the result is rule-independent (hence NonZero).
pub fn resolve_fill(path: &VectorPath, out_id: PathId) -> Result<VectorPath, BooleanError> {
    let mut edges = Vec::new();
    operand_edges(path, 0, &mut edges)?;
    let planar = build_planar(split_all(edges));
    let rule = path.fill_rule;
    Ok(extract(
        &planar,
        move |(wa, _)| rule_contains(rule, wa),
        out_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::CubicSeg;
    use crate::raster::rasterize_path;

    fn pid(_n: u128) -> PathId {
        PathId::nil() // identity is irrelevant to the geometry engine
    }

    /// Axis square as four line-cubics, closed (same shape as the
    /// path-module test helper, which is test-local there).
    fn square_sub(x0: f64, y0: f64, s: f64) -> SubPath {
        let p = [
            Vec2::new(x0, y0),
            Vec2::new(x0 + s, y0),
            Vec2::new(x0 + s, y0 + s),
            Vec2::new(x0, y0 + s),
        ];
        SubPath {
            closed: true,
            segments: (0..4)
                .map(|i| CubicSeg::line(p[i], p[(i + 1) % 4]))
                .collect(),
        }
    }

    fn square_path(x0: f64, y0: f64, s: f64, rule: FillRule) -> VectorPath {
        let mut p = VectorPath::new(pid(1), rule);
        p.subpaths.push(square_sub(x0, y0, s));
        p.normalize().unwrap();
        p
    }

    /// f64 shoelace over a subpath's flattened polyline (the §20.3
    /// orientation/area contract surface).
    fn sub_area(sub: &SubPath) -> f64 {
        let poly = crate::flatten::flatten_subpath(sub, 0.01);
        let mut sum = 0.0;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            sum += a.x * b.y - a.y * b.x;
        }
        sum / 2.0
    }

    fn path_area(p: &VectorPath) -> f64 {
        p.subpaths.iter().map(sub_area).sum()
    }

    #[test]
    fn disjoint_union_keeps_both() {
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let b = square_path(20.0, 0.0, 10.0, FillRule::NonZero);
        let u = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        assert_eq!(u.subpaths.len(), 2);
        assert!((path_area(&u).abs() - 200.0).abs() < 1e-6);
        // Disjoint intersect is empty.
        let i = path_boolean(&a, &b, BoolOp::Intersect, pid(9)).unwrap();
        assert!(i.subpaths.is_empty());
    }

    #[test]
    fn overlapping_squares_have_exact_boolean_areas() {
        // 10×10 squares overlapping in a 5×5 corner.
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let b = square_path(5.0, 5.0, 10.0, FillRule::NonZero);
        let cases = [
            (BoolOp::Union, 175.0),
            (BoolOp::Intersect, 25.0),
            (BoolOp::Subtract, 75.0),
            (BoolOp::Exclude, 150.0),
        ];
        for (op, want) in cases {
            let r = path_boolean(&a, &b, op, pid(9)).unwrap();
            let got: f64 = r.subpaths.iter().map(|s| sub_area(s).abs()).sum();
            assert!((got - want).abs() < 1e-6, "{op:?}: area {got}, want {want}");
        }
        // The union is one 8-vertex contour of straight edges.
        let u = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        assert_eq!(u.subpaths.len(), 1);
        assert_eq!(u.subpaths[0].segments.len(), 8);
    }

    #[test]
    fn subtract_carves_a_hole_with_cw_orientation() {
        let outer = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let inner = square_path(3.0, 3.0, 4.0, FillRule::NonZero);
        let r = path_boolean(&outer, &inner, BoolOp::Subtract, pid(9)).unwrap();
        assert_eq!(r.subpaths.len(), 2, "outer + hole");
        let mut areas: Vec<f64> = r.subpaths.iter().map(sub_area).collect();
        areas.sort_by(|x, y| x.partial_cmp(y).unwrap());
        // Hole is negative (CW), outer positive (CCW); net = 100 − 16.
        assert!((areas[0] + 16.0).abs() < 1e-6, "hole area {}", areas[0]);
        assert!((areas[1] - 100.0).abs() < 1e-6, "outer area {}", areas[1]);
    }

    #[test]
    fn shared_edge_union_is_seamless() {
        // Two squares sharing a full edge: the classic degenerate case
        // (coincident opposite edges must cancel exactly).
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let b = square_path(10.0, 0.0, 10.0, FillRule::NonZero);
        let u = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        assert_eq!(u.subpaths.len(), 1);
        assert_eq!(u.subpaths[0].segments.len(), 4, "one clean 20×10 rect");
        assert!((path_area(&u) - 200.0).abs() < 1e-6);
        // Their intersection is empty (edge, zero area).
        let i = path_boolean(&a, &b, BoolOp::Intersect, pid(9)).unwrap();
        assert!(i.subpaths.is_empty());
    }

    #[test]
    fn identical_operands_collapse_correctly() {
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let u = path_boolean(&a, &a, BoolOp::Union, pid(9)).unwrap();
        assert!((path_area(&u) - 100.0).abs() < 1e-6);
        assert_eq!(u.subpaths.len(), 1);
        let x = path_boolean(&a, &a, BoolOp::Exclude, pid(9)).unwrap();
        assert!(x.subpaths.is_empty(), "A ⊕ A = ∅");
        let s = path_boolean(&a, &a, BoolOp::Subtract, pid(9)).unwrap();
        assert!(s.subpaths.is_empty(), "A − A = ∅");
    }

    #[test]
    fn resolve_fill_matches_the_rasterizer_on_winding_discriminators() {
        // Concentric same-direction squares: NonZero fills the full
        // outer square; EvenOdd leaves an annulus — the same
        // discriminator pair the §20.2 rasterizer tests pin.
        for (rule, want_subs, want_area) in
            [(FillRule::NonZero, 1, 100.0), (FillRule::EvenOdd, 2, 84.0)]
        {
            let mut p = VectorPath::new(pid(2), rule);
            p.subpaths.push(square_sub(0.0, 0.0, 10.0));
            p.subpaths.push(square_sub(3.0, 3.0, 4.0));
            p.normalize().unwrap();
            let r = resolve_fill(&p, pid(9)).unwrap();
            assert_eq!(r.subpaths.len(), want_subs, "{rule:?}");
            assert!(
                (path_area(&r) - want_area).abs() < 1e-6,
                "{rule:?}: net area {}",
                path_area(&r)
            );
            // Rasterizer cross-check: the resolved NonZero path covers
            // the same pixels as the original under its own rule.
            let cov_orig = rasterize_path(&p, 0.05);
            let cov_res = rasterize_path(&r, 0.05);
            let (so, sr) = (cov_orig.total(), cov_res.total());
            assert!(
                (so - sr).abs() < 0.5,
                "{rule:?}: raster Σ {so} vs resolved {sr}"
            );
        }
    }

    #[test]
    fn bowtie_resolves_to_two_lobes_under_both_rules() {
        // Self-intersecting bowtie: (0,0) (10,10) (10,0) (0,10).
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let mut p = VectorPath::new(pid(3), rule);
            let pts = [
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(0.0, 10.0),
            ];
            let segs: Vec<CubicSeg> = (0..4)
                .map(|i| CubicSeg::line(pts[i], pts[(i + 1) % 4]))
                .collect();
            p.subpaths.push(SubPath {
                segments: segs,
                closed: true,
            });
            p.normalize().unwrap();
            let r = resolve_fill(&p, pid(9)).unwrap();
            assert_eq!(r.subpaths.len(), 2, "{rule:?}: two lobes");
            let total: f64 = r.subpaths.iter().map(|s| sub_area(s).abs()).sum();
            assert!((total - 50.0).abs() < 1e-6, "{rule:?}: Σ {total}");
        }
    }

    #[test]
    fn exclude_equals_union_minus_intersection() {
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let b = square_path(4.0, 4.0, 10.0, FillRule::NonZero);
        let x = path_boolean(&a, &b, BoolOp::Exclude, pid(9)).unwrap();
        let u = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        let i = path_boolean(&a, &b, BoolOp::Intersect, pid(9)).unwrap();
        let x2 = path_boolean(&u, &i, BoolOp::Subtract, pid(9)).unwrap();
        // Same region: identical net area and identical raster.
        assert!((path_area(&x) - path_area(&x2)).abs() < 1e-6);
        let (ca, cb) = (rasterize_path(&x, 0.05), rasterize_path(&x2, 0.05));
        assert!((ca.total() - cb.total()).abs() < 0.5);
    }

    #[test]
    fn area_inclusion_exclusion_holds_on_seeded_polygons() {
        // Deterministic LCG-seeded convex-ish polygons; the identity
        // area(A∪B) + area(A∩B) = area(A) + area(B) must hold exactly
        // (within refit tolerance) for every pair.
        let mut state = 0x243F_6A88_85A3_08D3u64;
        let mut rnd = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f64) / f64::from(1u32 << 31)
        };
        for case in 0..25 {
            let mk = |rnd: &mut dyn FnMut() -> f64, cx: f64, cy: f64| {
                // Star-convex fan around (cx, cy): sorted angles, radii 2–8.
                let n = 5 + (case % 4);
                let mut angs: Vec<f64> = (0..n).map(|_| rnd() * std::f64::consts::TAU).collect();
                angs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let mut p = VectorPath::new(pid(4), FillRule::NonZero);
                let pts: Vec<Vec2> = angs
                    .iter()
                    .map(|&t| {
                        Vec2::new(
                            cx + t.cos() * (2.0 + 6.0 * rnd()),
                            cy + t.sin() * (2.0 + 6.0 * rnd()),
                        )
                    })
                    .collect();
                let segs: Vec<CubicSeg> = (0..pts.len())
                    .map(|i| CubicSeg::line(pts[i], pts[(i + 1) % pts.len()]))
                    .collect();
                p.subpaths.push(SubPath {
                    segments: segs,
                    closed: true,
                });
                p.normalize().unwrap();
                p
            };
            let a = mk(&mut rnd, 0.0, 0.0);
            let (ox, oy) = (3.0 * rnd(), 3.0 * rnd());
            let b = mk(&mut rnd, ox, oy);
            let u = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
            let i = path_boolean(&a, &b, BoolOp::Intersect, pid(9)).unwrap();
            let lhs = path_area(&u) + path_area(&i);
            let rhs = path_area(&a) + path_area(&b);
            // Bound: the identity is exact on the subdivision; the refit
            // may move each output boundary by ≤ TOL_REFIT wherever star
            // vertices turn < 30° (Schneider smooths across them), so
            // the area slack scales with the operands' perimeters
            // (observed 0.066 px² on ~60 px of perimeter — exactly this
            // term, not an engine error).
            let perim: f64 = [&a, &b]
                .iter()
                .flat_map(|p| crate::flatten::flatten_path(p, 0.05))
                .map(|poly| {
                    (0..poly.len())
                        .map(|k| poly[k].distance(poly[(k + 1) % poly.len()]))
                        .sum::<f64>()
                })
                .sum();
            let bound = 2.0 * TOL_REFIT * perim + 1e-6;
            assert!(
                (lhs - rhs).abs() < bound,
                "case {case}: {lhs} vs {rhs} (bound {bound})"
            );
        }
    }

    #[test]
    fn curved_union_area_matches_the_two_circle_analytic() {
        // κ-cubic circles r=10 at distance 10: union area from the
        // standard lens formula; flatten+refit tolerances bound the gap.
        let circle = |cx: f64, cy: f64| -> VectorPath {
            let k = 0.552_284_749_830_793_4 * 10.0;
            let r = 10.0;
            let p0 = Vec2::new(cx + r, cy);
            let p1 = Vec2::new(cx, cy + r);
            let p2 = Vec2::new(cx - r, cy);
            let p3 = Vec2::new(cx, cy - r);
            let segs = vec![
                CubicSeg {
                    p0,
                    c1: Vec2::new(cx + r, cy + k),
                    c2: Vec2::new(cx + k, cy + r),
                    p1,
                },
                CubicSeg {
                    p0: p1,
                    c1: Vec2::new(cx - k, cy + r),
                    c2: Vec2::new(cx - r, cy + k),
                    p1: p2,
                },
                CubicSeg {
                    p0: p2,
                    c1: Vec2::new(cx - r, cy - k),
                    c2: Vec2::new(cx - k, cy - r),
                    p1: p3,
                },
                CubicSeg {
                    p0: p3,
                    c1: Vec2::new(cx + k, cy - r),
                    c2: Vec2::new(cx + r, cy - k),
                    p1: p0,
                },
            ];
            let mut p = VectorPath::new(pid(5), FillRule::NonZero);
            p.subpaths.push(SubPath {
                segments: segs,
                closed: true,
            });
            p.normalize().unwrap();
            p
        };
        let a = circle(0.0, 0.0);
        let b = circle(10.0, 0.0);
        let u = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        assert_eq!(u.subpaths.len(), 1);
        let r: f64 = 10.0;
        let d: f64 = 10.0;
        let lens = 2.0 * r * r * (d / (2.0 * r)).acos() - (d / 2.0) * (4.0 * r * r - d * d).sqrt();
        let want = 2.0 * std::f64::consts::PI * r * r - lens;
        let got = path_area(&u);
        // κ-circle vs true circle ≈ 0.03 % area; flatten 0.05 px and
        // refit 0.1 px add boundary-length-scaled slack.
        assert!(
            (got - want).abs() < want * 0.005,
            "union area {got}, analytic {want}"
        );
    }

    #[test]
    fn boolean_is_deterministic() {
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let b = square_path(5.0, 5.0, 10.0, FillRule::NonZero);
        let u1 = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        let u2 = path_boolean(&a, &b, BoolOp::Union, pid(9)).unwrap();
        assert_eq!(u1.subpaths, u2.subpaths);
    }

    #[test]
    fn out_of_range_coordinates_are_refused() {
        let a = square_path(0.0, 0.0, 10.0, FillRule::NonZero);
        let far = square_path(1e9, 0.0, 10.0, FillRule::NonZero);
        assert_eq!(
            path_boolean(&a, &far, BoolOp::Union, pid(9)).err(),
            Some(BooleanError::CoordinateOutOfRange)
        );
    }
}
