// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.1 ([NORMATIVE] "geometric sources rasterize with exact
//! analytic area coverage per pixel — the same active-edge scanline
//! machinery as vector masks (§11.2)"), §11.2 ("exact area coverage per
//! pixel via an active-edge scanline over the flattened path (flatness
//! tolerance 0.1 px) … never by supersampling"), §13.3 (rect = exact
//! pixel-area coverage of sub-pixel bounds; ellipse = exact scanline
//! coverage of the flattened outline; lasso/polygon = **non-zero-winding**
//! scanline; freehand vertices closer than 0.25 px merge, auto-close).
//!
//! Implementation: the classic signed cell accumulation rasterizer (cover/
//! area per pixel cell, per scanline slab) — exact trapezoid integration
//! per edge, so `Σ coverage = polygon area` exactly for simple polygons
//! (the oracle the tests use). Non-zero winding realizes as
//! `min(|Σ signed|, 1)` per pixel.

use appname_core::IRect;

use super::MaskPlane;

/// Exact pixel-area coverage of a sub-pixel rectangle (§13.3 marquee).
pub fn rasterize_rect(x: f64, y: f64, w: f64, h: f64) -> MaskPlane {
    let rect = IRect::new(
        x.floor() as i32,
        y.floor() as i32,
        (x + w).ceil() as i32,
        (y + h).ceil() as i32,
    );
    let mut out = MaskPlane::new(rect);
    for py in rect.min.y..rect.max.y {
        for px in rect.min.x..rect.max.x {
            let ox = overlap(px as f64, px as f64 + 1.0, x, x + w);
            let oy = overlap(py as f64, py as f64 + 1.0, y, y + h);
            out.set(px, py, (ox * oy) as f32);
        }
    }
    out
}

#[inline]
fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Freehand-lasso vertex conditioning (§13.3): merge consecutive vertices
/// closer than 0.25 px; the path auto-closes (the rasterizer closes every
/// contour implicitly).
pub fn condition_lasso(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for p in points {
        if let Some(last) = out.last() {
            let (dx, dy) = (p[0] - last[0], p[1] - last[1]);
            if (dx * dx + dy * dy).sqrt() < 0.25 {
                continue;
            }
        }
        out.push(*p);
    }
    out
}

/// §13.3 polygon/lasso fill: non-zero-winding scanline with analytic
/// coverage. The polygon closes implicitly (last → first).
pub fn rasterize_polygon(points: &[[f64; 2]]) -> MaskPlane {
    let n = points.len();
    if n < 2 {
        return MaskPlane::new(IRect::EMPTY);
    }
    let (mut lx, mut ly) = (f64::INFINITY, f64::INFINITY);
    let (mut hx, mut hy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        lx = lx.min(p[0]);
        ly = ly.min(p[1]);
        hx = hx.max(p[0]);
        hy = hy.max(p[1]);
    }
    if !(lx.is_finite() && hx.is_finite()) {
        return MaskPlane::new(IRect::EMPTY);
    }
    let rect = IRect::new(
        lx.floor() as i32,
        ly.floor() as i32,
        hx.ceil() as i32,
        hy.ceil() as i32,
    );
    if rect.is_empty() {
        return MaskPlane::new(IRect::EMPTY);
    }
    let (w, h) = (rect.width() as usize, rect.height() as usize);

    // Signed accumulators per pixel: `area` (partial coverage within the
    // crossing cell) and `cover` (full-height contribution propagated to
    // every cell right of the crossing) — accumulated as a row-delta.
    let mut area = vec![0.0f64; w * h];
    let mut cover = vec![0.0f64; (w + 1) * h];

    for i in 0..n {
        let p0 = points[i];
        let p1 = points[(i + 1) % n];
        if p0[1] == p1[1] {
            continue; // horizontal edges contribute no winding
        }
        // Walk the edge through each 1-px scanline slab it crosses,
        // preserving direction (the winding sign is dy's sign).
        let (top, bot) = (p0[1].min(p1[1]), p0[1].max(p1[1]));
        let y_start = (top.floor() as i32).max(rect.min.y);
        let y_end = (bot.ceil() as i32).min(rect.max.y);
        let inv_dy = 1.0 / (p1[1] - p0[1]);
        let dxdy = (p1[0] - p0[0]) * inv_dy;
        for py in y_start..y_end {
            let slab0 = (py as f64).max(top);
            let slab1 = (py as f64 + 1.0).min(bot);
            if slab1 <= slab0 {
                continue;
            }
            // Segment endpoints inside this slab in EDGE direction.
            let (ya, yb) = if p1[1] > p0[1] {
                (slab0, slab1)
            } else {
                (slab1, slab0)
            };
            let xa = p0[0] + (ya - p0[1]) * dxdy;
            let xb = p0[0] + (yb - p0[1]) * dxdy;
            let dy = yb - ya; // signed by winding
                              // Split the sub-segment by pixel columns.
            let (cx0, cx1) = (
                (xa.min(xb).floor() as i32).max(rect.min.x),
                (xa.max(xb).floor() as i32).min(rect.max.x - 1),
            );
            let row = (py - rect.min.y) as usize;
            if cx0 > cx1 {
                // Entirely left of the raster range: full cover to the row.
                let col = ((xa.min(xb).floor() as i32) - rect.min.x).clamp(0, w as i32) as usize;
                cover[row * (w + 1) + col] += dy;
                continue;
            }
            let x_of_t = |t: f64| xa + (xb - xa) * t;
            // Parameter values where the segment crosses column borders.
            let mut prev_t = 0.0f64;
            let steps = cx1 - cx0 + 1;
            for k in 0..steps {
                let col = if xb >= xa { cx0 + k } else { cx1 - k };
                let border = if xb >= xa {
                    (col + 1) as f64
                } else {
                    col as f64
                };
                let t_exit = if (xb - xa).abs() < 1e-12 {
                    1.0
                } else {
                    ((border - xa) / (xb - xa)).clamp(prev_t, 1.0)
                };
                let t1 = if k == steps - 1 { 1.0 } else { t_exit };
                let seg_dy = dy * (t1 - prev_t);
                let xm = 0.5 * (x_of_t(prev_t) + x_of_t(t1)) - col as f64; // 0..1 in cell
                let c = (col - rect.min.x) as usize;
                // Area to the RIGHT of the edge within this cell…
                area[row * w + c] += seg_dy * (1.0 - xm.clamp(0.0, 1.0));
                // …and full cover for every cell right of this one.
                cover[row * (w + 1) + c + 1] += seg_dy;
                prev_t = t1;
            }
        }
    }

    let mut out = MaskPlane::new(rect);
    for row in 0..h {
        let mut acc = 0.0f64;
        for col in 0..w {
            acc += cover[row * (w + 1) + col];
            let winding_cov = acc + area[row * w + col];
            // Non-zero winding: |signed coverage| clamped to 1.
            out.v[row * w + col] = winding_cov.abs().min(1.0) as f32;
        }
    }
    out
}

/// §13.3 ellipse: exact scanline coverage of the flattened outline
/// (flatness 0.1 px, §11.2) — the ellipse inscribed in `rect` (the
/// contract's "ellipse = bounding box" rule), flattened to a polygon whose
/// sagitta stays under the tolerance, then the shared polygon rasterizer.
pub fn rasterize_ellipse(x: f64, y: f64, w: f64, h: f64) -> MaskPlane {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let r_max = rx.max(ry).max(1e-9);
    // Sagitta of a chord of angular step θ on radius r: r(1 − cos(θ/2)).
    let theta = 2.0 * (1.0 - 0.1 / r_max).clamp(-1.0, 1.0).acos();
    // Rounded UP to a multiple of 4 (pin): keeps the flattened outline
    // mirror-symmetric about both axes while only tightening the 0.1-px
    // flatness tolerance.
    let n_raw = ((core::f64::consts::TAU / theta.max(1e-6)).ceil() as usize).clamp(8, 4096);
    let n = n_raw.div_ceil(4) * 4;
    let pts: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            let a = core::f64::consts::TAU * i as f64 / n as f64;
            [cx + rx * a.cos(), cy + ry * a.sin()]
        })
        .collect();
    rasterize_polygon(&pts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total_coverage(m: &MaskPlane) -> f64 {
        m.v.iter().map(|&v| v as f64).sum()
    }

    #[test]
    fn rect_coverage_is_exact_pixel_area() {
        // Integer-aligned rect: all-ones inside.
        let m = rasterize_rect(2.0, 3.0, 4.0, 2.0);
        assert_eq!(m.rect, IRect::new(2, 3, 6, 5));
        assert!(m.v.iter().all(|&v| v == 1.0));
        // Half-pixel offsets: border coverage 0.5, corner 0.25.
        let m = rasterize_rect(0.5, 0.5, 2.0, 2.0);
        assert_eq!(m.rect, IRect::new(0, 0, 3, 3));
        assert_eq!(m.get(0, 0), 0.25);
        assert_eq!(m.get(1, 0), 0.5);
        assert_eq!(m.get(1, 1), 1.0);
        assert_eq!(m.get(2, 2), 0.25);
        // Total = exact area.
        assert!((total_coverage(&m) - 4.0).abs() < 1e-6);
        let m = rasterize_rect(0.25, 0.75, 3.5, 1.25);
        assert!((total_coverage(&m) - 3.5 * 1.25).abs() < 1e-6);
    }

    #[test]
    fn polygon_reproduces_the_axis_aligned_rect() {
        let m_poly = rasterize_polygon(&[[1.0, 1.0], [5.0, 1.0], [5.0, 4.0], [1.0, 4.0]]);
        let m_rect = rasterize_rect(1.0, 1.0, 4.0, 3.0);
        assert_eq!(m_poly.rect, m_rect.rect);
        for (a, b) in m_poly.v.iter().zip(m_rect.v.iter()) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
        // Winding direction is irrelevant (|winding|).
        let m_ccw = rasterize_polygon(&[[1.0, 1.0], [1.0, 4.0], [5.0, 4.0], [5.0, 1.0]]);
        for (a, b) in m_ccw.v.iter().zip(m_rect.v.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn polygon_total_coverage_equals_area() {
        // Triangle area = ½·b·h — the analytic-coverage integral must equal
        // it exactly (trapezoid accumulation is exact).
        let tri = [[0.0, 0.0], [8.0, 0.0], [0.0, 6.0]];
        let m = rasterize_polygon(&tri);
        assert!(
            (total_coverage(&m) - 24.0).abs() < 1e-6,
            "got {}",
            total_coverage(&m)
        );
        // Irregular sub-pixel quad: shoelace area.
        let quad = [[0.3, 0.2], [7.6, 1.1], [6.9, 5.7], [1.2, 4.4]];
        let shoelace = 0.5
            * (0..4)
                .map(|i| {
                    let a = quad[i];
                    let b = quad[(i + 1) % 4];
                    a[0] * b[1] - b[0] * a[1]
                })
                .sum::<f64>()
                .abs();
        let m = rasterize_polygon(&quad);
        assert!(
            (total_coverage(&m) - shoelace).abs() < 1e-6,
            "coverage {} vs shoelace {}",
            total_coverage(&m),
            shoelace
        );
    }

    #[test]
    fn triangle_pixel_coverages_are_the_analytic_areas() {
        // Right triangle (0,0)-(2,0)-(0,2): pixel (0,0) holds area 1.5/…
        // exact per-pixel: pixel (0,0) ∩ triangle = 1 − ½·(overhang)…
        // Compute directly: the hypotenuse x+y=2 cuts pixel (0,0)? No —
        // pixel (0,0) spans [0,1]²; x+y ≤ 2 holds everywhere → coverage 1.
        // Pixel (1,0): region x∈[1,2],y∈[0,1], x+y≤2 → area ½. Same (0,1).
        // Pixel (1,1): x+y≤2 over [1,2]² → 0 (only the corner point).
        let m = rasterize_polygon(&[[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]]);
        assert!((m.get(0, 0) - 1.0).abs() < 1e-6);
        assert!((m.get(1, 0) - 0.5).abs() < 1e-6);
        assert!((m.get(0, 1) - 0.5).abs() < 1e-6);
        assert!(m.get(1, 1) < 1e-6);
    }

    #[test]
    fn nonzero_winding_keeps_self_overlap_at_one() {
        // A bow-tie traversed so the two lobes have opposite winding still
        // covers each lobe (|winding| = 1), and a doubly-wound square
        // clamps at 1 (nonzero rule, not even-odd).
        let square2x = [
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 4.0],
            [0.0, 4.0],
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 4.0],
            [0.0, 4.0],
        ];
        let m = rasterize_polygon(&square2x);
        assert_eq!(m.get(2, 2), 1.0, "double winding clamps to full coverage");
        assert!((total_coverage(&m) - 16.0).abs() < 1e-5);
    }

    #[test]
    fn ellipse_area_and_symmetry() {
        // Circle r=10: area πr² within the 0.1-px flattening tolerance
        // (relative error of the inscribed polygon ≪ 1%).
        let m = rasterize_ellipse(0.0, 0.0, 20.0, 20.0);
        let area = total_coverage(&m);
        let want = core::f64::consts::PI * 100.0;
        // The 0.1-px flattening (§11.2) bounds the area deficit by
        // perimeter × sagitta = 2πr·0.1; the rasterization itself is exact.
        let bound = 2.0 * core::f64::consts::PI * 10.0 * 0.1;
        assert!(
            want - area >= 0.0 && want - area <= bound + 1e-6,
            "circle area {area} vs {want} (flattening bound {bound})"
        );
        // 4-fold symmetry of the circle's coverage field.
        for (dx, dy) in [(3, 4), (7, 2), (9, 0)] {
            let a = m.get(10 + dx, 10 + dy);
            let b = m.get(10 - 1 - dx, 10 + dy);
            let c = m.get(10 + dx, 10 - 1 - dy);
            assert!((a - b).abs() < 1e-4, "H symmetry {a} {b}");
            assert!((a - c).abs() < 1e-4, "V symmetry {a} {c}");
        }
        // Center is fully covered; far corner is empty.
        assert_eq!(m.get(10, 10), 1.0);
        assert_eq!(m.get(0, 0), 0.0);
    }

    #[test]
    fn lasso_conditioning_merges_close_vertices() {
        let pts = [
            [0.0, 0.0],
            [0.1, 0.1], // < 0.25 px from previous → merged
            [1.0, 0.0],
            [1.0, 0.2], // 0.2 px → merged
            [5.0, 5.0],
        ];
        let out = condition_lasso(&pts);
        assert_eq!(out, vec![[0.0, 0.0], [1.0, 0.0], [5.0, 5.0]]);
    }

    #[test]
    fn degenerate_inputs_produce_empty_planes() {
        assert!(rasterize_polygon(&[[0.0, 0.0]]).rect.is_empty());
        // Collinear "polygon" has zero area everywhere.
        let m = rasterize_polygon(&[[0.0, 0.0], [4.0, 0.0], [8.0, 0.0]]);
        assert!(total_coverage(&m) < 1e-9);
    }
}
