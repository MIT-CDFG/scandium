// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.5 step 4–5 (presentation + commit shape) [NORMATIVE]:
//!
//! ```text
//! cov_p  = S.cov                                  // v1: no pencil/wet-edge
//! src_a  = opacity · cov_p · sel(p)               // selection gates (§13.1)
//! src    = (color · src_a, src_a)                 // premultiplied source
//! paint:  layer' = composite(layer, src)          // §4.5 kernel (normal built in)
//! erase:  layer'.rgba = layer.rgba · (1 − src_a)  // dst-out, alpha-only effect
//! mask/channel (single-component, §14.1):
//!   paint: value' = lerp(value, paint_gray, src_a)
//!   erase: value' = value · (1 − src_a)
//! ```
//!
//! Presentation always composites against the **pre-stroke** backdrop
//! (§15.5 step 4: never onto its own previous presentation), which is
//! exactly what the [`Backdrop`] trait hands in. The result is a list of
//! [`TilePatch`]es — full composited f32 tiles keyed by `appname-core`
//! `TileCoord` — that O-CORE's command implementation writes through core's
//! own (crate-internal) doors and records as the stroke's DeltaSet. This
//! crate mutates nothing (D-48).
//!
//! Locks (§40.5 family): the TRANSPARENCY lock's "coverage clamps to
//! existing alpha" constraint is [`PresentOptions::clamp_to_backdrop_alpha`]
//! (paint targets); the eraser's TRANSPARENCY hard block is a stage-4
//! command gate upstream — never reaches this module. Selection gating:
//! `sel(p)` via [`CoverageRead`]; an empty stroke∩selection intersection
//! yields zero patches (the §40.1 empty-DeltaSet history push).

use appname_core::{BlendMode, ContentHash};
use appname_core::{Buffer, IRect, IVec2, Selection, TileCoord, TILE};

use crate::brush::plan::StrokeError;
use crate::brush::raster::StrokeScratch;

/// Read access to the pre-stroke surface. Implemented for
/// `appname_core::Buffer` over core's public read surface; any raster a
/// test builds can implement it too.
pub trait Backdrop {
    /// Storage-normalized f32 components at a document pixel, zero-padded
    /// to 4; absent space reads all-zero (ES §2.4 edge rule).
    fn read_pixel(&self, p: IVec2) -> [f32; 4];

    /// Dense read of one lod-0 tile (row-major `TILE × TILE`). The default
    /// loops [`Backdrop::read_pixel`]; implementors with tile access
    /// override it to skip per-pixel lookups.
    fn read_tile_dense(&self, c: TileCoord, out: &mut [[f32; 4]]) {
        debug_assert_eq!(out.len(), TILE * TILE);
        let rect = c.pixel_rect();
        let mut i = 0;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                out[i] = self.read_pixel(IVec2::new(x, y));
                i += 1;
            }
        }
    }
}

impl Backdrop for Buffer {
    fn read_pixel(&self, p: IVec2) -> [f32; 4] {
        self.read_pixel_f32(p)
    }

    fn read_tile_dense(&self, c: TileCoord, out: &mut [[f32; 4]]) {
        debug_assert_eq!(out.len(), TILE * TILE);
        match self.get_tile(c) {
            Some(tile) => {
                let model = self.model();
                let mut i = 0;
                for y in 0..TILE {
                    for x in 0..TILE {
                        out[i] = tile.read_pixel_f32(x, y, model);
                        i += 1;
                    }
                }
            }
            None => out.fill([0.0; 4]),
        }
    }
}

/// A uniform (e.g. all-transparent) pre-stroke surface — oracle backdrops
/// for tests and a valid stand-in for an empty layer.
#[derive(Clone, Copy, Debug)]
pub struct UniformBackdrop(pub [f32; 4]);

impl Backdrop for UniformBackdrop {
    fn read_pixel(&self, _p: IVec2) -> [f32; 4] {
        self.0
    }

    fn read_tile_dense(&self, _c: TileCoord, out: &mut [[f32; 4]]) {
        out.fill(self.0);
    }
}

/// `sel(p)` — §13.1 selection coverage in `[0, 1]`.
pub trait CoverageRead {
    fn coverage(&self, p: IVec2) -> f32;
}

/// Core's selection slot: inactive = everything selected (§13.1); active =
/// the f32 mask's channel 0.
impl CoverageRead for Selection {
    fn coverage(&self, p: IVec2) -> f32 {
        if !self.active() {
            1.0
        } else {
            self.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0)
        }
    }
}

/// The §4.5 blend-kernel table for non-`normal` paint modes. The one shared
/// math library lands with O-RENDER's compositor (ES §14 "one math library,
/// tested once"); O-CORE passes it at wiring time. `normal` (premultiplied
/// source-over, Appendix A #0) is built into this module and needs no table.
pub trait BlendKernels {
    /// `composite(dst, src)` over premultiplied working-space values.
    fn composite(&self, mode: BlendMode, dst: [f32; 4], src: [f32; 4]) -> [f32; 4];
}

/// What the stroke deposits (contract param mapping is 1:1 — see module
/// docs of `brush`).
#[derive(Clone, Copy)]
pub enum PaintOp<'a> {
    /// `brush.stroke` on an RGBA surface: `color` is the §22.1 color already
    /// converted to **straight** working-space linear RGB (the §15.5 dab
    /// color; premultiplication happens here), `blend` the §4.5 wire mode.
    Paint {
        color: [f32; 3],
        blend: BlendMode,
        kernels: Option<&'a dyn BlendKernels>,
    },
    /// `brush.stroke` on a mask/channel target (§14.1 single-component):
    /// `value' = lerp(value, gray, src_a)`.
    PaintSingle { gray: f32 },
    /// `eraser.stroke` on an RGBA surface: dst-out.
    Erase,
    /// `eraser.stroke` on a mask/channel target: `value · (1 − src_a)`.
    EraseSingle,
}

/// Presentation-time options (the §15.5 step-4 inputs beyond the scratch).
#[derive(Clone, Copy, Default)]
pub struct PresentOptions<'a> {
    /// The stroke `opacity` param — the per-stroke ceiling (§15.5 step 4).
    /// Callers pass the same value given to `plan_stroke`; kept explicit so
    /// presentation stays a pure function of its arguments.
    pub opacity: f32,
    /// `sel(p)`; `None` = no selection = fully selected.
    pub selection: Option<&'a dyn CoverageRead>,
    /// §40.5/§11.1 TRANSPARENCY-lock constraint for paint: coverage clamps
    /// to existing alpha "like painting through the layer's own alpha" —
    /// source-atop (`out = color·src_a·αb + dst·(1−src_a)`, alpha
    /// invariant). v1 supports it with `normal` blend.
    pub clamp_to_backdrop_alpha: bool,
}

/// One composited result tile: the full post-stroke pixel plane (storage-
/// normalized f32, row-major `TILE × TILE`, zero-padded to 4 components).
/// O-CORE installs these through core's write doors and records the
/// DeltaSet from the same coords.
pub struct TilePatch {
    pub coord: TileCoord,
    pub pixels: Vec<[f32; 4]>,
}

/// Summary Debug (ES §1.6: diagnostics never dump pixel payloads).
impl core::fmt::Debug for TilePatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TilePatch")
            .field("coord", &self.coord)
            .finish_non_exhaustive()
    }
}

/// The presented stroke: patches sorted by `(y, x)` coord, plus the dirty
/// basis (union of dab bounds ∩ clip — the contract `dirtyRect` D).
#[derive(Debug)]
pub struct StrokeCommit {
    pub patches: Vec<TilePatch>,
    pub bounds: IRect,
}

/// Premultiplied source-over (§4.5 `normal`, Appendix A #0).
#[inline]
fn src_over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let ia = 1.0 - src[3];
    [
        src[0] + dst[0] * ia,
        src[1] + dst[1] * ia,
        src[2] + dst[2] * ia,
        src[3] + dst[3] * ia,
    ]
}

/// Present one touched tile against the pre-stroke backdrop. Returns `None`
/// when no pixel of the tile receives `src_a > 0` (nothing to write — e.g.
/// the stroke∩selection intersection is empty there).
pub fn present_tile<B: Backdrop + ?Sized>(
    scratch: &StrokeScratch,
    backdrop: &B,
    coord: TileCoord,
    op: &PaintOp<'_>,
    opts: &PresentOptions<'_>,
) -> Result<Option<TilePatch>, StrokeError> {
    check_op(op, opts)?;
    if !opts.opacity.is_finite() || !(0.0..=1.0).contains(&opts.opacity) {
        return Err(StrokeError::OpacityOutOfRange(opts.opacity));
    }
    let cov = match scratch.tile_coverage(coord) {
        Some(c) => c,
        None => return Ok(None),
    };
    let mut pixels = vec![[0.0f32; 4]; TILE * TILE];
    backdrop.read_tile_dense(coord, &mut pixels);

    let rect = coord.pixel_rect();
    let mut any = false;
    let mut i = 0;
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            let c = cov[i];
            if c > 0.0 {
                let sel = match opts.selection {
                    Some(s) => s.coverage(IVec2::new(x, y)).clamp(0.0, 1.0),
                    None => 1.0,
                };
                // §15.5: src_a = opacity · cov_p · sel(p); v1 cov_p = S.cov.
                let src_a = opts.opacity * c * sel;
                if src_a > 0.0 {
                    let dst = pixels[i];
                    match op {
                        PaintOp::Paint {
                            color,
                            blend,
                            kernels,
                        } => {
                            if opts.clamp_to_backdrop_alpha {
                                // §11.1 TRANSPARENCY lock: "paint clamps
                                // coverage to existing alpha, like painting
                                // through the layer's own alpha" — i.e.
                                // source-atop: alpha exactly invariant.
                                // v1 supports it for `normal` (check_op
                                // rejects other modes with the flag).
                                if dst[3] > 0.0 {
                                    let ia = 1.0 - src_a;
                                    pixels[i] = [
                                        color[0] * src_a * dst[3] + dst[0] * ia,
                                        color[1] * src_a * dst[3] + dst[1] * ia,
                                        color[2] * src_a * dst[3] + dst[2] * ia,
                                        dst[3],
                                    ];
                                    any = true;
                                }
                            } else {
                                let src =
                                    [color[0] * src_a, color[1] * src_a, color[2] * src_a, src_a];
                                pixels[i] = match blend {
                                    BlendMode::Normal => src_over(dst, src),
                                    other => kernels
                                        .expect("check_op verified the kernel table")
                                        .composite(*other, dst, src),
                                };
                                any = true;
                            }
                        }
                        PaintOp::PaintSingle { gray } => {
                            pixels[i][0] = dst[0] + (gray - dst[0]) * src_a;
                            any = true;
                        }
                        PaintOp::Erase => {
                            let ia = 1.0 - src_a;
                            pixels[i] = [dst[0] * ia, dst[1] * ia, dst[2] * ia, dst[3] * ia];
                            any = true;
                        }
                        PaintOp::EraseSingle => {
                            pixels[i][0] = dst[0] * (1.0 - src_a);
                            any = true;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    Ok(if any {
        Some(TilePatch { coord, pixels })
    } else {
        None
    })
}

/// §15.5 step 4 over every touched tile, in deterministic `(y, x)` order.
pub fn present<B: Backdrop + ?Sized>(
    scratch: &StrokeScratch,
    backdrop: &B,
    op: &PaintOp<'_>,
    opts: &PresentOptions<'_>,
) -> Result<StrokeCommit, StrokeError> {
    check_op(op, opts)?;
    let mut patches = Vec::new();
    for coord in scratch.touched_tiles() {
        if let Some(patch) = present_tile(scratch, backdrop, coord, op, opts)? {
            patches.push(patch);
        }
    }
    Ok(StrokeCommit {
        patches,
        bounds: scratch.bounds(),
    })
}

fn check_op(op: &PaintOp<'_>, opts: &PresentOptions<'_>) -> Result<(), StrokeError> {
    match op {
        PaintOp::Paint {
            color,
            blend,
            kernels,
        } => {
            if color.iter().any(|c| !c.is_finite()) {
                return Err(StrokeError::ColorNotFinite);
            }
            if opts.clamp_to_backdrop_alpha && *blend != BlendMode::Normal {
                // v1: the §11.1 source-atop lock path is defined for
                // `normal`; locked painting in other modes needs the §4.5
                // blend functions separated from their composites (kernel-
                // table extension flagged to O-RENDER/O-CORE, lane log).
                return Err(StrokeError::TransparencyLockBlendUnsupported(*blend));
            }
            if *blend != BlendMode::Normal && kernels.is_none() {
                return Err(StrokeError::BlendKernelUnavailable(*blend));
            }
        }
        PaintOp::PaintSingle { gray } => {
            if !gray.is_finite() {
                return Err(StrokeError::ColorNotFinite);
            }
        }
        PaintOp::Erase | PaintOp::EraseSingle => {}
    }
    Ok(())
}

/// Content identity of a presented stroke: BLAKE3 over patch coords + f32
/// pixel bytes in the commit's deterministic order (test/golden anchor;
/// same shape discipline as core's `region_hash`).
pub fn commit_hash(commit: &StrokeCommit) -> ContentHash {
    ContentHash::from_stream(|h| {
        h.update(&(commit.patches.len() as u32).to_le_bytes());
        for p in &commit.patches {
            h.update(&p.coord.y.to_le_bytes());
            h.update(&p.coord.x.to_le_bytes());
            for px in &p.pixels {
                for c in px {
                    h.update(&c.to_le_bytes());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::plan::{Dab, StrokePlan};
    use crate::brush::raster::rasterize_plan;
    use appname_core::TILE_I32;

    const BIG: IRect = IRect::new(-4096, -4096, 4096, 4096);

    fn one_dab_scratch(flow: f32) -> StrokeScratch {
        let d = Dab {
            center: [10.5, 10.5],
            diameter: 10.0,
            angle_rad: 0.0,
            roundness: 1.0,
            hardness: 1.0,
            flow,
            opacity: 1.0,
        };
        let plan = StrokePlan {
            bounds: d.pixel_rect(),
            dabs: vec![d],
            base_diameter: 10.0,
        };
        rasterize_plan(&plan, BIG)
    }

    fn pixel_of(commit: &StrokeCommit, p: IVec2) -> Option<[f32; 4]> {
        let (tx, ty) = (p.x.div_euclid(TILE_I32), p.y.div_euclid(TILE_I32));
        let patch = commit
            .patches
            .iter()
            .find(|q| q.coord.x == tx && q.coord.y == ty)?;
        let (px, py) = (
            p.x.rem_euclid(TILE_I32) as usize,
            p.y.rem_euclid(TILE_I32) as usize,
        );
        Some(patch.pixels[py * TILE + px])
    }

    fn opts(opacity: f32) -> PresentOptions<'static> {
        PresentOptions {
            opacity,
            selection: None,
            clamp_to_backdrop_alpha: false,
        }
    }

    #[test]
    fn paint_normal_over_opaque_backdrop_is_the_source_over_formula() {
        let s = one_dab_scratch(1.0);
        let backdrop = UniformBackdrop([0.0, 0.25, 0.0, 1.0]); // premul green-ish
        let op = PaintOp::Paint {
            color: [1.0, 0.0, 0.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let commit = present(&s, &backdrop, &op, &opts(0.5)).unwrap();
        // Center pixel: cov 1 → src_a = 0.5; out = src + dst·(1−0.5).
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        assert_eq!(got, [0.5, 0.125, 0.0, 1.0]);
        // Untouched pixel inside the patch keeps the backdrop exactly.
        let far = pixel_of(&commit, IVec2::new(1, 20)).unwrap();
        assert_eq!(far, [0.0, 0.25, 0.0, 1.0]);
        assert_eq!(commit.bounds, s.bounds());
    }

    #[test]
    fn paint_over_transparent_backdrop_deposits_premultiplied_color() {
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [0.2, 0.4, 0.8],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let commit = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        assert_eq!(got, [0.2, 0.4, 0.8, 1.0]);
        // Premultiplied validity: color ≤ alpha per channel at the AA rim.
        for patch in &commit.patches {
            for px in &patch.pixels {
                for ch in 0..3 {
                    assert!(px[ch] <= px[3] + 1e-6, "premul violation {px:?}");
                }
            }
        }
    }

    #[test]
    fn erase_is_dst_out_on_all_four_channels() {
        let s = one_dab_scratch(1.0);
        let backdrop = UniformBackdrop([0.5, 0.25, 0.125, 1.0]);
        let commit = present(&s, &backdrop, &PaintOp::Erase, &opts(0.75)).unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        assert_eq!(got, [0.125, 0.0625, 0.03125, 0.25]);
    }

    #[test]
    fn single_channel_paint_and_erase_follow_the_lerp_and_multiply_forms() {
        let s = one_dab_scratch(1.0);
        let backdrop = UniformBackdrop([0.4, 0.0, 0.0, 0.0]); // gray surface, channel 0
        let commit = present(
            &s,
            &backdrop,
            &PaintOp::PaintSingle { gray: 1.0 },
            &opts(0.5),
        )
        .unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        assert!(
            (got[0] - 0.7).abs() < 1e-6,
            "lerp(0.4, 1.0, 0.5) = 0.7, got {}",
            got[0]
        );
        let commit = present(&s, &backdrop, &PaintOp::EraseSingle, &opts(0.5)).unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        assert!(
            (got[0] - 0.2).abs() < 1e-6,
            "0.4·(1−0.5) = 0.2, got {}",
            got[0]
        );
    }

    #[test]
    fn flow_builds_coverage_but_opacity_caps_the_stroke() {
        // 8 overlapping dabs at flow 0.3: cov = 1 − 0.7⁸ ≈ 0.9424.
        let d = Dab {
            center: [10.5, 10.5],
            diameter: 10.0,
            angle_rad: 0.0,
            roundness: 1.0,
            hardness: 1.0,
            flow: 0.3,
            opacity: 1.0,
        };
        let plan = StrokePlan {
            bounds: d.pixel_rect(),
            dabs: vec![d; 8],
            base_diameter: 10.0,
        };
        let s = rasterize_plan(&plan, BIG);
        let op = PaintOp::Paint {
            color: [1.0, 1.0, 1.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let commit = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(0.5)).unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        let cov = 1.0 - 0.7f32.powi(8);
        assert!(
            (got[3] - 0.5 * cov).abs() < 1e-5,
            "α = opacity·cov, got {}",
            got[3]
        );
        assert!(
            got[3] < 0.5,
            "self-overlap never exceeds the opacity ceiling (§15.5)"
        );
    }

    #[test]
    fn selection_gates_coverage_and_empty_intersection_yields_no_patches() {
        struct Half; // selects x >= 11 only
        impl CoverageRead for Half {
            fn coverage(&self, p: IVec2) -> f32 {
                if p.x >= 11 {
                    1.0
                } else {
                    0.0
                }
            }
        }
        struct Nothing;
        impl CoverageRead for Nothing {
            fn coverage(&self, _p: IVec2) -> f32 {
                0.0
            }
        }
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [1.0, 0.0, 0.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let half = Half;
        let commit = present(
            &s,
            &UniformBackdrop([0.0; 4]),
            &op,
            &PresentOptions {
                opacity: 1.0,
                selection: Some(&half),
                clamp_to_backdrop_alpha: false,
            },
        )
        .unwrap();
        assert_eq!(
            pixel_of(&commit, IVec2::new(10, 10)).unwrap()[3],
            0.0,
            "gated out"
        );
        assert!(
            pixel_of(&commit, IVec2::new(12, 10)).unwrap()[3] > 0.0,
            "selected side painted"
        );
        let nothing = Nothing;
        let commit = present(
            &s,
            &UniformBackdrop([0.0; 4]),
            &op,
            &PresentOptions {
                opacity: 1.0,
                selection: Some(&nothing),
                clamp_to_backdrop_alpha: false,
            },
        )
        .unwrap();
        assert!(
            commit.patches.is_empty(),
            "empty stroke∩selection = empty DeltaSet (§40.1)"
        );
    }

    #[test]
    fn core_selection_type_gates_via_the_trait() {
        // Inactive core Selection = everything selected (§13.1).
        let sel = Selection::new();
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [1.0, 0.0, 0.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let commit = present(
            &s,
            &UniformBackdrop([0.0; 4]),
            &op,
            &PresentOptions {
                opacity: 1.0,
                selection: Some(&sel),
                clamp_to_backdrop_alpha: false,
            },
        )
        .unwrap();
        assert!(pixel_of(&commit, IVec2::new(10, 10)).unwrap()[3] > 0.0);
    }

    #[test]
    fn transparency_clamp_limits_paint_to_existing_alpha() {
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [1.0, 1.0, 1.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        // Backdrop alpha 0.25: src_a clamps to 0.25.
        let backdrop = UniformBackdrop([0.1, 0.1, 0.1, 0.25]);
        let commit = present(
            &s,
            &backdrop,
            &op,
            &PresentOptions {
                opacity: 1.0,
                selection: None,
                clamp_to_backdrop_alpha: true,
            },
        )
        .unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        assert!(
            (got[3] - 0.25).abs() < 1e-6,
            "alpha invariant under the lock (§11.1)"
        );
        // Source-atop at cov 1: out = color·1·αb + dst·0 = 0.25 per channel.
        for ch in 0..3 {
            assert!(
                (got[ch] - 0.25).abs() < 1e-6,
                "paint through own alpha, got {got:?}"
            );
        }
        // Fully transparent backdrop: nothing paints at all.
        let commit = present(
            &s,
            &UniformBackdrop([0.0; 4]),
            &op,
            &PresentOptions {
                opacity: 1.0,
                selection: None,
                clamp_to_backdrop_alpha: true,
            },
        )
        .unwrap();
        assert!(
            commit.patches.is_empty(),
            "TRANSPARENCY constrain over empty alpha paints nothing"
        );
    }

    #[test]
    fn non_normal_blend_without_kernels_errors_with_kernels_composites() {
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [1.0, 0.0, 0.0],
            blend: BlendMode::Multiply,
            kernels: None,
        };
        assert_eq!(
            present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap_err(),
            StrokeError::BlendKernelUnavailable(BlendMode::Multiply)
        );

        /// A stand-in §4.5 multiply kernel over premultiplied values:
        /// co = cs·cb + cs·(1−ab) + cb·(1−as); ao = as + ab·(1−as).
        struct MultiplyOnly;
        impl BlendKernels for MultiplyOnly {
            fn composite(&self, mode: BlendMode, dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
                assert_eq!(mode, BlendMode::Multiply);
                let (asrc, ab) = (src[3], dst[3]);
                let mut out = [0.0f32; 4];
                for ch in 0..3 {
                    out[ch] = src[ch] * dst[ch] + src[ch] * (1.0 - ab) + dst[ch] * (1.0 - asrc);
                }
                out[3] = asrc + ab * (1.0 - asrc);
                out
            }
        }
        let k = MultiplyOnly;
        let op = PaintOp::Paint {
            color: [0.5, 1.0, 0.0],
            blend: BlendMode::Multiply,
            kernels: Some(&k),
        };
        let backdrop = UniformBackdrop([0.5, 0.5, 0.5, 1.0]);
        let commit = present(&s, &backdrop, &op, &opts(1.0)).unwrap();
        let got = pixel_of(&commit, IVec2::new(10, 10)).unwrap();
        // src = (0.5,1,0,1) over dst (0.5,0.5,0.5,1): co = cs·cb.
        assert_eq!(got, [0.25, 0.5, 0.0, 1.0]);
    }

    #[test]
    fn buffer_backdrop_matches_uniform_zero_for_empty_buffers() {
        use appname_core::{ChannelKind, ColorModelKind, ProfileId};
        let buf = Buffer::new(ChannelKind::F32, ColorModelKind::Rgba, ProfileId::nil());
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [0.3, 0.6, 0.9],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let a = present(&s, &buf, &op, &opts(1.0)).unwrap();
        let b = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap();
        assert_eq!(commit_hash(&a).as_bytes(), commit_hash(&b).as_bytes());
    }

    #[test]
    fn zero_coverage_tiles_produce_no_patch() {
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [1.0, 0.0, 0.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        // The dab's conservative rect can touch tiles whose every pixel has
        // zero coverage; those must not emit patches.
        let commit = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap();
        for patch in &commit.patches {
            assert!(
                patch.pixels.iter().any(|p| p[3] > 0.0),
                "patch with no painted pixel"
            );
        }
        // Zero opacity: everything gates to src_a = 0 → no patches.
        let commit = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(0.0)).unwrap();
        assert!(commit.patches.is_empty());
    }

    #[test]
    fn commit_hash_is_order_stable_and_content_sensitive() {
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [1.0, 0.0, 0.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let a = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap();
        let b = present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap();
        assert_eq!(commit_hash(&a).as_bytes(), commit_hash(&b).as_bytes());
        let op2 = PaintOp::Paint {
            color: [1.0, 0.0, 0.1],
            blend: BlendMode::Normal,
            kernels: None,
        };
        let c = present(&s, &UniformBackdrop([0.0; 4]), &op2, &opts(1.0)).unwrap();
        assert_ne!(commit_hash(&a).as_bytes(), commit_hash(&c).as_bytes());
    }

    #[test]
    fn color_validation_rejects_non_finite() {
        let s = one_dab_scratch(1.0);
        let op = PaintOp::Paint {
            color: [f32::NAN, 0.0, 0.0],
            blend: BlendMode::Normal,
            kernels: None,
        };
        assert_eq!(
            present(&s, &UniformBackdrop([0.0; 4]), &op, &opts(1.0)).unwrap_err(),
            StrokeError::ColorNotFinite
        );
        assert_eq!(
            present(
                &s,
                &UniformBackdrop([0.0; 4]),
                &PaintOp::PaintSingle {
                    gray: f32::INFINITY
                },
                &opts(1.0)
            )
            .unwrap_err(),
            StrokeError::ColorNotFinite
        );
    }
}
