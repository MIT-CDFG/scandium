// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.3 "Quick selection" [the five-step normative block,
//! implemented VERBATIM per INTEGRATION-log Ruling A 2026-08-16]:
//! brush-stamp per-channel working-RGB statistics (mean `μ`, variance
//! `σ²`) exponentially decayed with a **half-life of 4 stamps** (step 1);
//! iterative 4-neighbor flood from the stamped region accepting `q` when
//! `maxᶜ |cq − μᶜ| / (2.5·σᶜ + ε) ≤ 1` with `ε = 10⁻⁴` **and**
//! `|∇I(q)| ≤ g₀ + 2·median(|∇I| under the footprint)` (step 2); monotone
//! within-stroke growth (step 3); alt-drag subtract (step 4 — see the
//! reading pin below); per stroke session Smooth(1) + the §13.2 1-px
//! re-AA, then ONE coalesced `selection.set{source:"quick"}` (step 5,
//! §10.5 session rules — the SESSION is the producer's; this engine is
//! the per-session state and emits one source plane at [`QuickSession::
//! finish`]). ES §39.4 T-09 (state machine, options: brush size px = 30
//! default); ALGORITHMS.md §8.B (Ruling A: this statistical flood GOVERNS
//! TOOL BEHAVIOR — **no graph cut in the tool loop**; the GrabCut engine
//! is the SEPARATE refine tier, `select::grabcut_refine`).
//!
//! Deterministic and ML-free: same (pixels, stamps) ⇒ same region.
//!
//! v1 pins (spec leaves open; logged in the lane log):
//! * `∇I` is the 3×3 Sobel gradient magnitude of **Rec. 709 luma** of
//!   straight working RGB, kernel normalized by 1/4 per axis (the §13.3
//!   magnetic-lasso block names Sobel-of-luminance; quick-select's `∇I`
//!   is read the same way).
//! * `g₀` ("a small noise floor") = **0.01**.
//! * Stamp footprint = round disk `‖p − c‖ ≤ D/2`; default diameter 30 px
//!   (§32.3 T-09 default; the contract wire form carries no brush-size
//!   key — gap flagged for the fold-in register).
//! * Step-4 reading: subtract-mode growth runs the SAME statistics/flood
//!   machinery seeded under the brush (the brushed surface IS the
//!   "complement statistics" surface — the user brushes what should
//!   leave the selection); the §13.2 `Subtract` op applies the result,
//!   which is a no-op outside the current selection by that math.

use appname_core::{IRect, IVec2};

use super::refine;
use super::MaskPlane;
use crate::brush::present::Backdrop;

/// §32.3 T-09 default brush size (px).
pub const DEFAULT_BRUSH_DIAMETER: f64 = 30.0;
/// §13.3 step-2 acceptance scale.
const SIGMA_K: f64 = 2.5;
/// §13.3 step-2 ε.
const EPSILON: f64 = 1e-4;
/// §13.3 step-1 half-life (stamps).
const HALF_LIFE_STAMPS: f64 = 4.0;
/// `g₀` noise floor (v1 pin, module docs).
const G0: f64 = 0.01;

/// One stroke session of the quick-selection tool (§13.3 steps 1–3;
/// §39.4 T-09 `active(stroking)`). Growth is monotone across
/// [`QuickSession::stamp`] calls; [`QuickSession::finish`] runs step 5's
/// Smooth(1) + 1-px re-AA and yields the source plane for ONE §13.2
/// boolean write.
pub struct QuickSession {
    canvas: IRect,
    /// Straight working-RGB snapshot of the sampling source over the
    /// canvas (row-major).
    rgb: Vec<[f32; 3]>,
    /// Sobel `|∇I|` of luma (row-major, canvas-sized).
    grad: Vec<f32>,
    /// §13.3 step-1 decayed statistics: total weight, per-channel sum and
    /// sum of squares.
    weight: f64,
    sum: [f64; 3],
    sumsq: [f64; 3],
    /// `|∇I|` samples pushed PER STAMP this stroke — a MULTISET, not the
    /// footprint union: at the 0.25·D spacing successive stamps overlap
    /// ≈4×, so pixels under overlapping stamps re-contribute and the
    /// median weights recently/densely-brushed pixels (review T1
    /// re-record: the pin now states the multiset the code computes;
    /// deterministic either way).
    grad_samples: Vec<f32>,
    /// Monotone accepted set (canvas-sized, row-major).
    accepted: Vec<bool>,
    stamped_any: bool,
}

impl QuickSession {
    /// Snapshot the sampling source (target layer or composite — the
    /// caller's §13.3 `sampleAllLayers` choice) and start an empty
    /// session.
    pub fn new<B: Backdrop + ?Sized>(src: &B, premultiplied: bool, canvas: IRect) -> QuickSession {
        let w = canvas.width().max(0) as usize;
        let h = canvas.height().max(0) as usize;
        let mut rgb = vec![[0.0f32; 3]; w * h];
        let mut luma = vec![0.0f32; w * h];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let p = IVec2::new(canvas.min.x + x, canvas.min.y + y);
                let v = src.read_pixel(p);
                let c = if premultiplied && v[3] > 0.0 {
                    [v[0] / v[3], v[1] / v[3], v[2] / v[3]]
                } else {
                    [v[0], v[1], v[2]]
                };
                let i = y as usize * w + x as usize;
                rgb[i] = c;
                // Rec. 709 luma of straight working RGB (v1 pin).
                luma[i] = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            }
        }
        let grad = sobel_magnitude(&luma, w, h);
        QuickSession {
            canvas,
            rgb,
            grad,
            weight: 0.0,
            sum: [0.0; 3],
            sumsq: [0.0; 3],
            grad_samples: Vec::new(),
            accepted: vec![false; w * h],
            stamped_any: false,
        }
    }

    #[inline]
    fn idx(&self, x: i32, y: i32) -> usize {
        let w = self.canvas.width() as usize;
        (y - self.canvas.min.y) as usize * w + (x - self.canvas.min.x) as usize
    }

    /// §13.3 step 1 + the step-3 re-run: accumulate one brush stamp's
    /// footprint statistics (half-life-4 exponential decay), mark the
    /// footprint as stamped/accepted, then re-grow the region (step 2).
    /// `center` is document-space; `diameter` px.
    pub fn stamp(&mut self, center: [f64; 2], diameter: f64) {
        let r = (diameter / 2.0).max(0.5);
        let (cx, cy) = (center[0], center[1]);
        let x0 = ((cx - r).floor() as i32).max(self.canvas.min.x);
        let x1 = ((cx + r).ceil() as i32 + 1).min(self.canvas.max.x);
        let y0 = ((cy - r).floor() as i32).max(self.canvas.min.y);
        let y1 = ((cy + r).ceil() as i32 + 1).min(self.canvas.max.y);
        // Decay the running model by one stamp (weight halves after 4).
        let lambda = 0.5f64.powf(1.0 / HALF_LIFE_STAMPS);
        self.weight *= lambda;
        for c in 0..3 {
            self.sum[c] *= lambda;
            self.sumsq[c] *= lambda;
        }
        let mut frontier = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
                let i = self.idx(x, y);
                let c = self.rgb[i];
                self.weight += 1.0;
                for (k, &ck) in c.iter().enumerate() {
                    self.sum[k] += ck as f64;
                    self.sumsq[k] += (ck as f64) * (ck as f64);
                }
                self.grad_samples.push(self.grad[i]);
                if !self.accepted[i] {
                    self.accepted[i] = true;
                    frontier.push(IVec2::new(x, y));
                }
                self.stamped_any = true;
            }
        }
        self.grow();
    }

    /// Current per-channel model (`μ`, `σ`).
    fn model(&self) -> ([f64; 3], [f64; 3]) {
        let mut mu = [0.0; 3];
        let mut sigma = [0.0; 3];
        if self.weight > 0.0 {
            for c in 0..3 {
                mu[c] = self.sum[c] / self.weight;
                let var = (self.sumsq[c] / self.weight - mu[c] * mu[c]).max(0.0);
                sigma[c] = var.sqrt();
            }
        }
        (mu, sigma)
    }

    /// §13.3 step-2 edge-stop threshold: `g₀ + 2·median(|∇I| under the
    /// footprint)` — "under the footprint" pinned as the PER-STAMP
    /// MULTISET accumulated this stroke (see `grad_samples`; review T1
    /// re-record — overlapping stamps re-contribute their pixels).
    fn edge_threshold(&self) -> f64 {
        if self.grad_samples.is_empty() {
            return G0;
        }
        let mut s: Vec<f32> = self.grad_samples.clone();
        s.sort_by(f32::total_cmp);
        let n = s.len();
        let median = if n % 2 == 1 {
            s[n / 2] as f64
        } else {
            (s[n / 2 - 1] as f64 + s[n / 2] as f64) / 2.0
        };
        G0 + 2.0 * median
    }

    /// §13.3 step 2: iterative 4-neighbor flood from the accepted region
    /// under the CURRENT model. Monotone: never un-accepts (step 3).
    fn grow(&mut self) {
        let (mu, sigma) = self.model();
        let gate = self.edge_threshold();
        let w = self.canvas.width();
        let h = self.canvas.height();
        // Seed the pass with every accepted pixel that still has an
        // unaccepted 4-neighbor (the region boundary).
        let mut queue: Vec<i32> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if !self.accepted[i] {
                    continue;
                }
                let boundary = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|&(dx, dy)| {
                        let (nx, ny) = (x + dx, y + dy);
                        nx >= 0
                            && ny >= 0
                            && nx < w
                            && ny < h
                            && !self.accepted[(ny * w + nx) as usize]
                    });
                if boundary {
                    queue.push(y * w + x);
                }
            }
        }
        while let Some(i) = queue.pop() {
            let (x, y) = (i % w, i / w);
            for &(dx, dy) in &[(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if self.accepted[j] {
                    continue;
                }
                // Color model test (step 2, ε = 10⁻⁴).
                let c = self.rgb[j];
                let color_ok = (0..3)
                    .all(|k| (c[k] as f64 - mu[k]).abs() / (SIGMA_K * sigma[k] + EPSILON) <= 1.0);
                // Edge stop (step 2).
                if color_ok && (self.grad[j] as f64) <= gate {
                    self.accepted[j] = true;
                    queue.push(ny * w + nx);
                }
            }
        }
    }

    /// Whether `p` is currently in the grown region (test/preview seam —
    /// §39.4 live region tint reads this).
    pub fn contains(&self, p: IVec2) -> bool {
        self.canvas.contains(p) && self.accepted[self.idx(p.x, p.y)]
    }

    /// §13.3 step 5: Smooth(1) + the §13.2 1-px re-AA over the grown
    /// region; the returned plane is the SOURCE for one §13.2 boolean
    /// write (the caller emits the single coalesced
    /// `selection.set{source:"quick"}`). `antialias:false` thresholds at
    /// 0.5 afterwards (§13.1).
    pub fn finish(&self, antialias: bool) -> MaskPlane {
        let w = self.canvas.width();
        let mut bounds = IRect::EMPTY;
        for (i, &a) in self.accepted.iter().enumerate() {
            if a {
                let (x, y) = (
                    self.canvas.min.x + i as i32 % w,
                    self.canvas.min.y + i as i32 / w,
                );
                bounds = bounds.union(&IRect::new(x, y, x + 1, y + 1));
            }
        }
        if bounds.is_empty() {
            return MaskPlane::new(IRect::EMPTY);
        }
        let mut m = MaskPlane::new(bounds);
        for y in bounds.min.y..bounds.max.y {
            for x in bounds.min.x..bounds.max.x {
                if self.accepted[self.idx(x, y)] {
                    m.set(x, y, 1.0);
                }
            }
        }
        let mut out = refine::smooth(&m, 1.0);
        if !antialias {
            out.threshold_half();
        }
        out
    }
}

/// 3×3 Sobel gradient magnitude, per-axis kernel normalized by 1/4,
/// clamped edge reads (v1 pin, module docs).
fn sobel_magnitude(luma: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    if w == 0 || h == 0 {
        return out;
    }
    let at = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        luma[y * w + x]
    };
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
            out[y as usize * w + x as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    out
}

/// Resample a pointer polyline into stamp centers at fixed arc-length
/// spacing (deterministic; first point always stamps). Spacing pin:
/// `max(0.25·diameter, 0.25)` px — the §15.2 brush default-clamp shape;
/// §13.3 says only "under each brush stamp along the drag" (logged).
pub fn stamps_along(points: &[[f64; 2]], diameter: f64) -> Vec<[f64; 2]> {
    let spacing = (0.25 * diameter).max(0.25);
    let mut out = Vec::new();
    let Some(first) = points.first() else {
        return out;
    };
    out.push(*first);
    let mut since = 0.0f64;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        if seg <= 0.0 {
            continue;
        }
        let mut t = 0.0;
        while since + (seg - t) >= spacing {
            let step = spacing - since;
            t += step;
            since = 0.0;
            let f = t / seg;
            out.push([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]);
        }
        since += seg - t;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canvas-backed test source.
    struct Img {
        w: i32,
        h: i32,
        px: Vec<[f32; 4]>,
    }

    impl Img {
        fn fill(w: i32, h: i32, f: impl Fn(i32, i32) -> f32) -> Img {
            let mut px = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let v = f(x, y);
                    px.push([v, v, v, 1.0]);
                }
            }
            Img { w, h, px }
        }
    }

    impl Backdrop for Img {
        fn read_pixel(&self, p: IVec2) -> [f32; 4] {
            if p.x < 0 || p.y < 0 || p.x >= self.w || p.y >= self.h {
                return [0.0; 4];
            }
            self.px[(p.y * self.w + p.x) as usize]
        }
    }

    fn canvas(w: i32, h: i32) -> IRect {
        IRect::new(0, 0, w, h)
    }

    #[test]
    fn flood_respects_the_color_model_two_tone() {
        // Left half 0.2, right half 0.8: a stamp in the left half grows
        // to the left region only — 0.8 fails the 2.5σ test (σ≈0 under a
        // uniform footprint ⇒ razor-sharp model, ε keeps it finite).
        let img = Img::fill(48, 32, |x, _| if x < 24 { 0.2 } else { 0.8 });
        let mut s = QuickSession::new(&img, false, canvas(48, 32));
        s.stamp([10.0, 16.0], 12.0);
        assert!(s.contains(IVec2::new(3, 3)), "left region floods");
        assert!(s.contains(IVec2::new(22, 28)), "left region floods far");
        for y in 0..32 {
            for x in 24..48 {
                assert!(
                    !s.contains(IVec2::new(x, y)),
                    "right region stays out at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn gradient_ridge_stops_the_flood_even_when_color_passes() {
        // Both sides AND the ridge value sit inside the color model (the
        // stamp straddles 0.45/0.55, ridge = 0.5 = μ, so the 2.5σ test
        // PASSES on the ridge), but the x=24 column carries a strong
        // luminance gradient: only the step-2 edge stop rejects it, while
        // the ridge-free twin floods everything.
        let ridged = Img::fill(48, 32, |x, _| match x {
            0..=23 => 0.45,
            24 => 0.5,
            _ => 0.55,
        });
        let mut s = QuickSession::new(&ridged, false, canvas(48, 32));
        // Footprint spans both uniform regions AND the ridge; the median
        // |∇I| under it stays ~0 (uniform majority), so the gate hugs g₀.
        s.stamp([24.0, 16.0], 20.0);
        // The uniform interiors flood from the stamped seeds…
        assert!(s.contains(IVec2::new(2, 2)));
        assert!(s.contains(IVec2::new(45, 29)));
        // …but ridge pixels beyond the stamped footprint stay rejected.
        assert!(
            !s.contains(IVec2::new(24, 0)) && !s.contains(IVec2::new(24, 31)),
            "gradient ridge never accepted outside the stamped footprint"
        );

        let flat = Img::fill(48, 32, |_, _| 0.5);
        let mut f = QuickSession::new(&flat, false, canvas(48, 32));
        f.stamp([24.0, 16.0], 20.0);
        assert!(
            f.contains(IVec2::new(24, 0)) && f.contains(IVec2::new(24, 31)),
            "without the ridge the same stamp floods the column"
        );
    }

    #[test]
    fn growth_is_monotone_within_a_stroke_and_the_model_decays() {
        // Left 0.2, right 0.8. Stamp left, then drag onto the right:
        // the decayed model tracks the surface (step 1) so the right
        // region joins, and nothing accepted ever leaves (step 3).
        let img = Img::fill(64, 32, |x, _| if x < 32 { 0.2 } else { 0.8 });
        let mut s = QuickSession::new(&img, false, canvas(64, 32));
        s.stamp([16.0, 16.0], 12.0);
        let before: Vec<bool> = s.accepted.clone();
        assert!(s.contains(IVec2::new(5, 5)));
        assert!(!s.contains(IVec2::new(50, 16)), "right not yet brushed");
        // Drag across: stamps march into the right region; after the
        // half-life decay the model accepts 0.8.
        for i in 0..8 {
            s.stamp([26.0 + 4.0 * i as f64, 16.0], 12.0);
        }
        for (i, was) in before.iter().enumerate() {
            if *was {
                assert!(s.accepted[i], "monotone growth: index {i} regressed");
            }
        }
        assert!(s.contains(IVec2::new(60, 30)), "right region joined");
    }

    #[test]
    fn finish_smooths_and_reaas_and_threshold_snaps() {
        let img = Img::fill(32, 32, |x, _| if x < 16 { 0.3 } else { 0.9 });
        let mut s = QuickSession::new(&img, false, canvas(32, 32));
        s.stamp([8.0, 16.0], 10.0);
        let aa = s.finish(true);
        assert!(!aa.rect.is_empty());
        // Interior fully selected; a 1-px AA band may ride the contour.
        assert_eq!(aa.get(8, 16), 1.0);
        // §34.4 property 13: coverage in [0,1].
        for v in &aa.v {
            assert!((0.0..=1.0).contains(v));
        }
        let hard = s.finish(false);
        for v in &hard.v {
            assert!(*v == 0.0 || *v == 1.0, "antialias:false snaps at 0.5");
        }
    }

    #[test]
    fn stamps_along_resamples_by_arc_length_deterministically() {
        let pts = [[0.0, 0.0], [30.0, 0.0]];
        let stamps = stamps_along(&pts, 30.0); // spacing 7.5
        assert_eq!(stamps.len(), 5, "0, 7.5, 15, 22.5, 30");
        assert_eq!(stamps[0], [0.0, 0.0]);
        assert!((stamps[1][0] - 7.5).abs() < 1e-9);
        assert!((stamps[4][0] - 30.0).abs() < 1e-9);
        // Degenerate: single point = single stamp; empty = none.
        assert_eq!(stamps_along(&[[3.0, 4.0]], 30.0), vec![[3.0, 4.0]]);
        assert!(stamps_along(&[], 30.0).is_empty());
    }
}
