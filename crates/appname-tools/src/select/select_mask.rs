// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.4 "The Select-and-Mask pipeline" [NORMATIVE] — the nine
//! stages IN THIS EXACT ORDER, composed tools-side over the merged
//! `appname-algo::matting` engines (O-ALGO Row B READY seam: engines
//! pure and merge-ready, consumer wiring is O-TOOLS'). ES §39.4 (the
//! Select-and-Mask button + §13.4 output-table commit contract), §13.2
//! (EDT, Smooth, Feather forms), §12.3 (session proxy preview is the
//! workspace's — O-UI, CP-1.0). Contract note: the workspace itself
//! writes NO history until stage 9; commits map onto catalog kinds per
//! the §13.4 output table ([`OutputMode`]); this module produces the
//! matte (+ decontaminated color) those commits consume.
//!
//! Stage mapping (engine seams named per the Row-B notes):
//! 1. input `m₀` + working-RGB guidance `I` (planes over one region);
//! 2. edge band `B = {p : |d(p)| ≤ r}` from the §13.2 EDT; **smart
//!    radius** varies r per pixel in `[r/4, 4r]` ∝ local gradient
//!    variance in a 16-px window (v1 pins: variance of Sobel `|∇I|` of
//!    Rec. 709 luma, box window half-width 8; normalized by the max
//!    variance over the 4r-inflated band; zero variance ⇒ r/4);
//! 3. guided-filter matting inside `B` — he10 via
//!    `matting::guided_tier::guided_filter_matting` (window radius `r`,
//!    `eps` default 1e-4), hard constraints `α=1` where `d < −r(p)`,
//!    `α=0` where `d > r(p)` (the trimap pin); the engine's gray-guide
//!    reduction (luma guidance) is its documented seam — recorded, not
//!    hidden. `r = 0` skips stages 2–3 (no edge detection);
//! 4. Smooth (§13.2), slider `0..=100` → disk radius px;
//! 5. Feather, `σ = radius/2`, slider `0..=250` px;
//! 6. Contrast `m ← clamp(0.5 + (m − 0.5)·(1 + c), 0, 1)`, `c ∈ 0..=1`;
//! 7. Shift edge `s = (shift/100)·max(1, r)` px along the outward
//!    distance-field normal (profile RESAMPLES via `d` — bilinear pull
//!    `m′(p) = m(p − s·n̂(p))`, `n̂ = ∇d/|∇d|` — not re-thresholded);
//! 8. Decontaminate — `matting::decontaminate` (ES §13.4 stage-8 formula
//!    over the wave-1 foreground estimate; band = stage-2 `B`); because
//!    it rewrites colors it forces the new-layer output modes and never
//!    touches the source layer (§13.4);
//! 9. output — [`OutputMode`] maps to the §13.4 emitted-command table;
//!    emission is the workspace consumer's (it holds the doors).
//!
//! Everything here is pure over (planes, mask, params): the §12.3
//! stage-forward caching is the interactive workspace's concern.

use appname_core::IRect;

use appname_algo::matting::decontaminate::{decontaminate, DecontaminateParams};
use appname_algo::matting::guided_tier::guided_filter_matting;
use appname_algo::plane::Plane;

use super::refine;
use super::MaskPlane;

/// §13.4 slider set (contract ranges in the docs above).
#[derive(Clone, Copy, Debug)]
pub struct SelectMaskParams {
    /// Edge-detection radius `r` (`0..=250` px; 0 = stages 2–3 skipped).
    pub radius: f64,
    /// Stage-2 smart radius (per-pixel `r/4..=4r`).
    pub smart_radius: bool,
    /// Stage-4 smooth (`0..=100` → disk radius px).
    pub smooth: f64,
    /// Stage-5 feather radius (`0..=250` px; σ = radius/2).
    pub feather: f64,
    /// Stage-6 contrast `c` (`0..=1`).
    pub contrast: f64,
    /// Stage-7 shift (`−100..=100`, percent).
    pub shift_edge: f64,
    /// Stage-8 decontaminate colors.
    pub decontaminate: bool,
    /// Stage-3 guided-filter regularizer (default 1e-4).
    pub eps: f64,
}

impl Default for SelectMaskParams {
    fn default() -> SelectMaskParams {
        SelectMaskParams {
            radius: 8.0,
            smart_radius: false,
            smooth: 0.0,
            feather: 0.0,
            contrast: 0.0,
            shift_edge: 0.0,
            decontaminate: false,
            eps: 1e-4,
        }
    }
}

/// §13.4 stage-9 output modes → emitted catalog kinds (the table
/// verbatim; the workspace consumer emits them through the doors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// `selection.set` (replace).
    Selection,
    /// `mask.add{from:"selection"}`.
    Mask,
    /// `layer.duplicate` + `mask.add` + `mask.apply`.
    NewLayer,
    /// `layer.duplicate` + `mask.add`.
    NewLayerWithMask,
}

impl OutputMode {
    /// §13.4 stage 8: decontaminate rewrites colors, so it forces the
    /// new-layer output modes and never touches the source layer.
    pub fn allowed_with_decontaminate(self) -> bool {
        matches!(self, OutputMode::NewLayer | OutputMode::NewLayerWithMask)
    }
}

/// Pipeline result: the stage-7 matte and (when stage 8 ran) the
/// defringed color planes for the forced new-layer outputs.
pub struct SelectMaskResult {
    /// The refined matte over the input region.
    pub matte: MaskPlane,
    /// Stage-8 `F̂` planes (region-sized) — `Some` iff
    /// `params.decontaminate`.
    pub decontaminated: Option<[Plane; 3]>,
}

/// Run ES §13.4 stages 1–8 over one region. `rgb` are straight
/// working-RGB guidance planes covering `region` (row-major,
/// `region.w × region.h`); `m0` is the current selection/mask (reads
/// outside its rect are 0).
pub fn select_and_mask(
    rgb: &[Plane; 3],
    region: IRect,
    m0: &MaskPlane,
    params: &SelectMaskParams,
) -> Result<SelectMaskResult, String> {
    let (w, h) = (
        region.width().max(0) as usize,
        region.height().max(0) as usize,
    );
    if w == 0 || h == 0 {
        return Err("select_and_mask: empty region".to_string());
    }
    for p in rgb {
        if p.w != w || p.h != h {
            return Err(format!(
                "select_and_mask: rgb {}x{} vs region {}x{} dimension mismatch",
                p.w, p.h, w, h
            ));
        }
    }
    if !(params.radius.is_finite() && (0.0..=250.0).contains(&params.radius)) {
        return Err("select_and_mask: radius outside 0..=250".to_string());
    }
    if !(params.smooth.is_finite() && (0.0..=100.0).contains(&params.smooth)) {
        return Err("select_and_mask: smooth outside 0..=100".to_string());
    }
    if !(params.feather.is_finite() && (0.0..=250.0).contains(&params.feather)) {
        return Err("select_and_mask: feather outside 0..=250".to_string());
    }
    if !(params.contrast.is_finite() && (0.0..=1.0).contains(&params.contrast)) {
        return Err("select_and_mask: contrast outside 0..=1".to_string());
    }
    if !(params.shift_edge.is_finite() && (-100.0..=100.0).contains(&params.shift_edge)) {
        return Err("select_and_mask: shift_edge outside -100..=100".to_string());
    }

    // Stage 1: m₀ restricted to the region.
    let mut m = MaskPlane::new(region);
    for y in region.min.y..region.max.y {
        for x in region.min.x..region.max.x {
            m.set(x, y, m0.get(x, y).clamp(0.0, 1.0));
        }
    }

    // Stages 2–3 (skipped at r = 0: no edge detection).
    let mut band: Option<Vec<bool>> = None;
    if params.radius >= 1.0 {
        let r = params.radius;
        let d = refine::signed_distance(&m, region);
        // Per-pixel radius r(p): base r, or the smart-radius map.
        let rp: Vec<f32> = if params.smart_radius {
            smart_radius_map(rgb, &d, r, w, h)
        } else {
            vec![r as f32; w * h]
        };
        let mut in_band = vec![false; w * h];
        for i in 0..w * h {
            in_band[i] = d[i].abs() <= rp[i];
        }
        // Stage-3 trimap: hard constraints outside the band.
        let mut trimap = Plane::new(w, h);
        for i in 0..w * h {
            trimap.data[i] = if d[i] < -rp[i] {
                1.0
            } else if d[i] > rp[i] {
                0.0
            } else {
                0.5
            };
        }
        // Luma guide (engine's documented gray-guide seam).
        let mut guide = Plane::new(w, h);
        for i in 0..w * h {
            guide.data[i] =
                0.2126 * rgb[0].data[i] + 0.7152 * rgb[1].data[i] + 0.0722 * rgb[2].data[i];
        }
        let coarse = Plane::from_vec(w, h, m.v.clone());
        let alpha = guided_filter_matting(&guide, &coarse, Some(&trimap), r as usize, params.eps)?;
        m.v.copy_from_slice(&alpha.data);
        band = Some(in_band);
    }

    // Stage 4: Smooth (§13.2 open+close, one final re-AA).
    if params.smooth >= 1.0 {
        m = crop_to(&refine::smooth(&m, params.smooth), region);
    }
    // Stage 5: Feather (σ = radius/2 — refine::feather's contract).
    if params.feather > 0.0 {
        m = crop_to(&refine::feather(&m, params.feather), region);
    }
    // Stage 6: Contrast.
    if params.contrast > 0.0 {
        let c = params.contrast;
        for v in &mut m.v {
            *v = (0.5 + (*v - 0.5) * (1.0 + c) as f32).clamp(0.0, 1.0);
        }
    }
    // Stage 7: Shift edge (profile translation via d, bilinear pull).
    let s = (params.shift_edge / 100.0) * params.radius.max(1.0);
    if s.abs() > 1e-9 {
        m = shift_edge(&m, region, s);
    }

    // Stage 8: Decontaminate.
    let decontaminated = if params.decontaminate {
        let alpha = Plane::from_vec(w, h, m.v.clone());
        let band_plane = band
            .map(|b| Plane::from_vec(w, h, b.iter().map(|&x| if x { 1.0 } else { 0.0 }).collect()));
        let out = decontaminate(
            rgb,
            &alpha,
            band_plane.as_ref(),
            &DecontaminateParams::default(),
        )?;
        Some(out.fg)
    } else {
        None
    };

    Ok(SelectMaskResult {
        matte: m,
        decontaminated,
    })
}

/// Stage-2 smart radius: per-pixel `r(p) ∈ [r/4, 4r]` proportional to
/// the local gradient variance in a 16-px window (v1 pins in the module
/// docs), normalized by the maximum variance over the 4r-inflated band.
fn smart_radius_map(rgb: &[Plane; 3], d: &[f32], r: f64, w: usize, h: usize) -> Vec<f32> {
    // Sobel |∇I| of luma.
    let luma: Vec<f32> = (0..w * h)
        .map(|i| 0.2126 * rgb[0].data[i] + 0.7152 * rgb[1].data[i] + 0.0722 * rgb[2].data[i])
        .collect();
    let at = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        luma[y * w + x]
    };
    let mut grad = vec![0.0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x - 1, y)
                - at(x - 1, y + 1))
                / 4.0;
            let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x, y - 1)
                - at(x + 1, y - 1))
                / 4.0;
            grad[y as usize * w + x as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    // Variance of |∇I| in a 16-px box window (half-width 8), via prefix
    // sums of g and g².
    let hw = 8isize;
    let mut sum = vec![0.0f64; (w + 1) * (h + 1)];
    let mut sq = vec![0.0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        for x in 0..w {
            let g = grad[y * w + x] as f64;
            sum[(y + 1) * (w + 1) + x + 1] =
                g + sum[y * (w + 1) + x + 1] + sum[(y + 1) * (w + 1) + x] - sum[y * (w + 1) + x];
            sq[(y + 1) * (w + 1) + x + 1] =
                g * g + sq[y * (w + 1) + x + 1] + sq[(y + 1) * (w + 1) + x] - sq[y * (w + 1) + x];
        }
    }
    let window_var = |x: usize, y: usize| -> f64 {
        let x0 = (x as isize - hw).max(0) as usize;
        let y0 = (y as isize - hw).max(0) as usize;
        let x1 = (x as isize + hw + 1).min(w as isize) as usize;
        let y1 = (y as isize + hw + 1).min(h as isize) as usize;
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        let s = sum[y1 * (w + 1) + x1] - sum[y0 * (w + 1) + x1] - sum[y1 * (w + 1) + x0]
            + sum[y0 * (w + 1) + x0];
        let q = sq[y1 * (w + 1) + x1] - sq[y0 * (w + 1) + x1] - sq[y1 * (w + 1) + x0]
            + sq[y0 * (w + 1) + x0];
        (q / n - (s / n) * (s / n)).max(0.0)
    };
    let mut var = vec![0.0f64; w * h];
    let mut vmax = 0.0f64;
    let wide = (4.0 * r) as f32;
    for y in 0..h {
        for x in 0..w {
            let v = window_var(x, y);
            var[y * w + x] = v;
            if d[y * w + x].abs() <= wide {
                vmax = vmax.max(v);
            }
        }
    }
    let (lo, hi) = (r / 4.0, 4.0 * r);
    (0..w * h)
        .map(|i| {
            let t = if vmax > 0.0 { var[i] / vmax } else { 0.0 };
            (lo + (hi - lo) * t) as f32
        })
        .collect()
}

/// Stage 7: translate the soft profile by `s` px along the outward
/// distance-field normal (`n̂ = ∇d/|∇d|`), resampling bilinearly — the
/// profile moves, it is not re-thresholded (§13.4).
fn shift_edge(m: &MaskPlane, region: IRect, s: f64) -> MaskPlane {
    let (w, h) = (region.width() as usize, region.height() as usize);
    let d = refine::signed_distance(m, region);
    let dval = |x: isize, y: isize| -> f64 {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        d[y * w + x] as f64
    };
    let sample = |fx: f64, fy: f64| -> f32 {
        // Bilinear over pixel centers, clamped to the region.
        let fx = fx.clamp(0.0, (w - 1) as f64);
        let fy = fy.clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let g = |x: usize, y: usize| m.v[y * w + x];
        let a = g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx;
        let b = g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx;
        a * (1.0 - ty) + b * ty
    };
    let mut out = MaskPlane::new(region);
    for y in 0..h as isize {
        for x in 0..w as isize {
            let gx = (dval(x + 1, y) - dval(x - 1, y)) / 2.0;
            let gy = (dval(x, y + 1) - dval(x, y - 1)) / 2.0;
            let len = (gx * gx + gy * gy).sqrt();
            let (nx, ny) = if len > 1e-9 {
                (gx / len, gy / len)
            } else {
                (0.0, 0.0)
            };
            let v = sample(x as f64 - s * nx, y as f64 - s * ny);
            out.v[y as usize * w + x as usize] = v;
        }
    }
    out
}

/// Restrict a refinement result (which may have inflated bounds) back to
/// the pipeline region.
fn crop_to(m: &MaskPlane, region: IRect) -> MaskPlane {
    let mut out = MaskPlane::new(region);
    for y in region.min.y..region.max.y {
        for x in region.min.x..region.max.x {
            out.set(x, y, m.get(x, y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Region planes for a vertical two-tone edge at `xe` with the mask
    /// edge deliberately offset from the true color edge.
    fn fixture(w: usize, h: usize, xe: usize) -> ([Plane; 3], IRect) {
        let mut r = Plane::new(w, h);
        let mut g = Plane::new(w, h);
        let mut b = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let fg = x < xe;
                *r.at_mut(x, y) = if fg { 0.9 } else { 0.1 };
                *g.at_mut(x, y) = if fg { 0.8 } else { 0.1 };
                *b.at_mut(x, y) = if fg { 0.1 } else { 0.9 };
            }
        }
        ([r, g, b], IRect::new(0, 0, w as i32, h as i32))
    }

    fn rect_mask(region: IRect, r: IRect) -> MaskPlane {
        let mut m = MaskPlane::new(region);
        for y in r.min.y..r.max.y {
            for x in r.min.x..r.max.x {
                m.set(x, y, 1.0);
            }
        }
        m
    }

    #[test]
    fn stage3_matting_pulls_the_matte_toward_the_guidance_edge() {
        // True color edge at x=20; the coarse mask says x<24: the
        // guided matte inside the band leans toward the guidance edge —
        // the misplaced strip (20..24) drops below the coarse value
        // while definite constraints hold outside the band.
        let ([r, g, b], region) = fixture(48, 32, 20);
        let m0 = rect_mask(region, IRect::new(0, 0, 24, 32));
        let out = select_and_mask(
            &[r, g, b],
            region,
            &m0,
            &SelectMaskParams {
                radius: 6.0,
                ..SelectMaskParams::default()
            },
        )
        .unwrap();
        assert_eq!(out.matte.get(2, 16), 1.0, "deep inside pinned (d < -r)");
        assert_eq!(out.matte.get(45, 16), 0.0, "deep outside pinned (d > r)");
        // The band matte leans toward the GUIDANCE edge (he10 α = a·I + b
        // per window): the misplaced strip (color-bg, coarse-fg) drops
        // well below its coarse value 1.0, and sits clearly below the
        // true-foreground side of the band at the same distance from the
        // coarse edge.
        let misplaced = out.matte.get(22, 16);
        assert!(
            misplaced < 0.8,
            "matte pulled off the coarse value in the band: got {misplaced}"
        );
        let true_fg = out.matte.get(19, 16);
        assert!(
            true_fg > misplaced + 0.1,
            "guidance edge separates the band: fg-side {true_fg} vs \
             misplaced {misplaced}"
        );
        assert!(
            out.matte.get(18, 16) > 0.5,
            "true-foreground side of the band stays selected"
        );
        for v in &out.matte.v {
            assert!((0.0..=1.0).contains(v), "§34.4 property 13");
        }
        assert!(out.decontaminated.is_none());
    }

    #[test]
    fn stage_order_contrast_and_shift_move_the_profile() {
        let ([r, g, b], region) = fixture(48, 32, 20);
        let m0 = rect_mask(region, IRect::new(0, 0, 20, 32));
        // radius 0: stages 2-3 skipped; feather then contrast steepens.
        let soft = select_and_mask(
            &[r.clone(), g.clone(), b.clone()],
            region,
            &m0,
            &SelectMaskParams {
                radius: 0.0,
                feather: 6.0,
                ..SelectMaskParams::default()
            },
        )
        .unwrap();
        let steep = select_and_mask(
            &[r.clone(), g.clone(), b.clone()],
            region,
            &m0,
            &SelectMaskParams {
                radius: 0.0,
                feather: 6.0,
                contrast: 1.0,
                ..SelectMaskParams::default()
            },
        )
        .unwrap();
        // Contrast steepens around 0.5: values below drop, above rise.
        let (s_lo, c_lo) = (soft.matte.get(23, 16), steep.matte.get(23, 16));
        let (s_hi, c_hi) = (soft.matte.get(17, 16), steep.matte.get(17, 16));
        assert!(s_lo < 0.5 && c_lo <= s_lo, "below-half steepens down");
        assert!(s_hi > 0.5 && c_hi >= s_hi, "above-half steepens up");

        // Shift edge +50% of max(1, r)=1 ⇒ tiny outward move; use a
        // bigger radius for a visible move: shift +100 with r=4 ⇒ +4 px.
        let shifted = select_and_mask(
            &[r, g, b],
            region,
            &m0,
            &SelectMaskParams {
                radius: 0.0,
                shift_edge: 100.0,
                ..SelectMaskParams::default()
            },
        )
        .unwrap();
        // With r=0, s = (100/100)·max(1,0) = 1 px outward: the column
        // just outside the old edge is now selected.
        assert!(
            shifted.matte.get(20, 16) > 0.5,
            "edge moved outward by s: got {}",
            shifted.matte.get(20, 16)
        );
        assert_eq!(shifted.matte.get(2, 16), 1.0, "interior untouched");
    }

    #[test]
    fn stage8_decontaminate_defringes_and_keeps_alpha() {
        let ([r, g, b], region) = fixture(48, 32, 20);
        let m0 = rect_mask(region, IRect::new(0, 0, 20, 32));
        let out = select_and_mask(
            &[r, g, b],
            region,
            &m0,
            &SelectMaskParams {
                radius: 4.0,
                decontaminate: true,
                ..SelectMaskParams::default()
            },
        )
        .unwrap();
        let fg = out.decontaminated.expect("stage 8 ran");
        // Foreground planes exist, are finite, and keep dimensions.
        assert_eq!(fg[0].w, 48);
        assert_eq!(fg[0].h, 32);
        for p in &fg {
            for v in &p.data {
                assert!(v.is_finite());
            }
        }
        // Deep-foreground color is preserved exactly (α ≥ fg_alpha_min).
        assert!((fg[0].at(2, 16) - 0.9).abs() < 1e-6);
        // Output-mode contract: decontaminate forces new-layer modes.
        assert!(!OutputMode::Selection.allowed_with_decontaminate());
        assert!(!OutputMode::Mask.allowed_with_decontaminate());
        assert!(OutputMode::NewLayer.allowed_with_decontaminate());
        assert!(OutputMode::NewLayerWithMask.allowed_with_decontaminate());
    }

    #[test]
    fn smart_radius_widens_the_band_only_where_the_boundary_is_busy() {
        // Top half: noisy checker boundary (hairy); bottom half: clean
        // edge. Smart radius must resolve MORE matting freedom (wider
        // per-pixel r) at the hairy rows — observable as a softer matte
        // off-edge value there vs the clean rows at the same offset.
        let (w, h) = (64usize, 48usize);
        let mut r = Plane::new(w, h);
        let mut g = Plane::new(w, h);
        let mut b = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let hairy = y < 24;
                let edge = 24 + if hairy { ((x + y) % 7) as i32 - 3 } else { 0 };
                let fg = (x as i32) < edge;
                *r.at_mut(x, y) = if fg { 0.85 } else { 0.15 };
                *g.at_mut(x, y) = if fg { 0.85 } else { 0.15 };
                *b.at_mut(x, y) = if fg { 0.15 } else { 0.85 };
            }
        }
        let region = IRect::new(0, 0, w as i32, h as i32);
        let m0 = rect_mask(region, IRect::new(0, 0, 24, h as i32));
        // The map itself: mean per-pixel radius along the boundary is
        // strictly wider on the hairy rows than on the clean rows.
        let mut m = MaskPlane::new(region);
        for y in 0..h as i32 {
            for x in 0..24 {
                m.set(x, y, 1.0);
            }
        }
        let d = refine::signed_distance(&m, region);
        let rgb_ref = [r.clone(), g.clone(), b.clone()];
        let rp = smart_radius_map(&rgb_ref, &d, 4.0, w, h);
        let band_mean = |rows: std::ops::Range<usize>| -> f64 {
            let mut acc = 0.0f64;
            let mut n = 0usize;
            for y in rows {
                for x in 0..w {
                    if d[y * w + x].abs() <= 4.0 {
                        acc += rp[y * w + x] as f64;
                        n += 1;
                    }
                }
            }
            acc / n as f64
        };
        let hairy_mean = band_mean(2..22);
        let clean_mean = band_mean(26..46);
        assert!(
            hairy_mean > clean_mean,
            "smart radius widens where the boundary is busy: hairy \
             {hairy_mean} vs clean {clean_mean}"
        );
        // Every r(p) honors the §13.4 bracket [r/4, 4r].
        for v in &rp {
            assert!((1.0..=16.0).contains(v), "r(p) {v} outside [r/4, 4r]");
        }
        // The full smart pipeline stays valid coverage.
        let smart = select_and_mask(
            &[r, g, b],
            region,
            &m0,
            &SelectMaskParams {
                radius: 4.0,
                smart_radius: true,
                ..SelectMaskParams::default()
            },
        )
        .unwrap();
        for v in smart.matte.v.iter() {
            assert!((0.0..=1.0).contains(v));
        }
    }
}
