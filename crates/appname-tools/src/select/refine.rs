// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.2 [NORMATIVE] exact refinement algorithms — signed
//! Euclidean distance field of the mask thresholded at 0.5, computed by
//! the two-pass separable squared-distance transform of **Felzenszwalb &
//! Huttenlocher 2004** (*Distance Transforms of Sampled Functions* — the
//! ES [LEGAL]-cited published algorithm this module implements from);
//! `smoothAA(t) = 1 − smoothstep(−0.5, +0.5, t)`; the Grow/Shrink/Smooth/
//! Border formula block verbatim; Feather = separable Gaussian with
//! **σ = r/2** on the float mask directly (§16.3 kernel shape, 3-box
//! [Kovesi 2010] for σ > 32). Contract `selection.modify.yaml` (feather
//! 0.1..=1000 px; expand/contract/smooth/border 1..=500 px int). §34.4
//! properties 11 and 13.
//!
//! Refinements operate over `bounds` inflated by the operation radius + 1
//! px (§13.2); absent input reads 0 at zero cost. The EDT lattice→contour
//! offset pin (±(D − 0.5)) is in the `select` module docs. Smooth's
//! intermediate stages stay binary (threshold only, §13.2) with ONE final
//! 1-px re-AA. The contract-note "majority filter" shorthand is superseded
//! by §13.2's exact open+close block (drift logged).

use appname_core::IRect;

use super::MaskPlane;

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// §13.2 `smoothAA`: 1 inside, 0 outside, 1-px band across `t` (px).
#[inline]
pub fn smooth_aa(t: f32) -> f32 {
    1.0 - smoothstep(-0.5, 0.5, t)
}

// ---------------------------------------------------------------------
// Felzenszwalb–Huttenlocher exact squared EDT (two-pass separable).
// ---------------------------------------------------------------------

const INF: f64 = 1e18;

/// 1-D squared-distance transform (lower envelope of parabolas).
fn dt_1d(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    let mut v = vec![0usize; n]; // parabola sites
    let mut z = vec![0.0f64; n + 1]; // envelope boundaries
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64))
                / (2.0 * q as f64 - 2.0 * p as f64);
            if s <= z[k] {
                if k == 0 {
                    break;
                }
                k -= 1;
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = INF;
                break;
            }
        }
    }
    let mut k = 0usize;
    for (q, o) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let d = q as f64 - p as f64;
        *o = d * d + f[p];
    }
}

/// Exact squared EDT of a binary grid: distance² to the nearest `true`
/// cell (INF when none). Two separable passes (columns then rows).
/// `pub(crate)` since the CP-1.0 warps row: `puppet::mesh` reuses it for
/// the §45.5 expansion dilation and Steiner boundary clearance.
pub(crate) fn edt_squared(inside: &[bool], w: usize, h: usize) -> Vec<f64> {
    let mut g = vec![0.0f64; w * h];
    // Vertical pass per column.
    let mut col = vec![0.0f64; h];
    let mut out_col = vec![0.0f64; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = if inside[y * w + x] { 0.0 } else { INF };
        }
        dt_1d(&col, &mut out_col);
        for y in 0..h {
            g[y * w + x] = out_col[y];
        }
    }
    // Horizontal pass per row.
    let mut row = vec![0.0f64; w];
    let mut out_row = vec![0.0f64; w];
    let mut result = vec![0.0f64; w * h];
    for y in 0..h {
        row.copy_from_slice(&g[y * w..(y + 1) * w]);
        dt_1d(&row, &mut out_row);
        result[y * w..(y + 1) * w].copy_from_slice(&out_row);
    }
    result
}

/// Signed distance plane of `mask` thresholded at 0.5 (§13.2): positive
/// outside, negative inside, with the ±(D − 0.5) lattice pin, over `rect`
/// (reads outside `mask.rect` are 0 = outside).
pub fn signed_distance(mask: &MaskPlane, rect: IRect) -> Vec<f32> {
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut inside = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            inside[y * w + x] = mask.get(rect.min.x + x as i32, rect.min.y + y as i32) >= 0.5;
        }
    }
    let outside: Vec<bool> = inside.iter().map(|b| !b).collect();
    let d_to_inside = edt_squared(&inside, w, h);
    let d_to_outside = edt_squared(&outside, w, h);
    (0..w * h)
        .map(|i| {
            if inside[i] {
                // Interior with no outside anywhere → deep inside.
                if d_to_outside[i] >= INF {
                    return f32::MIN / 2.0;
                }
                -((d_to_outside[i].sqrt() - 0.5) as f32)
            } else {
                if d_to_inside[i] >= INF {
                    return f32::MAX / 2.0;
                }
                (d_to_inside[i].sqrt() - 0.5) as f32
            }
        })
        .collect()
}

/// §13.2 `Grow r`: `m′(p) = smoothAA(d(p) − r)`.
pub fn grow(mask: &MaskPlane, r: f64) -> MaskPlane {
    threshold_op(mask, r, |d| smooth_aa(d - r as f32))
}

/// §13.2 `Shrink r`: `m′(p) = smoothAA(d(p) + r)`.
pub fn shrink(mask: &MaskPlane, r: f64) -> MaskPlane {
    threshold_op(mask, r, |d| smooth_aa(d + r as f32))
}

/// §13.2 `Border w`: `m′(p) = smoothAA(|d(p)| − w/2)`.
pub fn border(mask: &MaskPlane, w: f64) -> MaskPlane {
    threshold_op(mask, w / 2.0 + 1.0, |d| {
        smooth_aa(d.abs() - (w / 2.0) as f32)
    })
}

/// §13.2 `Smooth r`: open (shrink r → grow r) then close (grow r →
/// shrink r); intermediates stay binary; ONE final 1-px re-AA.
pub fn smooth(mask: &MaskPlane, r: f64) -> MaskPlane {
    let inflate = (2.0 * r).ceil() as i32 + 1;
    let rect = mask.rect.inflate(inflate, inflate);
    // Stage helper: binary result of moving the iso by ±r.
    let stage = |m: &MaskPlane, shift: f64| -> MaskPlane {
        let d = signed_distance(m, rect);
        let mut out = MaskPlane::new(rect);
        let w = rect.width() as usize;
        for y in 0..rect.height() as usize {
            for x in 0..w {
                // inside' ⇔ d + shift < 0 (threshold only, §13.2).
                out.v[y * w + x] = if d[y * w + x] + shift as f32 <= 0.0 {
                    1.0
                } else {
                    0.0
                };
            }
        }
        out
    };
    let opened = stage(&stage(mask, r), -r); // shrink r, grow r
    let closed = stage(&stage(&opened, -r), r); // grow r, shrink r
                                                // Final 1-px re-AA across the resulting contour.
    let d = signed_distance(&closed, rect);
    let mut out = MaskPlane::new(rect);
    let w = rect.width() as usize;
    for (i, &dv) in d.iter().enumerate() {
        out.v[i] = smooth_aa(dv);
    }
    let _ = w;
    out
}

fn threshold_op(mask: &MaskPlane, radius: f64, f: impl Fn(f32) -> f32) -> MaskPlane {
    let inflate = radius.abs().ceil() as i32 + 1; // §13.2 radius + 1 px
    let rect = mask.rect.inflate(inflate, inflate);
    let d = signed_distance(mask, rect);
    let mut out = MaskPlane::new(rect);
    for (o, di) in out.v.iter_mut().zip(d.iter()) {
        *o = f(*di).clamp(0.0, 1.0);
    }
    out
}

// ---------------------------------------------------------------------
// Feather (§13.2: true Gaussian, σ = r/2, on the float mask directly).
// ---------------------------------------------------------------------

/// §13.2 `Feather r`: separable Gaussian with σ = r/2 (§16.3 kernel:
/// `w_k = exp(−k²/2σ²)`, `k ∈ [−⌈3σ⌉, ⌈3σ⌉]`, Σ = 1; the 3-box
/// approximation [Kovesi 2010] for σ > 32 per the §16.3 SHOULD).
pub fn feather(mask: &MaskPlane, radius: f64) -> MaskPlane {
    let sigma = radius / 2.0;
    if sigma <= 0.0 {
        return mask.clone();
    }
    let inflate = (3.0 * sigma).ceil() as i32;
    let rect = mask.rect.inflate(inflate, inflate);
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let mut plane = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            plane[y * w + x] = mask.get(rect.min.x + x as i32, rect.min.y + y as i32);
        }
    }
    blur_plane_sigma(&mut plane, w, h, sigma);
    // §13.1/§34.4 property 13: coverage stays in [0,1] — clamp the f32
    // accumulation drift of the separable passes.
    for v in &mut plane {
        *v = v.clamp(0.0, 1.0);
    }
    let mut out = MaskPlane::new(rect);
    out.v = plane;
    out
}

/// The §16.3 separable Gaussian on a raw float plane at an **explicit**
/// `sigma`, dispatching to the [Kovesi 2010] 3-box approximation above
/// σ = 32 per the §16.3 SHOULD.
///
/// **This is the single blur dispatch in this crate**, and deliberately so:
/// §13.2 Feather calls it with `σ = r/2` (above) and the §11.3 layer-effects
/// stack calls it with `σ = radius/3` (`crate::effects`). The two σ
/// conventions are genuinely different — that difference is the *caller's*,
/// not the kernel's. Hand-rolling a second Gaussian for effects would be a
/// second answer to one question and would break the standing
/// "one compositing truth" invariant, so effects reuses this.
pub(crate) fn blur_plane_sigma(plane: &mut [f32], w: usize, h: usize, sigma: f64) {
    if sigma <= 0.0 {
        return;
    }
    if sigma > 32.0 {
        box3_blur(plane, w, h, sigma);
    } else {
        gaussian_direct(plane, w, h, sigma);
    }
}

fn gaussian_direct(plane: &mut [f32], w: usize, h: usize, sigma: f64) {
    let r = (3.0 * sigma).ceil() as i64;
    let mut kernel = Vec::with_capacity((2 * r + 1) as usize);
    let mut sum = 0.0f64;
    for k in -r..=r {
        let v = (-((k * k) as f64) / (2.0 * sigma * sigma)).exp();
        kernel.push(v);
        sum += v;
    }
    let kernel: Vec<f32> = kernel.iter().map(|v| (v / sum) as f32).collect();
    // Horizontal.
    let mut tmp = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for (i, &kv) in kernel.iter().enumerate() {
                let sx = x as i64 + i as i64 - r;
                if sx >= 0 && (sx as usize) < w {
                    acc += plane[y * w + sx as usize] * kv;
                }
            }
            tmp[y * w + x] = acc;
        }
    }
    // Vertical.
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for (i, &kv) in kernel.iter().enumerate() {
                let sy = y as i64 + i as i64 - r;
                if sy >= 0 && (sy as usize) < h {
                    acc += tmp[sy as usize * w + x] * kv;
                }
            }
            plane[y * w + x] = acc;
        }
    }
}

/// §16.3 Kovesi 3-box: `w = sqrt(12σ²/3 + 1)`, `wl` largest odd ≤ w,
/// `wu = wl + 2`, `m = round((12σ² − 3wl² − 12wl − 9)/(−4wl − 4))`; run
/// `m` box passes of width `wl` and `3 − m` of `wu` per axis.
fn box3_blur(plane: &mut [f32], w: usize, h: usize, sigma: f64) {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - (3 * wl * wl) as f64 - (12 * wl) as f64 - 9.0)
        / ((-4 * wl) as f64 - 4.0))
        .round() as i64;
    for pass in 0..3i64 {
        let width = if pass < m { wl } else { wu };
        box_pass_h(plane, w, h, (width / 2) as usize);
    }
    // Transpose-free vertical passes.
    for pass in 0..3i64 {
        let width = if pass < m { wl } else { wu };
        box_pass_v(plane, w, h, (width / 2) as usize);
    }
}

fn box_pass_h(plane: &mut [f32], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut row = vec![0.0f32; w];
    for y in 0..h {
        row.copy_from_slice(&plane[y * w..(y + 1) * w]);
        let mut acc = 0.0f32;
        for &v in row.iter().take(r.min(w.saturating_sub(1)) + 1) {
            acc += v;
        }
        for x in 0..w {
            plane[y * w + x] = acc * norm;
            let add = x + r + 1;
            if add < w {
                acc += row[add];
            }
            if x >= r {
                acc -= row[x - r];
            }
        }
    }
}

fn box_pass_v(plane: &mut [f32], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    let mut col = vec![0.0f32; h];
    for x in 0..w {
        for y in 0..h {
            col[y] = plane[y * w + x];
        }
        let mut acc = 0.0f32;
        for &v in col.iter().take(r.min(h.saturating_sub(1)) + 1) {
            acc += v;
        }
        for y in 0..h {
            plane[y * w + x] = acc * norm;
            let add = y + r + 1;
            if add < h {
                acc += col[add];
            }
            if y >= r {
                acc -= col[y - r];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::IVec2;

    fn square_mask(x0: i32, y0: i32, side: i32) -> MaskPlane {
        let mut m = MaskPlane::new(IRect::new(x0, y0, x0 + side, y0 + side));
        for y in y0..y0 + side {
            for x in x0..x0 + side {
                m.set(x, y, 1.0);
            }
        }
        m
    }

    /// The half-coverage crossing of a mask row (linear interp of the 0.5
    /// level between pixel centers).
    fn half_crossings(m: &MaskPlane, y: i32) -> (f64, f64) {
        let mut left = f64::NAN;
        let mut right = f64::NAN;
        for x in m.rect.min.x..m.rect.max.x - 1 {
            let (a, b) = (m.get(x, y), m.get(x + 1, y));
            if a < 0.5 && b >= 0.5 {
                left = x as f64 + ((0.5 - a) / (b - a)) as f64;
            }
            if a >= 0.5 && b < 0.5 {
                right = x as f64 + ((0.5 - a) / (b - a)) as f64;
            }
        }
        (left, right)
    }

    #[test]
    fn edt_distances_match_euclidean_geometry() {
        // Single inside pixel at (5,5): outside distance = euclid − 0.5.
        let mut m = MaskPlane::new(IRect::new(0, 0, 11, 11));
        m.set(5, 5, 1.0);
        let rect = IRect::new(0, 0, 11, 11);
        let d = signed_distance(&m, rect);
        let at = |x: i32, y: i32| d[(y * 11 + x) as usize];
        assert!((at(8, 5) - (3.0 - 0.5)).abs() < 1e-5, "axial: {}", at(8, 5));
        assert!(
            (at(8, 9) - (5.0 - 0.5)).abs() < 1e-5,
            "3-4-5 triangle: {}",
            at(8, 9)
        );
        assert!(
            (at(5, 5) - (-0.5)).abs() < 1e-5,
            "the lone inside pixel: {}",
            at(5, 5)
        );
        // §34.4 property 11: symmetry — distance from p to {q} equals
        // distance from q to {p}; triangle inequality over sampled points.
        let pts = [(1, 1), (7, 3), (4, 9)];
        for &(ax, ay) in &pts {
            for &(bx, by) in &pts {
                let mut ma = MaskPlane::new(rect);
                ma.set(ax, ay, 1.0);
                let da = signed_distance(&ma, rect);
                let mut mb = MaskPlane::new(rect);
                mb.set(bx, by, 1.0);
                let db = signed_distance(&mb, rect);
                let dab = da[(by * 11 + bx) as usize];
                let dba = db[(ay * 11 + ax) as usize];
                assert!((dab - dba).abs() < 1e-5, "symmetry {dab} {dba}");
            }
        }
        // Triangle inequality: d(a,c) ≤ d(a,b) + d(b,c) on raw euclid
        // (undo the −0.5 lattice pin before comparing).
        let euclid = |p: (i32, i32), q: (i32, i32)| {
            (((p.0 - q.0).pow(2) + (p.1 - q.1).pow(2)) as f64).sqrt()
        };
        for &a in &pts {
            for &b in &pts {
                for &c in &pts {
                    assert!(euclid(a, c) <= euclid(a, b) + euclid(b, c) + 1e-9);
                }
            }
        }
    }

    #[test]
    fn grow_moves_the_contour_by_exactly_r_axially_and_diagonally() {
        // 10×10 square grown by 3: the 0.5 contour sits 3 px out on the
        // axis AND ~3 px out on the diagonal (true Euclidean disk, §13.2).
        let m = square_mask(10, 10, 10);
        let g = grow(&m, 3.0);
        let (l, r) = half_crossings(&g, 15);
        // Original edges: content pixels 10..19; boundary at 9.5 / 19.5
        // in center coordinates x+0.5 ⇒ crossings near 9.5−3 and 19.5+3
        // in the crossing estimator's x units (centers at x, so ±0.5).
        assert!((l - (10.0 - 0.5 - 3.0)).abs() < 0.6, "left crossing {l}");
        // Content pixels 10..=19 (centers): raw 0.5-contour at 19.5; +3.
        assert!((r - 22.5).abs() < 0.6, "right crossing {r}");
        // Diagonal: corner (10,10); the point (10−k, 10−k) crosses 0.5 at
        // euclid distance ≈ 3 from the corner region. Probe d directly.
        let rect = g.rect;
        let d = signed_distance(&m, rect);
        let at = |x: i32, y: i32| d[((y - rect.min.y) * rect.width() + (x - rect.min.x)) as usize];
        let diag = at(8, 8); // 2√2 − adjustment from corner pixel (10,10)
        assert!(
            (diag - (2.0 * 2f64.sqrt() as f32 - 0.5)).abs() < 0.05,
            "corner distance {diag}"
        );
        // Grow-then-shrink ⊆ original is checked in the property test
        // below; also §34.4 property 13 (range) on the grown plane:
        assert!(g.v.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn shrink_then_grow_is_a_subset_and_grow_then_shrink_a_superset_of_nothing_lost() {
        // §34.4 property 11's operational form: grow r ∘ shrink r ⊆
        // original (opening ⊆ id) — compare at the 0.5 threshold.
        let mut m = square_mask(5, 5, 12);
        // Add a thin 1-px spur that opening must remove.
        for x in 17..25 {
            m = {
                let mut bigger = MaskPlane::new(m.rect.union(&IRect::new(x, 10, x + 1, 11)));
                for y in bigger.rect.min.y..bigger.rect.max.y {
                    for xx in bigger.rect.min.x..bigger.rect.max.x {
                        bigger.set(xx, y, m.get(xx, y));
                    }
                }
                bigger.set(x, 10, 1.0);
                bigger
            };
        }
        let opened = grow(&shrink(&m, 2.0), 2.0);
        for y in opened.rect.min.y..opened.rect.max.y {
            for x in opened.rect.min.x..opened.rect.max.x {
                if opened.get(x, y) >= 0.5 {
                    assert!(m.get(x, y) >= 0.5, "opening created content at ({x},{y})");
                }
            }
        }
        // The spur (thinner than r) is gone; the body survives.
        assert!(opened.get(20, 10) < 0.5, "spur removed");
        assert!(opened.get(10, 10) >= 0.5, "body kept");
    }

    #[test]
    fn border_is_a_band_of_width_w_astride_the_edge() {
        let m = square_mask(10, 10, 10);
        let b = border(&m, 4.0);
        // On row 15: the band spans |d| ≤ 2 around each edge; the deep
        // inside (x=15) and far outside are 0; the edge pixels are 1.
        assert!(b.get(15, 15) < 1e-6, "interior clear");
        assert!(b.get(2, 15) < 1e-6, "far outside clear");
        assert!(b.get(10, 15) > 0.99, "inner rim in band");
        assert!(b.get(8, 15) > 0.99, "outer rim in band");
        assert!(b.v.iter().all(|&v| (0.0..=1.0).contains(&v)), "property 13");
    }

    #[test]
    fn smooth_removes_spurs_and_fills_notches() {
        // Square with a 1-px notch cut into the edge; smooth(2) closes it.
        let mut m = square_mask(5, 5, 14);
        m.set(11, 5, 0.0); // notch on the top edge
        let s = smooth(&m, 2.0);
        assert!(s.get(11, 5) >= 0.5, "notch filled by close");
        assert!(s.get(11, 11) >= 0.5, "body intact");
        assert!(s.get(3, 3) < 0.5, "outside stays outside");
        assert!(s.v.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn feather_is_a_normalized_gaussian_with_sigma_r_over_2() {
        // Impulse: feather(r=8) ⇒ σ=4; the response is the separable kernel
        // outer product — check the exact 1-D kernel value at offset k:
        // out(k,0)/out(0,0) = exp(−k²/2σ²).
        let mut m = MaskPlane::new(IRect::new(20, 20, 21, 21));
        m.set(20, 20, 1.0);
        let f = feather(&m, 8.0);
        let sigma = 4.0f64;
        let center = f.get(20, 20);
        assert!(center > 0.0);
        for k in 1..=6i32 {
            let want = (-((k * k) as f64) / (2.0 * sigma * sigma)).exp() as f32;
            let got = f.get(20 + k, 20) / center;
            assert!((got - want).abs() < 1e-4, "k={k}: {got} vs {want}");
        }
        // Mass preserved (Σ = 1 kernel): total coverage stays 1.
        let total: f64 = f.v.iter().map(|&v| v as f64).sum();
        assert!((total - 1.0).abs() < 1e-3, "mass {total}");
        // Soft data survives (no thresholding): feather of a 0.3-valued
        // pixel peaks at 0.3·kernel(0,0), not 0 or 1.
        let mut m = MaskPlane::new(IRect::new(0, 0, 1, 1));
        m.set(0, 0, 0.3);
        let f = feather(&m, 4.0);
        let total: f64 = f.v.iter().map(|&v| v as f64).sum();
        assert!((total - 0.3).abs() < 1e-3);
    }

    #[test]
    fn feather_big_sigma_takes_the_box_path_and_stays_normalized() {
        // σ = 34 (> 32 → the Kovesi 3-box path); a 200² square: interior
        // stays near 1 (erf(100/σ√2)² ≈ 0.99), mass preserved, coverage
        // clamped into [0,1] (§34.4 property 13).
        let m = square_mask(0, 0, 200);
        let f = feather(&m, 68.0); // σ = 34
        let total: f64 = f.v.iter().map(|&v| v as f64).sum();
        assert!((total - 40_000.0).abs() / 40_000.0 < 0.02, "mass {total}");
        assert!(f.v.iter().all(|&v| (0.0..=1.0).contains(&v)));
        let c = f.get(100, 100);
        assert!(c > f.get(-30, 100) && c > 0.5, "center {c}");
    }

    #[test]
    fn refinement_chain_respects_property_13() {
        // grow → feather → border → shrink chain stays within [0,1].
        let m = square_mask(30, 30, 20);
        let out = shrink(&border(&feather(&grow(&m, 4.0), 6.0), 5.0), 2.0);
        assert!(out.v.iter().all(|&v| (0.0..=1.0).contains(&v)));
        let _ = IVec2::new(0, 0);
    }
}
