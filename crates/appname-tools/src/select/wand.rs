// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.3 Magic wand [NORMATIVE] — metric and AA weight:
//!
//! ```text
//! d(p) = 255 · max(|R_p − R_s|, |G_p − G_s|, |B_p − B_s|)      // working-space floats vs seed s
//! w(p) = 1 − smoothstep(tolerance − 0.5, tolerance + 0.5, d(p)) // 1-unit AA band
//! ```
//!
//! plus §22.6 [NORMATIVE]: the paint-bucket's acceptance predicate "and its
//! anti-aliased weight are the same function as the magic wand's … bucket
//! and wand MUST agree on the region for identical inputs (shared
//! implementation, tested)" — [`wand_weight`]/[`flood_weights`] ARE that
//! shared function, exported for core's `fill{content:…, seed}` flood
//! variant. Flood machinery per §22.6: span-stack scanline
//! (`Span {x1, x2, y, dy}`), one visit per pixel, no recursion; §13.3:
//! 4-way default (cannot leak through diagonal corners), 8-way by option;
//! `contiguous:false` evaluates the predicate over every canvas pixel;
//! `antialias:false` snaps `w` to the `w ≥ 0.5` binary; `sampleAllLayers`
//! chooses the SOURCE (layer vs composite) — the caller passes the chosen
//! reader, and the seed color obeys the same sampling rule by construction.
//!
//! v1 pin (module `select` docs): the metric reads STRAIGHT color —
//! premultiplied sources un-premultiply (`c/a` where `a > 0`, else 0)
//! before the compare, mirroring the §19.3 trim rule; §13.3 says only
//! "working-space floats". Logged as a spec question.

use appname_core::{IRect, IVec2};

use crate::brush::present::Backdrop;

use super::MaskPlane;

/// Wand/bucket parameters (contract `selection.set.yaml` wand rows).
#[derive(Clone, Copy, Debug)]
pub struct WandParams {
    /// 0..=255, default 32.
    pub tolerance: f32,
    /// Default true; false = whole-canvas predicate map.
    pub contiguous: bool,
    /// §13.3 tool option; 4-way is the wire default (no pinned wire key —
    /// contract gap note).
    pub eight_way: bool,
    /// Default true; false snaps `w` at 0.5.
    pub antialias: bool,
}

impl Default for WandParams {
    fn default() -> WandParams {
        WandParams {
            tolerance: 32.0,
            contiguous: true,
            eight_way: false,
            antialias: true,
        }
    }
}

/// The §13.3/§22.6 shared per-pixel weight.
#[inline]
pub fn wand_weight(pixel: [f32; 3], seed: [f32; 3], tolerance: f32) -> f32 {
    let d = 255.0
        * (pixel[0] - seed[0])
            .abs()
            .max((pixel[1] - seed[1]).abs())
            .max((pixel[2] - seed[2]).abs());
    1.0 - smoothstep(tolerance - 0.5, tolerance + 0.5, d)
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn straight_rgb(v: [f32; 4], premultiplied: bool) -> [f32; 3] {
    if premultiplied && v[3] > 0.0 {
        [v[0] / v[3], v[1] / v[3], v[2] / v[3]]
    } else {
        [v[0], v[1], v[2]]
    }
}

/// §22.6 span-stack record.
struct Span {
    x1: i32,
    x2: i32,
    y: i32,
    dy: i32,
}

/// Compute the wand selection coverage over `canvas` from `seed`.
/// `src` is the chosen sampling source (target layer or flattened
/// composite per `sampleAllLayers` — caller's choice); `premultiplied`
/// describes its storage. The caller validates the seed against the canvas
/// (contract live-state rule) — out-of-canvas seeds here yield an empty
/// plane.
pub fn wand_select<B: Backdrop + ?Sized>(
    src: &B,
    premultiplied: bool,
    canvas: IRect,
    seed: IVec2,
    params: &WandParams,
) -> MaskPlane {
    if !canvas.contains(seed) || canvas.is_empty() {
        return MaskPlane::new(IRect::EMPTY);
    }
    let seed_rgb = straight_rgb(src.read_pixel(seed), premultiplied);
    let weight = |p: IVec2| -> f32 {
        let w = wand_weight(
            straight_rgb(src.read_pixel(p), premultiplied),
            seed_rgb,
            params.tolerance,
        );
        if params.antialias {
            w
        } else if w >= 0.5 {
            1.0
        } else {
            0.0
        }
    };

    let mut out = MaskPlane::new(canvas);
    if !params.contiguous {
        // §13.3: evaluate w for every canvas pixel (data-parallel map).
        for y in canvas.min.y..canvas.max.y {
            for x in canvas.min.x..canvas.max.x {
                out.set(x, y, weight(IVec2::new(x, y)));
            }
        }
        return out;
    }

    // §22.6 span-stack flood through pixels with w > 0, one visit each.
    let w_us = canvas.width() as usize;
    let idx = |p: IVec2| -> usize {
        ((p.y - canvas.min.y) as usize) * w_us + (p.x - canvas.min.x) as usize
    };
    let mut visited = vec![false; w_us * canvas.height() as usize];
    let passes = |p: IVec2, out: &mut MaskPlane, visited: &mut [bool]| -> bool {
        if !canvas.contains(p) || visited[idx(p)] {
            return false;
        }
        visited[idx(p)] = true;
        let w = weight(p);
        if w > 0.0 {
            out.set(p.x, p.y, w);
            true
        } else {
            false
        }
    };

    let mut stack: Vec<Span> = Vec::new();
    // Seed span: expand left/right from the seed on its row.
    let mut x1 = seed.x;
    let mut x2 = seed.x;
    if !passes(seed, &mut out, &mut visited) {
        return out; // seed itself outside the region (w = 0)
    }
    while x1 > canvas.min.x && passes(IVec2::new(x1 - 1, seed.y), &mut out, &mut visited) {
        x1 -= 1;
    }
    while x2 + 1 < canvas.max.x && passes(IVec2::new(x2 + 1, seed.y), &mut out, &mut visited) {
        x2 += 1;
    }
    stack.push(Span {
        x1,
        x2,
        y: seed.y,
        dy: -1,
    });
    stack.push(Span {
        x1,
        x2,
        y: seed.y,
        dy: 1,
    });

    while let Some(span) = stack.pop() {
        let y = span.y + span.dy;
        if y < canvas.min.y || y >= canvas.max.y {
            continue;
        }
        // 8-way widens the parent span by one pixel each side (§13.3
        // diagonal connectivity).
        let ext = if params.eight_way { 1 } else { 0 };
        let scan_from = (span.x1 - ext).max(canvas.min.x);
        let scan_to = (span.x2 + ext).min(canvas.max.x - 1);
        let mut x = scan_from;
        while x <= scan_to {
            // Find the start of an accepted run.
            if !passes(IVec2::new(x, y), &mut out, &mut visited) {
                x += 1;
                continue;
            }
            let mut run1 = x;
            while run1 > canvas.min.x && passes(IVec2::new(run1 - 1, y), &mut out, &mut visited) {
                run1 -= 1;
            }
            let mut run2 = x;
            while run2 + 1 < canvas.max.x && passes(IVec2::new(run2 + 1, y), &mut out, &mut visited)
            {
                run2 += 1;
            }
            // Child continues; turn-around spans for the overhangs.
            stack.push(Span {
                x1: run1,
                x2: run2,
                y,
                dy: span.dy,
            });
            if run1 < span.x1 - ext {
                stack.push(Span {
                    x1: run1,
                    x2: span.x1 - ext - 1,
                    y,
                    dy: -span.dy,
                });
            }
            if run2 > span.x2 + ext {
                stack.push(Span {
                    x1: span.x2 + ext + 1,
                    x2: run2,
                    y,
                    dy: -span.dy,
                });
            }
            x = run2 + 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::present::UniformBackdrop;
    use crate::resample::{EdgePolicy, Plane};

    /// Backdrop over a Plane for synthetic images.
    struct Img(Plane);
    impl Backdrop for Img {
        fn read_pixel(&self, p: IVec2) -> [f32; 4] {
            self.0.get(p.x, p.y, EdgePolicy::Transparent)
        }
    }

    fn two_region_img() -> Img {
        // 16×8: left half red, right half blue, all opaque premul.
        let mut p = Plane::new(IRect::new(0, 0, 16, 8));
        for y in 0..8 {
            for x in 0..16 {
                let c = if x < 8 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                p.set(x, y, c);
            }
        }
        Img(p)
    }

    #[test]
    fn weight_formula_matches_the_spec_values() {
        // Identical color: d=0 → w = 1 − smoothstep(tol−0.5, tol+0.5, 0).
        // tol=32 → clamped below e0 → w = 1.
        assert_eq!(wand_weight([0.5; 3], [0.5; 3], 32.0), 1.0);
        // d exactly tolerance → t = 0.5 → smoothstep = 0.5 → w = 0.5.
        let seed = [0.0; 3];
        let px = [32.0 / 255.0, 0.0, 0.0];
        let w = wand_weight(px, seed, 32.0);
        assert!((w - 0.5).abs() < 1e-4, "band midpoint, got {w}");
        // Past the band → 0; MAX-per-channel (not Euclidean): a (33, 0, 0)
        // delta at tol 32 is outside even though the Euclidean/mean would
        // pass a naive metric.
        let px = [33.0 / 255.0, 0.0, 0.0];
        assert!(wand_weight(px, seed, 32.0) < 1e-4);
        // Well within → 1.
        let px = [20.0 / 255.0, 10.0 / 255.0, 0.0];
        assert_eq!(wand_weight(px, seed, 32.0), 1.0);
    }

    #[test]
    fn contiguous_flood_selects_exactly_the_seed_region() {
        let img = two_region_img();
        let canvas = IRect::new(0, 0, 16, 8);
        let m = wand_select(&img, true, canvas, IVec2::new(2, 3), &WandParams::default());
        for y in 0..8 {
            for x in 0..16 {
                let want = if x < 8 { 1.0 } else { 0.0 };
                assert_eq!(m.get(x, y), want, "({x},{y})");
            }
        }
        // Global variant selects BOTH nothing extra and the same left half…
        let m = wand_select(
            &img,
            true,
            canvas,
            IVec2::new(2, 3),
            &WandParams {
                contiguous: false,
                ..WandParams::default()
            },
        );
        assert_eq!(m.get(1, 1), 1.0);
        assert_eq!(
            m.get(12, 1),
            0.0,
            "blue half stays out (metric, not connectivity)"
        );
    }

    #[test]
    fn global_mode_reaches_disconnected_matches() {
        // Red frame corners with a blue cross separating them.
        let mut p = Plane::new(IRect::new(0, 0, 9, 9));
        for y in 0..9 {
            for x in 0..9 {
                let red = x != 4 && y != 4;
                let c = if red {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                p.set(x, y, c);
            }
        }
        let img = Img(p);
        let canvas = IRect::new(0, 0, 9, 9);
        // Contiguous from a corner: only that quadrant.
        let m = wand_select(&img, true, canvas, IVec2::new(1, 1), &WandParams::default());
        assert_eq!(m.get(1, 1), 1.0);
        assert_eq!(m.get(7, 7), 0.0, "4-way flood cannot cross the cross");
        // Global: every red pixel.
        let m = wand_select(
            &img,
            true,
            canvas,
            IVec2::new(1, 1),
            &WandParams {
                contiguous: false,
                ..WandParams::default()
            },
        );
        assert_eq!(m.get(7, 7), 1.0);
        assert_eq!(m.get(4, 4), 0.0);
    }

    #[test]
    fn four_way_cannot_leak_through_diagonal_corners_eight_way_can() {
        // Checkerboard of red/blue: 4-way from a red square stays put;
        // 8-way floods every red square (diagonal connectivity).
        let mut p = Plane::new(IRect::new(0, 0, 8, 8));
        for y in 0..8 {
            for x in 0..8 {
                let red = (x + y) % 2 == 0;
                let c = if red {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 1.0, 1.0]
                };
                p.set(x, y, c);
            }
        }
        let img = Img(p);
        let canvas = IRect::new(0, 0, 8, 8);
        let m4 = wand_select(&img, true, canvas, IVec2::new(0, 0), &WandParams::default());
        assert_eq!(m4.get(0, 0), 1.0);
        assert_eq!(
            m4.get(2, 2),
            0.0,
            "§13.3: 4-way cannot leak through corners"
        );
        let m8 = wand_select(
            &img,
            true,
            canvas,
            IVec2::new(0, 0),
            &WandParams {
                eight_way: true,
                ..WandParams::default()
            },
        );
        assert_eq!(m8.get(2, 2), 1.0, "8-way reaches diagonals");
        assert_eq!(m8.get(1, 0), 0.0, "blue never selected");
    }

    #[test]
    fn antialias_false_snaps_at_half() {
        // A ramp that crosses the tolerance band: with antialias off, the
        // coverage snaps to {0, 1} at w ≥ 0.5.
        let mut p = Plane::new(IRect::new(0, 0, 66, 1));
        for x in 0..66 {
            p.set(x, 0, [x as f32 / 255.0, 0.0, 0.0, 1.0]);
        }
        let img = Img(p);
        let canvas = IRect::new(0, 0, 66, 1);
        let soft = wand_select(&img, true, canvas, IVec2::new(0, 0), &WandParams::default());
        let hard = wand_select(
            &img,
            true,
            canvas,
            IVec2::new(0, 0),
            &WandParams {
                antialias: false,
                ..WandParams::default()
            },
        );
        let mut saw_fraction = false;
        for x in 0..66 {
            let s = soft.get(x, 0);
            let h = hard.get(x, 0);
            assert!(h == 0.0 || h == 1.0, "hard is binary");
            assert_eq!(h == 1.0, s >= 0.5, "snap rule w ≥ 0.5 at x={x}");
            if s > 0.0 && s < 1.0 {
                saw_fraction = true;
            }
        }
        assert!(saw_fraction, "the soft path exercised the AA band");
    }

    #[test]
    fn premultiplied_sources_compare_straight_color() {
        // Same straight red at alpha 1.0 and 0.5 (premul 0.5,0,0): the
        // straight-compare pin keeps them in one region.
        let mut p = Plane::new(IRect::new(0, 0, 4, 1));
        p.set(0, 0, [1.0, 0.0, 0.0, 1.0]);
        p.set(1, 0, [0.5, 0.0, 0.0, 0.5]);
        p.set(2, 0, [1.0, 0.0, 0.0, 1.0]);
        p.set(3, 0, [0.0, 0.0, 1.0, 1.0]);
        let img = Img(p);
        let m = wand_select(
            &img,
            true,
            IRect::new(0, 0, 4, 1),
            IVec2::new(0, 0),
            &WandParams::default(),
        );
        assert_eq!(m.get(0, 0), 1.0);
        assert_eq!(m.get(1, 0), 1.0, "straight color equal despite alpha");
        assert_eq!(m.get(2, 0), 1.0);
        assert_eq!(m.get(3, 0), 0.0);
    }

    #[test]
    fn out_of_canvas_seed_is_empty_and_uniform_selects_all() {
        let canvas = IRect::new(0, 0, 32, 32);
        let m = wand_select(
            &UniformBackdrop([0.2, 0.4, 0.6, 1.0]),
            true,
            canvas,
            IVec2::new(100, 0),
            &WandParams::default(),
        );
        assert!(m.rect.is_empty());
        let m = wand_select(
            &UniformBackdrop([0.2, 0.4, 0.6, 1.0]),
            true,
            canvas,
            IVec2::new(5, 5),
            &WandParams::default(),
        );
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(m.get(x, y), 1.0);
            }
        }
    }
}
