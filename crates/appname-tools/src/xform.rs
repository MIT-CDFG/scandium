// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §18.1 (transform = affine Mat3, [NORMATIVE] single-resample
//! commit), §18.3 ([NORMATIVE] identity bit-identical), §18.4 (perspective
//! DLT [LEGAL: Hartley & Zisserman 2004] with the exact 8×8 row layout,
//! Gaussian elimination with partial pivoting, degeneracy rejection),
//! §18.2 (sampling); contracts `transform.free.yaml` (column-major Mat3,
//! MUST be invertible), `transform.perspective.yaml` (quad TL/TR/BR/BL, no
//! three collinear, positive area); §34.4 property 5. DD ㉓.
//!
//! Rights basis (V39 discipline): DLT and homogeneous transform algebra are
//! textbook multiple-view geometry (Hartley & Zisserman 2004 — the ES
//! [LEGAL] pin); era clean-room per the resampling dossier's L0 framing.
//! Due diligence, not legal advice.
//!
//! v1 pins (frozen):
//! * `Mat3` is column-major over `[f64; 9]` (§0.3): element (row r, col c)
//!   at index `c*3 + r`; a point maps as `column · (x, y, 1)ᵀ`.
//! * Identity short-circuit: an identity matrix (or a no-op `Auto` scale)
//!   copies source pixels verbatim — the §18.3 "identity transform is
//!   bit-identical" guarantee — before any kernel touches data.
//! * `Auto` kernel rule scale metric: `s = sqrt(|det L|)` of the 2×2 linear
//!   part (area change root); for a homography, `L` is the Jacobian at the
//!   source-bounds center. Lanczos-3 if s < 1, Catmull-Rom if s > 1
//!   (§18.2/§19.3); pure translations/rotations (s = 1) use Catmull-Rom
//!   (identity is already short-circuited). Pin logged — §10.3 does not
//!   state the affine scale metric.
//! * Inverse mapping: destination pixel center `p = (x+0.5, y+0.5)` maps by
//!   `p_src = H⁻¹ · p` (homogeneous divide for perspective), then §18.2
//!   kernel sampling on the source lattice; out-of-source reads are
//!   transparent (§4.3; layer content is bounded).
//! * General transforms sample the full 2-D separable kernel product
//!   directly (4×4 / 6×6 taps per §18.2 support table) with per-pixel tap
//!   normalization; axis-aligned scale/translate paths may equivalently run
//!   the two-pass `resample` engine (the dossier's separable scheme) —
//!   equality within the §34.3 budget is a test below.

use appname_core::{IRect, Mat3};

use crate::resample::{auto_kernel, EdgePolicy, Kernel, Plane};

/// Column-major element access (§0.3).
#[inline]
fn m(mat: &Mat3, r: usize, c: usize) -> f64 {
    mat.0[c * 3 + r]
}

/// Column-major 3×3 multiply: `a · b`.
pub fn mat3_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [0.0f64; 9];
    for c in 0..3 {
        for r in 0..3 {
            out[c * 3 + r] = (0..3).map(|k| m(a, r, k) * m(b, k, c)).sum();
        }
    }
    Mat3(out)
}

/// Determinant of the full 3×3.
pub fn mat3_det(a: &Mat3) -> f64 {
    m(a, 0, 0) * (m(a, 1, 1) * m(a, 2, 2) - m(a, 1, 2) * m(a, 2, 1))
        - m(a, 0, 1) * (m(a, 1, 0) * m(a, 2, 2) - m(a, 1, 2) * m(a, 2, 0))
        + m(a, 0, 2) * (m(a, 1, 0) * m(a, 2, 1) - m(a, 1, 1) * m(a, 2, 0))
}

/// Inverse via the adjugate; `None` when `|det|` is not normal (the
/// contract's "MUST be invertible" gate — callers map to
/// `error.schema.out_of_range`).
pub fn mat3_inverse(a: &Mat3) -> Option<Mat3> {
    let det = mat3_det(a);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    let mut out = [0.0f64; 9];
    for r in 0..3 {
        for c in 0..3 {
            // Cofactor of (c, r) — transposed for the adjugate.
            let (r1, r2) = ((c + 1) % 3, (c + 2) % 3);
            let (c1, c2) = ((r + 1) % 3, (r + 2) % 3);
            let cof = m(a, r1, c1) * m(a, r2, c2) - m(a, r1, c2) * m(a, r2, c1);
            out[c * 3 + r] = cof * inv_det;
        }
    }
    Some(Mat3(out))
}

/// Apply to a point with homogeneous divide (`w = 1` for affine rows).
pub fn mat3_apply(a: &Mat3, p: [f64; 2]) -> [f64; 2] {
    let x = m(a, 0, 0) * p[0] + m(a, 0, 1) * p[1] + m(a, 0, 2);
    let y = m(a, 1, 0) * p[0] + m(a, 1, 1) * p[1] + m(a, 1, 2);
    let w = m(a, 2, 0) * p[0] + m(a, 2, 1) * p[1] + m(a, 2, 2);
    if w == 1.0 {
        [x, y]
    } else {
        [x / w, y / w]
    }
}

/// Is the bottom row exactly `(0, 0, 1)` (affine)?
pub fn mat3_is_affine(a: &Mat3) -> bool {
    m(a, 2, 0) == 0.0 && m(a, 2, 1) == 0.0 && m(a, 2, 2) == 1.0
}

/// The `Auto` scale metric (module pin): `sqrt(|det J|)` with `J` the
/// linear part (affine) or the Jacobian at `at` (homography).
pub fn scale_metric(a: &Mat3, at: [f64; 2]) -> f64 {
    if mat3_is_affine(a) {
        (m(a, 0, 0) * m(a, 1, 1) - m(a, 0, 1) * m(a, 1, 0))
            .abs()
            .sqrt()
    } else {
        // Numeric Jacobian of the projective map at `at`.
        let e = 0.5;
        let p0 = mat3_apply(a, at);
        let px = mat3_apply(a, [at[0] + e, at[1]]);
        let py = mat3_apply(a, [at[0], at[1] + e]);
        let (dxx, dxy) = ((px[0] - p0[0]) / e, (px[1] - p0[1]) / e);
        let (dyx, dyy) = ((py[0] - p0[0]) / e, (py[1] - p0[1]) / e);
        (dxx * dyy - dxy * dyx).abs().sqrt()
    }
}

/// Error from [`resolve_transform_kernel`]: the `resample` param named no
/// known §18.2 kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownKernelName;

/// Resolve the transform-family `resample` param (§10.3: default `auto`;
/// the §18.2 wire names are also accepted — v1 pin, lane log): `Ok(None)`
/// means the Auto no-op (scale exactly 1).
pub fn resolve_transform_kernel(
    name: &str,
    scale: f64,
) -> Result<Option<Kernel>, UnknownKernelName> {
    if name == "auto" {
        Ok(auto_kernel(scale))
    } else {
        Kernel::from_wire(name).map(Some).ok_or(UnknownKernelName)
    }
}

/// §18.4 [NORMATIVE] DLT: homography from 4 source→destination
/// correspondences, `h₃₃ = 1`, two rows per pair, Gaussian elimination with
/// partial pivoting. Returns the column-major `Mat3`, or `None` when the
/// system is singular (degenerate configuration).
pub fn dlt_homography(src: [[f64; 2]; 4], dst: [[f64; 2]; 4]) -> Option<Mat3> {
    let mut a = [[0.0f64; 9]; 8]; // 8×8 augmented with b in column 8
    for i in 0..4 {
        let [x, y] = src[i];
        let [xp, yp] = dst[i];
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -xp * x, -xp * y, xp];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -yp * x, -yp * y, yp];
    }
    // Gaussian elimination with partial pivoting (§18.4).
    for col in 0..8 {
        let piv = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        let prow = a[col]; // copy: the pivot row is not mutated below
        for arow in a.iter_mut().skip(col + 1) {
            let f = arow[col] / prow[col];
            for (av, pv) in arow[col..].iter_mut().zip(&prow[col..]) {
                *av -= f * pv;
            }
        }
    }
    let mut h = [0.0f64; 8];
    for row in (0..8).rev() {
        let mut acc = a[row][8];
        for k in (row + 1)..8 {
            acc -= a[row][k] * h[k];
        }
        h[row] = acc / a[row][row];
    }
    // h = (h11 h12 h13 h21 h22 h23 h31 h32) row-major; store column-major.
    Some(Mat3([h[0], h[3], h[6], h[1], h[4], h[7], h[2], h[5], 1.0]))
}

/// Contract degeneracy gate for `transform.perspective` quads
/// (TL/TR/BR/BL): no three collinear, positive area (not flipped /
/// self-intersecting) — §40.6 → `error.schema.out_of_range`.
pub fn quad_is_valid(q: [[f64; 2]; 4]) -> bool {
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    // Convex, consistently oriented, no three collinear: every consecutive
    // corner triple must turn the same way with nonzero cross product. In
    // the y-down document frame the TL/TR/BR/BL winding gives positive
    // cross products; a flipped or self-intersecting quad flips at least
    // one sign, a collinear triple zeroes one.
    let mut sign = 0.0f64;
    for i in 0..4 {
        let c = cross(q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        if c.abs() < 1e-9 || !c.is_finite() {
            return false; // three collinear (or NaN)
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false; // flipped / self-intersecting
        }
    }
    // Positive area in the y-down TL/TR/BR/BL convention.
    sign > 0.0
}

/// The destination pixel rect covered by transforming `src_rect` through
/// `h` (forward corners + 1 px kernel slack), for planning output planes.
pub fn transformed_bounds(h: &Mat3, src_rect: IRect) -> IRect {
    let corners = [
        [src_rect.min.x as f64, src_rect.min.y as f64],
        [src_rect.max.x as f64, src_rect.min.y as f64],
        [src_rect.max.x as f64, src_rect.max.y as f64],
        [src_rect.min.x as f64, src_rect.max.y as f64],
    ];
    let (mut lo_x, mut lo_y) = (f64::INFINITY, f64::INFINITY);
    let (mut hi_x, mut hi_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in corners {
        let p = mat3_apply(h, c);
        lo_x = lo_x.min(p[0]);
        lo_y = lo_y.min(p[1]);
        hi_x = hi_x.max(p[0]);
        hi_y = hi_y.max(p[1]);
    }
    if !(lo_x.is_finite() && lo_y.is_finite() && hi_x.is_finite() && hi_y.is_finite()) {
        return IRect::EMPTY;
    }
    IRect::new(
        (lo_x.floor() as i32) - 1,
        (lo_y.floor() as i32) - 1,
        (hi_x.ceil() as i32) + 1,
        (hi_y.ceil() as i32) + 1,
    )
}

/// Inverse-map resample of `src` through the forward map `h` into
/// `dst_rect`. Identity matrices short-circuit to a verbatim copy (§18.3
/// bit-identity). `kernel = None` (Auto no-op) also copies verbatim.
pub fn transform_plane(
    src: &Plane,
    h: &Mat3,
    dst_rect: IRect,
    kernel: Option<Kernel>,
) -> Option<Plane> {
    let kernel = match kernel {
        _ if h.is_identity() => {
            // §18.3 [NORMATIVE]: bit-identical. Copy the overlap verbatim.
            let mut out = Plane::new(dst_rect);
            for y in dst_rect.min.y..dst_rect.max.y {
                for x in dst_rect.min.x..dst_rect.max.x {
                    out.set(x, y, src.get(x, y, EdgePolicy::Transparent));
                }
            }
            return Some(out);
        }
        None => Kernel::CatmullRom, // Auto at scale 1 but non-identity map
        Some(k) => k,
    };
    let inv = mat3_inverse(h)?;
    let support = kernel.support();
    let mut out = Plane::new(dst_rect);
    for y in dst_rect.min.y..dst_rect.max.y {
        for x in dst_rect.min.x..dst_rect.max.x {
            let p = mat3_apply(&inv, [x as f64 + 0.5, y as f64 + 0.5]);
            // Source lattice coordinate (pixel centers at i + 0.5).
            let (sx, sy) = (p[0] - 0.5, p[1] - 0.5);
            if !sx.is_finite() || !sy.is_finite() {
                continue;
            }
            let (lo_x, hi_x) = ((sx - support).ceil() as i64, (sx + support).floor() as i64);
            let (lo_y, hi_y) = ((sy - support).ceil() as i64, (sy + support).floor() as i64);
            let mut acc = [0.0f64; 4];
            let mut wsum = 0.0f64;
            for ky in lo_y..=hi_y {
                let wy = kernel.weight(ky as f64 - sy);
                if wy == 0.0 {
                    continue;
                }
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
                let inv_w = 1.0 / wsum;
                out.set(
                    x,
                    y,
                    [
                        (acc[0] * inv_w) as f32,
                        (acc[1] * inv_w) as f32,
                        (acc[2] * inv_w) as f32,
                        (acc[3] * inv_w) as f32,
                    ],
                );
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate(tx: f64, ty: f64) -> Mat3 {
        Mat3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, 1.0])
    }

    fn scale(sx: f64, sy: f64) -> Mat3 {
        Mat3([sx, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 1.0])
    }

    fn checker(w: i32, h: i32) -> Plane {
        let mut p = Plane::new(IRect::new(0, 0, w, h));
        for y in 0..h {
            for x in 0..w {
                let v = ((x + y) % 2) as f32;
                p.set(x, y, [v, 1.0 - v, 0.25, 1.0]);
            }
        }
        p
    }

    #[test]
    fn mat3_algebra_round_trips() {
        let a = Mat3([2.0, 0.0, 0.0, 0.5, 1.5, 0.0, 10.0, -3.0, 1.0]);
        let inv = mat3_inverse(&a).unwrap();
        let id = mat3_mul(&a, &inv);
        for (i, v) in id.0.iter().enumerate() {
            let want = if i % 4 == 0 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-12, "index {i}: {v}");
        }
        let p = [7.0, -2.5];
        let q = mat3_apply(&a, p);
        let back = mat3_apply(&inv, q);
        assert!((back[0] - p[0]).abs() < 1e-12 && (back[1] - p[1]).abs() < 1e-12);
        // Singular rejects.
        assert!(mat3_inverse(&Mat3([0.0; 9])).is_none());
        assert!(mat3_inverse(&Mat3([1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0])).is_none());
    }

    #[test]
    fn dlt_recovers_known_transforms() {
        // Unit square → translated square: homography must equal the
        // translation exactly.
        let src = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let dst = [[5.0, 3.0], [6.0, 3.0], [6.0, 4.0], [5.0, 4.0]];
        let h = dlt_homography(src, dst).unwrap();
        for (s, d) in src.iter().zip(dst.iter()) {
            let p = mat3_apply(&h, *s);
            assert!((p[0] - d[0]).abs() < 1e-9 && (p[1] - d[1]).abs() < 1e-9);
        }
        assert!(mat3_is_affine(&h) || (m3(&h, 2, 0).abs() < 1e-12 && m3(&h, 2, 1).abs() < 1e-12));

        // A genuinely projective quad: corners map exactly; interior center
        // maps consistently under H (cross-check by re-solving inverse).
        let dst2 = [[0.0, 0.0], [10.0, 1.0], [9.0, 9.0], [1.0, 8.0]];
        let h2 = dlt_homography(src, dst2).unwrap();
        for (s, d) in src.iter().zip(dst2.iter()) {
            let p = mat3_apply(&h2, *s);
            assert!(
                (p[0] - d[0]).abs() < 1e-9 && (p[1] - d[1]).abs() < 1e-9,
                "{s:?}→{p:?} want {d:?}"
            );
        }
        let hinv = mat3_inverse(&h2).unwrap();
        let mid = mat3_apply(&h2, [0.5, 0.5]);
        let back = mat3_apply(&hinv, mid);
        assert!((back[0] - 0.5).abs() < 1e-9 && (back[1] - 0.5).abs() < 1e-9);

        fn m3(mat: &Mat3, r: usize, c: usize) -> f64 {
            mat.0[c * 3 + r]
        }
    }

    #[test]
    fn dlt_rejects_degenerate_input() {
        let src = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        // All four destinations collinear → singular system.
        let flat = [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0]];
        assert!(dlt_homography(src, flat).is_none());
    }

    #[test]
    fn quad_validity_gate_matches_the_contract() {
        // TL/TR/BR/BL, y-down: valid.
        assert!(quad_is_valid([
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0]
        ]));
        assert!(quad_is_valid([
            [0.0, 0.0],
            [19.0, 0.4],
            [18.8, 10.6],
            [0.2, 10.2]
        ]));
        // Three collinear.
        assert!(!quad_is_valid([
            [0.0, 0.0],
            [5.0, 0.0],
            [10.0, 0.0],
            [0.0, 10.0]
        ]));
        // Flipped (TL and TR swapped → negative orientation).
        assert!(!quad_is_valid([
            [10.0, 0.0],
            [0.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0]
        ]));
        // Self-intersecting (bow-tie).
        assert!(!quad_is_valid([
            [0.0, 0.0],
            [10.0, 0.0],
            [0.0, 10.0],
            [10.0, 10.0]
        ]));
    }

    #[test]
    fn identity_transform_is_bit_identical() {
        // §18.3 [NORMATIVE] / §34.4 property 5.
        let src = checker(16, 16);
        let out = transform_plane(&src, &Mat3::IDENTITY, src.rect, Some(Kernel::Lanczos3)).unwrap();
        assert_eq!(out.px, src.px);
    }

    #[test]
    fn integer_translate_nearest_is_lossless() {
        // §34.4 property 5: T⁻¹∘T on nearest/integer-translate is identity.
        let src = checker(12, 10);
        let t = translate(3.0, -2.0);
        let fwd_rect = transformed_bounds(&t, src.rect);
        let fwd = transform_plane(&src, &t, fwd_rect, Some(Kernel::Nearest)).unwrap();
        let back = transform_plane(
            &fwd,
            &mat3_inverse(&t).unwrap(),
            src.rect,
            Some(Kernel::Nearest),
        )
        .unwrap();
        assert_eq!(back.px, src.px);
    }

    #[test]
    fn four_quarter_turns_recover_the_source() {
        // §18.3: k·360° recovers within tolerance. Rotate 90° about the
        // plane center four times with CR sampling.
        let src = checker(16, 16);
        let c = 8.0;
        let rot90 = mat3_mul(
            &translate(c, c),
            &mat3_mul(
                &Mat3([0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
                &translate(-c, -c),
            ),
        );
        let mut cur = src.clone();
        for _ in 0..4 {
            cur = transform_plane(&cur, &rot90, src.rect, Some(Kernel::CatmullRom)).unwrap();
        }
        // Interior pixels (away from the boundary apron) recover closely.
        for y in 3..13 {
            for x in 3..13 {
                let a = cur.get(x, y, EdgePolicy::Transparent);
                let b = src.get(x, y, EdgePolicy::Transparent);
                for ch in 0..4 {
                    assert!(
                        (a[ch] - b[ch]).abs() < 0.02,
                        "({x},{y})[{ch}]: {} vs {}",
                        a[ch],
                        b[ch]
                    );
                }
            }
        }
    }

    #[test]
    fn axis_aligned_scale_matches_the_two_pass_engine_within_budget() {
        // The general 2-D sampler and the separable two-pass engine agree
        // within the §34.3 parity budget (≤ 1/255) on an axis-aligned scale.
        use crate::resample::resize_plane;
        let src = checker(16, 12);
        let dst_rect = IRect::new(0, 0, 24, 18);
        let s = scale(1.5, 1.5);
        let a = transform_plane(&src, &s, dst_rect, Some(Kernel::CatmullRom)).unwrap();
        let b = resize_plane(&src, dst_rect, Kernel::CatmullRom, EdgePolicy::Transparent);
        for (pa, pb) in a.px.iter().zip(b.px.iter()) {
            for c in 0..4 {
                assert!(
                    (pa[c] - pb[c]).abs() <= 1.0 / 255.0 + 1e-4,
                    "{pa:?} vs {pb:?}"
                );
            }
        }
    }

    #[test]
    fn scale_metric_and_auto_resolution() {
        assert!((scale_metric(&scale(2.0, 2.0), [0.0, 0.0]) - 2.0).abs() < 1e-12);
        assert!((scale_metric(&scale(0.5, 0.5), [0.0, 0.0]) - 0.5).abs() < 1e-12);
        // Rotation preserves area → metric 1.
        let r = Mat3([0.6, 0.8, 0.0, -0.8, 0.6, 0.0, 0.0, 0.0, 1.0]);
        assert!((scale_metric(&r, [0.0, 0.0]) - 1.0).abs() < 1e-12);
        assert_eq!(
            resolve_transform_kernel("auto", 0.5).unwrap(),
            Some(Kernel::Lanczos3)
        );
        assert_eq!(
            resolve_transform_kernel("auto", 2.0).unwrap(),
            Some(Kernel::CatmullRom)
        );
        assert_eq!(resolve_transform_kernel("auto", 1.0).unwrap(), None);
        assert_eq!(
            resolve_transform_kernel("bicubic", 0.5).unwrap(),
            Some(Kernel::CatmullRom)
        );
        assert!(
            resolve_transform_kernel("cubic", 1.0).is_err(),
            "unknown names reject"
        );
    }

    #[test]
    fn transformed_bounds_cover_the_forward_image() {
        let src_rect = IRect::new(0, 0, 10, 10);
        let t = mat3_mul(&translate(100.0, 50.0), &scale(2.0, 3.0));
        let b = transformed_bounds(&t, src_rect);
        assert!(b.contains_rect(&IRect::new(100, 50, 120, 80)));
    }
}
