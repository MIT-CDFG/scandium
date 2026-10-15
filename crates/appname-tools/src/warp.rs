// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §18.4 (grid warp [NORMATIVE]: bicubic Bézier patches, Catmull-
//! Rom→Bézier control derivation, commit = per-pixel inverse map by Newton
//! iteration seeded from the bilinear-cell inverse, then §18.2 bicubic
//! sampling; preset displacement table), §18.1 (`WarpGrid` model); contract
//! `transform.warp.yaml` (wire mesh = the FULL `(3·cols+1)×(3·rows+1)`
//! Bézier control lattice, row-major; preset kinds + `bendPct` −100..=100).
//! DD ㉓. [LEGAL, §18.4] "The shapes are generic geometry; the displacement
//! functions are original" — spec-owned math, no external basis needed.
//!
//! v1 pins (frozen):
//! * Mesh domain: the undeformed lattice spans the source rect uniformly —
//!   patch-space `(s, t) ∈ [0, cols] × [0, rows]` maps linearly onto the
//!   source rect. The wire mesh carries deformed DESTINATION control
//!   positions.
//! * Identity short-circuit: a mesh equal to the undeformed base lattice
//!   copies pixels verbatim (§18.3 bit-identity discipline).
//! * Newton: exactly **3** iterations (the §18.4 "2–3 steps" upper bound,
//!   fixed for determinism — no convergence-dependent exit), seeded from
//!   the inverse **bilinear** map of the cell's corner quad; a candidate
//!   cell is any whose control-point hull (Bézier convex-hull property)
//!   contains the pixel, tried in row-major order; accepted when the local
//!   coords land in `[−0.001, 1.001]` and the residual is `< 0.1 px`;
//!   no acceptance → transparent. All pinned; changing any changes goldens.
//! * Sampling: §18.2 **bicubic (Catmull-Rom)** per §18.4's commit sentence;
//!   out-of-source reads transparent (§4.3).
//! * Presets: the §18.4 table (`arc`, `bulge`, `flag`, `wave`, `fisheye`,
//!   `rise`) over the 4×4 GRID points (cells = 3×3), `bend = bendPct/100`,
//!   then Catmull-Rom→Bézier. Wire kind `fish` binds to the `fisheye`
//!   formula (naming fork — logged). The five wire kinds with no §18.4
//!   math (`arc_lower`, `arc_upper`, `shell`, `squeeze`, `twist`) reject as
//!   unsupported in v1 (spec gap — logged; never-invent rule).

use appname_core::IRect;

use crate::resample::{EdgePolicy, Kernel, Plane};

/// A validated §18.1/§10.3 warp mesh: `(3·cols+1)×(3·rows+1)` Bézier
/// control positions, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpMesh {
    pub cols: u32,
    pub rows: u32,
    pub points: Vec<[f64; 2]>,
}

/// Mesh/preset validation and evaluation failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpError {
    /// cols/rows outside 1..=16 (contract).
    GridSizeOutOfRange,
    /// `points.len() != (3·cols+1)·(3·rows+1)` (contract cross-field).
    PointCountMismatch { expected: usize, actual: usize },
    /// A control coordinate is NaN/infinite.
    PointNotFinite { index: usize },
    /// `bendPct` outside −100..=100 (contract).
    BendOutOfRange,
    /// A wire preset kind with no §18.4 displacement math (v1 gap).
    PresetUnsupported,
}

impl core::fmt::Display for WarpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WarpError::GridSizeOutOfRange => write!(f, "mesh cols/rows outside 1..=16"),
            WarpError::PointCountMismatch { expected, actual } => {
                write!(
                    f,
                    "mesh point count {actual} != (3*cols+1)*(3*rows+1) = {expected}"
                )
            }
            WarpError::PointNotFinite { index } => write!(f, "mesh point {index} not finite"),
            WarpError::BendOutOfRange => write!(f, "bendPct outside -100..=100"),
            WarpError::PresetUnsupported => {
                write!(f, "preset kind has no §18.4 displacement math in v1 (arc_lower/arc_upper/shell/squeeze/twist)")
            }
        }
    }
}

impl std::error::Error for WarpError {}

impl WarpMesh {
    pub fn lattice_dims(cols: u32, rows: u32) -> (usize, usize) {
        ((3 * cols + 1) as usize, (3 * rows + 1) as usize)
    }

    /// Validate the contract invariants.
    pub fn new(cols: u32, rows: u32, points: Vec<[f64; 2]>) -> Result<WarpMesh, WarpError> {
        if !(1..=16).contains(&cols) || !(1..=16).contains(&rows) {
            return Err(WarpError::GridSizeOutOfRange);
        }
        let (nx, ny) = Self::lattice_dims(cols, rows);
        if points.len() != nx * ny {
            return Err(WarpError::PointCountMismatch {
                expected: nx * ny,
                actual: points.len(),
            });
        }
        for (i, p) in points.iter().enumerate() {
            if !p[0].is_finite() || !p[1].is_finite() {
                return Err(WarpError::PointNotFinite { index: i });
            }
        }
        Ok(WarpMesh { cols, rows, points })
    }

    /// The undeformed base lattice over `rect` (uniform Bézier controls of
    /// the identity mapping).
    pub fn base_lattice(cols: u32, rows: u32, rect: IRect) -> WarpMesh {
        let (nx, ny) = Self::lattice_dims(cols, rows);
        let (w, h) = (rect.width() as f64, rect.height() as f64);
        let mut points = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                points.push([
                    rect.min.x as f64 + w * i as f64 / (nx - 1) as f64,
                    rect.min.y as f64 + h * j as f64 / (ny - 1) as f64,
                ]);
            }
        }
        WarpMesh { cols, rows, points }
    }

    #[inline]
    fn ctrl(&self, i: usize, j: usize) -> [f64; 2] {
        let (nx, _) = Self::lattice_dims(self.cols, self.rows);
        self.points[j * nx + i]
    }

    /// Forward patch evaluation at `(s, t)` ∈ [0, cols]×[0, rows]:
    /// tensor-product cubic Bézier of the cell's 4×4 control window.
    pub fn eval(&self, s: f64, t: f64) -> [f64; 2] {
        let (ci, u) = split_cell(s, self.cols);
        let (cj, v) = split_cell(t, self.rows);
        let bu = bernstein(u);
        let bv = bernstein(v);
        let mut out = [0.0f64; 2];
        for (j, &bvj) in bv.iter().enumerate() {
            for (i, &bui) in bu.iter().enumerate() {
                let p = self.ctrl(3 * ci + i, 3 * cj + j);
                let w = bui * bvj;
                out[0] += p[0] * w;
                out[1] += p[1] * w;
            }
        }
        out
    }

    /// Partial derivatives (∂F/∂u, ∂F/∂v) in CELL-LOCAL coords at `(s, t)`.
    fn jacobian(&self, s: f64, t: f64) -> [[f64; 2]; 2] {
        let (ci, u) = split_cell(s, self.cols);
        let (cj, v) = split_cell(t, self.rows);
        let bu = bernstein(u);
        let bv = bernstein(v);
        let du = bernstein_d(u);
        let dv = bernstein_d(v);
        let mut fu = [0.0f64; 2];
        let mut fv = [0.0f64; 2];
        for j in 0..4 {
            for i in 0..4 {
                let p = self.ctrl(3 * ci + i, 3 * cj + j);
                for c in 0..2 {
                    fu[c] += p[c] * du[i] * bv[j];
                    fv[c] += p[c] * bu[i] * dv[j];
                }
            }
        }
        [fu, fv]
    }
}

#[inline]
fn split_cell(s: f64, n: u32) -> (usize, f64) {
    let c = (s.floor() as i64).clamp(0, n as i64 - 1) as usize;
    (c, s - c as f64)
}

#[inline]
fn bernstein(u: f64) -> [f64; 4] {
    let iu = 1.0 - u;
    [iu * iu * iu, 3.0 * iu * iu * u, 3.0 * iu * u * u, u * u * u]
}

#[inline]
fn bernstein_d(u: f64) -> [f64; 4] {
    let iu = 1.0 - u;
    [
        -3.0 * iu * iu,
        3.0 * iu * iu - 6.0 * iu * u,
        6.0 * iu * u - 3.0 * u * u,
        3.0 * u * u,
    ]
}

/// §18.4 Catmull-Rom→Bézier: build the Bézier control lattice from
/// `(cols+1)×(rows+1)` GRID points — per axis, tangent
/// `T_i = (P_{i+1} − P_{i−1})/2` (one-sided at edges), inner controls
/// `P_i ± T_i/3`. Tensor-product: rows first, then columns.
pub fn mesh_from_grid(cols: u32, rows: u32, grid: &[[f64; 2]]) -> Result<WarpMesh, WarpError> {
    if !(1..=16).contains(&cols) || !(1..=16).contains(&rows) {
        return Err(WarpError::GridSizeOutOfRange);
    }
    let (gx, gy) = ((cols + 1) as usize, (rows + 1) as usize);
    assert_eq!(grid.len(), gx * gy, "grid point count");
    // Expand each grid row to a Bézier control row (1-D CR→Bézier).
    let row_ctrl: Vec<Vec<[f64; 2]>> = (0..gy)
        .map(|j| cr_to_bezier_1d(&(0..gx).map(|i| grid[j * gx + i]).collect::<Vec<_>>()))
        .collect();
    // Expand columns of the intermediate lattice.
    let nx = 3 * cols as usize + 1;
    let ny = 3 * rows as usize + 1;
    let mut points = vec![[0.0f64; 2]; nx * ny];
    for x in 0..nx {
        let col: Vec<[f64; 2]> = (0..gy).map(|j| row_ctrl[j][x]).collect();
        let expanded = cr_to_bezier_1d(&col);
        for (y, p) in expanded.iter().enumerate() {
            points[y * nx + x] = *p;
        }
    }
    WarpMesh::new(cols, rows, points)
}

/// 1-D Catmull-Rom→Bézier control expansion of `n` knots into `3(n−1)+1`
/// controls (§18.4 tangent rule).
fn cr_to_bezier_1d(p: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = p.len();
    debug_assert!(n >= 2);
    let tangent = |i: usize| -> [f64; 2] {
        if i == 0 {
            [p[1][0] - p[0][0], p[1][1] - p[0][1]] // one-sided
        } else if i == n - 1 {
            [p[n - 1][0] - p[n - 2][0], p[n - 1][1] - p[n - 2][1]]
        } else {
            [
                (p[i + 1][0] - p[i - 1][0]) / 2.0,
                (p[i + 1][1] - p[i - 1][1]) / 2.0,
            ]
        }
    };
    let mut out = Vec::with_capacity(3 * (n - 1) + 1);
    for i in 0..n - 1 {
        let t0 = tangent(i);
        let t1 = tangent(i + 1);
        out.push(p[i]);
        out.push([p[i][0] + t0[0] / 3.0, p[i][1] + t0[1] / 3.0]);
        out.push([p[i + 1][0] - t1[0] / 3.0, p[i + 1][1] - t1[1] / 3.0]);
    }
    out.push(p[n - 1]);
    out
}

/// The §18.4 preset displacement table over the 4×4 grid (cells 3×3);
/// `bend ∈ [−1, 1]`. Wire `fish` = the `fisheye` row (pin). Returns the
/// deformed Bézier mesh over `rect`.
pub fn preset_mesh(kind: &str, bend_pct: f64, rect: IRect) -> Result<WarpMesh, WarpError> {
    if !(-100.0..=100.0).contains(&bend_pct) || !bend_pct.is_finite() {
        return Err(WarpError::BendOutOfRange);
    }
    let bend = bend_pct / 100.0;
    let (w, h) = (rect.width() as f64, rect.height() as f64);
    let c = [rect.min.x as f64 + w / 2.0, rect.min.y as f64 + h / 2.0];
    let r = w.min(h) / 2.0;
    let (gx, gy) = (4usize, 4usize);
    let mut grid = Vec::with_capacity(gx * gy);
    for j in 0..gy {
        for i in 0..gx {
            let u = i as f64 / (gx - 1) as f64;
            let v = j as f64 / (gy - 1) as f64;
            let p = [rect.min.x as f64 + w * u, rect.min.y as f64 + h * v];
            let mut d = [0.0f64; 2];
            match kind {
                "arc" => d[1] = -bend * 0.25 * h * 4.0 * u * (1.0 - u),
                "bulge" => d[1] = bend * 0.25 * h * 4.0 * u * (1.0 - u) * (2.0 * v - 1.0),
                "flag" => d[1] = bend * 0.25 * h * (core::f64::consts::TAU * u).sin(),
                "wave" => {
                    d[1] = bend
                        * 0.25
                        * h
                        * (core::f64::consts::TAU * u).sin()
                        * (core::f64::consts::PI * v).cos()
                }
                "fish" | "fisheye" => {
                    let mut q = [(p[0] - c[0]) / r, (p[1] - c[1]) / r];
                    let len = (q[0] * q[0] + q[1] * q[1]).sqrt();
                    if len > 1.0 {
                        q = [q[0] / len, q[1] / len];
                    }
                    let ql = len.min(1.0);
                    d = [
                        bend * 0.5 * r * q[0] * (1.0 - ql),
                        bend * 0.5 * r * q[1] * (1.0 - ql),
                    ];
                }
                "rise" => d[1] = -bend * 0.25 * h * u * u,
                "arc_lower" | "arc_upper" | "shell" | "squeeze" | "twist" => {
                    return Err(WarpError::PresetUnsupported)
                }
                _ => return Err(WarpError::PresetUnsupported),
            }
            grid.push([p[0] + d[0], p[1] + d[1]]);
        }
    }
    mesh_from_grid(3, 3, &grid)
}

/// Inverse bilinear seed: local `(u, v)` of `p` in the quad
/// `(q00, q10, q01, q11)` (cell corners), or `None` if degenerate.
fn inverse_bilinear(
    p: [f64; 2],
    q00: [f64; 2],
    q10: [f64; 2],
    q01: [f64; 2],
    q11: [f64; 2],
) -> Option<[f64; 2]> {
    // Solve p = (1-u)(1-v)q00 + u(1-v)q10 + (1-u)v q01 + uv q11 by Newton
    // (2 fixed steps from (0.5, 0.5)) — deterministic and robust for the
    // mildly-deformed quads a 16×16-max grid produces.
    let mut u = 0.5;
    let mut v = 0.5;
    for _ in 0..2 {
        let iu = 1.0 - u;
        let iv = 1.0 - v;
        let f = [
            iu * iv * q00[0] + u * iv * q10[0] + iu * v * q01[0] + u * v * q11[0] - p[0],
            iu * iv * q00[1] + u * iv * q10[1] + iu * v * q01[1] + u * v * q11[1] - p[1],
        ];
        let fu = [
            iv * (q10[0] - q00[0]) + v * (q11[0] - q01[0]),
            iv * (q10[1] - q00[1]) + v * (q11[1] - q01[1]),
        ];
        let fv = [
            iu * (q01[0] - q00[0]) + u * (q11[0] - q10[0]),
            iu * (q01[1] - q00[1]) + u * (q11[1] - q10[1]),
        ];
        let det = fu[0] * fv[1] - fu[1] * fv[0];
        if det.abs() < 1e-12 {
            return None;
        }
        u -= (f[0] * fv[1] - f[1] * fv[0]) / det;
        v -= (fu[0] * f[1] - fu[1] * f[0]) / det;
    }
    if u.is_finite() && v.is_finite() {
        Some([u, v])
    } else {
        None
    }
}

/// Commit-path warp of a source plane through the deformed mesh into
/// `dst_rect` (§18.4 commit: per-pixel Newton inverse + §18.2 bicubic).
/// The mesh's undeformed domain is `src.rect` (module pin).
pub fn warp_plane(src: &Plane, mesh: &WarpMesh, dst_rect: IRect) -> Plane {
    // Identity short-circuit (module pin): the base lattice reproduces
    // only up to f64 rounding through the CR→Bézier path, so detection
    // uses a pinned 1e-9 px per-coordinate tolerance.
    let base = WarpMesh::base_lattice(mesh.cols, mesh.rows, src.rect);
    let is_identity = mesh
        .points
        .iter()
        .zip(base.points.iter())
        .all(|(a, b)| (a[0] - b[0]).abs() <= 1e-9 && (a[1] - b[1]).abs() <= 1e-9);
    if is_identity {
        let mut out = Plane::new(dst_rect);
        for y in dst_rect.min.y..dst_rect.max.y {
            for x in dst_rect.min.x..dst_rect.max.x {
                out.set(x, y, src.get(x, y, EdgePolicy::Transparent));
            }
        }
        return out;
    }

    let (cols, rows) = (mesh.cols as usize, mesh.rows as usize);
    // Per-cell control hulls (Bézier convex-hull property) as bboxes.
    let mut hulls = Vec::with_capacity(cols * rows);
    for cj in 0..rows {
        for ci in 0..cols {
            let (mut lx, mut ly) = (f64::INFINITY, f64::INFINITY);
            let (mut hx, mut hy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for j in 0..4 {
                for i in 0..4 {
                    let p = mesh.ctrl(3 * ci + i, 3 * cj + j);
                    lx = lx.min(p[0]);
                    ly = ly.min(p[1]);
                    hx = hx.max(p[0]);
                    hy = hy.max(p[1]);
                }
            }
            hulls.push([lx, ly, hx, hy]);
        }
    }

    let sw = src.rect.width() as f64;
    let sh = src.rect.height() as f64;
    let kernel = Kernel::CatmullRom; // §18.4 commit sampling
    let support = kernel.support();
    let mut out = Plane::new(dst_rect);
    for y in dst_rect.min.y..dst_rect.max.y {
        for x in dst_rect.min.x..dst_rect.max.x {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            let mut hit: Option<[f64; 2]> = None;
            'cells: for cj in 0..rows {
                for ci in 0..cols {
                    let hull = &hulls[cj * cols + ci];
                    if p[0] < hull[0] || p[0] > hull[2] || p[1] < hull[1] || p[1] > hull[3] {
                        continue;
                    }
                    // Seed: inverse bilinear of the cell's corner quad.
                    let q00 = mesh.ctrl(3 * ci, 3 * cj);
                    let q10 = mesh.ctrl(3 * ci + 3, 3 * cj);
                    let q01 = mesh.ctrl(3 * ci, 3 * cj + 3);
                    let q11 = mesh.ctrl(3 * ci + 3, 3 * cj + 3);
                    let Some(seed) = inverse_bilinear(p, q00, q10, q01, q11) else {
                        continue;
                    };
                    let mut s = ci as f64 + seed[0].clamp(-0.25, 1.25);
                    let mut t = cj as f64 + seed[1].clamp(-0.25, 1.25);
                    // §18.4 Newton, pinned at exactly 3 iterations.
                    for _ in 0..3 {
                        let f = mesh.eval(s, t);
                        let jac = mesh.jacobian(s, t);
                        let (rx, ry) = (f[0] - p[0], f[1] - p[1]);
                        let det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0];
                        if det.abs() < 1e-12 {
                            break;
                        }
                        s -= (rx * jac[1][1] - ry * jac[1][0]) / det;
                        t -= (jac[0][0] * ry - jac[0][1] * rx) / det;
                    }
                    let (lu, lv) = (s - ci as f64, t - cj as f64);
                    if (-0.001..=1.001).contains(&lu) && (-0.001..=1.001).contains(&lv) {
                        let f = mesh.eval(s, t);
                        let res = ((f[0] - p[0]).powi(2) + (f[1] - p[1]).powi(2)).sqrt();
                        if res < 0.1 {
                            hit = Some([s, t]);
                            break 'cells;
                        }
                    }
                }
            }
            let Some([s, t]) = hit else { continue };
            // (s, t) → undeformed source position, then §18.2 CR sampling.
            let sxc = src.rect.min.x as f64 + sw * s / cols as f64;
            let syc = src.rect.min.y as f64 + sh * t / rows as f64;
            let (sx, sy) = (sxc - 0.5, syc - 0.5);
            let (lo_x, hi_x) = ((sx - support).ceil() as i64, (sx + support).floor() as i64);
            let (lo_y, hi_y) = ((sy - support).ceil() as i64, (sy + support).floor() as i64);
            let mut acc = [0.0f64; 4];
            let mut wsum = 0.0f64;
            for ky in lo_y..=hi_y {
                let wy = kernel.weight(ky as f64 - sy);
                for kx in lo_x..=hi_x {
                    let w = kernel.weight(kx as f64 - sx) * wy;
                    if w == 0.0 {
                        continue;
                    }
                    let v = src.get(kx as i32, ky as i32, EdgePolicy::Transparent);
                    for c in 0..4 {
                        acc[c] += v[c] as f64 * w;
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
    out
}

/// Union bbox of the deformed mesh (the contract's "warped mesh bounds"
/// dirty basis), with 1 px kernel slack.
pub fn mesh_bounds(mesh: &WarpMesh) -> IRect {
    let (mut lx, mut ly) = (f64::INFINITY, f64::INFINITY);
    let (mut hx, mut hy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &mesh.points {
        lx = lx.min(p[0]);
        ly = ly.min(p[1]);
        hx = hx.max(p[0]);
        hy = hy.max(p[1]);
    }
    if !(lx.is_finite() && ly.is_finite()) {
        return IRect::EMPTY;
    }
    IRect::new(
        lx.floor() as i32 - 1,
        ly.floor() as i32 - 1,
        hx.ceil() as i32 + 1,
        hy.ceil() as i32 + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_matches_the_contract() {
        assert_eq!(
            WarpMesh::new(0, 3, vec![]).unwrap_err(),
            WarpError::GridSizeOutOfRange
        );
        assert_eq!(
            WarpMesh::new(17, 3, vec![]).unwrap_err(),
            WarpError::GridSizeOutOfRange
        );
        // cols=rows=2 needs (7)² = 49 points; 16 is the contract's own
        // invalid example.
        let err = WarpMesh::new(2, 2, vec![[0.0, 0.0]; 16]).unwrap_err();
        assert_eq!(
            err,
            WarpError::PointCountMismatch {
                expected: 49,
                actual: 16
            }
        );
        let mut pts = vec![[0.0f64, 0.0]; 16];
        pts[3] = [f64::NAN, 0.0];
        assert_eq!(
            WarpMesh::new(1, 1, pts).unwrap_err(),
            WarpError::PointNotFinite { index: 3 }
        );
        assert!(WarpMesh::new(1, 1, vec![[0.0, 0.0]; 16]).is_ok());
    }

    #[test]
    fn base_lattice_is_the_identity_map() {
        let rect = IRect::new(0, 0, 90, 60);
        let mesh = WarpMesh::base_lattice(3, 3, rect);
        for (s, t) in [(0.0, 0.0), (1.5, 0.75), (3.0, 3.0), (0.33, 2.4)] {
            let p = mesh.eval(s, t);
            assert!(
                (p[0] - (rect.min.x as f64 + 90.0 * s / 3.0)).abs() < 1e-9,
                "s={s}"
            );
            assert!(
                (p[1] - (rect.min.y as f64 + 60.0 * t / 3.0)).abs() < 1e-9,
                "t={t}"
            );
        }
    }

    #[test]
    fn cr_to_bezier_reproduces_the_tangent_rule() {
        // Knots on a parabola-ish polyline; §18.4: inner controls at
        // P_i ± T_i/3 with T_i = (P_{i+1} − P_{i−1})/2, one-sided ends.
        let p = [[0.0, 0.0], [10.0, 5.0], [20.0, 0.0]];
        let ctrl = cr_to_bezier_1d(&p);
        assert_eq!(ctrl.len(), 7);
        assert_eq!(ctrl[0], [0.0, 0.0]);
        // One-sided start tangent = P1 − P0 = (10, 5) → ctrl1 = P0 + T/3.
        assert!((ctrl[1][0] - 10.0 / 3.0).abs() < 1e-12 && (ctrl[1][1] - 5.0 / 3.0).abs() < 1e-12);
        // Interior tangent at P1 = (P2 − P0)/2 = (10, 0) → ctrl2 = P1 − T/3.
        assert!(
            (ctrl[2][0] - (10.0 - 10.0 / 3.0)).abs() < 1e-12 && (ctrl[2][1] - 5.0).abs() < 1e-12
        );
        assert_eq!(ctrl[3], [10.0, 5.0]);
        assert_eq!(ctrl[6], [20.0, 0.0]);
    }

    #[test]
    fn preset_displacements_match_the_18_4_table() {
        let rect = IRect::new(0, 0, 100, 80);
        // arc, bend 1: grid point at u=1/3 gets d.y = −0.25·80·4·(1/3)(2/3).
        let mesh = preset_mesh("arc", 100.0, rect).unwrap();
        // Grid point (i=1, j=0) is Bézier control (3, 0) (grid points are
        // every third control).
        let p = mesh.ctrl(3, 0);
        let want_dy = -0.25 * 80.0 * 4.0 * (1.0 / 3.0) * (2.0 / 3.0);
        assert!((p[1] - want_dy).abs() < 1e-9, "arc: {} vs {want_dy}", p[1]);
        assert!((p[0] - 100.0 / 3.0).abs() < 1e-9, "x undisplaced");
        // rise, bend −0.5 at u=1: d.y = −(−0.5)·0.25·80·1 = +10.
        let mesh = preset_mesh("rise", -50.0, rect).unwrap();
        let p = mesh.ctrl(9, 0);
        assert!((p[1] - 10.0).abs() < 1e-9, "rise end: {}", p[1]);
        // bulge at v=0 (2v−1 = −1): d.y = +0.25·H·4u(1−u)·(−1) — the same
        // value as arc's top row at bend 1.
        let mesh = preset_mesh("bulge", 100.0, rect).unwrap();
        let p = mesh.ctrl(3, 0);
        assert!((p[1] - want_dy).abs() < 1e-9, "bulge top: {}", p[1]);
        // fish binds to the fisheye formula: center-ward points move.
        let mesh = preset_mesh("fish", 100.0, rect).unwrap();
        assert_ne!(mesh.points, WarpMesh::base_lattice(3, 3, rect).points);
        // Unsupported wire kinds reject (v1 spec gap).
        for k in [
            "arc_lower",
            "arc_upper",
            "shell",
            "squeeze",
            "twist",
            "nonsense",
        ] {
            assert_eq!(
                preset_mesh(k, 10.0, rect).unwrap_err(),
                WarpError::PresetUnsupported,
                "{k}"
            );
        }
        assert_eq!(
            preset_mesh("arc", 101.0, rect).unwrap_err(),
            WarpError::BendOutOfRange
        );
    }

    #[test]
    fn newton_inverse_round_trips_forward_points() {
        // Deformed mesh; for sampled (s,t), p = F(s,t) must invert back to
        // within the pinned 0.1 px residual via the warp machinery. We test
        // through warp_plane indirectly: warp an impulse-free smooth ramp
        // and verify the identity-bend preset is a verbatim copy while a
        // real bend moves content in the displacement direction.
        let rect = IRect::new(0, 0, 60, 60);
        let mut src = Plane::new(rect);
        for y in 0..60 {
            for x in 0..60 {
                let v = y as f32 / 59.0;
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        // bend 0 preset == base lattice → verbatim copy.
        let mesh0 = preset_mesh("arc", 0.0, rect).unwrap();
        let out0 = warp_plane(&src, &mesh0, rect);
        assert_eq!(out0.px, src.px, "zero bend is the identity short-circuit");
        // arc bend −1 pulls content DOWN at mid-columns (d.y positive for
        // negative bend): the value at a mid pixel must come from a smaller
        // source y ⇒ output value < source value at that pixel.
        let mesh = preset_mesh("arc", -80.0, rect).unwrap();
        let out = warp_plane(
            &src,
            &mesh,
            mesh_bounds(&mesh).intersect(&IRect::new(0, 0, 60, 90)),
        );
        let mid = out.get(30, 40, EdgePolicy::Transparent);
        let src_mid = src.get(30, 40, EdgePolicy::Transparent);
        assert!(mid[3] > 0.9, "mid pixel is covered");
        assert!(
            mid[0] < src_mid[0] - 0.05,
            "content displaced downward: {} vs {}",
            mid[0],
            src_mid[0]
        );
    }

    #[test]
    fn forward_inverse_consistency_on_a_deformed_mesh() {
        // Direct Newton test: for lattice samples of a wavy mesh, run the
        // machinery's inverse (via a probe pixel at F(s,t)) and check the
        // pinned acceptance (residual < 0.1 px) holds broadly.
        let rect = IRect::new(0, 0, 100, 100);
        let mesh = preset_mesh("wave", 60.0, rect).unwrap();
        let mut tested = 0;
        let mut max_res = 0.0f64;
        for tj in 0..=6 {
            for ti in 0..=6 {
                let s = 3.0 * ti as f64 / 6.0;
                let t = 3.0 * tj as f64 / 6.0;
                let p = mesh.eval(s, t);
                // Re-invert through eval+jacobian Newton from the true seed
                // neighborhood (the warp loop's per-cell machinery).
                let mut s2 = s + 0.2;
                let mut t2 = t - 0.15;
                for _ in 0..3 {
                    let f = mesh.eval(s2, t2);
                    let j = mesh.jacobian(s2, t2);
                    let (rx, ry) = (f[0] - p[0], f[1] - p[1]);
                    let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
                    if det.abs() < 1e-12 {
                        break;
                    }
                    s2 -= (rx * j[1][1] - ry * j[1][0]) / det;
                    t2 -= (j[0][0] * ry - j[0][1] * rx) / det;
                }
                let f = mesh.eval(s2, t2);
                let res = ((f[0] - p[0]).powi(2) + (f[1] - p[1]).powi(2)).sqrt();
                max_res = max_res.max(res);
                tested += 1;
            }
        }
        assert_eq!(tested, 49);
        assert!(
            max_res < 0.1,
            "3-step Newton residual bound, worst {max_res}"
        );
    }

    #[test]
    fn warp_is_deterministic() {
        let rect = IRect::new(0, 0, 40, 40);
        let mut src = Plane::new(rect);
        for y in 0..40 {
            for x in 0..40 {
                src.set(
                    x,
                    y,
                    [((x * 7 + y * 13) % 11) as f32 / 10.0, 0.5, 0.25, 1.0],
                );
            }
        }
        let mesh = preset_mesh("bulge", 45.0, rect).unwrap();
        let a = warp_plane(&src, &mesh, rect);
        let b = warp_plane(&src, &mesh, rect);
        assert_eq!(a.px, b.px);
    }
}
