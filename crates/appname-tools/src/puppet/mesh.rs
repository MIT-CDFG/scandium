// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §45.5 "Puppet warp mesh and pins" [NORMATIVE] + §18.4 —
//! "Constrained Delaunay over the layer's alpha hull (`alpha > 0`
//! boundary polygon, simplified to 2-px tolerance), dilated by
//! `expansionPx` before triangulation … Density presets map to target
//! triangle counts — fewer = 500, normal = 2000, more = 8000 — via
//! §18.4's interior edge-length budget `√(2·area/target)`.
//! Triangulation is deterministic: hull vertices enumerate in document
//! scan order, and the CDT inserts Steiner points on a fixed grid
//! ordering, so identical inputs mesh identically on every platform."
//!
//! v1 pins (recorded, not invented — lane log this date):
//! * Coverage model: the "alpha hull" is realized as the `alpha > 0`
//!   COVERAGE BITMAP; dilation by `expansionPx` is the exact Euclidean
//!   disc (squared-EDT threshold `d² ≤ r²`, reusing the §13.2 EDT).
//!   Boundary polygons trace the dilated bitmap's pixel-edge contour
//!   (crack following, inside kept on the left, deterministic corner
//!   rule), then simplify with Douglas–Peucker at the §45.5 2-px
//!   tolerance anchored at each loop's lexicographically smallest
//!   vertex (a deterministic closed-loop split).
//! * CDT realization: boundary loops subdivide to the edge budget `h`,
//!   then a CONFORMING Delaunay triangulation is built by Bowyer–Watson
//!   insertion in the fixed documented order (loop points in trace
//!   order, then Steiner points row-major), with triangles culled to
//!   the dilated coverage (centroid test). Constraint edges are
//!   recovered by the subdivision density rather than by edge flips —
//!   a conforming-not-flip-enforced CDT; recorded as a pin (the §45.5
//!   determinism clause — fixed hull enumeration + fixed Steiner
//!   ordering — is the normative surface, and the golden/determinism
//!   tests hold it).
//! * Steiner lattice: hex-staggered rows (row pitch `h·√3/2`, alternate
//!   rows offset `h/2`) clipped to the dilated coverage with a `h/2`
//!   boundary clearance (squared-EDT of the complement) — the stagger
//!   both approximates the equilateral target density and removes the
//!   cocircular four-point degeneracy a square lattice feeds the
//!   incircle predicate. Incircle ties (|det| within a scale-relative
//!   epsilon) count as OUTSIDE — deterministic, orientation-preserving.
//! * Insertion robustness: a point whose cavity fails the closed-fan
//!   validity check is SKIPPED deterministically (never a panic; the
//!   mesh stays valid). Duplicate points (< 1e-6 px) are skipped.
//! * Holes in the coverage (a donut layer) stay holes: their boundary
//!   loops trace and subdivide like the outer hull, and the centroid
//!   cull removes interior-of-hole triangles. "Pixels outside the mesh
//!   are untouched" (§45.5) is the rasterizer's contract.

use appname_core::IRect;

use crate::select::refine::edt_squared;

/// §45.5 density presets → target triangle counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshDensity {
    /// 500 triangles.
    Fewer,
    /// 2000 triangles (§18.4's figure).
    Normal,
    /// 8000 triangles.
    More,
}

impl MeshDensity {
    pub fn target_triangles(self) -> usize {
        match self {
            MeshDensity::Fewer => 500,
            MeshDensity::Normal => 2000,
            MeshDensity::More => 8000,
        }
    }

    /// Wire enum mapping (`density: fewer|normal|more`).
    pub fn from_wire(s: &str) -> Option<MeshDensity> {
        Some(match s {
            "fewer" => MeshDensity::Fewer,
            "normal" => MeshDensity::Normal,
            "more" => MeshDensity::More,
            _ => return None,
        })
    }
}

/// A binary coverage bitmap over a doc-space rect (the alpha > 0 mask,
/// or the §45.5 selection ≥ 0.5 region when a selection scopes the
/// mesh).
#[derive(Clone, Debug)]
pub struct CoverageMask {
    pub rect: IRect,
    /// Row-major over `rect`.
    pub inside: Vec<bool>,
}

impl CoverageMask {
    pub fn new(rect: IRect, inside: Vec<bool>) -> CoverageMask {
        assert_eq!(
            inside.len(),
            (rect.width() as usize) * (rect.height() as usize),
            "coverage buffer must match its rect"
        );
        CoverageMask { rect, inside }
    }

    /// Build from a predicate over doc pixel coords.
    pub fn from_fn(rect: IRect, mut f: impl FnMut(i32, i32) -> bool) -> CoverageMask {
        let mut inside = Vec::with_capacity((rect.width() * rect.height()) as usize);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                inside.push(f(x, y));
            }
        }
        CoverageMask { rect, inside }
    }

    fn at(&self, x: i32, y: i32) -> bool {
        if x < self.rect.min.x
            || y < self.rect.min.y
            || x >= self.rect.max.x
            || y >= self.rect.max.y
        {
            return false;
        }
        let w = self.rect.width() as usize;
        self.inside[(y - self.rect.min.y) as usize * w + (x - self.rect.min.x) as usize]
    }

    /// Covered pixel count.
    pub fn area(&self) -> usize {
        self.inside.iter().filter(|b| **b).count()
    }

    /// Point-in-coverage at continuous doc coords (containing pixel).
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        self.at(x.floor() as i32, y.floor() as i32)
    }

    /// Exact Euclidean disc dilation by `r` px (module pin: squared-EDT
    /// threshold). `r = 0` returns a copy padded by nothing.
    pub fn dilate(&self, r: i32) -> CoverageMask {
        if r <= 0 {
            return self.clone();
        }
        let rect = self.rect.inflate(r, r);
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        let mut inside = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                inside[y * w + x] = self.at(rect.min.x + x as i32, rect.min.y + y as i32);
            }
        }
        let d2 = edt_squared(&inside, w, h);
        let r2 = f64::from(r) * f64::from(r);
        let dilated = (0..w * h).map(|i| d2[i] <= r2).collect();
        CoverageMask {
            rect,
            inside: dilated,
        }
    }

    /// Squared distance to the NEAREST UNCOVERED pixel, per covered
    /// pixel (boundary clearance for the Steiner lattice).
    fn clearance_sq(&self) -> Vec<f64> {
        let outside: Vec<bool> = self.inside.iter().map(|b| !b).collect();
        edt_squared(
            &outside,
            self.rect.width() as usize,
            self.rect.height() as usize,
        )
    }
}

/// The §45.5 rest mesh: triangle soup with shared vertices, all
/// triangles CCW (y-down signed-area convention).
#[derive(Clone, Debug, PartialEq)]
pub struct PuppetMesh {
    /// Rest vertex positions (doc px).
    pub verts: Vec<[f64; 2]>,
    /// CCW vertex-index triples.
    pub tris: Vec<[u32; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// §45.5 "Zero-area alpha hull (empty layer)": no covered pixels.
    EmptyCoverage,
    /// Triangulation produced no usable triangles (degenerate sliver
    /// coverage).
    Degenerate,
}

impl core::fmt::Display for MeshError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MeshError::EmptyCoverage => write!(f, "zero-area alpha hull (§45.5)"),
            MeshError::Degenerate => write!(f, "coverage too degenerate to triangulate (§45.5)"),
        }
    }
}

impl std::error::Error for MeshError {}

impl PuppetMesh {
    /// Signed area ×2 of triangle `t` at positions `pos` (positive =
    /// CCW under the y-down convention used throughout).
    pub fn area2(pos: &[[f64; 2]], t: [u32; 3]) -> f64 {
        let a = pos[t[0] as usize];
        let b = pos[t[1] as usize];
        let c = pos[t[2] as usize];
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }

    /// Rest centroid of triangle `i`.
    pub fn centroid(&self, i: usize) -> [f64; 2] {
        let t = self.tris[i];
        let a = self.verts[t[0] as usize];
        let b = self.verts[t[1] as usize];
        let c = self.verts[t[2] as usize];
        [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0]
    }

    /// Containing triangle of a point (first hit in triangle order —
    /// deterministic; boundary points hit the lower-index triangle).
    pub fn containing_triangle(&self, p: [f64; 2]) -> Option<usize> {
        for (i, t) in self.tris.iter().enumerate() {
            let a = self.verts[t[0] as usize];
            let b = self.verts[t[1] as usize];
            let c = self.verts[t[2] as usize];
            let e = |p0: [f64; 2], p1: [f64; 2]| {
                (p1[0] - p0[0]) * (p[1] - p0[1]) - (p1[1] - p0[1]) * (p[0] - p0[0])
            };
            if e(a, b) >= -1e-9 && e(b, c) >= -1e-9 && e(c, a) >= -1e-9 {
                return Some(i);
            }
        }
        None
    }

    /// Nearest rest vertex (lowest index on ties — §45.5 tie rule
    /// analog).
    pub fn nearest_vertex(&self, p: [f64; 2]) -> usize {
        let mut best = 0usize;
        let mut best_d = f64::INFINITY;
        for (i, v) in self.verts.iter().enumerate() {
            let d = (v[0] - p[0]).powi(2) + (v[1] - p[1]).powi(2);
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        best
    }

    /// Shared-edge triangle adjacency (sorted neighbor lists —
    /// deterministic iteration for the §18.4 rotation smoothing).
    pub fn adjacency(&self) -> Vec<Vec<u32>> {
        use std::collections::BTreeMap;
        let mut edge_owner: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
        for (i, t) in self.tris.iter().enumerate() {
            for k in 0..3 {
                let a = t[k];
                let b = t[(k + 1) % 3];
                let key = (a.min(b), a.max(b));
                edge_owner.entry(key).or_default().push(i as u32);
            }
        }
        let mut adj = vec![Vec::new(); self.tris.len()];
        for owners in edge_owner.values() {
            if owners.len() == 2 {
                adj[owners[0] as usize].push(owners[1]);
                adj[owners[1] as usize].push(owners[0]);
            }
        }
        for l in &mut adj {
            l.sort_unstable();
        }
        adj
    }
}

/// §18.4 interior edge-length budget `√(2·area/target)`, floored at 1 px
/// (degenerate-coverage guard).
pub fn edge_budget(area_px: usize, target_triangles: usize) -> f64 {
    ((2.0 * area_px as f64) / target_triangles as f64)
        .sqrt()
        .max(1.0)
}

// ---------------------------------------------------------------------------
// Boundary tracing (crack following on the coverage bitmap).
// ---------------------------------------------------------------------------

/// Trace all closed boundary loops of the coverage as pixel-corner
/// polygons, inside on the left. Deterministic: directed edges are
/// generated in scan order and chained from the lexicographically
/// smallest unvisited start point; at a checkerboard corner the
/// outgoing edge with the smallest direction ordinal wins.
fn trace_boundary_loops(mask: &CoverageMask) -> Vec<Vec<[f64; 2]>> {
    use std::collections::BTreeMap;
    // Directed edges keyed by start corner (corner coords are integers).
    let mut edges: BTreeMap<(i64, i64), Vec<(i64, i64)>> = BTreeMap::new();
    let mut push = |from: (i64, i64), to: (i64, i64)| {
        edges.entry(from).or_default().push(to);
    };
    for y in mask.rect.min.y..mask.rect.max.y {
        for x in mask.rect.min.x..mask.rect.max.x {
            if !mask.at(x, y) {
                continue;
            }
            let (xl, yl) = (i64::from(x), i64::from(y));
            if !mask.at(x, y - 1) {
                push((xl, yl), (xl + 1, yl)); // top edge, rightward
            }
            if !mask.at(x + 1, y) {
                push((xl + 1, yl), (xl + 1, yl + 1)); // right edge, down
            }
            if !mask.at(x, y + 1) {
                push((xl + 1, yl + 1), (xl, yl + 1)); // bottom, leftward
            }
            if !mask.at(x - 1, y) {
                push((xl, yl + 1), (xl, yl)); // left edge, up
            }
        }
    }
    for v in edges.values_mut() {
        v.sort_unstable(); // deterministic corner resolution
    }
    let mut loops = Vec::new();
    // Lexicographically smallest remaining start each round.
    while let Some((&start, _)) = edges.iter().find(|(_, v)| !v.is_empty()) {
        let mut chain: Vec<(i64, i64)> = vec![start];
        let mut cur = start;
        loop {
            let outs = edges.get_mut(&cur).expect("chained edge start exists");
            debug_assert!(!outs.is_empty(), "boundary chains are closed");
            let next = outs.remove(0);
            if next == start {
                break;
            }
            chain.push(next);
            cur = next;
        }
        if chain.len() >= 3 {
            loops.push(
                chain
                    .into_iter()
                    .map(|(x, y)| [x as f64, y as f64])
                    .collect(),
            );
        }
    }
    loops
}

/// Douglas–Peucker over an open polyline segment (indices inclusive),
/// keeping points deviating more than `tol`.
fn dp_keep(points: &[[f64; 2]], lo: usize, hi: usize, tol: f64, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let a = points[lo];
    let b = points[hi];
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let ab2 = abx * abx + aby * aby;
    let mut worst = 0.0f64;
    let mut worst_i = lo;
    for (i, p) in points.iter().enumerate().take(hi).skip(lo + 1) {
        let d = if ab2 == 0.0 {
            ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
        } else {
            ((p[0] - a[0]) * aby - (p[1] - a[1]) * abx).abs() / ab2.sqrt()
        };
        if d > worst {
            worst = d;
            worst_i = i;
        }
    }
    if worst > tol {
        keep[worst_i] = true;
        dp_keep(points, lo, worst_i, tol, keep);
        dp_keep(points, worst_i, hi, tol, keep);
    }
}

/// Closed-loop simplification at the §45.5 2-px tolerance: anchor at the
/// lexicographically smallest vertex and its farthest counterpart
/// (deterministic split), DP both halves.
fn simplify_loop(points: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = points.len();
    if n <= 4 {
        return points.to_vec();
    }
    // Rotate so the lexicographically smallest vertex is index 0.
    let start = (0..n)
        .min_by(|&a, &b| {
            points[a]
                .partial_cmp(&points[b])
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .unwrap_or(0);
    let rot: Vec<[f64; 2]> = (0..n).map(|i| points[(start + i) % n]).collect();
    // Farthest vertex from the anchor.
    let far = (1..n)
        .max_by(|&a, &b| {
            let da = (rot[a][0] - rot[0][0]).powi(2) + (rot[a][1] - rot[0][1]).powi(2);
            let db = (rot[b][0] - rot[0][0]).powi(2) + (rot[b][1] - rot[0][1]).powi(2);
            da.partial_cmp(&db).unwrap_or(core::cmp::Ordering::Equal)
        })
        .unwrap_or(n / 2);
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    dp_keep(&rot, 0, far, tol, &mut keep);
    // Second half wraps: treat as open segment far..n (virtual endpoint 0).
    let mut wrapped: Vec<[f64; 2]> = rot[far..].to_vec();
    wrapped.push(rot[0]);
    let mut keep2 = vec![false; wrapped.len()];
    dp_keep(&wrapped, 0, wrapped.len() - 1, tol, &mut keep2);
    for (i, k) in keep2.iter().enumerate().take(wrapped.len() - 1) {
        if *k {
            keep[far + i] = true;
        }
    }
    rot.iter()
        .zip(keep.iter())
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect()
}

/// Subdivide each polygon edge to segments no longer than `h`.
fn subdivide_loop(points: &[[f64; 2]], h: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let n = points.len();
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let segs = (len / h).ceil().max(1.0) as usize;
        for k in 0..segs {
            let t = k as f64 / segs as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Bowyer–Watson Delaunay (fixed insertion order, tie-as-outside).
// ---------------------------------------------------------------------------

fn incircle_ccw(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], eps: f64) -> bool {
    // For CCW (a, b, c): det > 0 ⇔ d strictly inside the circumcircle.
    let m = [
        [
            a[0] - d[0],
            a[1] - d[1],
            (a[0] - d[0]).powi(2) + (a[1] - d[1]).powi(2),
        ],
        [
            b[0] - d[0],
            b[1] - d[1],
            (b[0] - d[0]).powi(2) + (b[1] - d[1]).powi(2),
        ],
        [
            c[0] - d[0],
            c[1] - d[1],
            (c[0] - d[0]).powi(2) + (c[1] - d[1]).powi(2),
        ],
    ];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    det > eps
}

/// Conforming Delaunay of `points` (module pins). Returns CCW triples
/// into `points`.
fn delaunay(points: &[[f64; 2]]) -> Vec<[u32; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    // Scale-relative incircle epsilon (module pin: ties are OUTSIDE).
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in points {
        for c in 0..2 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2))
        .sqrt()
        .max(1.0);
    let eps = 1e-9 * diag.powi(4);
    // Super-triangle far outside the bbox.
    let cx = (lo[0] + hi[0]) / 2.0;
    let cy = (lo[1] + hi[1]) / 2.0;
    let r = diag * 16.0 + 16.0;
    let mut pts: Vec<[f64; 2]> = points.to_vec();
    let s0 = pts.len() as u32;
    // CCW under y-down: (top, right-bottom, left-bottom) — verify via
    // signed area and swap if needed.
    pts.push([cx, cy - 2.0 * r]);
    pts.push([cx + 2.0 * r, cy + r]);
    pts.push([cx - 2.0 * r, cy + r]);
    let mut tri0 = [s0, s0 + 1, s0 + 2];
    if PuppetMesh::area2(&pts, tri0) < 0.0 {
        tri0.swap(1, 2);
    }
    let mut tris: Vec<[u32; 3]> = vec![tri0];

    for (pi, p) in points.iter().enumerate() {
        // Duplicate guard (module pin).
        if points[..pi]
            .iter()
            .any(|q| (q[0] - p[0]).abs() < 1e-6 && (q[1] - p[1]).abs() < 1e-6)
        {
            continue;
        }
        let mut bad = Vec::new();
        for (ti, t) in tris.iter().enumerate() {
            if incircle_ccw(
                pts[t[0] as usize],
                pts[t[1] as usize],
                pts[t[2] as usize],
                *p,
                eps,
            ) {
                bad.push(ti);
            }
        }
        if bad.is_empty() {
            continue; // outside every circumcircle (numerical edge) — skip
        }
        // Directed cavity boundary: edges of bad triangles whose reverse
        // is not also a bad-triangle edge.
        use std::collections::BTreeSet;
        let mut directed: BTreeSet<(u32, u32)> = BTreeSet::new();
        for &ti in &bad {
            let t = tris[ti];
            for k in 0..3 {
                directed.insert((t[k], t[(k + 1) % 3]));
            }
        }
        let boundary: Vec<(u32, u32)> = directed
            .iter()
            .filter(|(a, b)| !directed.contains(&(*b, *a)))
            .copied()
            .collect();
        // Closed-fan validity: every boundary vertex appears exactly
        // once as a start and once as an end.
        {
            use std::collections::BTreeMap;
            let mut starts: BTreeMap<u32, u32> = BTreeMap::new();
            let mut ends: BTreeMap<u32, u32> = BTreeMap::new();
            for (a, b) in &boundary {
                *starts.entry(*a).or_default() += 1;
                *ends.entry(*b).or_default() += 1;
            }
            let ok = !boundary.is_empty()
                && starts.values().all(|c| *c == 1)
                && ends.values().all(|c| *c == 1)
                && starts.len() == ends.len();
            if !ok {
                continue; // fragmented cavity — skip point (module pin)
            }
        }
        // Remove bad triangles (descending indices), fan the cavity.
        for &ti in bad.iter().rev() {
            tris.swap_remove(ti);
        }
        let pidx = pi as u32;
        for (a, b) in boundary {
            let t = [a, b, pidx];
            if PuppetMesh::area2(&pts, t).abs() > 1e-12 {
                tris.push(if PuppetMesh::area2(&pts, t) > 0.0 {
                    t
                } else {
                    [b, a, pidx]
                });
            }
        }
    }
    // Drop super-triangle fans.
    tris.retain(|t| t.iter().all(|&v| v < s0));
    tris
}

// ---------------------------------------------------------------------------
// The §45.5 mesh builder.
// ---------------------------------------------------------------------------

/// Build the §45.5 rest mesh over `coverage` (alpha > 0, or the
/// selection ≥ 0.5 region): dilate by `expansion_px`, trace + simplify
/// the boundary, subdivide to the §18.4 edge budget, add the staggered
/// Steiner lattice, triangulate, cull to coverage.
pub fn build_mesh(
    coverage: &CoverageMask,
    density: MeshDensity,
    expansion_px: i32,
) -> Result<PuppetMesh, MeshError> {
    if coverage.area() == 0 {
        return Err(MeshError::EmptyCoverage);
    }
    let dilated = coverage.dilate(expansion_px.clamp(0, 32));
    let area = dilated.area();
    let h = edge_budget(area, density.target_triangles());

    // 1. Boundary loops: trace → simplify (2 px, §45.5) → subdivide (h).
    let mut points: Vec<[f64; 2]> = Vec::new();
    for lp in trace_boundary_loops(&dilated) {
        let simplified = simplify_loop(&lp, 2.0);
        if simplified.len() < 3 {
            continue;
        }
        points.extend(subdivide_loop(&simplified, h));
    }
    // 2. Steiner lattice (hex-staggered, h/2 clearance) — row-major
    //    fixed ordering (§45.5 determinism clause).
    let clear2 = dilated.clearance_sq();
    let row_pitch = h * 3.0f64.sqrt() / 2.0;
    let clearance = (h / 2.0) * (h / 2.0);
    let w = dilated.rect.width() as usize;
    let mut j = 0usize;
    loop {
        let y = f64::from(dilated.rect.min.y) + (j as f64 + 0.5) * row_pitch;
        if y >= f64::from(dilated.rect.max.y) {
            break;
        }
        let x_off = if j.is_multiple_of(2) { 0.5 * h } else { h };
        let mut i = 0usize;
        loop {
            let x = f64::from(dilated.rect.min.x) + x_off + i as f64 * h;
            if x >= f64::from(dilated.rect.max.x) {
                break;
            }
            let px = (x.floor() as i32 - dilated.rect.min.x) as usize;
            let py = (y.floor() as i32 - dilated.rect.min.y) as usize;
            if dilated.contains_point(x, y) && clear2[py * w + px] >= clearance {
                points.push([x, y]);
            }
            i += 1;
        }
        j += 1;
    }
    if points.len() < 3 {
        return Err(MeshError::Degenerate);
    }

    // 3. Triangulate + cull to the dilated coverage.
    let tris_all = delaunay(&points);
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for t in tris_all {
        let c = [
            (points[t[0] as usize][0] + points[t[1] as usize][0] + points[t[2] as usize][0]) / 3.0,
            (points[t[0] as usize][1] + points[t[1] as usize][1] + points[t[2] as usize][1]) / 3.0,
        ];
        if dilated.contains_point(c[0], c[1]) && PuppetMesh::area2(&points, t).abs() > 1e-9 {
            tris.push(t);
        }
    }
    if tris.is_empty() {
        return Err(MeshError::Degenerate);
    }
    // 4. Compact to referenced vertices (stable first-use order).
    let mut remap: Vec<Option<u32>> = vec![None; points.len()];
    let mut verts = Vec::new();
    let mut out_tris = Vec::with_capacity(tris.len());
    for t in &tris {
        let mut nt = [0u32; 3];
        for (k, &vi) in t.iter().enumerate() {
            let slot = &mut remap[vi as usize];
            let idx = match slot {
                Some(i) => *i,
                None => {
                    let i = verts.len() as u32;
                    verts.push(points[vi as usize]);
                    *slot = Some(i);
                    i
                }
            };
            nt[k] = idx;
        }
        out_tris.push(nt);
    }
    Ok(PuppetMesh {
        verts,
        tris: out_tris,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc_mask(r: i32) -> CoverageMask {
        let rect = IRect::new(0, 0, 2 * r + 8, 2 * r + 8);
        let c = f64::from(r) + 4.0;
        CoverageMask::from_fn(rect, |x, y| {
            let dx = f64::from(x) + 0.5 - c;
            let dy = f64::from(y) + 0.5 - c;
            dx * dx + dy * dy <= f64::from(r) * f64::from(r)
        })
    }

    fn l_mask() -> CoverageMask {
        // The §45.5 golden fixture shape: an L (64×64 with a 40×40 bite).
        let rect = IRect::new(0, 0, 64, 64);
        CoverageMask::from_fn(rect, |x, y| !(x >= 24 && y < 40))
    }

    #[test]
    fn empty_coverage_is_typed() {
        let m = CoverageMask::from_fn(IRect::new(0, 0, 8, 8), |_, _| false);
        assert_eq!(
            build_mesh(&m, MeshDensity::Normal, 2),
            Err(MeshError::EmptyCoverage)
        );
    }

    #[test]
    fn dilation_is_the_euclidean_disc() {
        let rect = IRect::new(0, 0, 9, 9);
        let m = CoverageMask::from_fn(rect, |x, y| x == 4 && y == 4);
        let d = m.dilate(2);
        // Distance ≤ 2 in Euclidean metric: (4±2, 4), (4, 4±2), the
        // diagonal (5, 5) at √2 — but NOT (6, 6) at 2√2.
        assert!(d.at(6, 4) && d.at(4, 6) && d.at(5, 5));
        assert!(!d.at(6, 6));
        assert!(!d.at(7, 4), "distance 3 stays out");
        // r = 0 is the identity.
        let same = m.dilate(0);
        assert_eq!(same.area(), 1);
    }

    #[test]
    fn mesh_is_deterministic_and_ccw() {
        let m = l_mask();
        let a = build_mesh(&m, MeshDensity::Normal, 2).unwrap();
        let b = build_mesh(&m, MeshDensity::Normal, 2).unwrap();
        assert_eq!(a, b, "§45.5: identical inputs mesh identically");
        for t in &a.tris {
            assert!(
                PuppetMesh::area2(&a.verts, *t) > 0.0,
                "every triangle CCW (y-down)"
            );
        }
    }

    #[test]
    fn density_presets_bracket_their_targets() {
        let m = disc_mask(48);
        // Loose brackets: the conforming construction approximates the
        // §18.4 budget, it does not hit counts exactly.
        for (density, target) in [
            (MeshDensity::Fewer, 500usize),
            (MeshDensity::Normal, 2000),
            (MeshDensity::More, 8000),
        ] {
            let mesh = build_mesh(&m, density, 2).unwrap();
            let n = mesh.tris.len();
            assert!(
                n >= target / 4 && n <= target * 4,
                "{density:?}: {n} triangles vs target {target}"
            );
        }
        // Monotone: more density ⇒ more triangles.
        let f = build_mesh(&m, MeshDensity::Fewer, 2).unwrap().tris.len();
        let n = build_mesh(&m, MeshDensity::Normal, 2).unwrap().tris.len();
        let x = build_mesh(&m, MeshDensity::More, 2).unwrap().tris.len();
        assert!(f < n && n < x, "{f} < {n} < {x}");
    }

    #[test]
    fn expansion_grows_the_hull() {
        let m = disc_mask(20);
        let tight = build_mesh(&m, MeshDensity::Fewer, 0).unwrap();
        let wide = build_mesh(&m, MeshDensity::Fewer, 8).unwrap();
        let bbox = |mesh: &PuppetMesh| {
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for v in &mesh.verts {
                for c in 0..2 {
                    lo[c] = lo[c].min(v[c]);
                    hi[c] = hi[c].max(v[c]);
                }
            }
            (lo, hi)
        };
        let (tl, th) = bbox(&tight);
        let (wl, wh) = bbox(&wide);
        assert!(wl[0] < tl[0] && wl[1] < tl[1] && wh[0] > th[0] && wh[1] > th[1]);
    }

    #[test]
    fn holes_stay_holes() {
        // Donut: disc of r=24 minus disc of r=10.
        let rect = IRect::new(0, 0, 56, 56);
        let m = CoverageMask::from_fn(rect, |x, y| {
            let dx = f64::from(x) + 0.5 - 28.0;
            let dy = f64::from(y) + 0.5 - 28.0;
            let d2 = dx * dx + dy * dy;
            d2 <= 24.0 * 24.0 && d2 > 10.0 * 10.0
        });
        let mesh = build_mesh(&m, MeshDensity::Normal, 0).unwrap();
        // No triangle centroid inside the hole.
        for i in 0..mesh.tris.len() {
            let c = mesh.centroid(i);
            let d2 = (c[0] - 28.0).powi(2) + (c[1] - 28.0).powi(2);
            assert!(d2 > 8.0 * 8.0, "centroid {c:?} landed in the hole");
        }
        // The point-location probe agrees.
        assert!(mesh.containing_triangle([28.0, 28.0]).is_none());
        assert!(mesh.containing_triangle([28.0, 45.0]).is_some());
    }

    #[test]
    fn adjacency_is_symmetric_and_edge_shared() {
        let mesh = build_mesh(&disc_mask(16), MeshDensity::Fewer, 2).unwrap();
        let adj = mesh.adjacency();
        for (i, ns) in adj.iter().enumerate() {
            for &nb in ns {
                assert!(adj[nb as usize].contains(&(i as u32)), "symmetry");
                let a: std::collections::BTreeSet<u32> = mesh.tris[i].iter().copied().collect();
                let b: std::collections::BTreeSet<u32> =
                    mesh.tris[nb as usize].iter().copied().collect();
                assert_eq!(a.intersection(&b).count(), 2, "share exactly one edge");
            }
        }
    }

    #[test]
    fn nearest_vertex_and_containing_triangle_are_deterministic() {
        let mesh = build_mesh(&disc_mask(16), MeshDensity::Fewer, 2).unwrap();
        let v = mesh.nearest_vertex([20.0, 20.0]);
        assert_eq!(v, mesh.nearest_vertex([20.0, 20.0]));
        assert!(mesh.containing_triangle([20.0, 20.0]).is_some());
        assert!(mesh.containing_triangle([-50.0, -50.0]).is_none());
    }
}
