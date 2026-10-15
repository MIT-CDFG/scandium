// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §16.3 Gaussian Blur block (the separable kernel and the
//! Kovesi 3-box path), §11.2 step 2 (mask feather — the SAME kernel at
//! `σ = feather_px/3`, FI-10 Option B).
//!
//! **THE one plane-level Gaussian, and the only place its arithmetic
//! exists.** Relocated here from `appname-tools/src/blur.rs` (which had it
//! from `appname-filters/src/corpus/gaussian.rs`) by the O-INT
//! architecture re-ruling for the §11.2 feather sub-slice.
//!
//! WHY THIS CRATE. Two consumers must share ONE implementation or the
//! §16.3 filter blur and the §11.2 mask feather can drift apart:
//! `appname-tools` (the filter corpus, via [`crate::rgba_blur`] re-exported
//! from `appname_tools::blur`) and `appname-core` (the §11.2 mask
//! evaluator, whose feathered coverage the render compositor consumes).
//! `appname-tools` already depends on `appname-core`, so a `core → tools`
//! edge is a hard cargo cycle. `appname-algo` is a TRUE LEAF — zero
//! `appname-*` dependencies — so `core → algo` cannot cycle now or later.
//!
//! **NAME DISCIPLINE.** [`crate::plane::Plane`] in this crate is a
//! DIFFERENT type — single-channel `{w, h, data: Vec<f32>}` for the
//! engine ports. The RGBA carrier here is deliberately named
//! [`RgbaPlane`] / [`RgbaView`] and namespaced in this module; the two are
//! never merged, shadowed, or "unified".
//!
//! **WHY A LOCAL [`PlaneRect`] AND NOT `appname_core::IRect`.** The
//! upstream `appname_tools::resample::Plane` carries `rect: IRect`, and
//! `IRect` is `appname-core`'s §2.1 document geometry. Moving the RGBA
//! carrier here VERBATIM would therefore drag `IRect` (and `IVec2`, and
//! serde) into this crate and re-create the very cycle the re-ruling
//! exists to avoid — the second latent cycle in this slice. So the kernel
//! travels with a MINIMAL, document-free rect of its own, exactly matching
//! this crate's charter (pure functions over planar data + dimensions,
//! never documents). `appname_tools::blur` adapts `IRect` ↔ [`PlaneRect`]
//! at zero copy (both are four `i32`s and the pixel buffers move whole),
//! so `appname_tools::resample::Plane` and its ~560 mentions across the
//! workspace are untouched and no caller breaks.
//!
//! [NORMATIVE, §16.3] `σ = radius/3` (the kernel's 3σ envelope equals the
//! UI radius); 1-D kernel `w_k = exp(−k²/2σ²)`, `k ∈ [−⌈3σ⌉, +⌈3σ⌉]`,
//! normalized `Σ = 1`; horizontal then vertical pass on premultiplied
//! RGBA (all four channels). For `σ > 32` the implementation SHOULD switch
//! to the 3-box approximation [Kovesi 2010, "Fast almost-Gaussian
//! filtering" — the ES citation].
//!
//! **Kernel reach is [`gaussian_reach`] = `⌈3σ⌉`, and it is the ONE number
//! every ROI/dirty-margin decision keys on** (§16.3 "ROI:
//! `out.inflate(⌈3σ⌉)`"). The 3-box path's total support is the sum of its
//! three box half-widths, which is `≤ ⌈3σ⌉` by construction (asserted in
//! [`tests::box_support_never_exceeds_the_declared_reach`]) — so the
//! declared reach is a true upper bound on BOTH paths, which is what makes
//! it safe as an invalidation margin.
//!
//! **★ Tile-invariance caveat [load-bearing, carried forward].** The
//! direct path is position-invariant to the byte: each output pixel is a
//! fixed-order sum over its taps, so a tiled evaluation and a full-plane
//! evaluation agree exactly. The 3-box path is NOT: [`box_pass`] carries a
//! RUNNING SUM along each scanline (`acc += add − sub`), so the float
//! rounding at a given pixel depends on where that scanline STARTED. Two
//! different `out_rect`s therefore produce different low bits under the box
//! path. **Callers that need byte-stable results across differing output
//! rects MUST evaluate over ONE canonical rect and sample from it — never
//! blur per output tile.** The §11.2 mask feather is exactly such a
//! caller and reaches this path routinely: mask feather ranges to 1000 px,
//! `σ = feather_px/3`, so every `feather_px > 96` takes the box path. See
//! the path choice in [`gaussian_blur_rgba`].
//!
//! Rights basis: Gaussian filtering is textbook signal processing (§16.3
//! [LEGAL] blanket); Kovesi 2010 is the ES-cited published basis for the
//! box path. Select-and-screen judgment only, no release claim.

/// The kernel's own integer rect, half-open `[min, max)` — a
/// document-free stand-in for `appname_core::IRect` that keeps this crate
/// a true leaf (see the module doc). Four `i32`s in the same order, so the
/// adapter in `appname_tools::blur` is field-for-field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlaneRect {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl PlaneRect {
    pub const fn new(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> PlaneRect {
        PlaneRect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.max_x - self.min_x
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.max_y - self.min_y
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Grow (positive) or shrink (negative) on all four sides — the
    /// `IRect::inflate` semantics the ROI math expects.
    #[inline]
    pub fn inflate(&self, dx: i32, dy: i32) -> PlaneRect {
        PlaneRect {
            min_x: self.min_x - dx,
            min_y: self.min_y - dy,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
        }
    }
}

/// Out-of-plane read behavior. Mirrors `appname_tools::resample::EdgePolicy`
/// (the two are adapted, never merged — see the module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeMode {
    /// Reads outside clamp to the nearest edge sample.
    Clamp,
    /// Reads outside are transparent black `(0,0,0,0)` — correct under
    /// premultiplication (§4.3/§16.3).
    Transparent,
}

/// A borrowed dense premultiplied f32 RGBA plane over `rect` (row-major, 4
/// components per px). Borrowed so an `appname_tools::resample::Plane` can
/// enter the kernel with NO copy.
#[derive(Clone, Copy, Debug)]
pub struct RgbaView<'a> {
    pub rect: PlaneRect,
    pub px: &'a [[f32; 4]],
}

impl<'a> RgbaView<'a> {
    /// # Panics
    /// If `px.len()` does not match `rect`'s area.
    pub fn new(rect: PlaneRect, px: &'a [[f32; 4]]) -> RgbaView<'a> {
        assert_eq!(
            px.len(),
            (rect.width().max(0) as usize) * (rect.height().max(0) as usize),
            "RgbaView: pixel count does not match rect"
        );
        RgbaView { rect, px }
    }

    /// Sample at document-space `(x, y)`; outside `rect`, `edge` decides.
    /// Arithmetic identical to `appname_tools::resample::Plane::get`.
    #[inline]
    pub fn get(&self, x: i32, y: i32, edge: EdgeMode) -> [f32; 4] {
        let (w, h) = (self.rect.width(), self.rect.height());
        let (mut ix, mut iy) = (x - self.rect.min_x, y - self.rect.min_y);
        if ix < 0 || iy < 0 || ix >= w || iy >= h {
            match edge {
                EdgeMode::Transparent => return [0.0; 4],
                EdgeMode::Clamp => {
                    ix = ix.clamp(0, w - 1);
                    iy = iy.clamp(0, h - 1);
                }
            }
        }
        self.px[(iy * w + ix) as usize]
    }
}

/// An owned dense premultiplied f32 RGBA plane over `rect`.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbaPlane {
    pub rect: PlaneRect,
    pub px: Vec<[f32; 4]>,
}

impl RgbaPlane {
    pub fn new(rect: PlaneRect) -> RgbaPlane {
        let n = (rect.width().max(0) as usize) * (rect.height().max(0) as usize);
        RgbaPlane {
            rect,
            px: vec![[0.0; 4]; n],
        }
    }

    #[inline]
    pub fn view(&self) -> RgbaView<'_> {
        RgbaView {
            rect: self.rect,
            px: &self.px,
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32, edge: EdgeMode) -> [f32; 4] {
        self.view().get(x, y, edge)
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, v: [f32; 4]) {
        let (ix, iy) = (x - self.rect.min_x, y - self.rect.min_y);
        debug_assert!(ix >= 0 && iy >= 0 && ix < self.rect.width() && iy < self.rect.height());
        self.px[(iy * self.rect.width() + ix) as usize] = v;
    }
}

/// [NORMATIVE, §16.3] The kernel's reach in px: `⌈3σ⌉`. This is the ROI
/// inflation (`out.inflate(⌈3σ⌉)`) and — for the §11.2 mask feather at
/// `σ = feather_px/3` — the `Rmask` invalidation margin. A true upper
/// bound on the support of BOTH the direct and the 3-box path.
#[inline]
pub fn gaussian_reach(sigma: f64) -> i32 {
    if !(sigma > 0.0) {
        return 0;
    }
    (3.0 * sigma).ceil() as i32
}

/// The §16.3 pipeline over planes: direct separable kernel for σ ≤ 32,
/// Kovesi 3-box beyond. Reads outside `input.rect` clamp to its edge
/// (the block's canvas-clamp rule; the evaluator hands a canvas-clipped
/// plane).
///
/// See the module doc's tile-invariance caveat before calling this once
/// per output tile.
pub fn gaussian_blur_rgba(input: RgbaView<'_>, out_rect: PlaneRect, sigma: f64) -> RgbaPlane {
    if sigma <= 0.0 {
        let mut out = RgbaPlane::new(out_rect);
        for y in out_rect.min_y..out_rect.max_y {
            for x in out_rect.min_x..out_rect.max_x {
                out.set(x, y, input.get(x, y, EdgeMode::Clamp));
            }
        }
        return out;
    }
    if sigma > 32.0 {
        // ★ THE PATH CHOICE THE TILE-INVARIANCE CAVEAT IS ABOUT. Below
        // this line the result is NOT rect-invariant: `box_pass` carries a
        // running sum along each scanline, so the low bits at a pixel
        // depend on where that scanline started, i.e. on `out_rect`. A
        // caller that blurs per output tile therefore gets DIFFERENT
        // values for the same pixel depending on which tile asked, and the
        // seam is visible. §11.2 mask feather reaches here for every
        // `feather_px > 96` (σ = feather_px/3, feather ranges to 1000), so
        // its evaluator MUST blur over one canonical rect and sample from
        // it. Pinned by
        // `tests::direct_path_is_rect_invariant_and_the_box_path_is_not`.
        return box3_rgba(input, out_rect, sigma);
    }
    let r = gaussian_reach(sigma) as i64;
    let mut kernel = Vec::with_capacity((2 * r + 1) as usize);
    let mut sum = 0.0f64;
    for k in -r..=r {
        let v = (-((k * k) as f64) / (2.0 * sigma * sigma)).exp();
        kernel.push(v);
        sum += v;
    }
    let kernel: Vec<f32> = kernel.iter().map(|v| (v / sum) as f32).collect();

    // Horizontal into (out cols × needed rows), then vertical.
    let mid_rect = PlaneRect::new(
        out_rect.min_x,
        out_rect.min_y - r as i32,
        out_rect.max_x,
        out_rect.max_y + r as i32,
    );
    let mut mid = RgbaPlane::new(mid_rect);
    for y in mid_rect.min_y..mid_rect.max_y {
        for x in mid_rect.min_x..mid_rect.max_x {
            let mut acc = [0.0f32; 4];
            for (i, &kv) in kernel.iter().enumerate() {
                let sx = x + i as i32 - r as i32;
                let v = input.get(sx, y, EdgeMode::Clamp);
                for c in 0..4 {
                    acc[c] += v[c] * kv;
                }
            }
            mid.set(x, y, acc);
        }
    }
    let mut out = RgbaPlane::new(out_rect);
    for y in out_rect.min_y..out_rect.max_y {
        for x in out_rect.min_x..out_rect.max_x {
            let mut acc = [0.0f32; 4];
            for (i, &kv) in kernel.iter().enumerate() {
                let sy = y + i as i32 - r as i32;
                let v = mid.get(x, sy, EdgeMode::Clamp);
                for c in 0..4 {
                    acc[c] += v[c] * kv;
                }
            }
            out.set(x, y, acc);
        }
    }
    out
}

/// The three Kovesi box half-widths for `σ` (§16.3): ideal
/// `w = sqrt(12σ²/3 + 1)`, `wl` largest odd ≤ w, `wu = wl + 2`,
/// `m = round((12σ² − 3wl² − 12wl − 9)/(−4wl − 4))`; `m` passes of `wl`
/// then `3 − m` of `wu`. Exposed so the reach bound is testable.
pub(crate) fn box3_half_widths(sigma: f64) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - (3 * wl * wl) as f64 - (12 * wl) as f64 - 9.0)
        / ((-4 * wl) as f64 - 4.0))
        .round() as i64;
    let mut out = [0usize; 3];
    for (pass, slot) in out.iter_mut().enumerate() {
        let w = if (pass as i64) < m { wl } else { wu };
        *slot = (w / 2) as usize;
    }
    out
}

/// §16.3 3-box path (σ > 32): the Kovesi widths, three horizontal then
/// three vertical running-sum passes over the materialized ROI.
fn box3_rgba(input: RgbaView<'_>, out_rect: PlaneRect, sigma: f64) -> RgbaPlane {
    let widths = box3_half_widths(sigma);
    let reach = gaussian_reach(sigma);
    let work_rect = out_rect.inflate(reach, reach);
    let mut work = RgbaPlane::new(work_rect);
    for y in work_rect.min_y..work_rect.max_y {
        for x in work_rect.min_x..work_rect.max_x {
            work.set(x, y, input.get(x, y, EdgeMode::Clamp));
        }
    }
    let (w, h) = (work_rect.width() as usize, work_rect.height() as usize);
    for r in widths {
        box_pass(&mut work.px, w, h, r, true);
    }
    for r in widths {
        box_pass(&mut work.px, w, h, r, false);
    }
    let mut out = RgbaPlane::new(out_rect);
    for y in out_rect.min_y..out_rect.max_y {
        for x in out_rect.min_x..out_rect.max_x {
            out.set(x, y, work.get(x, y, EdgeMode::Clamp));
        }
    }
    out
}

/// One running-sum box pass. NOTE the module doc's tile-invariance
/// caveat: `acc` is carried along the scanline, so results depend on where
/// the line started.
///
/// Each of the four channels accumulates INDEPENDENTLY (`acc[c] += …`),
/// with no cross-channel term anywhere in this function or in the direct
/// path above. That is what makes blurring a splatted single-channel
/// coverage value `[v, v, v, v]` byte-identical, per channel, to a true
/// single-channel blur — the property the §11.2 mask feather rides so it
/// can reuse this RGBA kernel unchanged instead of forking a second one.
fn box_pass(px: &mut [[f32; 4]], w: usize, h: usize, r: usize, horizontal: bool) {
    let norm = 1.0 / (2 * r + 1) as f32;
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let idx = |o: usize, i: usize| if horizontal { o * w + i } else { i * w + o };
    let mut line = vec![[0.0f32; 4]; inner];
    for o in 0..outer {
        for i in 0..inner {
            line[i] = px[idx(o, i)];
        }
        let clamp_at = |i: i64| line[(i.clamp(0, inner as i64 - 1)) as usize];
        let mut acc = [0.0f32; 4];
        for i in -(r as i64)..=(r as i64) {
            let v = clamp_at(i);
            for c in 0..4 {
                acc[c] += v[c];
            }
        }
        for i in 0..inner {
            for c in 0..4 {
                px[idx(o, i)][c] = acc[c] * norm;
            }
            let add = clamp_at(i as i64 + r as i64 + 1);
            let sub = clamp_at(i as i64 - r as i64);
            for c in 0..4 {
                acc[c] += add[c] - sub[c];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(at: (i32, i32), extent: PlaneRect) -> RgbaPlane {
        let mut p = RgbaPlane::new(extent);
        p.set(at.0, at.1, [1.0, 0.5, 0.25, 1.0]);
        p
    }

    #[test]
    fn reach_is_ceil_three_sigma() {
        assert_eq!(gaussian_reach(3.0), 9);
        assert_eq!(gaussian_reach(2.5), 8); // ⌈7.5⌉
        assert_eq!(gaussian_reach(0.0), 0);
    }

    /// The bound the §11.2 `Rmask` margin rests on: the 3-box path's total
    /// support (sum of its three half-widths, per axis) never exceeds the
    /// DECLARED reach `⌈3σ⌉`. If this ever failed, a dirty margin of
    /// `⌈3σ⌉` would be too small under the box path and seams would return.
    #[test]
    fn box_support_never_exceeds_the_declared_reach() {
        let mut sigma = 32.5f64;
        while sigma <= 400.0 {
            let support: usize = box3_half_widths(sigma).iter().sum();
            let reach = gaussian_reach(sigma) as usize;
            assert!(
                support <= reach,
                "σ={sigma}: box support {support} > declared reach {reach}"
            );
            sigma += 0.5;
        }
    }

    #[test]
    fn impulse_response_is_the_separable_kernel() {
        // σ = 2 (radius 6): response at (dx,0) relative to center equals
        // kernel[dx] exactly (outer product row through the impulse).
        let extent = PlaneRect::new(0, 0, 41, 41);
        let src = impulse((20, 20), extent);
        let out = gaussian_blur_rgba(src.view(), extent, 2.0);
        let center = out.get(20, 20, EdgeMode::Transparent)[0];
        assert!(center > 0.0);
        let sigma = 2.0f64;
        for dx in 1..=5i32 {
            let want = (-((dx * dx) as f64) / (2.0 * sigma * sigma)).exp() as f32;
            let got = out.get(20 + dx, 20, EdgeMode::Transparent)[0] / center;
            assert!((got - want).abs() < 1e-4, "dx={dx}: {got} vs {want}");
            // Separability: the diagonal is the product of the axes.
            let diag = out.get(20 + dx, 20 + dx, EdgeMode::Transparent)[0] / center;
            assert!(
                (diag - want * want).abs() < 1e-4,
                "diag {dx}: {diag} vs {}",
                want * want
            );
        }
        // Mass preserved (Σ kernel = 1) on all four channels.
        for c in 0..4 {
            let total: f64 = out.px.iter().map(|p| p[c] as f64).sum();
            let src_total: f64 = src.px.iter().map(|p| p[c] as f64).sum();
            assert!(
                (total - src_total).abs() < 1e-3,
                "channel {c} mass {total} vs {src_total}"
            );
        }
    }

    #[test]
    fn constants_survive_and_premultiplied_edges_darken_naturally() {
        let extent = PlaneRect::new(0, 0, 32, 32);
        let mut src = RgbaPlane::new(extent);
        src.px.fill([0.25, 0.5, 0.125, 1.0]);
        let out = gaussian_blur_rgba(src.view(), extent, 4.0);
        for p in &out.px {
            for c in 0..4 {
                assert!((p[c] - [0.25, 0.5, 0.125, 1.0][c]).abs() < 1e-5);
            }
        }
        // Half-transparent plane: the premul edge mixes toward zero —
        // no color leaks past alpha (§16.3 "correct look" note).
        let mut src = RgbaPlane::new(extent);
        for y in 0..32 {
            for x in 0..16 {
                src.set(x, y, [1.0, 0.0, 0.0, 1.0]);
            }
        }
        let out = gaussian_blur_rgba(src.view(), extent, 3.0);
        for p in &out.px {
            assert!(p[0] <= p[3] + 1e-5, "premul validity: {p:?}");
        }
    }

    #[test]
    fn box_path_stays_close_to_the_direct_kernel_at_the_crossover() {
        let extent = PlaneRect::new(0, 0, 96, 8);
        let mut src = RgbaPlane::new(extent);
        for y in 0..8 {
            for x in 0..96 {
                let v = x as f32 / 95.0;
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let direct = gaussian_blur_rgba(src.view(), extent, 32.0);
        let boxed = gaussian_blur_rgba(src.view(), extent, 33.0);
        let mut worst = 0.0f32;
        for (a, b) in boxed.px.iter().zip(direct.px.iter()) {
            worst = worst.max((a[0] - b[0]).abs());
        }
        assert!(
            worst < 0.02,
            "box(33) vs direct(32) drift bound, worst {worst}"
        );
        // Constants still exact through the box path.
        let mut flat = RgbaPlane::new(extent);
        flat.px.fill([0.6, 0.3, 0.2, 1.0]);
        let out = gaussian_blur_rgba(flat.view(), extent, 40.0);
        for p in &out.px {
            assert!((p[0] - 0.6).abs() < 1e-4 && (p[3] - 1.0).abs() < 1e-4);
        }
    }

    /// **THE seam property `Rmask` rests on**, measured against the real
    /// kernel: a ONE-PIXEL change to the mask coverage changes the
    /// feathered output *within* Chebyshev distance `Rmask = ⌈3σ⌉` and
    /// **nowhere beyond it**. That is precisely what makes `Rmask` a
    /// sufficient invalidation margin — an incremental compositor that
    /// re-renders the edit rect inflated by `Rmask` re-renders every pixel
    /// that actually changed.
    ///
    /// The test also shows the margin is TIGHT, not merely safe: there is
    /// a pixel at distance exactly `Rmask` whose value DID change, so a
    /// margin of `Rmask − 1` would miss it and leave a seam.
    #[test]
    fn a_one_pixel_mask_edit_changes_exactly_the_rmask_neighbourhood() {
        // feather_px = 12 ⇒ σ = 4 ⇒ Rmask = ⌈3σ⌉ = 12 = ⌈feather_px⌉.
        let feather_px = 12.0f64;
        let sigma = feather_px / 3.0;
        let r = gaussian_reach(sigma);
        assert_eq!(r, feather_px.ceil() as i32, "the FI-10 identity");

        let extent = PlaneRect::new(-40, -40, 41, 41);
        let base = RgbaPlane::new(extent); // all-zero coverage
        let mut edited = RgbaPlane::new(extent);
        edited.set(0, 0, [1.0, 1.0, 1.0, 1.0]); // the one-pixel mask edit

        let a = gaussian_blur_rgba(base.view(), extent, sigma);
        let b = gaussian_blur_rgba(edited.view(), extent, sigma);

        let mut tight_witness = false;
        for y in extent.min_y..extent.max_y {
            for x in extent.min_x..extent.max_x {
                let d =
                    b.get(x, y, EdgeMode::Transparent)[0] - a.get(x, y, EdgeMode::Transparent)[0];
                let cheb = x.abs().max(y.abs());
                if cheb > r {
                    assert_eq!(
                        d, 0.0,
                        "feather leaked past Rmask={r} at ({x},{y}) (Chebyshev {cheb})"
                    );
                } else if cheb == r && d != 0.0 {
                    tight_witness = true;
                }
            }
        }
        assert!(
            tight_witness,
            "no changed pixel at distance exactly Rmask={r}: the margin would be \
             loose, and this test could not tell Rmask from Rmask−1"
        );
    }

    /// TEETH for the rect-invariance property below. The σ = 4 case in
    /// [`direct_path_is_rect_invariant_and_the_box_path_is_not`] cannot
    /// tell a CORRECT vertical intermediate (`mid_rect` inflated by `r`)
    /// from an UNDER-inflated one: with `r = 12` on a 24-row plane every
    /// `mid` row the vertical pass reaches lies in the Clamp-extended
    /// region, where neighbouring rows are equal by construction — so
    /// shrinking `mid_rect` moves the full-plane and the sub-rect result
    /// IDENTICALLY and rect-invariance survives a genuine seam bug.
    /// (Measured: shrinking `mid_rect` by one row per edge leaves that
    /// test green.)
    ///
    /// Here σ = 1 (`r = 3`) and the sub-rect's inflated row span stays
    /// strictly INSIDE the source content, where consecutive rows differ.
    /// An under-inflated `mid_rect` then clamps the sub-rect's outermost
    /// taps onto the wrong row while the full-plane blur reads the right
    /// one, the two disagree, and this test reds.
    #[test]
    fn rect_invariance_has_teeth_against_an_under_inflated_intermediate() {
        let extent = PlaneRect::new(0, 0, 64, 64);
        let mut src = RgbaPlane::new(extent);
        for y in 0..64 {
            for x in 0..64 {
                // Row-varying content: consecutive rows MUST differ, or a
                // clamped tap would coincide with the correct one and the
                // bug would hide exactly as it does in the σ = 4 case.
                let v = ((x * 5 + y * 11) % 13) as f32 / 12.0;
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let sigma = 1.0f64;
        let r = gaussian_reach(sigma);
        let sub = PlaneRect::new(20, 30, 44, 34);
        assert!(
            sub.min_y - r > extent.min_y && sub.max_y + r < extent.max_y,
            "the sub-rect's inflated span must stay inside the source content, \
             else Clamp hides an under-inflated intermediate"
        );
        let full = gaussian_blur_rgba(src.view(), extent, sigma);
        let part = gaussian_blur_rgba(src.view(), sub, sigma);
        for y in sub.min_y..sub.max_y {
            for x in sub.min_x..sub.max_x {
                assert_eq!(
                    part.get(x, y, EdgeMode::Transparent),
                    full.get(x, y, EdgeMode::Transparent),
                    "an under-inflated vertical intermediate shows here, at ({x},{y})"
                );
            }
        }
    }

    /// EXHIBITS the module doc's tile-invariance caveat as a fact, not a
    /// worry: the DIRECT path agrees to the byte between a full-plane blur
    /// and a sub-rect blur, while the 3-box path does NOT. This is the
    /// reason the §11.2 feather evaluator evaluates over one canonical
    /// rect.
    #[test]
    fn direct_path_is_rect_invariant_and_the_box_path_is_not() {
        let extent = PlaneRect::new(0, 0, 160, 24);
        let mut src = RgbaPlane::new(extent);
        for y in 0..24 {
            for x in 0..160 {
                let v = ((x * 7 + y * 13) % 17) as f32 / 16.0;
                src.set(x, y, [v, v, v, 1.0]);
            }
        }
        let sub = PlaneRect::new(60, 6, 100, 18);

        // Direct path (σ = 4): sub-rect result is byte-identical to the
        // corresponding window of the full-plane result.
        let full = gaussian_blur_rgba(src.view(), extent, 4.0);
        let part = gaussian_blur_rgba(src.view(), sub, 4.0);
        for y in sub.min_y..sub.max_y {
            for x in sub.min_x..sub.max_x {
                assert_eq!(
                    part.get(x, y, EdgeMode::Transparent),
                    full.get(x, y, EdgeMode::Transparent),
                    "direct path must be rect-invariant at ({x},{y})"
                );
            }
        }

        // Box path (σ = 40): the running sum makes it rect-DEPENDENT.
        let full_b = gaussian_blur_rgba(src.view(), extent, 40.0);
        let part_b = gaussian_blur_rgba(src.view(), sub, 40.0);
        let mut any_diff = false;
        for y in sub.min_y..sub.max_y {
            for x in sub.min_x..sub.max_x {
                if part_b.get(x, y, EdgeMode::Transparent)
                    != full_b.get(x, y, EdgeMode::Transparent)
                {
                    any_diff = true;
                }
            }
        }
        assert!(
            any_diff,
            "the box path is expected to be rect-DEPENDENT (running sum); if this \
             ever goes green the caveat can be relaxed — until then the §11.2 \
             feather evaluator must keep using one canonical rect"
        );
    }

    /// **THE PROPERTY THE §11.2 FEATHER RIDES**, measured rather than
    /// argued: blurring a single-channel coverage value SPLATTED across
    /// all four RGBA channels is BYTE-IDENTICAL, per channel, to blurring
    /// it in any one channel. `box_pass` and the direct pass accumulate
    /// each channel independently with no cross-channel term, so the four
    /// lanes are four identical scalar computations.
    ///
    /// This is why the mask feather reuses this RGBA kernel unchanged
    /// (parity with §16.3 by construction) instead of forking a
    /// single-channel twin that could drift. Both paths are exercised —
    /// σ = 4 (direct) and σ = 40 (box).
    #[test]
    fn splatted_single_channel_coverage_blurs_identically_in_all_four_lanes() {
        let extent = PlaneRect::new(0, 0, 48, 40);
        // Splat: every pixel is [v, v, v, v] for a scalar coverage v.
        let mut splat = RgbaPlane::new(extent);
        // Isolate: the SAME v in channel 0 only, zeros elsewhere.
        let mut isolated = RgbaPlane::new(extent);
        for y in 0..40 {
            for x in 0..48 {
                let v = ((x * 3 + y * 7) % 11) as f32 / 10.0;
                splat.set(x, y, [v, v, v, v]);
                isolated.set(x, y, [v, 0.0, 0.0, 0.0]);
            }
        }
        for sigma in [4.0f64, 40.0f64] {
            let a = gaussian_blur_rgba(splat.view(), extent, sigma);
            let b = gaussian_blur_rgba(isolated.view(), extent, sigma);
            for (i, (pa, pb)) in a.px.iter().zip(b.px.iter()).enumerate() {
                // Channel 0 agrees to the BYTE between the splatted and
                // the isolated run: the lanes do not interact.
                assert_eq!(
                    pa[0], pb[0],
                    "σ={sigma}: splat vs isolated diverged at px {i}"
                );
                // And all four splatted lanes carry the same bits.
                for c in 1..4 {
                    assert_eq!(pa[c], pa[0], "σ={sigma}: lane {c} differs at px {i}");
                }
                // The isolated run leaves the other lanes at zero — no
                // cross-channel bleed in either direction.
                for c in 1..4 {
                    assert_eq!(pb[c], 0.0, "σ={sigma}: bleed into lane {c} at px {i}");
                }
            }
        }
    }
}
