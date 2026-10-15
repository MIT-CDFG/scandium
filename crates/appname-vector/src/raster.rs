// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Scanline EXACT-COVERAGE rasterizer v1 (CPU reference).
//!
//! Spec: ES §20.2 "Fill rasterization — sparse signed-area coverage"
//! [NORMATIVE]: (1) flatten (adaptive De Casteljau, §20.1); (2) walk every
//! edge, split at scanline and cell boundaries, accumulate signed `cover`
//! (Δy) and partial `area`; (3) per scanline, left→right winding sweep;
//! (4) winding → alpha per fill rule: `nonzero: a = min(1, |w|)`;
//! `evenodd: m = |w| mod 2; a = m ≤ 1 ? m : 2 − m`; (5) output a Gray f32
//! coverage buffer — the IDENTICAL representation as a selection mask
//! (§13.1), which is how path → selection conversion is literally free.
//! Also §11.2 (vector-mask rasterization rule: exact area coverage via an
//! active-edge scanline, never supersampling).
//!
//! Era-basis (dossier `contracts/algorithms/dossiers/vector-rasterization
//! .yaml`): "scanline active-edge with exact-area coverage accumulation
//! (cell-based, libart/FreeType-smooth lineage — public art since the
//! 1990s)"; determinism class D (bit-identical CPU); patents: [none];
//! NV_path-style vendor GPU approaches explicitly rejected. Clean-room own
//! Rust; §20.2's [LEGAL] note: a public, independently re-implementable
//! technique, no code taken from FreeType or stb.
//!
//! Consistency precedent: `appname-tools/src/select/shape.rs` (the §13.3
//! selection engines) uses the same cover/area cell math for marquee/
//! lasso sources — read for consistency, re-implemented here in this
//! lane's fence with curves + both fill rules + multi-subpath winding.
//! v1 materializes the full cell grid per row (dense rows); the dossier's
//! sparse per-scanline cell lists and per-tile binning are the CP-1.0
//! perf pass (budget: 10k-node redraw ≤ 8 ms @1080p — not claimed yet).
//!
//! Oracle invariant (tested): for simple polygons the per-edge trapezoid
//! integration is EXACT in the f64 accumulators, so `Σ coverage ==
//! shoelace area` up to the f32 quantization of the OUTPUT plane
//! (§13.1 stores masks as f32: ~2⁻²⁴ per partially-covered boundary
//! pixel — the tests bound the error by the polygon perimeter).

use appname_core::{IRect, TileCoord, TILE_I32};

use crate::flatten::flatten_path;
use crate::geom::Vec2;
use crate::path::{FillRule, VectorPath};

/// A single-component f32 coverage plane over an integer pixel rect —
/// core's §13.1 mask convention (values [0, 1], coverage-not-blur). The
/// O-CORE install seam (selection/mask doors, CP-1.0 `selection.from_path`)
/// consumes this via `rect`/`get`/`tile_coords`.
#[derive(Clone, Debug, PartialEq)]
pub struct CoveragePlane {
    pub rect: IRect,
    /// Row-major, `rect.width() × rect.height()`; empty for empty rects.
    pub v: Vec<f32>,
}

impl CoveragePlane {
    pub fn new(rect: IRect) -> CoveragePlane {
        let n = if rect.is_empty() {
            0
        } else {
            rect.width() as usize * rect.height() as usize
        };
        CoveragePlane {
            rect,
            v: vec![0.0; n],
        }
    }

    /// Coverage at absolute pixel (x, y); 0.0 outside the rect.
    pub fn get(&self, x: i32, y: i32) -> f32 {
        if self.rect.is_empty()
            || x < self.rect.min.x
            || x >= self.rect.max.x
            || y < self.rect.min.y
            || y >= self.rect.max.y
        {
            return 0.0;
        }
        let w = self.rect.width() as usize;
        self.v[(y - self.rect.min.y) as usize * w + (x - self.rect.min.x) as usize]
    }

    /// Σ coverage (f64 — the analytic-area oracle side).
    pub fn total(&self) -> f64 {
        self.v.iter().map(|&c| c as f64).sum()
    }

    /// Core tile keys the plane touches at LOD 0 (§2.4 sparse-tile
    /// convention; the install seam iterates these against 256-px tiles).
    pub fn tile_coords(&self) -> Vec<TileCoord> {
        self.rect
            .tiles(TILE_I32)
            .map(|(x, y)| TileCoord { lod: 0, x, y })
            .collect()
    }
}

/// §20.2 step 4 [NORMATIVE] winding → alpha.
#[inline]
fn resolve(rule: FillRule, w: f64) -> f32 {
    match rule {
        FillRule::NonZero => w.abs().min(1.0) as f32,
        FillRule::EvenOdd => {
            let m = w.abs() % 2.0;
            (if m <= 1.0 { m } else { 2.0 - m }) as f32
        }
    }
}

/// Rasterize closed polylines (each implicitly closed last→first) with
/// exact per-edge trapezoid coverage. The shared engine under
/// [`rasterize_path`]; public so CP-1.0 boolean output (already flat) can
/// rasterize without re-flattening.
pub fn rasterize_polylines(polys: &[Vec<Vec2>], rule: FillRule) -> CoveragePlane {
    // Bounds over all vertices.
    let (mut lx, mut ly) = (f64::INFINITY, f64::INFINITY);
    let (mut hx, mut hy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for poly in polys {
        for p in poly {
            lx = lx.min(p.x);
            ly = ly.min(p.y);
            hx = hx.max(p.x);
            hy = hy.max(p.y);
        }
    }
    if !(lx.is_finite() && hx.is_finite()) {
        return CoveragePlane::new(IRect::EMPTY);
    }
    let rect = IRect::new(
        lx.floor() as i32,
        ly.floor() as i32,
        hx.ceil() as i32,
        hy.ceil() as i32,
    );
    if rect.is_empty() {
        return CoveragePlane::new(IRect::EMPTY);
    }
    let (w, h) = (rect.width() as usize, rect.height() as usize);

    // Dense cell rows (v1): `area` = partial coverage inside the crossing
    // cell; `cover` = full-height winding propagated to cells right of the
    // crossing, stored as a row-delta at column+1.
    let mut area = vec![0.0f64; w * h];
    let mut cover = vec![0.0f64; (w + 1) * h];

    for poly in polys {
        let n = poly.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let p0 = poly[i];
            let p1 = poly[(i + 1) % n]; // implicit close
            if p0.y == p1.y {
                continue; // horizontal edges carry no winding
            }
            let (top, bot) = (p0.y.min(p1.y), p0.y.max(p1.y));
            let y_start = (top.floor() as i32).max(rect.min.y);
            let y_end = (bot.ceil() as i32).min(rect.max.y);
            let inv_dy = 1.0 / (p1.y - p0.y);
            let dxdy = (p1.x - p0.x) * inv_dy;
            for py in y_start..y_end {
                let slab0 = (py as f64).max(top);
                let slab1 = (py as f64 + 1.0).min(bot);
                if slab1 <= slab0 {
                    continue;
                }
                // Slab endpoints in EDGE direction (dy signed by winding).
                let (ya, yb) = if p1.y > p0.y {
                    (slab0, slab1)
                } else {
                    (slab1, slab0)
                };
                let xa = p0.x + (ya - p0.y) * dxdy;
                let xb = p0.x + (yb - p0.y) * dxdy;
                let dy = yb - ya;
                let row = (py - rect.min.y) as usize;
                let (cx0, cx1) = (
                    (xa.min(xb).floor() as i32).max(rect.min.x),
                    (xa.max(xb).floor() as i32).min(rect.max.x - 1),
                );
                if cx0 > cx1 {
                    // Entirely left of the raster range: full cover to the
                    // whole row (right of column 0).
                    let col =
                        ((xa.min(xb).floor() as i32) - rect.min.x).clamp(0, w as i32) as usize;
                    cover[row * (w + 1) + col] += dy;
                    continue;
                }
                let x_of_t = |t: f64| xa + (xb - xa) * t;
                // Split the sub-segment at column borders (cell walk).
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
                    // Mean x inside this cell, in cell units 0..1 — exact
                    // trapezoid: covered fraction right of the edge.
                    let xm = 0.5 * (x_of_t(prev_t) + x_of_t(t1)) - col as f64;
                    let c = (col - rect.min.x) as usize;
                    area[row * w + c] += seg_dy * (1.0 - xm.clamp(0.0, 1.0));
                    cover[row * (w + 1) + c + 1] += seg_dy;
                    prev_t = t1;
                }
            }
        }
    }

    // §20.2 step 3: left→right winding sweep per scanline; step 4 resolve.
    let mut out = CoveragePlane::new(rect);
    for row in 0..h {
        let mut acc = 0.0f64;
        for col in 0..w {
            acc += cover[row * (w + 1) + col];
            out.v[row * w + col] = resolve(rule, acc + area[row * w + col]);
        }
    }
    out
}

/// §20.2 fill rasterization: flatten at `tol` (0.1 px for display — pass
/// [`crate::flatten::TOL_DISPLAY`]), then exact-coverage scanline.
pub fn rasterize_path(path: &VectorPath, tol: f64) -> CoveragePlane {
    // Open subpaths fill as if closed (implicit last→first edge — the
    // §13.3 auto-close convention shared with the lasso engines).
    let polys = flatten_path(path, tol);
    rasterize_polylines(&polys, path.fill_rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds::tests_support::circle_path;
    use crate::flatten::TOL_DISPLAY;
    use crate::path::{CubicSeg, SubPath};
    use appname_core::PathId;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    /// f64 shoelace area of a closed polygon (§20.3 numerical policy:
    /// orientation/area by the f64 shoelace sum) — the analytic oracle.
    fn shoelace(pts: &[Vec2]) -> f64 {
        let n = pts.len();
        let mut s = 0.0;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            s += a.x * b.y - b.x * a.y;
        }
        s * 0.5
    }

    fn poly_path(pts: &[Vec2], rule: FillRule) -> VectorPath {
        let mut path = VectorPath::new(PathId::nil(), rule);
        let n = pts.len();
        path.subpaths.push(SubPath {
            closed: true,
            segments: (0..n)
                .map(|i| CubicSeg::line(pts[i], pts[(i + 1) % n]))
                .collect(),
        });
        path
    }

    /// Oracle tolerance: the trapezoid accumulation is analytic in f64,
    /// but the OUTPUT plane stores f32 (§13.1 mask convention), so
    /// Σcoverage carries ~2⁻²⁴ quantization per partially-covered
    /// boundary pixel — the observable bound scales with the perimeter.
    fn f32_oracle_bound(pts: &[Vec2]) -> f64 {
        let n = pts.len();
        let perim: f64 = (0..n).map(|i| pts[i].distance(pts[(i + 1) % n])).sum();
        1e-9 + (perim + 4.0) * 1.5e-7
    }

    #[test]
    fn sum_coverage_equals_shoelace_for_simple_polygons() {
        // Axis rect, sub-pixel offsets.
        let rect = [v(0.25, 0.75), v(3.75, 0.75), v(3.75, 2.0), v(0.25, 2.0)];
        let cov = rasterize_path(&poly_path(&rect, FillRule::NonZero), TOL_DISPLAY);
        assert!((cov.total() - shoelace(&rect).abs()).abs() < f32_oracle_bound(&rect));

        // Triangle with irrational-ish coordinates.
        let tri = [v(0.3, 0.1), v(7.9, 2.3), v(2.2, 6.8)];
        let cov = rasterize_path(&poly_path(&tri, FillRule::NonZero), TOL_DISPLAY);
        assert!((cov.total() - shoelace(&tri).abs()).abs() < f32_oracle_bound(&tri));

        // Both orientations (winding sign must not change coverage).
        let tri_cw: Vec<Vec2> = tri.iter().rev().cloned().collect();
        let cov_cw = rasterize_path(&poly_path(&tri_cw, FillRule::NonZero), TOL_DISPLAY);
        assert!((cov_cw.total() - cov.total()).abs() < f32_oracle_bound(&tri));
    }

    #[test]
    fn sum_coverage_equals_shoelace_seeded_random_polygons() {
        // Fixed-seed star-convex polygons (simple by construction):
        // random radii on a sorted angle fan around a center.
        let mut s: u64 = 0xDEADBEEFCAFED00D;
        let mut nextf = move || {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
        };
        for case in 0..50 {
            let n = 3 + (nextf() * 9.0) as usize;
            let (cx, cy) = (nextf() * 40.0, nextf() * 40.0);
            let pts: Vec<Vec2> = (0..n)
                .map(|i| {
                    let ang = core::f64::consts::TAU * (i as f64 + 0.5 * nextf()) / n as f64;
                    let r = 1.0 + nextf() * 15.0;
                    v(cx + r * ang.cos(), cy + r * ang.sin())
                })
                .collect();
            let oracle = shoelace(&pts).abs();
            let bound = f32_oracle_bound(&pts);
            let cov = rasterize_path(&poly_path(&pts, FillRule::NonZero), TOL_DISPLAY);
            assert!(
                (cov.total() - oracle).abs() < bound,
                "case {case}: Σcov {} vs shoelace {oracle} (bound {bound})",
                cov.total()
            );
            // EvenOdd equals NonZero on simple polygons.
            let cov_eo = rasterize_path(&poly_path(&pts, FillRule::EvenOdd), TOL_DISPLAY);
            assert!((cov_eo.total() - oracle).abs() < bound);
        }
    }

    #[test]
    fn winding_rules_concentric_squares() {
        // Outer 10×10 square + inner 4×4 square, SAME orientation:
        // winding 2 inside the inner square.
        let outer = [v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0), v(0.0, 10.0)];
        let inner = [v(3.0, 3.0), v(7.0, 3.0), v(7.0, 7.0), v(3.0, 7.0)];
        let mut path = poly_path(&outer, FillRule::NonZero);
        path.subpaths
            .push(poly_path(&inner, FillRule::NonZero).subpaths.remove(0));

        // NonZero: |2| clamps to 1 → full outer area.
        let nz = rasterize_path(&path, TOL_DISPLAY);
        assert!((nz.total() - 100.0).abs() < 1e-9, "nonzero {}", nz.total());
        assert_eq!(nz.get(5, 5), 1.0);

        // EvenOdd: m = 2 → alpha 0 inside → annulus 100 − 16 = 84.
        path.fill_rule = FillRule::EvenOdd;
        let eo = rasterize_path(&path, TOL_DISPLAY);
        assert!((eo.total() - 84.0).abs() < 1e-9, "evenodd {}", eo.total());
        assert_eq!(eo.get(5, 5), 0.0);
        assert_eq!(eo.get(1, 5), 1.0);
    }

    #[test]
    fn winding_rules_reversed_hole_nonzero_annulus() {
        // Inner square REVERSED: winding 1 − 1 = 0 inside → annulus under
        // BOTH rules (the vector-mask donut case, §11.2).
        let outer = [v(0.0, 0.0), v(10.0, 0.0), v(10.0, 10.0), v(0.0, 10.0)];
        let inner_rev = [v(3.0, 7.0), v(7.0, 7.0), v(7.0, 3.0), v(3.0, 3.0)];
        let mut path = poly_path(&outer, FillRule::NonZero);
        path.subpaths
            .push(poly_path(&inner_rev, FillRule::NonZero).subpaths.remove(0));
        let nz = rasterize_path(&path, TOL_DISPLAY);
        assert!((nz.total() - 84.0).abs() < 1e-9);
        assert_eq!(nz.get(5, 5), 0.0);
        path.fill_rule = FillRule::EvenOdd;
        let eo = rasterize_path(&path, TOL_DISPLAY);
        assert!((eo.total() - 84.0).abs() < 1e-9);
    }

    #[test]
    fn bowtie_self_intersection_covers_both_lobes() {
        // Crossing quad: two unit-area triangle lobes with winding ±1;
        // both rules fill both lobes (|w| = 1 each side of the crossing).
        let bow = [v(0.0, 0.0), v(2.0, 0.0), v(0.0, 2.0), v(2.0, 2.0)];
        let nz = rasterize_path(&poly_path(&bow, FillRule::NonZero), TOL_DISPLAY);
        assert!((nz.total() - 2.0).abs() < 1e-9, "bowtie nz {}", nz.total());
        let eo = rasterize_path(&poly_path(&bow, FillRule::EvenOdd), TOL_DISPLAY);
        assert!((eo.total() - 2.0).abs() < 1e-9, "bowtie eo {}", eo.total());
    }

    #[test]
    fn circle_coverage_matches_area_within_flatten_bound() {
        // Σcov vs πr²: flatten tolerance ε bounds the polyline's deviation
        // from the curve, so the area error is ≤ perimeter · ε (+ κ-cubic
        // circle approximation ~2.8e-4·r on the radius).
        for (r, tol) in [(8.0, 0.1), (8.0, 0.05), (20.0, 0.1)] {
            let path = circle_path(30.0, 30.0, r);
            let cov = rasterize_path(&path, tol);
            let expect = core::f64::consts::PI * r * r;
            let bound = 2.0 * core::f64::consts::PI * r * (tol + 3e-4 * r);
            let err = (cov.total() - expect).abs();
            assert!(err <= bound, "r={r} tol={tol}: err {err} > bound {bound}");
        }
    }

    #[test]
    fn coverage_values_in_unit_range_and_deterministic() {
        let path = circle_path(5.0, 5.0, 4.5);
        let a = rasterize_path(&path, TOL_DISPLAY);
        assert!(a.v.iter().all(|&c| (0.0..=1.0).contains(&c)));
        let b = rasterize_path(&path, TOL_DISPLAY);
        assert_eq!(a, b); // bit-identical double run (determinism class D)
    }

    #[test]
    fn subpixel_rect_cell_values_match_analytic_cells() {
        // Half-pixel offset 2×2 square: corners 0.25, edges 0.5, center 1
        // (the §13.3 marquee worked case — consistency with the tools
        // lane's rasterize_rect oracle).
        let rect = [v(0.5, 0.5), v(2.5, 0.5), v(2.5, 2.5), v(0.5, 2.5)];
        let cov = rasterize_path(&poly_path(&rect, FillRule::NonZero), TOL_DISPLAY);
        assert_eq!(cov.rect, IRect::new(0, 0, 3, 3));
        assert!((cov.get(0, 0) - 0.25).abs() < 1e-6);
        assert!((cov.get(1, 0) - 0.5).abs() < 1e-6);
        assert!((cov.get(1, 1) - 1.0).abs() < 1e-6);
        assert!((cov.get(2, 2) - 0.25).abs() < 1e-6);
        assert!((cov.total() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn tile_coords_cover_the_plane() {
        let rect = [
            v(-10.0, -10.0),
            v(300.0, -10.0),
            v(300.0, 300.0),
            v(-10.0, 300.0),
        ];
        let cov = rasterize_path(&poly_path(&rect, FillRule::NonZero), TOL_DISPLAY);
        let tiles = cov.tile_coords();
        // (-1..2) × (-1..2) at 256-px tiles = 9 (core §2.1 worked cases).
        assert_eq!(tiles.len(), 9);
        assert_eq!(
            tiles[0],
            TileCoord {
                lod: 0,
                x: -1,
                y: -1
            }
        );
    }
}
