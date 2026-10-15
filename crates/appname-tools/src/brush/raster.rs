// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.5 (GPU stamping) — the CPU mirror of the `cs_stamp` WGSL,
//! op-for-op in f32 ("identical math"; the golden-reference discipline of
//! §16.4 applies to the brush too, and this CPU path is the §34.3 parity
//! oracle for the CP-0.5 GPU pipeline).
//!
//! [NORMATIVE §15.5 step 2] Dab accumulation into the stroke scratch:
//!
//! ```text
//! a     = flow · t(p) · (1 − S.cov)     // screen accumulation
//! S.cov = S.cov + a
//! ```
//!
//! Tip coverage (`tip_coverage` + the analytic 1-px AA term):
//!
//! ```text
//! l  = rotate(−angle) · (p − center) · inv_radius;  l.y /= roundness
//! r  = |l|
//! t  = 1 − smoothstep(hardness, 1, r)
//! aa = clamp((1 − r)/max(inv_radius, 1e-6) + 0.5, 0, 1)
//! t  = min(t, aa)
//! ```
//!
//! v1 pins (module docs `brush`): constant-color stroke — §15.5's own
//! statement "for a constant-color brush `S.color` is the paint color
//! everywhere it is defined" lets v1 carry coverage only; the `S.colw`
//! accumulator arrives with per-dab-color tools (clone/smudge). Dabs
//! accumulate in march order (bit-frozen; the WGSL per-pixel loop iterates
//! dabs in the same order). `hardness == 1` short-circuits the degenerate
//! `smoothstep(1, 1, r)` to the hard-disk limit `r < 1 → 1`.

use std::collections::BTreeMap;

use appname_core::{IRect, IVec2, TileCoord, TILE, TILE_I32};

use crate::brush::plan::StrokePlan;

/// Per-stroke scratch buffer `S` (§15.5 step 1), v1: coverage only.
/// Tiles are keyed `(y, x)` so every iteration is deterministic.
pub struct StrokeScratch {
    clip: IRect,
    /// Union of dab pixel rects ∩ clip (the contract dirty basis).
    bounds: IRect,
    tiles: BTreeMap<(i32, i32), Box<[f32]>>,
}

impl StrokeScratch {
    /// Coverage at a document pixel (0 outside touched tiles/clip).
    pub fn coverage_at(&self, p: IVec2) -> f32 {
        let (tx, ty) = (p.x.div_euclid(TILE_I32), p.y.div_euclid(TILE_I32));
        match self.tiles.get(&(ty, tx)) {
            Some(cov) => {
                let (px, py) = (
                    p.x.rem_euclid(TILE_I32) as usize,
                    p.y.rem_euclid(TILE_I32) as usize,
                );
                cov[py * TILE + px]
            }
            None => 0.0,
        }
    }

    /// Touched tile coords, sorted by `(y, x)` — the deterministic
    /// presentation order.
    pub fn touched_tiles(&self) -> Vec<TileCoord> {
        self.tiles
            .keys()
            .map(|&(ty, tx)| TileCoord::new(0, tx, ty))
            .collect()
    }

    /// Raw coverage plane of one touched tile (row-major `TILE × TILE`).
    pub fn tile_coverage(&self, c: TileCoord) -> Option<&[f32]> {
        self.tiles.get(&(c.y, c.x)).map(|b| &b[..])
    }

    /// The clip rect the stroke was rasterized against.
    pub fn clip(&self) -> IRect {
        self.clip
    }

    /// Union of dab pixel rects ∩ clip (empty when nothing was stamped).
    pub fn bounds(&self) -> IRect {
        self.bounds
    }
}

/// WGSL `smoothstep` mirror (e0 < e1 guaranteed by the `hardness == 1`
/// short-circuit at the call site).
#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Tip coverage `t(p)` of ONE dab at ONE document pixel — the §15.5
/// `tip_coverage` + analytic-AA math, extracted verbatim from the
/// [`rasterize_plan`] inner loop so the CP-1.0 retouch tools (per-dab
/// evaluation order, §21.5) evaluate THE same formula bit-for-bit
/// (single-source rule; the loop below calls this same function).
#[inline]
pub fn dab_coverage_at(dab: &crate::brush::plan::Dab, p: IVec2) -> f32 {
    let inv_radius = 2.0f32 / dab.diameter;
    let (cos_a, sin_a) = if dab.angle_rad == 0.0 {
        (1.0f32, 0.0f32) // exact fast path, no libm (module pin)
    } else {
        (dab.angle_rad.cos(), dab.angle_rad.sin())
    };
    dab_coverage_inner(
        dab,
        p.x as f32 + 0.5,
        p.y as f32 + 0.5,
        inv_radius,
        cos_a,
        sin_a,
        1.0f32 / dab.roundness,
        dab.center[0] as f32,
        dab.center[1] as f32,
        dab.hardness >= 1.0,
    )
}

/// The shared per-pixel core (per-dab constants hoisted by the callers —
/// identical math either way).
#[allow(clippy::too_many_arguments)]
#[inline]
fn dab_coverage_inner(
    dab: &crate::brush::plan::Dab,
    pxf: f32,
    pyf: f32,
    inv_radius: f32,
    cos_a: f32,
    sin_a: f32,
    inv_roundness: f32,
    cxf: f32,
    cyf: f32,
    hard: bool,
) -> f32 {
    let lx = (pxf - cxf) * inv_radius;
    let ly = (pyf - cyf) * inv_radius;
    // rotate by −angle.
    let rx = cos_a * lx + sin_a * ly;
    let ry = (-sin_a * lx + cos_a * ly) * inv_roundness;
    let r = (rx * rx + ry * ry).sqrt();
    let t = if r >= 1.0 {
        0.0
    } else if hard {
        1.0
    } else {
        1.0 - smoothstep(dab.hardness, 1.0, r)
    };
    // Analytic 1-px AA on hard rims (non-pencil v1).
    let aa = ((1.0 - r) / inv_radius.max(1e-6) + 0.5).clamp(0.0, 1.0);
    t.min(aa)
}

/// §15.5 steps 1–3: rasterize the planned dabs into a fresh scratch,
/// clipped to `clip` (the canvas/surface rect O-CORE passes; out-of-bounds
/// dab area is dropped here and only here).
pub fn rasterize_plan(plan: &StrokePlan, clip: IRect) -> StrokeScratch {
    let mut scratch = StrokeScratch {
        clip,
        bounds: IRect::EMPTY,
        tiles: BTreeMap::new(),
    };
    for dab in &plan.dabs {
        let rect = dab.pixel_rect().intersect(&clip);
        if rect.is_empty() {
            continue;
        }
        scratch.bounds = scratch.bounds.union(&rect);

        // Shader-mirror per-dab constants (f32, §15.5 `Dab` block).
        let inv_radius = 2.0f32 / dab.diameter;
        let (cos_a, sin_a) = if dab.angle_rad == 0.0 {
            (1.0f32, 0.0f32) // exact fast path, no libm (module pin)
        } else {
            (dab.angle_rad.cos(), dab.angle_rad.sin())
        };
        let inv_roundness = 1.0f32 / dab.roundness;
        let cxf = dab.center[0] as f32;
        let cyf = dab.center[1] as f32;
        let hard = dab.hardness >= 1.0;

        for (tx, ty) in rect.tiles(TILE_I32) {
            let coord = TileCoord::new(0, tx, ty);
            let tile_rect = coord.pixel_rect();
            let span = rect.intersect(&tile_rect);
            if span.is_empty() {
                continue;
            }
            let cov = scratch
                .tiles
                .entry((ty, tx))
                .or_insert_with(|| vec![0.0f32; TILE * TILE].into_boxed_slice());
            for y in span.min.y..span.max.y {
                let py = (y - tile_rect.min.y) as usize;
                let row = &mut cov[py * TILE..(py + 1) * TILE];
                for x in span.min.x..span.max.x {
                    let px = (x - tile_rect.min.x) as usize;
                    // p = pixel center, document space (§15.5: tile_org +
                    // gid + 0.5).
                    let pxf = x as f32 + 0.5;
                    let pyf = y as f32 + 0.5;
                    let t = dab_coverage_inner(
                        dab,
                        pxf,
                        pyf,
                        inv_radius,
                        cos_a,
                        sin_a,
                        inv_roundness,
                        cxf,
                        cyf,
                        hard,
                    );
                    if t > 0.0 {
                        let c = row[px];
                        row[px] = c + dab.flow * t * (1.0 - c); // NORMATIVE screen increment
                    }
                }
            }
        }
    }
    scratch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::plan::Dab;

    fn dab(center: [f64; 2], diameter: f32, hardness: f32, flow: f32) -> Dab {
        Dab {
            center,
            diameter,
            angle_rad: 0.0,
            roundness: 1.0,
            hardness,
            flow,
            opacity: 1.0,
        }
    }

    fn plan_of(dabs: Vec<Dab>) -> StrokePlan {
        let mut bounds = IRect::EMPTY;
        for d in &dabs {
            bounds = bounds.union(&d.pixel_rect());
        }
        StrokePlan {
            dabs,
            bounds,
            base_diameter: 0.0,
        }
    }

    const BIG: IRect = IRect::new(-4096, -4096, 4096, 4096);

    #[test]
    fn hard_dab_center_is_opaque_and_outside_is_zero() {
        // Center at a pixel center: r = 0 → t = 1 (AA = clamp(radius+0.5)=1).
        let p = plan_of(vec![dab([10.5, 10.5], 10.0, 1.0, 1.0)]);
        let s = rasterize_plan(&p, BIG);
        assert_eq!(s.coverage_at(IVec2::new(10, 10)), 1.0);
        // 3 px inside the rim on the +x axis: pixel (12,10) center (12.5,10.5),
        // distance 2 → r=0.4 → hard full coverage.
        assert_eq!(s.coverage_at(IVec2::new(12, 10)), 1.0);
        // Well outside: distance 7 → r=1.4, AA ≤ 0.
        assert_eq!(s.coverage_at(IVec2::new(17, 10)), 0.0);
        assert_eq!(s.coverage_at(IVec2::new(100, 100)), 0.0);
    }

    #[test]
    fn hard_rim_gets_the_analytic_one_px_aa_ramp() {
        // Radius 5 dab at (10.5,10.5). Pixel (15,10): center (15.5,10.5),
        // dist 5 → r = 1.0 → t_cov = 0, aa = clamp(0 + 0.5) = 0.5 → min = 0.
        // Pixel (14,10): dist 4 → r = 0.8, aa = clamp(0.2·5 + 0.5) = 1 → t=1.
        // The half-covered rim pixel is dist 4.5..5.5; take (15,10) exact:
        // computed t = min(t_cov(1.0)=0, 0.5) = 0 — rim pixel at exactly r=1
        // is the shader's own result. One-in from that, dist 4.6 is not on
        // the integer grid here, so assert the ramp pixel between full and
        // empty instead:
        let p = plan_of(vec![dab([10.0, 10.5], 10.0, 1.0, 1.0)]);
        let s = rasterize_plan(&p, BIG);
        // pixel (14,10) center (14.5,10.5): dist 4.5 → r=0.9 → t_cov=1 (hard),
        // aa = clamp((1−0.9)·5 + 0.5) = 1 → 1 (f32 rounding of r allowed).
        assert!((s.coverage_at(IVec2::new(14, 10)) - 1.0).abs() < 1e-5);
        // pixel (15,10) center (15.5,10.5): dist 5.5 → r=1.1 → 0 (outside).
        assert_eq!(s.coverage_at(IVec2::new(15, 10)), 0.0);
        // pixel (14,10) vs a slightly-shifted dab: put rim mid-pixel.
        let p = plan_of(vec![dab([10.25, 10.5], 10.0, 1.0, 1.0)]);
        let _ = rasterize_plan(&p, BIG); // shifted-rim narration below uses the 9.5 px dab
                                         // pixel (15,10) center (15.5,10.5): dist 5.25 → r = 1.05 → t_cov 0,
                                         // aa = clamp(−0.05·5 + 0.5) = 0.25 — but min(t,aa) with t=0 is 0.
                                         // The AA ramp lives where r slightly < 1: pixel (14,10) dist 4.25 →
                                         // r=0.85 → full. So exercise a sub-pixel rim: diameter 9.5 at
                                         // (10.0,10.5): pixel (14,10) dist 4.5 → r ≈ 0.947, t_cov = 1 (hard),
                                         // aa = clamp((1−0.947)·4.75+0.5) = 0.752 → partial coverage.
        let p = plan_of(vec![dab([10.0, 10.5], 9.5, 1.0, 1.0)]);
        let s = rasterize_plan(&p, BIG);
        let c = s.coverage_at(IVec2::new(14, 10));
        assert!(
            c > 0.7 && c < 0.8,
            "AA rim pixel partially covered, got {c}"
        );
    }

    #[test]
    fn soft_falloff_matches_the_smoothstep_formula() {
        // hardness 0.5, radius 8, dab at pixel-center (20.5, 20.5).
        let h = 0.5f32;
        let p = plan_of(vec![dab([20.5, 20.5], 16.0, h, 1.0)]);
        let s = rasterize_plan(&p, BIG);
        for (px, dist) in [(20i32, 0.0f32), (23, 3.0), (25, 5.0), (27, 7.0)] {
            let r = dist / 8.0;
            let want = if r >= 1.0 {
                0.0
            } else {
                1.0 - smoothstep(h, 1.0, r)
            };
            let aa = ((1.0 - r) / (1.0 / 8.0f32) + 0.5).clamp(0.0, 1.0);
            let want = want.min(aa);
            let got = s.coverage_at(IVec2::new(px, 20));
            assert!((got - want).abs() < 1e-6, "r={r}: got {got} want {want}");
        }
    }

    #[test]
    fn screen_accumulation_is_the_closed_form() {
        // Two identical dabs, flow 0.5: cov = 1 − (1 − 0.5·t)².
        let d = dab([10.5, 10.5], 10.0, 1.0, 0.5);
        let p = plan_of(vec![d, d]);
        let s = rasterize_plan(&p, BIG);
        let center = s.coverage_at(IVec2::new(10, 10)); // t = 1
        assert!(
            (center - 0.75).abs() < 1e-6,
            "1−(1−0.5)² = 0.75, got {center}"
        );
        // Five dabs: 1 − 0.5⁵.
        let p = plan_of(vec![d; 5]);
        let s = rasterize_plan(&p, BIG);
        let center = s.coverage_at(IVec2::new(10, 10));
        assert!(
            (center - (1.0 - 0.5f32.powi(5))).abs() < 1e-6,
            "got {center}"
        );
        // Coverage never exceeds 1 no matter how many dabs.
        let p = plan_of(vec![d; 64]);
        let s = rasterize_plan(&p, BIG);
        assert!(s.coverage_at(IVec2::new(10, 10)) <= 1.0);
    }

    #[test]
    fn roundness_squashes_the_minor_axis() {
        let mut d = dab([50.5, 50.5], 20.0, 1.0, 1.0);
        d.roundness = 0.5;
        let p = plan_of(vec![d]);
        let s = rasterize_plan(&p, BIG);
        // Major axis (x): r at dist 8 = 0.8 → covered.
        assert_eq!(s.coverage_at(IVec2::new(58, 50)), 1.0);
        // Minor axis (y): dist 8 → ry = 0.8·2 = 1.6 → zero.
        assert_eq!(s.coverage_at(IVec2::new(50, 58)), 0.0);
        // Minor axis inside the squashed rim: dist 4 → ry = 0.8 → covered.
        assert_eq!(s.coverage_at(IVec2::new(50, 54)), 1.0);
    }

    #[test]
    fn angle_rotates_the_ellipse() {
        use core::f32::consts::FRAC_PI_2;
        let mut d = dab([50.5, 50.5], 20.0, 1.0, 1.0);
        d.roundness = 0.5;
        d.angle_rad = FRAC_PI_2; // minor axis now along x
        let p = plan_of(vec![d]);
        let s = rasterize_plan(&p, BIG);
        assert_eq!(
            s.coverage_at(IVec2::new(50, 58)),
            1.0,
            "major axis along y now"
        );
        assert_eq!(
            s.coverage_at(IVec2::new(58, 50)),
            0.0,
            "minor axis along x now"
        );
    }

    #[test]
    fn clip_drops_out_of_bounds_area_and_tiles() {
        let clip = IRect::new(0, 0, 64, 64);
        // Dab fully outside.
        let p = plan_of(vec![dab([200.5, 200.5], 20.0, 1.0, 1.0)]);
        let s = rasterize_plan(&p, clip);
        assert!(s.touched_tiles().is_empty());
        assert!(s.bounds().is_empty());
        // Dab straddling the clip edge: outside pixels stay zero.
        let p = plan_of(vec![dab([63.5, 32.5], 20.0, 1.0, 1.0)]);
        let s = rasterize_plan(&p, clip);
        assert!(s.coverage_at(IVec2::new(60, 32)) > 0.0);
        assert_eq!(s.coverage_at(IVec2::new(64, 32)), 0.0, "beyond clip");
        assert!(s.bounds().max.x <= 64);
    }

    #[test]
    fn tiles_are_touched_sparsely_and_deterministically() {
        // A dab overlapping tile (0,0) and (1,0) only (y placed so the
        // conservative +1 AA rect stays inside tile row 0).
        let p = plan_of(vec![dab([255.5, 12.5], 20.0, 1.0, 1.0)]);
        let s = rasterize_plan(&p, BIG);
        let tiles = s.touched_tiles();
        assert_eq!(tiles.len(), 2);
        assert_eq!((tiles[0].x, tiles[0].y), (0, 0));
        assert_eq!((tiles[1].x, tiles[1].y), (1, 0));
        // Coverage continuity across the tile seam.
        let a = s.coverage_at(IVec2::new(255, 12));
        let b = s.coverage_at(IVec2::new(256, 12));
        assert!(a > 0.0 && b > 0.0);
    }

    #[test]
    fn negative_document_space_rasterizes() {
        let p = plan_of(vec![dab([-10.5, -10.5], 10.0, 1.0, 1.0)]);
        let s = rasterize_plan(&p, BIG);
        assert_eq!(s.coverage_at(IVec2::new(-11, -11)), 1.0);
        let tiles = s.touched_tiles();
        assert_eq!(tiles.len(), 1);
        assert_eq!((tiles[0].x, tiles[0].y), (-1, -1));
    }
}
