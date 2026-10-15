// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Implemented clean-room from the published method (S. Schaefer,
// T. McPhail, J. Warren, "Image Deformation Using Moving Least Squares",
// SIGGRAPH 2006 — census paper key schaefer06). Census row
// B2-moving-least-squares.yaml (key `moving-least-squares`, upstream
// Jarvis73/Moving-Least-Squares, MIT) records that repository as a
// licensed behavior cross-check ONLY. Acquisition scope, stated
// precisely: the round-1 B2 census itself DID shallow-clone it and read
// its LICENSE (the sanctioned, recorded act — `fetched: 2026-08-11` on
// the census row); it was not fetched, opened, or consulted by this
// lane in this build wave, and it is not staged in any build tree —
// `files_taken: []`, verdict ported-later. No expression from it is
// present here; every formula below is derived from the paper.

//! Spec: ALGORITHMS.md §19 `warps_upright` — "Point-handle warps: MLS
//! deformation (rigid variant default) [R-schaefer06] (L2)"; contract
//! `contracts/algorithms/algorithms.yaml` area `warps_upright` primary
//! `forward_warp_mls_arap_dlt` (level L2, basis [schaefer06, igarashi05,
//! sorkine07, hartley04], register key P-7312805 "Expired - Lifetime",
//! expiry 2024-04-06). ES §45.4 (displacement-grid contract the lattice
//! output feeds), §45.5 (puppet-warp session that consumes the
//! point-handle tier), §45.6 (perspective contract, adjacent), §18.4
//! (warp family), §19.2 (straighten adjacency). INTEGRATION-NOTES §6
//! order 9.
//!
//! SCOPE (item 9 = MLS only): the ARAP solver (igarashi05/sorkine07) is
//! the libigl LINKED dependency — INTEGRATION-NOTES §6 item 26, not this
//! module; the DLT homography (hartley04) lives in the 0.5 transform
//! stack. This module is the schaefer06 point-handle core that the
//! `transform.puppet` row's "MLS rigid" tier and the `warps_upright`
//! forward-warp path name.
//!
//! Method (paper §2): given handle pairs p_i → q_i and a query point v,
//! weights w_i = 1/‖p_i − v‖^(2α) (α = 1 default), weighted centroids
//! p*, q*, centered p̂_i = p_i − p*, q̂_i = q_i − q*:
//!
//! * **Affine** (paper §2.1): f(v) = (v − p*)·M + q* with
//!   M = (Σ w_i p̂_iᵀ p̂_i)⁻¹ (Σ w_i p̂_iᵀ q̂_i) — closed-form 2×2 solve.
//! * **Similarity** (paper §2.2): the μ_s-normalized form; collapsed
//!   here to f(v) = (v − p*)·S + q* with S = [[c1, −c2], [c2, c1]]/μ_s,
//!   c1 = Σ w_i q̂_i·p̂_i, c2 = Σ w_i (q̂_ix p̂_iy − q̂_iy p̂_ix),
//!   μ_s = Σ w_i ‖p̂_i‖² (algebraically identical to the paper's
//!   Σ q̂_i (1/μ_s A_i) expansion, exercised by the reproduction tests).
//! * **Rigid** (paper §2.3): the similarity form normalized to a pure
//!   rotation — the paper's closed form μ_r = ‖(c1, c2)‖ replacing μ_s,
//!   which is exactly the ‖v − p*‖-preserving scaling of the →f(v)
//!   direction (the paper's f_r(v) = ‖v − p*‖·→f(v)/‖→f(v)‖ + q*).
//!
//! Evaluation is displacement-form u(v) = f(v) − v so that handles at
//! rest yield u ≡ 0 bit-exactly (M, S, R all collapse to the identity
//! with exact arithmetic-order cancellation) — the mesh analog of the
//! §45.4/§45.5 identity properties.
//!
//! Degenerate cases (deterministic, documented):
//! * zero handles → identity map;
//! * one handle, or all handles coincident (μ_s = 0) → pure translation
//!   by q* − p*;
//! * collinear handles → the affine normal matrix is singular
//!   (det ≤ εᵣ·tr²); the affine variant falls back to the similarity
//!   form, which stays well-posed whenever any p̂_i ≠ 0;
//! * rigid with μ_r = 0 (pathological cancellation) → translation;
//! * query within 1e-6 px of a handle → snaps to that handle's exact
//!   image q_i + (v − p_i) (lowest handle index wins), realizing the
//!   paper's interpolation property f(p_i) = q_i without the weight
//!   singularity.
//!
//! **Backward mapping (honest note).** [`warp_plane`] resamples by
//! evaluating the deformation with SWAPPED handles (q_i → p_i) at each
//! output pixel — the standard approximation of the inverse map: exact
//! at handles and for handle sets whose forward map is globally affine /
//! similarity (reproduction property), approximate elsewhere (the true
//! inverse of an MLS forward map is not itself an MLS map). The
//! forward-warp splatting kernel pair (tiled Rust/WGSL) that the
//! `warps_upright` name ultimately describes is O-RENDER's later parity
//! item; this module is the deterministic CPU reference core it will be
//! held against. Sampling is bilinear with edge clamp (crate
//! `at_clamped` convention); the tools-side consumer supplies
//! alpha/padded planes to realize the §45.4/§45.5 outside-⇒-transparent
//! policy at the adapter seam.
//!
//! Determinism: single-threaded, fixed iteration order, f64 internal
//! accumulation, no RNG. The default α = 1.0 path uses only +,−,×,÷,√
//! (IEEE-exact operations — no libm transcendentals); non-default α
//! routes through `powf`, which is platform-libm-dependent, so pinned
//! goldens use the default α only. Pure function over planes: no
//! document access; consumers route mutations through the
//! `appname.command.v1` envelope spine (D-48).

use crate::plane::Plane;

/// Which schaefer06 closed-form variant to evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MlsVariant {
    /// Paper §2.1 — full affine least squares (may shear/scale).
    Affine,
    /// Paper §2.2 — similarity (rotation + uniform scale).
    Similarity,
    /// Paper §2.3 — rigid (rotation only; the ALGORITHMS.md §19 default).
    Rigid,
}

/// Handle pair: (source point p, deformed point q), pixel coordinates.
pub type MlsHandle = ([f32; 2], [f32; 2]);

/// Squared snap radius around a handle (1e-6 px) inside which the query
/// takes the handle's exact image — interpolation property without the
/// 1/d² weight singularity.
const SNAP_D2: f64 = 1e-12;

/// Relative singularity threshold for the affine normal matrix:
/// det ≤ EPS_DET·tr² ⇒ collinear handle set ⇒ similarity fallback.
const EPS_DET: f64 = 1e-12;

/// Exact-on-axis 2-vector length: avoids `hypot` (libm) and returns
/// `|x|` exactly when `y == 0` (and vice versa), which the bit-exact
/// rest-identity property relies on.
#[inline]
fn len2_exact(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        x.abs()
    } else if x == 0.0 {
        y.abs()
    } else {
        (x * x + y * y).sqrt()
    }
}

/// MLS weight for squared distance `d2 > 0`: w = 1/d²ᵅ (distance
/// exponent 2α per the paper). α = 1.0 short-circuits to plain division
/// so the default path never calls `powf` (determinism note above).
#[inline]
fn weight(d2: f64, alpha: f64) -> f64 {
    if alpha == 1.0 {
        1.0 / d2
    } else {
        1.0 / d2.powf(alpha)
    }
}

/// Core evaluation in f64: returns the displacement u(v) = f(v) − v.
fn displacement_f64(
    vx: f64,
    vy: f64,
    handles: &[MlsHandle],
    variant: MlsVariant,
    alpha: f64,
) -> (f64, f64) {
    let n = handles.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    // Snap branch: exact interpolation at (and vanishingly near) handles.
    for &(p, q) in handles {
        let dx = f64::from(p[0]) - vx;
        let dy = f64::from(p[1]) - vy;
        if dx * dx + dy * dy <= SNAP_D2 {
            return (f64::from(q[0]) - vx - dx, f64::from(q[1]) - vy - dy);
        }
    }
    if n == 1 {
        // Single handle: pure translation (exact, no centroid rounding).
        let (p, q) = handles[0];
        return (
            f64::from(q[0]) - f64::from(p[0]),
            f64::from(q[1]) - f64::from(p[1]),
        );
    }

    // Pass 1: weights and weighted centroids.
    let mut sw = 0.0f64;
    let mut spx = 0.0f64;
    let mut spy = 0.0f64;
    let mut sqx = 0.0f64;
    let mut sqy = 0.0f64;
    for &(p, q) in handles {
        let px = f64::from(p[0]);
        let py = f64::from(p[1]);
        let ddx = px - vx;
        let ddy = py - vy;
        let w = weight(ddx * ddx + ddy * ddy, alpha);
        sw += w;
        spx += w * px;
        spy += w * py;
        sqx += w * f64::from(q[0]);
        sqy += w * f64::from(q[1]);
    }
    let pcx = spx / sw;
    let pcy = spy / sw;
    let qcx = sqx / sw;
    let qcy = sqy / sw;
    let dx = vx - pcx;
    let dy = vy - pcy;
    let tx = qcx - pcx; // translation part q* − p*
    let ty = qcy - pcy;

    // Pass 2: centered second moments. (sxx, sxy, syy) is the affine
    // normal matrix Σ w p̂ᵀp̂; (bxx, bxy, byx, byy) is Σ w p̂ᵀq̂;
    // (mu_s, c1, c2) drive the similarity/rigid closed forms.
    let mut sxx = 0.0f64;
    let mut sxy = 0.0f64;
    let mut syy = 0.0f64;
    let mut bxx = 0.0f64;
    let mut bxy = 0.0f64;
    let mut byx = 0.0f64;
    let mut byy = 0.0f64;
    let mut mu_s = 0.0f64;
    let mut c1 = 0.0f64;
    let mut c2 = 0.0f64;
    for &(p, q) in handles {
        let px = f64::from(p[0]);
        let py = f64::from(p[1]);
        let ddx = px - vx;
        let ddy = py - vy;
        let w = weight(ddx * ddx + ddy * ddy, alpha);
        let hx = px - pcx;
        let hy = py - pcy;
        let gx = f64::from(q[0]) - qcx;
        let gy = f64::from(q[1]) - qcy;
        sxx += w * (hx * hx);
        sxy += w * (hx * hy);
        syy += w * (hy * hy);
        bxx += w * (hx * gx);
        bxy += w * (hx * gy);
        byx += w * (hy * gx);
        byy += w * (hy * gy);
        mu_s += w * (hx * hx + hy * hy);
        c1 += w * (gx * hx + gy * hy);
        c2 += w * (gx * hy - gy * hx);
    }

    match variant {
        MlsVariant::Affine => {
            let det = sxx * syy - sxy * sxy;
            let tr = sxx + syy;
            if det.abs() <= EPS_DET * tr * tr {
                // Collinear handles: singular normal matrix — documented
                // deterministic fallback to the similarity form.
                return similarity_like(dx, dy, tx, ty, c1, c2, mu_s);
            }
            // M = A⁻¹B, row-vector convention f = (v − p*)·M + q*.
            let m00 = (syy * bxx - sxy * byx) / det;
            let m01 = (syy * bxy - sxy * byy) / det;
            let m10 = (sxx * byx - sxy * bxx) / det;
            let m11 = (sxx * byy - sxy * bxy) / det;
            (
                tx + dx * (m00 - 1.0) + dy * m10,
                ty + dx * m01 + dy * (m11 - 1.0),
            )
        }
        MlsVariant::Similarity => similarity_like(dx, dy, tx, ty, c1, c2, mu_s),
        MlsVariant::Rigid => {
            let mu_r = len2_exact(c1, c2);
            similarity_like(dx, dy, tx, ty, c1, c2, mu_r)
        }
    }
}

/// Shared similarity-structure evaluation: displacement of
/// f(v) = (v − p*)·[[c1, −c2], [c2, c1]]/μ + q*. With μ = μ_s this is
/// the paper's similarity form; with μ = ‖(c1, c2)‖ the rigid form.
/// μ = 0 (all p̂ zero, or rigid cancellation) degrades to translation.
#[inline]
fn similarity_like(dx: f64, dy: f64, tx: f64, ty: f64, c1: f64, c2: f64, mu: f64) -> (f64, f64) {
    if mu == 0.0 {
        return (tx, ty);
    }
    let r00 = c1 / mu;
    let r01 = -c2 / mu;
    let r10 = c2 / mu;
    let r11 = c1 / mu;
    (
        tx + dx * (r00 - 1.0) + dy * r10,
        ty + dx * r01 + dy * (r11 - 1.0),
    )
}

/// Deform a single point: returns f(v) for the given variant.
///
/// α is the paper's weight falloff exponent (w = 1/‖p − v‖^2α), default
/// 1.0 in every consumer contract; larger α localizes handle influence.
pub fn mls_deform_point(
    v: [f32; 2],
    handles: &[MlsHandle],
    variant: MlsVariant,
    alpha: f32,
) -> [f32; 2] {
    let (ux, uy) = displacement_f64(
        f64::from(v[0]),
        f64::from(v[1]),
        handles,
        variant,
        f64::from(alpha),
    );
    [(f64::from(v[0]) + ux) as f32, (f64::from(v[1]) + uy) as f32]
}

/// Displacement lattice for the tiled consumer: node (i, j) holds
/// u(i·spacing, j·spacing) = f(node) − node, row-major. Spacing 1 is the
/// dense per-pixel reference; [`MlsGrid::sample`] interpolates
/// bilinearly between nodes (the §45.4 field-representation shape).
#[derive(Clone, Debug)]
pub struct MlsGrid {
    /// Node count along x (covers `[0, (nodes_x−1)·spacing] ⊇ [0, w−1]`).
    pub nodes_x: usize,
    /// Node count along y.
    pub nodes_y: usize,
    /// Node pitch in pixels (≥ 1).
    pub spacing: usize,
    /// Row-major x-displacements, `nodes_x · nodes_y` values.
    pub dx: Vec<f32>,
    /// Row-major y-displacements.
    pub dy: Vec<f32>,
}

impl MlsGrid {
    /// Bilinear displacement sample at pixel coordinates (x, y),
    /// clamped to the node extent. Exact at node positions (zero-weight
    /// terms vanish), so a spacing-1 grid reproduces the dense
    /// evaluation bit-exactly at integer pixels.
    pub fn sample(&self, x: f32, y: f32) -> [f32; 2] {
        let sp = self.spacing as f64;
        let gx = (f64::from(x) / sp).clamp(0.0, (self.nodes_x - 1) as f64);
        let gy = (f64::from(y) / sp).clamp(0.0, (self.nodes_y - 1) as f64);
        let x0 = gx.floor();
        let y0 = gy.floor();
        let fx = gx - x0;
        let fy = gy - y0;
        let i0 = x0 as usize;
        let j0 = y0 as usize;
        let i1 = (i0 + 1).min(self.nodes_x - 1);
        let j1 = (j0 + 1).min(self.nodes_y - 1);
        let idx = |i: usize, j: usize| j * self.nodes_x + i;
        let lerp2 = |v: &[f32]| -> f64 {
            let v00 = f64::from(v[idx(i0, j0)]);
            let v10 = f64::from(v[idx(i1, j0)]);
            let v01 = f64::from(v[idx(i0, j1)]);
            let v11 = f64::from(v[idx(i1, j1)]);
            v00 * (1.0 - fx) * (1.0 - fy)
                + v10 * fx * (1.0 - fy)
                + v01 * (1.0 - fx) * fy
                + v11 * fx * fy
        };
        [lerp2(&self.dx) as f32, lerp2(&self.dy) as f32]
    }
}

/// Evaluate the MLS displacement field on a coarse lattice covering a
/// `w × h` pixel domain. `spacing` ≥ 1; spacing 1 IS the dense
/// per-pixel reference evaluation.
pub fn mls_displacement_grid(
    w: usize,
    h: usize,
    spacing: usize,
    handles: &[MlsHandle],
    variant: MlsVariant,
    alpha: f32,
) -> MlsGrid {
    assert!(spacing >= 1, "grid spacing must be >= 1");
    assert!(w >= 1 && h >= 1, "grid domain must be non-empty");
    let nodes_x = (w - 1).div_ceil(spacing) + 1;
    let nodes_y = (h - 1).div_ceil(spacing) + 1;
    let mut dx = vec![0.0f32; nodes_x * nodes_y];
    let mut dy = vec![0.0f32; nodes_x * nodes_y];
    for j in 0..nodes_y {
        for i in 0..nodes_x {
            let (ux, uy) = displacement_f64(
                (i * spacing) as f64,
                (j * spacing) as f64,
                handles,
                variant,
                f64::from(alpha),
            );
            dx[j * nodes_x + i] = ux as f32;
            dy[j * nodes_x + i] = uy as f32;
        }
    }
    MlsGrid {
        nodes_x,
        nodes_y,
        spacing,
        dx,
        dy,
    }
}

/// Bilinear source sample with edge clamp, f64 arithmetic. Exact at
/// integer coordinates (zero-weight taps contribute exactly 0).
fn sample_bilinear_clamped(src: &Plane, x: f64, y: f64) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let xi = x0 as isize;
    let yi = y0 as isize;
    let s00 = f64::from(src.at_clamped(xi, yi));
    let s10 = f64::from(src.at_clamped(xi + 1, yi));
    let s01 = f64::from(src.at_clamped(xi, yi + 1));
    let s11 = f64::from(src.at_clamped(xi + 1, yi + 1));
    (s00 * (1.0 - fx) * (1.0 - fy) + s10 * fx * (1.0 - fy) + s01 * (1.0 - fx) * fy + s11 * fx * fy)
        as f32
}

/// Swap handle pairs (p → q becomes q → p): the backward-map
/// approximation of the inverse deformation (module doc, honest note).
fn swapped(handles: &[MlsHandle]) -> Vec<MlsHandle> {
    handles.iter().map(|&(p, q)| (q, p)).collect()
}

/// Warp one plane by the MLS deformation defined by FORWARD handle
/// pairs p_i → q_i: backward mapping (swapped-handle inverse
/// approximation) evaluated densely per output pixel + bilinear source
/// sampling with edge clamp.
pub fn warp_plane(src: &Plane, handles: &[MlsHandle], variant: MlsVariant, alpha: f32) -> Plane {
    let inv = swapped(handles);
    let mut out = Plane::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            let (ux, uy) = displacement_f64(x as f64, y as f64, &inv, variant, f64::from(alpha));
            *out.at_mut(x, y) = sample_bilinear_clamped(src, x as f64 + ux, y as f64 + uy);
        }
    }
    out
}

/// Lattice-path warp: the backward displacement field is evaluated on a
/// `spacing`-pitch grid and bilinearly interpolated per pixel — the
/// tiled consumer's cost shape (§45.4 field representation). Converges
/// to [`warp_plane`] as spacing → 1, but is NOT bit-exact even at
/// spacing 1: the grid stores displacements as f32 while the dense path
/// samples with unrounded f64 values, so outputs agree only to f32
/// rounding (measured max deviation 1.8e-7 ≈ 1–2 output ulps on the
/// fractional-handle fixture; hashes differ). Pinned by
/// `warp_determinism_at_fractional_handle_coordinates`.
pub fn warp_plane_lattice(
    src: &Plane,
    handles: &[MlsHandle],
    variant: MlsVariant,
    alpha: f32,
    spacing: usize,
) -> Plane {
    let inv = swapped(handles);
    let grid = mls_displacement_grid(src.w, src.h, spacing, &inv, variant, alpha);
    let mut out = Plane::new(src.w, src.h);
    for y in 0..src.h {
        for x in 0..src.w {
            let [ux, uy] = grid.sample(x as f32, y as f32);
            *out.at_mut(x, y) =
                sample_bilinear_clamped(src, x as f64 + f64::from(ux), y as f64 + f64::from(uy));
        }
    }
    out
}

/// Weighted centroids (p*, q*) at a query point — the paper's local
/// frame. Exposed for the rigid-property tests (‖f(v) − q*‖ = ‖v − p*‖).
#[cfg(test)]
pub(crate) fn centroids(v: [f32; 2], handles: &[MlsHandle], alpha: f32) -> ([f64; 2], [f64; 2]) {
    let vx = f64::from(v[0]);
    let vy = f64::from(v[1]);
    let mut sw = 0.0f64;
    let mut spx = 0.0f64;
    let mut spy = 0.0f64;
    let mut sqx = 0.0f64;
    let mut sqy = 0.0f64;
    for &(p, q) in handles {
        let px = f64::from(p[0]);
        let py = f64::from(p[1]);
        let ddx = px - vx;
        let ddy = py - vy;
        let w = weight(ddx * ddx + ddy * ddy, f64::from(alpha));
        sw += w;
        spx += w * px;
        spy += w * py;
        sqx += w * f64::from(q[0]);
        sqy += w * f64::from(q[1]);
    }
    ([spx / sw, spy / sw], [sqx / sw, sqy / sw])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    const ALL: [MlsVariant; 3] = [
        MlsVariant::Affine,
        MlsVariant::Similarity,
        MlsVariant::Rigid,
    ];

    /// Well-spread non-degenerate handle source set on a 64×64 domain.
    fn base_points() -> Vec<[f32; 2]> {
        vec![
            [10.0, 12.0],
            [52.0, 9.0],
            [55.0, 50.0],
            [14.0, 48.0],
            [33.0, 30.0],
        ]
    }

    fn fixture_plane(w: usize, h: usize, seed: u64) -> Plane {
        let mut rng = SplitMix64(seed);
        let mut p = Plane::new(w, h);
        for v in p.data.iter_mut() {
            *v = rng.next_f64() as f32;
        }
        p
    }

    /// The fixed deformation fixture behind the pinned golden hashes:
    /// 5 handles, center pushed right-down, one corner pulled.
    fn golden_handles() -> Vec<MlsHandle> {
        let p = base_points();
        vec![
            (p[0], [12.0, 10.0]),
            (p[1], p[1]),
            (p[2], [51.0, 54.0]),
            (p[3], p[3]),
            (p[4], [39.0, 34.5]),
        ]
    }

    fn apply_affine(a: [[f32; 2]; 2], t: [f32; 2], p: [f32; 2]) -> [f32; 2] {
        [
            a[0][0] * p[0] + a[0][1] * p[1] + t[0],
            a[1][0] * p[0] + a[1][1] * p[1] + t[1],
        ]
    }

    fn rot_scale(s: f32, th: f32) -> [[f32; 2]; 2] {
        [[s * th.cos(), -s * th.sin()], [s * th.sin(), s * th.cos()]]
    }

    fn query_points() -> Vec<[f32; 2]> {
        let mut qs = Vec::new();
        for j in 0..9 {
            for i in 0..9 {
                qs.push([i as f32 * 7.5 + 1.0, j as f32 * 7.5 + 1.5]);
            }
        }
        qs
    }

    #[test]
    fn exact_interpolation_at_handles() {
        let handles = golden_handles();
        for variant in ALL {
            for &(p, q) in &handles {
                let f = mls_deform_point(p, &handles, variant, 1.0);
                assert_eq!(f, q, "f(p_i) must equal q_i exactly ({variant:?})");
            }
        }
    }

    #[test]
    fn identity_when_handles_at_rest() {
        // q = p everywhere ⇒ f(v) = v BIT-exactly over the whole grid
        // (displacement-form evaluation, module-doc exactness argument).
        let handles: Vec<MlsHandle> = base_points().into_iter().map(|p| (p, p)).collect();
        for variant in ALL {
            for v in query_points() {
                let f = mls_deform_point(v, &handles, variant, 1.0);
                assert_eq!(
                    f, v,
                    "rest handles must be a bit-exact identity ({variant:?})"
                );
            }
        }
    }

    #[test]
    fn affine_variant_reproduces_affine_map() {
        // q_i = A·p_i + t (non-degenerate A) ⇒ the affine variant
        // reproduces A·v + t everywhere (paper §2.1 reproduction).
        let a = [[1.3, 0.4], [-0.2, 0.9]];
        let t = [5.0, -3.0];
        let handles: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, apply_affine(a, t, p)))
            .collect();
        for v in query_points() {
            let f = mls_deform_point(v, &handles, MlsVariant::Affine, 1.0);
            let e = apply_affine(a, t, v);
            assert!(
                (f[0] - e[0]).abs() < 1e-3 && (f[1] - e[1]).abs() < 1e-3,
                "affine reproduction failed at {v:?}: {f:?} vs {e:?}"
            );
        }
    }

    #[test]
    fn similarity_variant_reproduces_similarity_map() {
        // q_i = s·R·p_i + t ⇒ similarity variant reproduces it; the
        // rigid variant must NOT (s ≠ 1 is exactly what rigid forbids).
        let s = 1.7f32;
        let a = rot_scale(s, 0.6);
        let t = [-4.0, 8.0];
        let handles: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, apply_affine(a, t, p)))
            .collect();
        let mut rigid_max_dev = 0.0f32;
        for v in query_points() {
            let e = apply_affine(a, t, v);
            let fs = mls_deform_point(v, &handles, MlsVariant::Similarity, 1.0);
            assert!(
                (fs[0] - e[0]).abs() < 1e-3 && (fs[1] - e[1]).abs() < 1e-3,
                "similarity reproduction failed at {v:?}: {fs:?} vs {e:?}"
            );
            let fr = mls_deform_point(v, &handles, MlsVariant::Rigid, 1.0);
            rigid_max_dev = rigid_max_dev.max((fr[0] - e[0]).abs().max((fr[1] - e[1]).abs()));
        }
        assert!(
            rigid_max_dev > 1.0,
            "rigid must not reproduce a scaling similarity (max dev {rigid_max_dev})"
        );
    }

    #[test]
    fn rigid_variant_reproduces_rotation_translation() {
        // s = 1: rotation + translation is reproduced by BOTH the rigid
        // and similarity variants.
        let a = rot_scale(1.0, -0.8);
        let t = [3.0, 2.0];
        let handles: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, apply_affine(a, t, p)))
            .collect();
        for variant in [MlsVariant::Rigid, MlsVariant::Similarity] {
            for v in query_points() {
                let f = mls_deform_point(v, &handles, variant, 1.0);
                let e = apply_affine(a, t, v);
                assert!(
                    (f[0] - e[0]).abs() < 1e-3 && (f[1] - e[1]).abs() < 1e-3,
                    "rotation reproduction failed ({variant:?}) at {v:?}"
                );
            }
        }
    }

    #[test]
    fn rigid_preserves_distance_to_centroid_under_scaling_handles() {
        // Pure 2× scaling handle set: the paper's rigid property says
        // ‖f_r(v) − q*(v)‖ = ‖v − p*(v)‖ — no scale leaks through —
        // while the similarity variant scales by exactly 2.
        let c = [32.0f32, 32.0];
        let handles: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, [2.0 * (p[0] - c[0]) + c[0], 2.0 * (p[1] - c[1]) + c[1]]))
            .collect();
        for v in query_points() {
            // Skip queries inside the snap radius of a handle.
            if handles.iter().any(|&(p, _)| {
                let dx = p[0] - v[0];
                let dy = p[1] - v[1];
                (dx * dx + dy * dy) < 1e-3
            }) {
                continue;
            }
            let ([pcx, pcy], [qcx, qcy]) = centroids(v, &handles, 1.0);
            let din = ((f64::from(v[0]) - pcx).powi(2) + (f64::from(v[1]) - pcy).powi(2)).sqrt();
            let fr = mls_deform_point(v, &handles, MlsVariant::Rigid, 1.0);
            let dr = ((f64::from(fr[0]) - qcx).powi(2) + (f64::from(fr[1]) - qcy).powi(2)).sqrt();
            assert!(
                (dr - din).abs() < 1e-4 * din.max(1.0),
                "rigid scaled: |f−q*| = {dr} vs |v−p*| = {din} at {v:?}"
            );
            let fs = mls_deform_point(v, &handles, MlsVariant::Similarity, 1.0);
            let ds = ((f64::from(fs[0]) - qcx).powi(2) + (f64::from(fs[1]) - qcy).powi(2)).sqrt();
            assert!(
                (ds - 2.0 * din).abs() < 1e-3 * din.max(1.0),
                "similarity must scale by 2: {ds} vs {}",
                2.0 * din
            );
        }
    }

    /// Sum over sample points of the Jacobian's singular-value deviation
    /// from 1 — a quantitative local-scale-distortion metric (2×2 SVD in
    /// closed form, Jacobian by central differences).
    fn distortion_metric(handles: &[MlsHandle], variant: MlsVariant) -> f64 {
        let eps = 0.25f64;
        let eval = |x: f64, y: f64| -> (f64, f64) {
            let (ux, uy) = displacement_f64(x, y, handles, variant, 1.0);
            (x + ux, y + uy)
        };
        let mut total = 0.0f64;
        for v in query_points() {
            if handles.iter().any(|&(p, _)| {
                let dx = f64::from(p[0]) - f64::from(v[0]);
                let dy = f64::from(p[1]) - f64::from(v[1]);
                (dx * dx + dy * dy) < 4.0
            }) {
                continue;
            }
            let x = f64::from(v[0]);
            let y = f64::from(v[1]);
            let (fxp, fyp) = eval(x + eps, y);
            let (fxm, fym) = eval(x - eps, y);
            let (gxp, gyp) = eval(x, y + eps);
            let (gxm, gym) = eval(x, y - eps);
            let a = (fxp - fxm) / (2.0 * eps); // ∂f_x/∂x
            let c = (fyp - fym) / (2.0 * eps); // ∂f_y/∂x
            let b = (gxp - gxm) / (2.0 * eps); // ∂f_x/∂y
            let d = (gyp - gym) / (2.0 * eps); // ∂f_y/∂y
            let e = (a + d) / 2.0;
            let f = (a - d) / 2.0;
            let g = (c + b) / 2.0;
            let h = (c - b) / 2.0;
            let q = (e * e + h * h).sqrt();
            let r = (f * f + g * g).sqrt();
            let s1 = q + r;
            let s2 = (q - r).abs();
            total += (s1 - 1.0).abs() + (s2 - 1.0).abs();
        }
        total
    }

    #[test]
    fn rigid_distorts_less_than_affine_on_bend() {
        // Bend fixture: a horizontal chain of handles whose right arm
        // rotates 40° about the chain center — the paper's qualitative
        // "rigid looks most natural" claim made quantitative: the rigid
        // variant's summed local scale distortion must undercut the
        // affine variant's.
        let pivot = [32.0f32, 32.0];
        let th = 40.0f32.to_radians();
        let bend = |p: [f32; 2]| -> [f32; 2] {
            let dx = p[0] - pivot[0];
            let dy = p[1] - pivot[1];
            [
                pivot[0] + dx * th.cos() - dy * th.sin(),
                pivot[1] + dx * th.sin() + dy * th.cos(),
            ]
        };
        let handles: Vec<MlsHandle> = vec![
            ([8.0, 32.0], [8.0, 32.0]),
            ([20.0, 32.0], [20.0, 32.0]),
            (pivot, pivot),
            ([44.0, 32.0], bend([44.0, 32.0])),
            ([56.0, 32.0], bend([56.0, 32.0])),
        ];
        let da = distortion_metric(&handles, MlsVariant::Affine);
        let dr = distortion_metric(&handles, MlsVariant::Rigid);
        assert!(
            dr < da,
            "rigid distortion {dr} must undercut affine {da} on the bend fixture"
        );
    }

    #[test]
    fn collinear_handles_fall_back_to_similarity() {
        // All sources on one line: the affine normal matrix is singular;
        // the documented fallback makes Affine == Similarity exactly.
        let handles: Vec<MlsHandle> = vec![
            ([10.0, 30.0], [12.0, 28.0]),
            ([25.0, 30.0], [27.0, 29.0]),
            ([40.0, 30.0], [41.0, 33.0]),
            ([55.0, 30.0], [54.0, 36.0]),
        ];
        for v in query_points() {
            let fa = mls_deform_point(v, &handles, MlsVariant::Affine, 1.0);
            let fs = mls_deform_point(v, &handles, MlsVariant::Similarity, 1.0);
            assert!(fa[0].is_finite() && fa[1].is_finite());
            assert_eq!(fa, fs, "collinear fallback must equal similarity at {v:?}");
        }
    }

    #[test]
    fn degenerate_zero_one_and_coincident_handles() {
        let v = [17.0f32, 23.0];
        for variant in ALL {
            // Zero handles: identity.
            assert_eq!(mls_deform_point(v, &[], variant, 1.0), v);
            // One handle: exact translation by q − p.
            let one = [([10.0f32, 10.0], [13.0f32, 6.0])];
            assert_eq!(mls_deform_point(v, &one, variant, 1.0), [20.0, 19.0]);
            // All handles coincident: translation by q* − p*.
            let coin = [
                ([10.0f32, 10.0], [15.0f32, 11.0]),
                ([10.0f32, 10.0], [15.0f32, 11.0]),
            ];
            let f = mls_deform_point(v, &coin, variant, 1.0);
            assert!((f[0] - 22.0).abs() < 1e-4 && (f[1] - 24.0).abs() < 1e-4);
            // Query exactly at a handle: exact interpolation (snap).
            let handles = golden_handles();
            let (p0, q0) = handles[0];
            assert_eq!(mls_deform_point(p0, &handles, variant, 1.0), q0);
        }
    }

    #[test]
    fn lattice_converges_to_dense() {
        // Smooth deformation on 64×64: the spacing-1 GRID is this
        // test's reference — sampling the grid at its own integer nodes
        // returns the stored f32 values bit-exactly. (That is a
        // grid-internal property; the spacing-1 grid is NOT bit-exact
        // vs the f64 dense `warp_plane` path — see the
        // `warp_plane_lattice` doc and the fractional-handle test.)
        // Spacing-4 agrees within the stated bound; spacing-2 error ≤
        // spacing-4 error (convergence direction).
        let handles = golden_handles();
        let (w, h) = (64usize, 64usize);
        let dense = mls_displacement_grid(w, h, 1, &handles, MlsVariant::Rigid, 1.0);
        let g2 = mls_displacement_grid(w, h, 2, &handles, MlsVariant::Rigid, 1.0);
        let g4 = mls_displacement_grid(w, h, 4, &handles, MlsVariant::Rigid, 1.0);
        let mut max2 = 0.0f32;
        let mut max4 = 0.0f32;
        for y in 0..h {
            for x in 0..w {
                let d = dense.sample(x as f32, y as f32);
                let e = [
                    dense.dx[y * dense.nodes_x + x],
                    dense.dy[y * dense.nodes_x + x],
                ];
                assert_eq!(d, e, "spacing-1 sample must be bit-exact at nodes");
                let s2 = g2.sample(x as f32, y as f32);
                let s4 = g4.sample(x as f32, y as f32);
                max2 = max2.max((s2[0] - d[0]).abs()).max((s2[1] - d[1]).abs());
                max4 = max4.max((s4[0] - d[0]).abs()).max((s4[1] - d[1]).abs());
            }
        }
        // Stated bound: ≤ 0.35 px max displacement error at spacing 4 on
        // this fixture (empirical max 0.283 px, worst near the pulled
        // handles where the weight field bends; slack for platforms).
        assert!(max4 <= 0.35, "spacing-4 vs dense max err {max4} > 0.35 px");
        assert!(
            max2 <= max4 + 1e-6,
            "convergence direction violated: {max2} vs {max4}"
        );
    }

    #[test]
    fn identity_warp_is_bit_exact() {
        // Rest handles ⇒ zero displacement everywhere ⇒ sampling at
        // exact integer coordinates ⇒ the warp is a bit-exact no-op over
        // the WHOLE plane (edge rows included: no out-of-range sampling
        // ever happens, exceeding the required interior-only bound).
        let src = fixture_plane(48, 40, 0x4d4c_5301);
        let handles: Vec<MlsHandle> = base_points().into_iter().map(|p| (p, p)).collect();
        for variant in ALL {
            let out = warp_plane(&src, &handles, variant, 1.0);
            assert_eq!(
                plane_hash(&out.data),
                plane_hash(&src.data),
                "identity warp must be bit-exact ({variant:?})"
            );
            let out_l = warp_plane_lattice(&src, &handles, variant, 1.0, 4);
            assert_eq!(
                plane_hash(&out_l.data),
                plane_hash(&src.data),
                "identity lattice warp must be bit-exact ({variant:?})"
            );
        }
    }

    #[test]
    fn translation_warp_shifts_interior() {
        // All handles translated by (3, 0): backward mapping samples the
        // source at v − t, so out(x, y) == src(x − 3, y) in the interior
        // (f32-exact; the sub-1e-12 backward-map residual rounds away).
        let src = fixture_plane(48, 40, 0x4d4c_5302);
        let handles: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, [p[0] + 3.0, p[1]]))
            .collect();
        let out = warp_plane(&src, &handles, MlsVariant::Similarity, 1.0);
        for y in 0..src.h {
            for x in 3..src.w {
                assert_eq!(
                    out.at(x, y),
                    src.at(x - 3, y),
                    "translation warp mismatch at ({x}, {y})"
                );
            }
        }
        // Edge clamp bound: the first 3 columns replicate inside the
        // source range — every clamped output equals SOME source sample
        // from the same row's left edge region.
        for y in 0..src.h {
            for x in 0..3 {
                assert_eq!(out.at(x, y), src.at(0, y));
            }
        }
    }

    #[test]
    fn determinism_two_runs_and_pinned_goldens() {
        // Two-run bit-hash equality per variant, then pinned goldens on
        // the fixed fixture (pinned from this crate's own runs AFTER the
        // behavioral oracles above were green — rule 8).
        let src = fixture_plane(64, 64, 0x4d4c_5300);
        let handles = golden_handles();
        let pinned: [(MlsVariant, u64); 3] = [
            (MlsVariant::Affine, 15704756179384280945),
            (MlsVariant::Similarity, 1339749198877729119),
            (MlsVariant::Rigid, 392822906490084021),
        ];
        for (variant, want) in pinned {
            let a = warp_plane(&src, &handles, variant, 1.0);
            let b = warp_plane(&src, &handles, variant, 1.0);
            let ha = plane_hash(&a.data);
            assert_eq!(ha, plane_hash(&b.data), "two-run determinism ({variant:?})");
            assert_eq!(ha, want, "pinned golden mismatch ({variant:?})");
        }
        // Lattice path golden (rigid, spacing 4) — the tiled consumer's
        // cost shape must be equally deterministic.
        let l = warp_plane_lattice(&src, &handles, MlsVariant::Rigid, 1.0, 4);
        assert_eq!(
            plane_hash(&l.data),
            375294097184180816,
            "pinned lattice golden mismatch"
        );
    }

    #[test]
    fn handles_outside_domain_reproduce_translation() {
        // Handle sites far OUTSIDE the raster (negative and beyond-edge
        // coordinates) sharing one translation q = p + t: the
        // reproduction property is location-independent, so f(v) = v + t
        // everywhere for every variant, and the backward-mapped warp
        // shifts the interior f32-exactly (measured exact on the
        // fixture: the ~1e-12 backward-map residual rounds away in the
        // bilinear taps).
        let t = [3.0f32, -2.0];
        let sources: Vec<[f32; 2]> = vec![
            [-20.0, -15.0],
            [80.0, -10.0],
            [90.0, 70.0],
            [-25.0, 60.0],
            [33.0, 90.0],
        ];
        let handles: Vec<MlsHandle> = sources
            .iter()
            .map(|&p| (p, [p[0] + t[0], p[1] + t[1]]))
            .collect();
        for variant in ALL {
            for v in query_points() {
                let f = mls_deform_point(v, &handles, variant, 1.0);
                assert!(
                    (f[0] - (v[0] + t[0])).abs() < 1e-4 && (f[1] - (v[1] + t[1])).abs() < 1e-4,
                    "out-of-domain translation reproduction failed at {v:?} ({variant:?}): {f:?}"
                );
            }
        }
        let src = fixture_plane(32, 24, 0x4d4c_5303);
        let out = warp_plane(&src, &handles, MlsVariant::Rigid, 1.0);
        for y in 0..src.h - 2 {
            for x in 3..src.w {
                assert_eq!(
                    out.at(x, y),
                    src.at(x - 3, y + 2),
                    "interior shift mismatch at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn duplicated_handle_pairs_leave_the_map_unchanged() {
        // Listing every pair twice doubles every weight uniformly; all
        // normalized quantities are weight-scale invariant, so the map
        // must be unchanged (measured deviation on this fixture: 0.0 at
        // f32 — asserted at 1e-5 to claim the property, not the
        // incidental bit-coincidence).
        let handles = golden_handles();
        let mut doubled = Vec::with_capacity(handles.len() * 2);
        for &hd in &handles {
            doubled.push(hd);
            doubled.push(hd);
        }
        for variant in ALL {
            for v in query_points() {
                let a = mls_deform_point(v, &handles, variant, 1.0);
                let b = mls_deform_point(v, &doubled, variant, 1.0);
                assert!(
                    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5,
                    "duplicated handles changed the map at {v:?} ({variant:?})"
                );
            }
            // Interpolation survives duplication (snap: first match wins,
            // and both copies carry the SAME q).
            for &(p, q) in &doubled {
                assert_eq!(mls_deform_point(p, &doubled, variant, 1.0), q);
            }
        }
        // Coincident DUPLICATED handle (p_i == p_j, same q): queries just
        // outside the snap radius sit in the near-singular weight regime —
        // the map must stay finite and continuous into the handle image
        // (f(p + δ) = q + O(δ); the local frame is set by the far
        // handles, so the deviation is a small multiple of δ).
        let coin: Vec<MlsHandle> = vec![
            ([20.0, 20.0], [24.0, 18.0]),
            ([20.0, 20.0], [24.0, 18.0]),
            ([50.0, 22.0], [50.0, 22.0]),
            ([35.0, 48.0], [35.0, 48.0]),
        ];
        for variant in ALL {
            let f = mls_deform_point([20.001, 20.0], &coin, variant, 1.0);
            assert!(f[0].is_finite() && f[1].is_finite());
            let d = ((f[0] - 24.0).powi(2) + (f[1] - 18.0).powi(2)).sqrt();
            assert!(
                d < 1e-2,
                "coincident-pair continuity broke ({variant:?}): f = {f:?}, dist {d}"
            );
        }
    }

    #[test]
    fn alpha_two_powf_path_properties() {
        // α ≠ 1 routes through platform-libm `powf`, so per the module
        // determinism note it gets NO pinned golden. The oracles here
        // are libm-independent properties plus same-process two-run bit
        // equality.
        let handles = golden_handles();
        // Exact interpolation survives (the snap branch is weight-free).
        for variant in ALL {
            for &(p, q) in &handles {
                assert_eq!(mls_deform_point(p, &handles, variant, 2.0), q);
            }
        }
        // Translation reproduction is weight-independent — it must hold
        // for ANY α.
        let t_handles: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, [p[0] + 4.0, p[1] + 1.0]))
            .collect();
        for variant in ALL {
            for v in query_points() {
                let f = mls_deform_point(v, &t_handles, variant, 2.0);
                assert!(
                    (f[0] - (v[0] + 4.0)).abs() < 1e-3 && (f[1] - (v[1] + 1.0)).abs() < 1e-3,
                    "α = 2 translation reproduction failed at {v:?} ({variant:?})"
                );
            }
        }
        // α must be CONSEQUENTIAL on an overdetermined handle set (4
        // handles, one displaced): the α = 2 map measurably differs from
        // the α = 1 map at a near-handle query. (With ≤ 2 handles the
        // similarity fit — and with ≤ 3 the affine fit — is exactly
        // determined and weight-independent, which would mask the powf
        // path entirely; measured separations here are ≥ 2.6e-3.)
        let four: Vec<MlsHandle> = vec![
            ([10.0, 30.0], [15.0, 30.0]),
            ([50.0, 30.0], [45.0, 30.0]),
            ([30.0, 52.0], [30.0, 52.0]),
            ([8.0, 55.0], [8.0, 55.0]),
        ];
        let v = [10.4f32, 30.3];
        for variant in ALL {
            let f1 = mls_deform_point(v, &four, variant, 1.0);
            let f2 = mls_deform_point(v, &four, variant, 2.0);
            let sep = ((f1[0] - f2[0]).powi(2) + (f1[1] - f2[1]).powi(2)).sqrt();
            assert!(
                sep > 1e-3,
                "α = 2 map did not separate from α = 1 ({variant:?}): {sep}"
            );
        }
        // Near-handle continuity on the powf path: 1/d⁴ weights make the
        // displaced handle utterly dominant just outside the snap radius.
        for variant in ALL {
            let f = mls_deform_point([10.001, 30.0], &four, variant, 2.0);
            let d = ((f[0] - 15.0).powi(2) + ((f[1] - 30.0) - 0.0).powi(2)).sqrt();
            assert!(
                d < 1e-2,
                "α = 2 near-handle continuity broke ({variant:?}): {f:?}"
            );
        }
        // Two-run bit equality (same process ⇒ same libm).
        let src = fixture_plane(32, 32, 0x4d4c_5307);
        let a = warp_plane(&src, &handles, MlsVariant::Rigid, 2.0);
        let b = warp_plane(&src, &handles, MlsVariant::Rigid, 2.0);
        assert_eq!(
            plane_hash(&a.data),
            plane_hash(&b.data),
            "α = 2 two-run determinism"
        );
    }

    #[test]
    fn lattice_pitch_larger_than_image() {
        // Spacing beyond both dimensions still covers the domain with a
        // 2×2 node lattice; sampling clamps inside the node extent.
        let (w, h) = (10usize, 9usize);
        let handles = golden_handles();
        let g = mls_displacement_grid(w, h, 64, &handles, MlsVariant::Rigid, 1.0);
        assert_eq!(
            (g.nodes_x, g.nodes_y),
            (2, 2),
            "pathological-pitch node count"
        );
        for y in 0..h {
            for x in 0..w {
                let s = g.sample(x as f32, y as f32);
                assert!(s[0].is_finite() && s[1].is_finite());
            }
        }
        // Rest handles: the zero field interpolates to zero — the warp
        // is a bit-exact no-op even at pathological pitch.
        let src = fixture_plane(w, h, 0x4d4c_5304);
        let rest: Vec<MlsHandle> = base_points().into_iter().map(|p| (p, p)).collect();
        let out = warp_plane_lattice(&src, &rest, MlsVariant::Rigid, 1.0, 64);
        assert_eq!(
            plane_hash(&out.data),
            plane_hash(&src.data),
            "rest-handle warp at pitch 64 must be a bit-exact no-op"
        );
        // Translation handles: the displacement field is constant, so
        // even the coarsest lattice interpolates it (to lerp rounding)
        // and must match the dense warp.
        let tr: Vec<MlsHandle> = base_points()
            .into_iter()
            .map(|p| (p, [p[0] + 2.0, p[1] + 1.0]))
            .collect();
        let dense = warp_plane(&src, &tr, MlsVariant::Similarity, 1.0);
        let lat = warp_plane_lattice(&src, &tr, MlsVariant::Similarity, 1.0, 64);
        for i in 0..src.data.len() {
            assert!(
                (dense.data[i] - lat.data[i]).abs() < 1e-5,
                "constant-field lattice warp diverged from dense at {i}"
            );
        }
    }

    #[test]
    fn rigid_swapped_handle_roundtrip_bounds_identity_error() {
        // The swapped-handle "inverse" is an APPROXIMATION (module-doc
        // honest note): the true inverse of an MLS map is not itself an
        // MLS map, so f_swap ∘ f is only near-identity. Point-level
        // round trip on the golden fixture: measured max error
        // 1.6160 px over the 9×9 query grid (worst near the pulled
        // handles); asserted at 2.4 px with platform margin. At the
        // handles themselves the snap branch makes the round trip
        // EXACT. This documents the approximation's size — it is not an
        // exactness claim.
        let handles = golden_handles();
        let inv: Vec<MlsHandle> = handles.iter().map(|&(p, q)| (q, p)).collect();
        let mut max_err = 0.0f64;
        for v in query_points() {
            let f = mls_deform_point(v, &handles, MlsVariant::Rigid, 1.0);
            let b = mls_deform_point(f, &inv, MlsVariant::Rigid, 1.0);
            let dx = f64::from(b[0]) - f64::from(v[0]);
            let dy = f64::from(b[1]) - f64::from(v[1]);
            max_err = max_err.max((dx * dx + dy * dy).sqrt());
        }
        assert!(
            max_err < 2.4,
            "rigid swapped-handle round trip exceeded the recorded empirical bound: {max_err}"
        );
        for &(p, q) in &handles {
            let f = mls_deform_point(p, &handles, MlsVariant::Rigid, 1.0);
            assert_eq!(f, q);
            assert_eq!(
                mls_deform_point(f, &inv, MlsVariant::Rigid, 1.0),
                p,
                "round trip at a handle must be exact (snap)"
            );
        }
    }

    #[test]
    fn warp_determinism_at_fractional_handle_coordinates() {
        // Non-integer handle sites exercise the weight/centroid math off
        // the integer lattice: two-run bit equality for the dense and
        // lattice paths, and dense-vs-spacing-1 agreement pinned at the
        // observed f32-ulp level. The grid stores displacements as f32
        // while the dense path samples with the unrounded f64 values —
        // measured max deviation 1.8e-7 (1–2 output ulps), hashes
        // differ; `warp_plane_lattice`'s doc states this relationship
        // (the doc's former bit-exact-at-spacing-1 claim was corrected
        // in the wave-2 fix round; this test is its pin).
        let src = fixture_plane(40, 32, 0x4d4c_5305);
        let handles: Vec<MlsHandle> = vec![
            ([10.25, 12.75], [12.5, 10.125]),
            ([30.5, 8.25], [30.5, 8.25]),
            ([33.75, 27.5], [31.25, 29.0]),
            ([7.125, 24.375], [7.125, 24.375]),
        ];
        for variant in ALL {
            let a = warp_plane(&src, &handles, variant, 1.0);
            let b = warp_plane(&src, &handles, variant, 1.0);
            assert_eq!(
                plane_hash(&a.data),
                plane_hash(&b.data),
                "dense two-run determinism ({variant:?})"
            );
            let l1 = warp_plane_lattice(&src, &handles, variant, 1.0, 1);
            let l2 = warp_plane_lattice(&src, &handles, variant, 1.0, 1);
            assert_eq!(
                plane_hash(&l1.data),
                plane_hash(&l2.data),
                "lattice two-run determinism ({variant:?})"
            );
            for i in 0..src.data.len() {
                assert!(
                    (a.data[i] - l1.data[i]).abs() <= 3e-7,
                    "spacing-1 lattice deviated beyond f32 rounding at {i} ({variant:?})"
                );
            }
        }
    }

    #[test]
    fn nan_handle_pinned_current_behavior() {
        // Documents current behavior, not a contract. A NaN handle
        // coordinate defeats the snap test (NaN ≤ r² is false) and
        // poisons the weight/centroid sums: the displacement — and any
        // warp through it — is NaN, and replays are bit-identical (the
        // same operations produce the same NaN payloads). The 8×8 plane
        // keeps every pixel outside the swapped-handle snap sites, so
        // the whole output is NaN.
        let mut handles = golden_handles();
        handles[2].0[0] = f32::NAN;
        let f = mls_deform_point([5.0, 6.0], &handles, MlsVariant::Rigid, 1.0);
        assert!(f[0].is_nan() && f[1].is_nan(), "expected NaN propagation");
        let src = fixture_plane(8, 8, 0x4d4c_5306);
        let a = warp_plane(&src, &handles, MlsVariant::Rigid, 1.0);
        let b = warp_plane(&src, &handles, MlsVariant::Rigid, 1.0);
        assert!(
            a.data.iter().all(|v| v.is_nan()),
            "NaN did not propagate to the warp"
        );
        assert_eq!(
            plane_hash(&a.data),
            plane_hash(&b.data),
            "NaN replay determinism"
        );
    }
}
