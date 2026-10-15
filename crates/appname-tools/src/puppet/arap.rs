// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §18.4 "Puppet warp" [NORMATIVE] — "Solver [LEGAL: Sorkine &
//! Alexa 2007, "As-Rigid-As-Possible Surface Modeling"], alternating per
//! iteration: local step — per triangle, the best-fit rotation of rest
//! edges onto current edges (closed-form 2×2 polar decomposition);
//! rigidity ∈ [0,1] (default 0.7) then smooths the fitted rotations by
//! ⌈3·rigidity⌉ rounds of neighbor averaging; global step — solve the
//! sparse Laplacian system for vertex positions with position pins as
//! hard constraints. The matrix factors once (sparse Cholesky) per
//! mesh + pin topology, so each iteration is a back-substitution." Plus
//! §45.5 [NORMATIVE]: rotation pins (16 px, nearest pin, tie → lower pin
//! index, re-imposed after every averaging pass), depth adoption
//! (nearest pin within 16 px else 0), warm starting (collinear
//! stationarity), the commit convergence rule (f64 CPU,
//! `‖Δx‖∞ < 0.05 px` or 100 iterations), and the §18.4 apply
//! (per-triangle closed-form affine inverse map + §18.2 sampling;
//! outside-mesh transparent; §45.5 lexicographic (depth, triangleIndex)
//! overlap rule).
//!
//! SOLVER BOUNDARY (the ruled consumption shape): the global step's
//! sparse SPD solve sits behind [`ArapGlobalSolver`]. The ELECTED
//! production backend is libigl's `min_quad_with_fixed` machinery
//! consumed as a cpp shim TU from a pinned external checkout
//! (deps/libigl.yaml; O-INT ruling 2026-08-17: file-level MPL
//! compliance, census-arm verbatim; riders: Eigen record
//! (deps/eigen.yaml, EIGEN_MPL2_ONLY), copyleft-exclusion guard, MLS =
//! own) — the `arap-libigl` feature compiles it (`arap_libigl` module).
//! Everything OUTSIDE the solve — weights, local step, smoothing,
//! rotation/depth pin semantics, iteration, convergence, rasterization —
//! is THIS module's own Rust, implemented from the spec text and the
//! published paper (sorkine07); no libigl source was read or consulted.
//! The `#[cfg(test)]` dense f64 Cholesky below is a test oracle
//! exercising the driver deterministically in the gate; it ships in no
//! binary and does not supersede the libigl election.
//!
//! v1 pins (recorded, not invented — lane log this date):
//! * Edge weights: per-triangle cotangent halves (`w_ij += cot(angle
//!   opposite (i,j)) / 2`, sorkine07 discretization), computed on the
//!   REST mesh, each contribution clamped to ≥ 1e-7 (sliver-triangle
//!   PSD guard — deterministic, recorded robustness pin).
//! * Local step closed form: with rest edge e and current edge f per
//!   triangle, θ_t = atan2(Σ w·cross(e, f), Σ w·dot(e, f)) — the 2×2
//!   polar decomposition's rotation, always det +1 (§45.5 "the local
//!   step's polar decomposition picks the nearest rotation"; inverted
//!   triangles allowed).
//! * Rotation smoothing averages unit vectors (cos θ, sin θ) over the
//!   shared-edge neighborhood INCLUDING self, then re-normalizes;
//!   rotation-pinned triangles are re-imposed after every round (§45.5).
//! * Pin → vertex constraint: a pin constrains its NEAREST rest vertex
//!   (lowest index on ties). Coincident pins snapping to one vertex:
//!   the LOWEST pin index wins (deterministic, recorded).
//! * Depth adoption uses REST centroids (topology-stable under drags).
//! * Rasterizer painter's order: triangles sorted ascending by
//!   (depth, triangleIndex) and drawn overwriting — per-pixel the
//!   §45.5 lexicographic max wins. Inside test: all three edge
//!   functions ≥ −1e-9 (shared edges resolve by draw order,
//!   deterministic). Kernel: `resample:"auto"` picks per triangle by
//!   the affine scale metric √|area_current/area_rest| (§18.2 rule);
//!   the exact-scale-1 case samples Catmull-Rom (the §18.2 "no-op"
//!   applies to axis-aligned integer copies only — continuous inverse
//!   maps still need a kernel; recorded pin).
//! * Identity short-circuit: deformed positions bit-equal to rest copy
//!   pixels verbatim (§45.5 committed no-op bit-identity).

use appname_core::IRect;

use crate::puppet::mesh::PuppetMesh;
use crate::puppet::{PuppetPin, ROTATION_PIN_RADIUS};
use crate::resample::{auto_kernel, EdgePolicy, Kernel, Plane};

/// Typed ARAP failures (never a panic at the engine boundary).
#[derive(Clone, Debug, PartialEq)]
pub enum ArapError {
    /// No pins (the wire schema forbids this; defensive).
    NoPins,
    /// Every vertex is constrained — nothing to solve.
    NoFreeVertices,
    /// The global solver failed (singular/unfactored system).
    SolverFailed(&'static str),
}

impl core::fmt::Display for ArapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ArapError::NoPins => write!(f, "at least one pin is required (§10.3 len ≥ 1)"),
            ArapError::NoFreeVertices => write!(f, "every mesh vertex is pin-constrained"),
            ArapError::SolverFailed(m) => write!(f, "ARAP global solve failed: {m}"),
        }
    }
}

impl std::error::Error for ArapError {}

/// The global-step solver boundary (module doc). `factor` receives the
/// free-vertex count and the symmetric positive-definite Laplacian as
/// deterministic-ordered triplets; `solve2` back-substitutes the two
/// coordinate RHS vectors against the last factorization.
pub trait ArapGlobalSolver {
    fn factor(&mut self, n_free: usize, triplets: &[(u32, u32, f64)]) -> Result<(), ArapError>;
    fn solve2(&mut self, bx: &[f64], by: &[f64]) -> Result<(Vec<f64>, Vec<f64>), ArapError>;
}

/// Precomputed, topology-fixed solve state (§18.4 factor-once rule:
/// rebuild on mesh or pin-topology change; position edits alone re-run
/// [`solve_arap`] against the same setup).
#[derive(Debug)]
pub struct ArapSetup {
    /// Constrained vertex → pin index (deterministic winner per module
    /// pin).
    pub pinned: Vec<(u32, u32)>,
    /// Per-vertex: Some(free index) | None (pinned).
    pub free_index: Vec<Option<u32>>,
    /// Free count.
    pub n_free: usize,
    /// Per-triangle, per-edge (k, k+1) cotan weights (clamped, rest).
    pub tri_edge_w: Vec<[f64; 3]>,
    /// Shared-edge triangle adjacency (sorted).
    pub adjacency: Vec<Vec<u32>>,
    /// Per-triangle rotation-pin angle (radians), §45.5 16-px rule.
    pub rotation_pin: Vec<Option<f64>>,
    /// Per-triangle depth (§45.5 adoption rule).
    pub tri_depth: Vec<i64>,
    /// Laplacian triplets over free indices (deterministic order).
    pub triplets: Vec<(u32, u32, f64)>,
}

fn cot_clamped(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    // cot of the angle at vertex a, formed by edges a→b and a→c.
    let u = [b[0] - a[0], b[1] - a[1]];
    let v = [c[0] - a[0], c[1] - a[1]];
    let dot = u[0] * v[0] + u[1] * v[1];
    let cross = (u[0] * v[1] - u[1] * v[0]).abs().max(1e-12);
    (dot / cross).max(1e-7) // module pin: PSD guard clamp
}

impl ArapSetup {
    pub fn new(mesh: &PuppetMesh, pins: &[PuppetPin]) -> Result<ArapSetup, ArapError> {
        if pins.is_empty() {
            return Err(ArapError::NoPins);
        }
        let nv = mesh.verts.len();
        // Pin → nearest rest vertex; lowest pin index wins a vertex.
        let mut pinned_map: std::collections::BTreeMap<u32, u32> =
            std::collections::BTreeMap::new();
        for (pi, pin) in pins.iter().enumerate() {
            let v = mesh.nearest_vertex(pin.rest) as u32;
            pinned_map.entry(v).or_insert(pi as u32);
        }
        let pinned: Vec<(u32, u32)> = pinned_map.into_iter().collect();
        let mut free_index = vec![None; nv];
        let mut n_free = 0u32;
        for (v, slot) in free_index.iter_mut().enumerate() {
            if !pinned.iter().any(|(pv, _)| *pv == v as u32) {
                *slot = Some(n_free);
                n_free += 1;
            }
        }
        if n_free == 0 {
            return Err(ArapError::NoFreeVertices);
        }

        // Cotan weights per triangle edge (module pin).
        let mut tri_edge_w = Vec::with_capacity(mesh.tris.len());
        for t in &mesh.tris {
            let p = [
                mesh.verts[t[0] as usize],
                mesh.verts[t[1] as usize],
                mesh.verts[t[2] as usize],
            ];
            // Edge (k, k+1) is opposite vertex (k+2).
            let w = [
                cot_clamped(p[2], p[0], p[1]) / 2.0,
                cot_clamped(p[0], p[1], p[2]) / 2.0,
                cot_clamped(p[1], p[2], p[0]) / 2.0,
            ];
            tri_edge_w.push(w);
        }

        // §45.5 rotation-pin + depth adoption (REST centroids).
        let mut rotation_pin = Vec::with_capacity(mesh.tris.len());
        let mut tri_depth = Vec::with_capacity(mesh.tris.len());
        for i in 0..mesh.tris.len() {
            let c = mesh.centroid(i);
            let mut best: Option<(f64, u32)> = None;
            for (pi, pin) in pins.iter().enumerate() {
                let d = ((c[0] - pin.rest[0]).powi(2) + (c[1] - pin.rest[1]).powi(2)).sqrt();
                if d <= ROTATION_PIN_RADIUS {
                    let better = match best {
                        None => true,
                        // Nearest wins; tie → lower pin index (§45.5).
                        Some((bd, _)) => d < bd,
                    };
                    if better {
                        best = Some((d, pi as u32));
                    }
                }
            }
            tri_depth.push(best.map(|(_, pi)| pins[pi as usize].depth).unwrap_or(0));
            rotation_pin.push(
                best.and_then(|(_, pi)| pins[pi as usize].rotation_deg.map(|deg| deg.to_radians())),
            );
        }

        // Laplacian triplets over free vertices (deterministic order:
        // triangle-major, then the fixed edge order).
        let mut triplets: Vec<(u32, u32, f64)> = Vec::new();
        for (t, w3) in mesh.tris.iter().zip(tri_edge_w.iter()) {
            for k in 0..3 {
                let (i, j) = (t[k] as usize, t[(k + 1) % 3] as usize);
                let w = w3[k];
                match (free_index[i], free_index[j]) {
                    (Some(fi), Some(fj)) => {
                        triplets.push((fi, fi, w));
                        triplets.push((fj, fj, w));
                        triplets.push((fi, fj, -w));
                        triplets.push((fj, fi, -w));
                    }
                    (Some(fi), None) => triplets.push((fi, fi, w)),
                    (None, Some(fj)) => triplets.push((fj, fj, w)),
                    (None, None) => {}
                }
            }
        }

        Ok(ArapSetup {
            pinned,
            free_index,
            n_free: n_free as usize,
            tri_edge_w,
            adjacency: mesh.adjacency(),
            rotation_pin,
            tri_depth,
            triplets,
        })
    }
}

/// A converged (or iteration-capped) solve.
#[derive(Clone, Debug)]
pub struct ArapSolution {
    /// Deformed vertex positions (pinned vertices at their pin targets).
    pub positions: Vec<[f64; 2]>,
    /// Local-global iterations executed.
    pub iterations: usize,
    /// `‖Δx‖∞ < tol` reached (vs the 100-iteration cap).
    pub converged: bool,
}

/// §45.5 commit convergence constants.
pub const COMMIT_TOL_PX: f64 = 0.05;
pub const COMMIT_MAX_ITERS: usize = 100;

/// Run the §18.4 local-global iteration to the §45.5 convergence rule.
/// `warm_start`, when given, seeds the iteration (collinear-stationarity
/// rule; length must equal the vertex count). Single-pin payloads are
/// the caller's rigid-translate bypass (§45.5 degenerate table) — this
/// solver requires the system the spec gives it.
#[allow(clippy::too_many_arguments)]
pub fn solve_arap(
    mesh: &PuppetMesh,
    setup: &ArapSetup,
    pins: &[PuppetPin],
    solver: &mut dyn ArapGlobalSolver,
    warm_start: Option<&[[f64; 2]]>,
    rigidity: f64,
    max_iters: usize,
    tol: f64,
) -> Result<ArapSolution, ArapError> {
    let nv = mesh.verts.len();
    solver.factor(setup.n_free, &setup.triplets)?;

    // Initial positions: warm start or rest, pins imposed.
    let mut x: Vec<[f64; 2]> = match warm_start {
        Some(w) if w.len() == nv => w.to_vec(),
        _ => mesh.verts.clone(),
    };
    // Displacement-form hard constraint (module pin): the snapped vertex
    // carries the PIN'S displacement — `v_rest + (target − rest)` — so a
    // pin at rest constrains its vertex exactly to rest (§45.5 identity)
    // and the snap radius never silently displaces the constraint (§45.5
    // "no snap" rationale honored at the solve).
    for &(v, pi) in &setup.pinned {
        let pin = &pins[pi as usize];
        let vr = mesh.verts[v as usize];
        x[v as usize] = [
            vr[0] + (pin.target[0] - pin.rest[0]),
            vr[1] + (pin.target[1] - pin.rest[1]),
        ];
    }

    let smoothing_rounds = (3.0 * rigidity.clamp(0.0, 1.0)).ceil() as usize;
    let mut iterations = 0usize;
    let mut converged = false;

    for _ in 0..max_iters {
        iterations += 1;
        // ---- Local step: per-triangle best-fit rotation (module pin).
        let mut theta: Vec<f64> = Vec::with_capacity(mesh.tris.len());
        for (ti, t) in mesh.tris.iter().enumerate() {
            if let Some(pinned_angle) = setup.rotation_pin[ti] {
                theta.push(pinned_angle);
                continue;
            }
            let w3 = setup.tri_edge_w[ti];
            let mut dot = 0.0f64;
            let mut cross = 0.0f64;
            for k in 0..3 {
                let (i, j) = (t[k] as usize, t[(k + 1) % 3] as usize);
                let e = [
                    mesh.verts[j][0] - mesh.verts[i][0],
                    mesh.verts[j][1] - mesh.verts[i][1],
                ];
                let f = [x[j][0] - x[i][0], x[j][1] - x[i][1]];
                dot += w3[k] * (e[0] * f[0] + e[1] * f[1]);
                cross += w3[k] * (e[0] * f[1] - e[1] * f[0]);
            }
            theta.push(if dot == 0.0 && cross == 0.0 {
                0.0
            } else {
                cross.atan2(dot)
            });
        }
        // ---- §45.5 rigidity smoothing (+ rotation-pin re-imposition).
        for _ in 0..smoothing_rounds {
            let mut next = theta.clone();
            for (ti, nb) in setup.adjacency.iter().enumerate() {
                let mut c = theta[ti].cos();
                let mut s = theta[ti].sin();
                for &n in nb {
                    c += theta[n as usize].cos();
                    s += theta[n as usize].sin();
                }
                if c != 0.0 || s != 0.0 {
                    next[ti] = s.atan2(c);
                }
            }
            // Re-impose hard-constrained rotations (§45.5).
            for (ti, rp) in setup.rotation_pin.iter().enumerate() {
                if let Some(a) = rp {
                    next[ti] = *a;
                }
            }
            theta = next;
        }
        // ---- Global step: assemble RHS, back-substitute.
        let mut bx = vec![0.0f64; setup.n_free];
        let mut by = vec![0.0f64; setup.n_free];
        for (ti, t) in mesh.tris.iter().enumerate() {
            let (c, s) = (theta[ti].cos(), theta[ti].sin());
            let w3 = setup.tri_edge_w[ti];
            for k in 0..3 {
                let (i, j) = (t[k] as usize, t[(k + 1) % 3] as usize);
                let e = [
                    mesh.verts[i][0] - mesh.verts[j][0],
                    mesh.verts[i][1] - mesh.verts[j][1],
                ];
                let re = [c * e[0] - s * e[1], s * e[0] + c * e[1]];
                let w = w3[k];
                // Energy term w‖(x_i − x_j) − R e‖²: rows i (+) and j (−).
                match (setup.free_index[i], setup.free_index[j]) {
                    (Some(fi), Some(fj)) => {
                        bx[fi as usize] += w * re[0];
                        by[fi as usize] += w * re[1];
                        bx[fj as usize] -= w * re[0];
                        by[fj as usize] -= w * re[1];
                    }
                    (Some(fi), None) => {
                        bx[fi as usize] += w * re[0] + w * x[j][0];
                        by[fi as usize] += w * re[1] + w * x[j][1];
                    }
                    (None, Some(fj)) => {
                        bx[fj as usize] += -w * re[0] + w * x[i][0];
                        by[fj as usize] += -w * re[1] + w * x[i][1];
                    }
                    (None, None) => {}
                }
            }
        }
        let (sx, sy) = solver.solve2(&bx, &by)?;
        // ---- Update + §45.5 convergence.
        let mut delta = 0.0f64;
        for (v, fi) in setup.free_index.iter().enumerate() {
            if let Some(fi) = fi {
                let nx = [sx[*fi as usize], sy[*fi as usize]];
                delta = delta
                    .max((nx[0] - x[v][0]).abs())
                    .max((nx[1] - x[v][1]).abs());
                x[v] = nx;
            }
        }
        if delta < tol {
            converged = true;
            break;
        }
    }
    Ok(ArapSolution {
        positions: x,
        iterations,
        converged,
    })
}

// ---------------------------------------------------------------------------
// §18.4 apply: per-triangle affine inverse map + §18.2 sampling.
// ---------------------------------------------------------------------------

/// Bbox of the deformed mesh, with 1-px slack (dirty-basis shape).
pub fn deformed_bounds(positions: &[[f64; 2]], tris: &[[u32; 3]]) -> IRect {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for t in tris {
        for &v in t {
            let p = positions[v as usize];
            for c in 0..2 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
    }
    if !lo[0].is_finite() {
        return IRect::EMPTY;
    }
    IRect::new(
        lo[0].floor() as i32 - 1,
        lo[1].floor() as i32 - 1,
        hi[0].ceil() as i32 + 1,
        hi[1].ceil() as i32 + 1,
    )
}

/// Rasterize the deformed mesh over `out_rect`: painter's order by
/// (depth, triangleIndex) ascending (§45.5 lexicographic-max rule),
/// per-triangle closed-form affine inverse map, §18.2 sampling
/// (`kernel = None` ⇒ auto by the per-triangle scale metric), pixels
/// outside every deformed triangle transparent (§45.5). Deformed
/// positions bit-equal to rest copy verbatim (identity pin).
pub fn rasterize_mesh(
    src: &Plane,
    mesh: &PuppetMesh,
    positions: &[[f64; 2]],
    tri_depth: &[i64],
    out_rect: IRect,
    kernel: Option<Kernel>,
) -> Plane {
    let mut out = Plane::new(out_rect);
    if positions == mesh.verts.as_slice() {
        // §45.5 committed no-op bit-identity (module pin).
        for y in out_rect.min.y..out_rect.max.y {
            for x in out_rect.min.x..out_rect.max.x {
                out.set(x, y, src.get(x, y, EdgePolicy::Transparent));
            }
        }
        return out;
    }
    // Painter's order (module pin).
    let mut order: Vec<usize> = (0..mesh.tris.len()).collect();
    order.sort_by_key(|&i| (tri_depth.get(i).copied().unwrap_or(0), i));

    for ti in order {
        let t = mesh.tris[ti];
        let (a, b, c) = (
            positions[t[0] as usize],
            positions[t[1] as usize],
            positions[t[2] as usize],
        );
        let (ra, rb, rc) = (
            mesh.verts[t[0] as usize],
            mesh.verts[t[1] as usize],
            mesh.verts[t[2] as usize],
        );
        // Affine current → rest from the 3 correspondences.
        let d = [b[0] - a[0], b[1] - a[1]];
        let e = [c[0] - a[0], c[1] - a[1]];
        let det = d[0] * e[1] - d[1] * e[0];
        if det.abs() < 1e-12 {
            continue; // zero-area current triangle draws nothing
        }
        let rd = [rb[0] - ra[0], rb[1] - ra[1]];
        let re = [rc[0] - ra[0], rc[1] - ra[1]];
        let rest_det = rd[0] * re[1] - rd[1] * re[0];
        // Per-triangle kernel (module pin: auto by scale metric).
        let k = match kernel {
            Some(k) => k,
            None => {
                let scale = (det.abs() / rest_det.abs().max(1e-12)).sqrt();
                auto_kernel(scale).unwrap_or(Kernel::CatmullRom)
            }
        };
        let support = k.support();
        // Bbox ∩ out_rect.
        let lo_x = a[0].min(b[0]).min(c[0]).floor() as i32;
        let lo_y = a[1].min(b[1]).min(c[1]).floor() as i32;
        let hi_x = a[0].max(b[0]).max(c[0]).ceil() as i32;
        let hi_y = a[1].max(b[1]).max(c[1]).ceil() as i32;
        let span = IRect::new(lo_x, lo_y, hi_x + 1, hi_y + 1).intersect(&out_rect);
        for y in span.min.y..span.max.y {
            for x in span.min.x..span.max.x {
                let p = [f64::from(x) + 0.5, f64::from(y) + 0.5];
                // Barycentric via the (a, d, e) frame.
                let q = [p[0] - a[0], p[1] - a[1]];
                let u = (q[0] * e[1] - q[1] * e[0]) / det;
                let v = (d[0] * q[1] - d[1] * q[0]) / det;
                if u < -1e-9 || v < -1e-9 || u + v > 1.0 + 1e-9 {
                    continue;
                }
                // Rest-space source position (+ the −0.5 center shift).
                let sx = ra[0] + u * rd[0] + v * re[0] - 0.5;
                let sy = ra[1] + u * rd[1] + v * re[1] - 0.5;
                let (lo_kx, hi_kx) = ((sx - support).ceil() as i64, (sx + support).floor() as i64);
                let (lo_ky, hi_ky) = ((sy - support).ceil() as i64, (sy + support).floor() as i64);
                let mut acc = [0.0f64; 4];
                let mut wsum = 0.0f64;
                for ky in lo_ky..=hi_ky {
                    let wy = k.weight(ky as f64 - sy);
                    if wy == 0.0 {
                        continue;
                    }
                    for kx in lo_kx..=hi_kx {
                        let w = k.weight(kx as f64 - sx) * wy;
                        if w == 0.0 {
                            continue;
                        }
                        let s = src.get(kx as i32, ky as i32, EdgePolicy::Transparent);
                        for ch in 0..4 {
                            acc[ch] += f64::from(s[ch]) * w;
                        }
                        wsum += w;
                    }
                }
                if wsum != 0.0 {
                    let iw = 1.0 / wsum;
                    out.set(
                        x,
                        y,
                        [
                            (acc[0] * iw) as f32,
                            (acc[1] * iw) as f32,
                            (acc[2] * iw) as f32,
                            (acc[3] * iw) as f32,
                        ],
                    );
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Test oracle: dense f64 Cholesky (cfg(test) — NOT a shipping backend).
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod test_oracle {
    use super::{ArapError, ArapGlobalSolver};

    /// Dense LLᵀ over the free-vertex Laplacian. Deterministic f64;
    /// adequate for the in-gate driver tests (small meshes). The
    /// PRODUCTION backend is the libigl election (module doc) — this
    /// oracle exists so the driver's math is testable in the gate
    /// without the external checkout.
    #[derive(Default)]
    pub struct DenseCholesky {
        n: usize,
        l: Vec<f64>, // lower-triangular factor, row-major n×n
    }

    impl ArapGlobalSolver for DenseCholesky {
        fn factor(&mut self, n: usize, triplets: &[(u32, u32, f64)]) -> Result<(), ArapError> {
            let mut a = vec![0.0f64; n * n];
            for &(i, j, v) in triplets {
                a[i as usize * n + j as usize] += v;
            }
            // In-place LLᵀ.
            for i in 0..n {
                for j in 0..=i {
                    let mut sum = a[i * n + j];
                    for k in 0..j {
                        sum -= a[i * n + k] * a[j * n + k];
                    }
                    if i == j {
                        if sum <= 0.0 {
                            return Err(ArapError::SolverFailed("matrix not positive definite"));
                        }
                        a[i * n + j] = sum.sqrt();
                    } else {
                        a[i * n + j] = sum / a[j * n + j];
                    }
                }
                for j in (i + 1)..n {
                    a[i * n + j] = 0.0;
                }
            }
            self.n = n;
            self.l = a;
            Ok(())
        }

        #[allow(clippy::needless_range_loop)] // index symmetry mirrors the math
        fn solve2(&mut self, bx: &[f64], by: &[f64]) -> Result<(Vec<f64>, Vec<f64>), ArapError> {
            if self.n != bx.len() || self.n != by.len() {
                return Err(ArapError::SolverFailed("rhs length mismatch"));
            }
            let n = self.n;
            let solve = |b: &[f64]| -> Vec<f64> {
                let mut y = vec![0.0f64; n];
                for i in 0..n {
                    let mut s = b[i];
                    for k in 0..i {
                        s -= self.l[i * n + k] * y[k];
                    }
                    y[i] = s / self.l[i * n + i];
                }
                let mut x = vec![0.0f64; n];
                for i in (0..n).rev() {
                    let mut s = y[i];
                    for k in (i + 1)..n {
                        s -= self.l[k * n + i] * x[k];
                    }
                    x[i] = s / self.l[i * n + i];
                }
                x
            };
            Ok((solve(bx), solve(by)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_oracle::DenseCholesky;
    use super::*;
    use crate::puppet::mesh::{build_mesh, CoverageMask, MeshDensity};
    use crate::puppet::PuppetPin;

    fn disc_mesh(r: i32) -> PuppetMesh {
        let rect = IRect::new(0, 0, 2 * r + 8, 2 * r + 8);
        let c = f64::from(r) + 4.0;
        let m = CoverageMask::from_fn(rect, |x, y| {
            let dx = f64::from(x) + 0.5 - c;
            let dy = f64::from(y) + 0.5 - c;
            dx * dx + dy * dy <= f64::from(r) * f64::from(r)
        });
        build_mesh(&m, MeshDensity::Fewer, 2).unwrap()
    }

    fn checker(rect: IRect) -> Plane {
        let mut p = Plane::new(rect);
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let v = if (x / 4 + y / 4) % 2 == 0 { 1.0 } else { 0.25 };
                p.set(x, y, [v, v * 0.5, 1.0 - v, 1.0]);
            }
        }
        p
    }

    fn solve(
        mesh: &PuppetMesh,
        pins: &[PuppetPin],
        rigidity: f64,
        warm: Option<&[[f64; 2]]>,
    ) -> ArapSolution {
        solve_with(mesh, pins, rigidity, warm, COMMIT_MAX_ITERS, COMMIT_TOL_PX)
    }

    fn solve_with(
        mesh: &PuppetMesh,
        pins: &[PuppetPin],
        rigidity: f64,
        warm: Option<&[[f64; 2]]>,
        max_iters: usize,
        tol: f64,
    ) -> ArapSolution {
        let setup = ArapSetup::new(mesh, pins).unwrap();
        let mut solver = DenseCholesky::default();
        solve_arap(
            mesh,
            &setup,
            pins,
            &mut solver,
            warm,
            rigidity,
            max_iters,
            tol,
        )
        .unwrap()
    }

    #[test]
    fn rest_pins_solve_to_the_rest_state() {
        // §45.5: pins at rest positions solve to the rest state.
        let mesh = disc_mesh(16);
        let pins = vec![
            PuppetPin::at([12.0, 20.0]),
            PuppetPin::at([28.0, 20.0]),
            PuppetPin::at([20.0, 30.0]),
        ];
        let sol = solve(&mesh, &pins, 0.7, None);
        assert!(sol.converged, "trivial fixed point converges immediately");
        let max_dev = mesh
            .verts
            .iter()
            .zip(sol.positions.iter())
            .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
            .fold(0.0f64, f64::max);
        assert!(max_dev < 1e-9, "rest solve deviates {max_dev}");
    }

    #[test]
    fn uniform_pin_translation_translates_rigidly() {
        // Zero-energy configuration: pure translation.
        let mesh = disc_mesh(16);
        let delta = [7.0, -4.0];
        let mk = |p: [f64; 2]| PuppetPin {
            rest: p,
            target: [p[0] + delta[0], p[1] + delta[1]],
            rotation_deg: None,
            depth: 0,
        };
        let pins = vec![mk([12.0, 20.0]), mk([28.0, 20.0]), mk([20.0, 30.0])];
        // Commit rule (0.05 px / 100 iters) leaves at most a few-times-tol
        // residual crawl; a tight-tolerance run pins the mathematical
        // fixed point.
        let sol = solve_with(&mesh, &pins, 0.7, None, 1000, 1e-6);
        assert!(sol.converged);
        for (r, x) in mesh.verts.iter().zip(sol.positions.iter()) {
            assert!(
                (x[0] - r[0] - delta[0]).abs() < 1e-3 && (x[1] - r[1] - delta[1]).abs() < 1e-3,
                "vertex {r:?} → {x:?} is not the rigid translation"
            );
        }
    }

    #[test]
    fn rotated_pins_recover_the_rotation() {
        // Pins moved through a rigid rotation about the disc center:
        // ARAP's zero-energy solution is that rotation.
        let mesh = disc_mesh(16);
        let c = [20.0, 20.0];
        let th = 25.0f64.to_radians();
        let rot = |p: [f64; 2]| {
            let dx = p[0] - c[0];
            let dy = p[1] - c[1];
            [
                c[0] + th.cos() * dx - th.sin() * dy,
                c[1] + th.sin() * dx + th.cos() * dy,
            ]
        };
        // Pins placed exactly ON mesh vertices (the session tier places
        // pins on the mesh; vertex-exact rests make the rigid motion the
        // solver's exact zero-energy target).
        let mk = |p: [f64; 2]| {
            let v = mesh.verts[mesh.nearest_vertex(p)];
            PuppetPin {
                rest: v,
                target: rot(v),
                rotation_deg: None,
                depth: 0,
            }
        };
        let pins = vec![mk([12.0, 20.0]), mk([28.0, 20.0]), mk([20.0, 30.0])];
        // Tight-tolerance run (the mathematical fixed point; the §45.5
        // commit rule's 0.05-px stop is exercised elsewhere).
        let sol = solve_with(&mesh, &pins, 0.7, None, 2000, 1e-6);
        let max_dev = mesh
            .verts
            .iter()
            .zip(sol.positions.iter())
            .map(|(r, x)| {
                let e = rot(*r);
                (e[0] - x[0]).abs().max((e[1] - x[1]).abs())
            })
            .fold(0.0f64, f64::max);
        assert!(
            max_dev < 0.05,
            "solution deviates {max_dev} px from the rigid rotation"
        );
    }

    #[test]
    fn rotation_pin_bends_the_neighborhood() {
        // §45.5: one anchor + one 30° rotation pin — the wire-expressible
        // bend. Near-pin geometry rotates; the anchor holds.
        let mesh = disc_mesh(16);
        let anchor = PuppetPin::at([12.0, 20.0]);
        let mut dial = PuppetPin::at([28.0, 20.0]);
        dial.rotation_deg = Some(30.0);
        let pins = vec![anchor, dial];
        let sol = solve(&mesh, &pins, 0.7, None);
        // The mesh deformed (not the rest state).
        let moved = mesh
            .verts
            .iter()
            .zip(sol.positions.iter())
            .map(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs())
            .fold(0.0f64, f64::max);
        assert!(moved > 0.5, "a rotation pin deforms the mesh: {moved}");
        // Determinism ×2.
        let sol2 = solve(&mesh, &pins, 0.7, None);
        assert_eq!(sol.positions, sol2.positions);
        // Rigidity raises coherence: smoothing rounds differ (0 vs 3),
        // so the solutions differ.
        let loose = solve(&mesh, &pins, 0.0, None);
        let stiff = solve(&mesh, &pins, 1.0, None);
        assert_ne!(loose.positions, stiff.positions);
    }

    #[test]
    fn collinear_pins_are_stationary_under_warm_start() {
        // §45.5 degenerate table: all pins collinear — warm-started
        // resolves stay bounded (no snap).
        let mesh = disc_mesh(16);
        let pins = vec![
            PuppetPin::at([12.0, 20.0]),
            PuppetPin::at([20.0, 20.0]),
            PuppetPin::at([28.0, 20.0]),
        ];
        let first = solve(&mesh, &pins, 0.7, None);
        let second = solve(&mesh, &pins, 0.7, Some(&first.positions));
        let drift = first
            .positions
            .iter()
            .zip(second.positions.iter())
            .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
            .fold(0.0f64, f64::max);
        assert!(drift < COMMIT_TOL_PX, "frame-to-frame drift {drift}");
    }

    #[test]
    fn depth_and_rotation_pin_assignment_follow_the_16px_rule() {
        let mesh = disc_mesh(16);
        let mut a = PuppetPin::at([20.0, 20.0]);
        a.depth = 3;
        a.rotation_deg = Some(45.0);
        let far = PuppetPin::at([12.0, 30.0]);
        let setup = ArapSetup::new(&mesh, &[a, far]).unwrap();
        // Triangles whose rest centroid is within 16 px of (20,20) adopt
        // depth 3 + the pinned rotation; far triangles keep 0/None.
        for (ti, d) in setup.tri_depth.iter().enumerate() {
            let c = mesh.centroid(ti);
            let da = ((c[0] - 20.0f64).powi(2) + (c[1] - 20.0).powi(2)).sqrt();
            let dfar = ((c[0] - 12.0f64).powi(2) + (c[1] - 30.0).powi(2)).sqrt();
            if da <= 16.0 && da <= dfar {
                assert_eq!(*d, 3, "triangle {ti} near the depth pin");
                assert!(setup.rotation_pin[ti].is_some());
            }
            if da > 16.0 && dfar > 16.0 {
                assert_eq!(*d, 0);
                assert!(setup.rotation_pin[ti].is_none());
            }
        }
    }

    #[test]
    fn rasterize_identity_is_bit_exact() {
        let mesh = disc_mesh(16);
        let rect = IRect::new(0, 0, 40, 40);
        let src = checker(rect);
        let depths = vec![0i64; mesh.tris.len()];
        let out = rasterize_mesh(&src, &mesh, &mesh.verts, &depths, rect, None);
        assert_eq!(out, src, "§45.5 committed no-op bit-identity");
    }

    #[test]
    fn rasterize_translation_moves_content_exactly() {
        let mesh = disc_mesh(16);
        let rect = IRect::new(0, 0, 48, 48);
        let src = checker(rect);
        let moved: Vec<[f64; 2]> = mesh.verts.iter().map(|v| [v[0] + 8.0, v[1]]).collect();
        let depths = vec![0i64; mesh.tris.len()];
        let out_rect = deformed_bounds(&moved, &mesh.tris).intersect(&rect);
        let out = rasterize_mesh(&src, &mesh, &moved, &depths, out_rect, None);
        // Strictly interior probe points (away from triangle borders by
        // more than the kernel support): exact integer-shift sampling.
        for (x, y) in [(24, 20), (28, 24), (20, 24)] {
            assert_eq!(
                out.get(x, y, EdgePolicy::Transparent),
                src.get(x - 8, y, EdgePolicy::Transparent),
                "({x},{y})"
            );
        }
        // Outside the deformed mesh: transparent (§45.5).
        assert_eq!(
            out.get(out_rect.min.x, out_rect.min.y, EdgePolicy::Transparent),
            [0.0; 4]
        );
    }

    #[test]
    fn overlap_resolves_by_lexicographic_depth_then_index() {
        // Two hand-built triangles deformed onto the SAME square; the
        // §45.5 painter's rule keeps the higher (depth, index).
        let mesh = PuppetMesh {
            verts: vec![
                [0.0, 0.0],
                [8.0, 0.0],
                [0.0, 8.0],
                [20.0, 0.0],
                [28.0, 0.0],
                [20.0, 8.0],
            ],
            tris: vec![[0, 1, 2], [3, 4, 5]],
        };
        let rect = IRect::new(0, 0, 32, 16);
        // Source: left triangle over red, right triangle over blue.
        let mut src = Plane::new(rect);
        for y in 0..16 {
            for x in 0..32 {
                let v = if x < 16 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                src.set(x, y, v);
            }
        }
        // Both deformed onto the square at (40, 0) — overlap.
        let positions = vec![
            [40.0, 0.0],
            [48.0, 0.0],
            [40.0, 8.0],
            [40.0, 0.0],
            [48.0, 0.0],
            [40.0, 8.0],
        ];
        let out_rect = IRect::new(40, 0, 48, 8);
        // Equal depth: higher INDEX (blue) wins.
        let out = rasterize_mesh(&src, &mesh, &positions, &[0, 0], out_rect, None);
        assert!(
            out.get(42, 2, EdgePolicy::Transparent)[2] > 0.9,
            "index tie → blue"
        );
        // Depth: left triangle depth +1 wins despite the lower index.
        let out = rasterize_mesh(&src, &mesh, &positions, &[1, 0], out_rect, None);
        assert!(
            out.get(42, 2, EdgePolicy::Transparent)[0] > 0.9,
            "depth beats index"
        );
    }

    #[test]
    fn inverted_triangles_still_render() {
        // §45.5: folds are legitimate; the polar decomposition and the
        // rasterizer both tolerate inversion.
        let mesh = PuppetMesh {
            verts: vec![[0.0, 0.0], [8.0, 0.0], [0.0, 8.0]],
            tris: vec![[0, 1, 2]],
        };
        let rect = IRect::new(0, 0, 16, 16);
        let src = checker(rect);
        // Mirror horizontally about x = 4 (inverts orientation).
        let positions = vec![[8.0, 0.0], [0.0, 0.0], [8.0, 8.0]];
        let out = rasterize_mesh(&src, &mesh, &positions, &[0], rect, None);
        let mut painted = 0;
        for y in 0..16 {
            for x in 0..16 {
                if out.get(x, y, EdgePolicy::Transparent)[3] > 0.0 {
                    painted += 1;
                }
            }
        }
        assert!(painted > 10, "inverted triangle painted {painted} px");
    }

    #[test]
    fn setup_rejects_are_typed() {
        let mesh = disc_mesh(8);
        assert_eq!(ArapSetup::new(&mesh, &[]).unwrap_err(), ArapError::NoPins);
        // A single-triangle mesh with pins on every vertex: no free DOF.
        let tiny = PuppetMesh {
            verts: vec![[0.0, 0.0], [8.0, 0.0], [0.0, 8.0]],
            tris: vec![[0, 1, 2]],
        };
        let pins = vec![
            PuppetPin::at([0.0, 0.0]),
            PuppetPin::at([8.0, 0.0]),
            PuppetPin::at([0.0, 8.0]),
        ];
        assert_eq!(
            ArapSetup::new(&tiny, &pins).unwrap_err(),
            ArapError::NoFreeVertices
        );
    }
}
