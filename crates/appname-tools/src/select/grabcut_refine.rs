// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ALGORITHMS.md §8.B [NORMATIVE] ladder — "wand/color-range →
//! quick-select → **GrabCut refine** → live-wire"; INTEGRATION-log
//! Ruling A (2026-08-16): the ES §13.3 statistical flood governs TOOL
//! BEHAVIOR (no graph cut in the tool loop), the already-ported GrabCut
//! engine (`appname_algo::grabcut`, INTEGRATION-NOTES item 8, clean-room
//! rother04/boykov01, register keys P-7660463 "Expired - Fee Related" /
//! P-6973212 "Expired - Lifetime" — quoted select-and-screen, not a
//! release claim) wires as the SEPARATE refine tier. O-ALGO's item-8
//! seam is honored unchanged: pure planes in, labels out
//! (`labels_to_mask` is the adapter-seam output shape).
//!
//! This module is that refine tier's TOOLS-SIDE entry point: selection
//! in → refined selection out, as a [`MaskPlane`] source the consumer
//! writes through the SAME §13.2 selection doors (`select::combine` with
//! `clip = result.rect`, then the core install doors). No §10.3 command
//! kind addresses the refine tier today — the wire surface waits on a
//! contract reading (escalated in the lane log, not invented here); the
//! Select-and-Mask workspace (O-UI, CP-1.0) is the interactive consumer.
//!
//! Trimap derivation (v1 pins, logged): the selection thresholds at 0.5;
//! the §13.2 signed EDT bands it — `d ≤ −band` ⇒ definite foreground,
//! `d ≥ +band` ⇒ definite background, in between probable (side by the
//! 0.5 threshold). Definite labels are seeds the engine never flips
//! (boykov01), so refinement is local to the band by construction. The
//! binary cut re-antialiases with the §13.2 1-px smoothstep
//! (`refine::grow(·, 0)` = `smoothAA(d)`).

use appname_core::{IRect, IVec2};

use appname_algo::grabcut::{
    grabcut, label_is_fg, LABEL_BG, LABEL_FG, LABEL_PROB_BG, LABEL_PROB_FG,
};
use appname_algo::plane::Plane;

use super::refine;
use super::MaskPlane;
use crate::brush::present::Backdrop;

/// Refine-tier parameters.
#[derive(Clone, Copy, Debug)]
pub struct GrabcutRefineParams {
    /// Uncertain-band half-width around the selection edge (px).
    pub band: f64,
    /// Iteration bound (engine clamps at the §8.B contract bound 5).
    pub max_iterations: usize,
}

impl Default for GrabcutRefineParams {
    fn default() -> GrabcutRefineParams {
        GrabcutRefineParams {
            band: 8.0,
            max_iterations: 5,
        }
    }
}

/// Refine `selection` against the image (§8.B GrabCut tier). `src` is
/// the sampling source (target layer or composite — the caller's
/// choice), `canvas` clips. Returns the refined coverage over the
/// band-inflated selection bounds; combine it through the §13.2 doors
/// with `op = Replace` and `clip = result.rect` (pixels outside the
/// returned rect are untouched by construction — the definite labels
/// reproduce the thresholded input there).
///
/// Errors only on engine-reported trimap failures (adapter seam).
pub fn grabcut_refine<B: Backdrop + ?Sized>(
    src: &B,
    premultiplied: bool,
    canvas: IRect,
    selection: &MaskPlane,
    selection_bounds: IRect,
    params: &GrabcutRefineParams,
) -> Result<MaskPlane, String> {
    if selection_bounds.is_empty() {
        return Ok(MaskPlane::new(IRect::EMPTY));
    }
    let band = params.band.max(1.0);
    let pad = band.ceil() as i32 + 1;
    let region = IRect::new(
        selection_bounds.min.x - pad,
        selection_bounds.min.y - pad,
        selection_bounds.max.x + pad,
        selection_bounds.max.y + pad,
    )
    .intersect(&canvas);
    if region.is_empty() {
        return Ok(MaskPlane::new(IRect::EMPTY));
    }
    let (w, h) = (region.width() as usize, region.height() as usize);

    // Selection restricted to the region (absent reads 0).
    let mut sel = MaskPlane::new(region);
    for y in region.min.y..region.max.y {
        for x in region.min.x..region.max.x {
            sel.set(x, y, selection.get(x, y));
        }
    }
    // §13.2 signed EDT of the 0.5-thresholded mask over the region.
    let d = refine::signed_distance(&sel, region);

    let mut trimap = vec![LABEL_PROB_BG; w * h];
    for (i, &di) in d.iter().enumerate() {
        trimap[i] = if di <= -(band as f32) {
            LABEL_FG
        } else if di >= band as f32 {
            LABEL_BG
        } else if sel.v[i] >= 0.5 {
            LABEL_PROB_FG
        } else {
            LABEL_PROB_BG
        };
    }

    // Straight working-RGB planes over the region (adapter seam shape).
    let mut planes = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
    for y in 0..h {
        for x in 0..w {
            let p = IVec2::new(region.min.x + x as i32, region.min.y + y as i32);
            let v = src.read_pixel(p);
            let c = if premultiplied && v[3] > 0.0 {
                [v[0] / v[3], v[1] / v[3], v[2] / v[3]]
            } else {
                [v[0], v[1], v[2]]
            };
            for (k, plane) in planes.iter_mut().enumerate() {
                *plane.at_mut(x, y) = c[k];
            }
        }
    }

    let result = grabcut(&planes, &trimap, params.max_iterations)?;

    // Binary labels → §13.2 1-px re-AA (grow by 0 = smoothAA(d)).
    let mut hard = MaskPlane::new(region);
    for (i, &l) in result.labels.iter().enumerate() {
        if label_is_fg(l) {
            hard.v[i] = 1.0;
        }
    }
    Ok(refine::grow(&hard, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Img {
        w: i32,
        h: i32,
        px: Vec<[f32; 4]>,
    }

    impl Backdrop for Img {
        fn read_pixel(&self, p: IVec2) -> [f32; 4] {
            if p.x < 0 || p.y < 0 || p.x >= self.w || p.y >= self.h {
                return [0.0; 4];
            }
            self.px[(p.y * self.w + p.x) as usize]
        }
    }

    /// Two-tone image: a red disc on blue. A sloppy rectangular
    /// selection around the disc refines to hug the color boundary.
    #[test]
    fn refine_snaps_a_sloppy_rect_to_the_two_tone_boundary() {
        let (w, h) = (48, 48);
        let disc = |x: i32, y: i32| {
            let (dx, dy) = (x - 24, y - 24);
            dx * dx + dy * dy <= 12 * 12
        };
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.push(if disc(x, y) {
                    [0.9, 0.1, 0.1, 1.0]
                } else {
                    [0.1, 0.1, 0.9, 1.0]
                });
            }
        }
        let img = Img { w, h, px };
        let canvas = IRect::new(0, 0, w, h);
        // Sloppy selection: a rect overshooting the disc by ~4 px.
        let mut sel = MaskPlane::new(IRect::new(8, 8, 40, 40));
        for y in 8..40 {
            for x in 8..40 {
                sel.set(x, y, 1.0);
            }
        }
        let refined = grabcut_refine(
            &img,
            false,
            canvas,
            &sel,
            IRect::new(8, 8, 40, 40),
            &GrabcutRefineParams::default(),
        )
        .unwrap();
        // Disc interior selected, far background dropped.
        assert!(refined.get(24, 24) > 0.99, "disc center stays selected");
        assert!(refined.get(24, 14) > 0.99, "disc interior stays selected");
        assert!(
            refined.get(10, 10) < 0.01,
            "blue corner inside the sloppy rect is refined away"
        );
        assert!(
            refined.get(38, 24) < 0.01,
            "blue overshoot right of the disc is refined away"
        );
        // Coverage invariant (§34.4 property 13).
        for v in &refined.v {
            assert!((0.0..=1.0).contains(v));
        }
        // Determinism: the engine is RNG-free — a second run is
        // bit-identical.
        let again = grabcut_refine(
            &img,
            false,
            canvas,
            &sel,
            IRect::new(8, 8, 40, 40),
            &GrabcutRefineParams::default(),
        )
        .unwrap();
        assert_eq!(refined.v, again.v);
    }

    #[test]
    fn empty_selection_region_yields_empty_plane() {
        let img = Img {
            w: 8,
            h: 8,
            px: vec![[0.5, 0.5, 0.5, 1.0]; 64],
        };
        let sel = MaskPlane::new(IRect::EMPTY);
        let out = grabcut_refine(
            &img,
            false,
            IRect::new(0, 0, 8, 8),
            &sel,
            IRect::EMPTY,
            &GrabcutRefineParams::default(),
        )
        .unwrap();
        assert!(out.rect.is_empty());
    }
}
