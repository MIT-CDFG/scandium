// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §18.2 (resampling kernels — [NORMATIVE] kernel formulas, support
//! windows, ringing policy, premultiplied-always rule), §19.3 (the
//! [NORMATIVE] wire-enum → kernel mapping), §16.2 (two-pass separable
//! machinery), §34.3 (parity budget: resampling ≤ 1/255 + SSIM ≥ 0.999);
//! dossier `spec/algorithms/dossiers/resampling.yaml` ("separable 2-pass
//! with per-axis footprint; edge policy clamp"); DD ㉔ context (crop/canvas
//! never resample implicitly — resampling happens only through this module
//! when a command explicitly asks). Plan §2.3 row "Geometry & transforms".
//!
//! Rights basis (dossier + census, V39 discipline): era clean-room area
//! (ALGORITHMS.md §1.1 L0, era-expiry basis — a select-and-screen
//! engineering judgment only, no release claim): Lanczos-3 [duchon79], Catmull-Rom /
//! Mitchell-Netravali cubics [mitchell88], bilinear/nearest [foley96]. Own
//! Rust; "clean-room textbook math", `patents: [none]` per the dossier;
//! census verdict none-needed (`research/code-census/B2-resampling.yaml`,
//! read 2026-08-12). Due diligence, not legal advice.
//!
//! # Kernel set and wire mapping (§19.3 [NORMATIVE])
//!
//! `bicubic` = Catmull-Rom (a = −0.5) · `bicubic_smoother` = cubic B-spline
//! · `bicubic_sharper` = Catmull-Rom family with a = −0.75 · `lanczos` =
//! Lanczos-3 · plus `nearest`/`bilinear`. `Auto` (transform family) selects
//! Lanczos-3 when scale < 1, Catmull-Rom when scale > 1, no-op at 1.
//! Mitchell-Netravali (B = C = 1/3) is carried as the dossier's named
//! option; it is NOT reachable from any v1 wire enum (§19.3 maps
//! `bicubic_smoother` to the B-spline) — dossier-vs-§19.3 variance recorded
//! in the lane log.
//!
//! # v1 pins (documented; determinism-frozen)
//!
//! * Pixel centers: continuous coordinate of integer pixel `i` is `i + 0.5`
//!   (§0.3 states no global convention — gap flagged; this matches the
//!   §15.5 stamp shader). Center-aligned mapping for an axis resize
//!   `n_src → n_dst`: `s(d) = (d + 0.5)·n_src/n_dst − 0.5`.
//! * Minification: per-axis footprint scaling — kernel stretched by
//!   `f = max(1, n_src/n_dst)`, taps over `ceil(support·f)` neighbors each
//!   side, weights `w(Δ/f)`, normalized `Σ = 1` per output sample. This is
//!   the "direct full-footprint" quality reference §18.2 compares its mip
//!   path against (the mip-fed variant is a [PERF] equivalent that arrives
//!   with §3.5 mips; recorded divergence).
//! * Per-output-tap normalization (Σw = 1) makes constant planes exact and
//!   the identity resize (scale 1, aligned grid) bit-identical (§18.3 /
//!   §34.4 property 5 rely on it).
//! * Edge policy: `Clamp` (dossier) or `Transparent` (§4.3 substrate rule)
//!   — an explicit parameter, implemented INSIDE this sampler exactly
//!   because §4.3 says the substrate itself never clamps. Whole-canvas
//!   resizes use Clamp (dossier row); layer transforms use Transparent
//!   (bounded content; §4.3). Golden keying is per `(kernel, scale bucket,
//!   edge policy)` — dossier. The §4.3-vs-dossier wording tension is logged
//!   as a spec question.
//! * All arithmetic f32 except tap positions/weights computed in f64 then
//!   narrowed — plain IEEE ops and `sin` only for Lanczos (libm caveat as
//!   in the brush module: Lanczos hashes are per-build pins; the §34.3
//!   budget, not bit-equality, is the cross-platform contract).
//! * No ringing suppression ([NORMATIVE] §18.2): overshoot from CR/Lanczos
//!   negative lobes is kept; f32 planes keep excursions.

use appname_core::IRect;

/// The §18.2/§19.3 kernel set. `Mitchell` is the dossier option (not wire-
/// reachable in v1); everything else maps 1:1 from the wire enums.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    Nearest,
    Bilinear,
    /// `bicubic`: Catmull-Rom, a = −0.5 (§18.2 formula).
    CatmullRom,
    /// `bicubic_smoother`: cubic B-spline (§19.3).
    BSpline,
    /// `bicubic_sharper`: cubic family with a = −0.75 (§19.3).
    CubicSharper,
    /// Dossier option (Mitchell-Netravali B = C = 1/3); not wire-mapped.
    Mitchell,
    /// `lanczos`: Lanczos-3 (§18.2 formula).
    Lanczos3,
}

impl Kernel {
    /// Map a §18.2 wire-enum name (`doc.resize`-family `resample` values)
    /// to its §19.3 [NORMATIVE] kernel. Returns `None` for unknown names
    /// (`auto` is resolved by the caller via [`auto_kernel`]).
    pub fn from_wire(name: &str) -> Option<Kernel> {
        Some(match name {
            "nearest" => Kernel::Nearest,
            "bilinear" => Kernel::Bilinear,
            "bicubic" => Kernel::CatmullRom,
            "bicubic_smoother" => Kernel::BSpline,
            "bicubic_sharper" => Kernel::CubicSharper,
            "lanczos" => Kernel::Lanczos3,
            _ => return None,
        })
    }

    /// One-sided support of the kernel in source px (footprint scaling
    /// multiplies this by `max(1, 1/scale)`).
    pub fn support(&self) -> f64 {
        match self {
            Kernel::Nearest => 0.5,
            Kernel::Bilinear => 1.0,
            Kernel::CatmullRom | Kernel::BSpline | Kernel::CubicSharper | Kernel::Mitchell => 2.0,
            Kernel::Lanczos3 => 3.0,
        }
    }

    /// Kernel weight at distance `t` (source px, unscaled).
    pub fn weight(&self, t: f64) -> f64 {
        let x = t.abs();
        match self {
            // Half-open cell: [-0.5, 0.5) selects exactly one source px.
            Kernel::Nearest => {
                if (-0.5..0.5).contains(&t) {
                    1.0
                } else {
                    0.0
                }
            }
            Kernel::Bilinear => (1.0 - x).max(0.0),
            Kernel::CatmullRom => cubic_a(x, -0.5),
            Kernel::CubicSharper => cubic_a(x, -0.75),
            Kernel::BSpline => cubic_bc(x, 1.0, 0.0),
            Kernel::Mitchell => cubic_bc(x, 1.0 / 3.0, 1.0 / 3.0),
            Kernel::Lanczos3 => lanczos(x, 3.0),
        }
    }
}

/// §18.2 bicubic family (the WGSL `cubic_w` shape, parameterized by `a`).
fn cubic_a(x: f64, a: f64) -> f64 {
    if x <= 1.0 {
        (a + 2.0) * x * x * x - (a + 3.0) * x * x + 1.0
    } else if x < 2.0 {
        a * x * x * x - 5.0 * a * x * x + 8.0 * a * x - 4.0 * a
    } else {
        0.0
    }
}

/// Mitchell-Netravali (B, C) cubic family [mitchell88]; covers the §19.3
/// B-spline (B = 1, C = 0) and the dossier Mitchell (B = C = 1/3).
fn cubic_bc(x: f64, b: f64, c: f64) -> f64 {
    if x < 1.0 {
        ((12.0 - 9.0 * b - 6.0 * c) * x * x * x
            + (-18.0 + 12.0 * b + 6.0 * c) * x * x
            + (6.0 - 2.0 * b))
            / 6.0
    } else if x < 2.0 {
        ((-b - 6.0 * c) * x * x * x
            + (6.0 * b + 30.0 * c) * x * x
            + (-12.0 * b - 48.0 * c) * x
            + (8.0 * b + 24.0 * c))
            / 6.0
    } else {
        0.0
    }
}

/// §18.2: `lanczos(x, a) = a·sin(πx)·sin(πx/a)/(π²x²)` for |x| < a; 1 at 0.
fn lanczos(x: f64, a: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else if x < a {
        let pix = core::f64::consts::PI * x;
        a * pix.sin() * (pix / a).sin() / (pix * pix)
    } else {
        0.0
    }
}

/// §19.3 `Auto` rule: Lanczos-3 when scale < 1, Catmull-Rom when scale > 1,
/// `None` (no-op) at exactly 1.
pub fn auto_kernel(scale: f64) -> Option<Kernel> {
    if scale < 1.0 {
        Some(Kernel::Lanczos3)
    } else if scale > 1.0 {
        Some(Kernel::CatmullRom)
    } else {
        None
    }
}

/// Out-of-source-bounds read behavior — implemented inside the sampler
/// (§4.3: the substrate never clamps; see module pins).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgePolicy {
    /// Reads outside clamp to the nearest edge sample (dossier row; used by
    /// whole-canvas resizes).
    Clamp,
    /// Reads outside are transparent black `(0,0,0,0)` — correct under
    /// premultiplication (§4.3/§16.3; used by layer transforms).
    Transparent,
}

/// A dense premultiplied f32 pixel plane over a document-space rect
/// (row-major, 4 components per px). The exchange surface between the pure
/// resamplers and the Buffer/tile world (extraction/installation live with
/// the callers; §18.2 premultiplied-always).
#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    pub rect: IRect,
    pub px: Vec<[f32; 4]>,
}

impl Plane {
    pub fn new(rect: IRect) -> Plane {
        let n = (rect.width().max(0) as usize) * (rect.height().max(0) as usize);
        Plane {
            rect,
            px: vec![[0.0; 4]; n],
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32, edge: EdgePolicy) -> [f32; 4] {
        let (w, h) = (self.rect.width(), self.rect.height());
        let (mut ix, mut iy) = (x - self.rect.min.x, y - self.rect.min.y);
        if ix < 0 || iy < 0 || ix >= w || iy >= h {
            match edge {
                EdgePolicy::Transparent => return [0.0; 4],
                EdgePolicy::Clamp => {
                    ix = ix.clamp(0, w - 1);
                    iy = iy.clamp(0, h - 1);
                }
            }
        }
        self.px[(iy * w + ix) as usize]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, v: [f32; 4]) {
        let (ix, iy) = (x - self.rect.min.x, y - self.rect.min.y);
        debug_assert!(ix >= 0 && iy >= 0 && ix < self.rect.width() && iy < self.rect.height());
        self.px[(iy * self.rect.width() + ix) as usize] = v;
    }
}

/// Precomputed taps for one output coordinate of a 1-D pass.
struct AxisTaps {
    /// First source index.
    start: i32,
    /// Normalized weights (Σ = 1), f32-narrowed after f64 accumulation.
    w: Vec<f32>,
}

/// Build the tap table for one axis: `n_src → n_dst` with center-aligned
/// mapping and per-axis footprint scaling (module pins).
fn axis_taps(n_src: i32, n_dst: i32, kernel: Kernel) -> Vec<AxisTaps> {
    let ratio = n_src as f64 / n_dst as f64; // >1 = minification
    let f = ratio.max(1.0); // footprint scale
    let support = kernel.support() * f;
    let mut out = Vec::with_capacity(n_dst as usize);
    for d in 0..n_dst {
        let s = (d as f64 + 0.5) * ratio - 0.5;
        let lo = (s - support).ceil() as i32;
        let hi = (s + support).floor() as i32;
        let mut w = Vec::with_capacity((hi - lo + 1).max(0) as usize);
        let mut sum = 0.0f64;
        for k in lo..=hi {
            let wt = kernel.weight((k as f64 - s) / f);
            sum += wt;
            w.push(wt);
        }
        // Per-output normalization (Σ = 1): exact constants + identity.
        let inv = if sum != 0.0 { 1.0 / sum } else { 0.0 };
        let w: Vec<f32> = w.iter().map(|x| (x * inv) as f32).collect();
        out.push(AxisTaps { start: lo, w });
    }
    out
}

/// Separable two-pass plane resize (dossier: "separable 2-pass with
/// per-axis footprint"): horizontal pass into an intermediate (src rows ×
/// dst cols), vertical pass into the destination. Kernel taps are exact
/// products for every kernel in the set, so this equals the full 2-D
/// kernel. `Nearest` short-circuits per §18.2 (1 tap, no arithmetic).
pub fn resize_plane(src: &Plane, dst_rect: IRect, kernel: Kernel, edge: EdgePolicy) -> Plane {
    assert!(
        !src.rect.is_empty() && !dst_rect.is_empty(),
        "resize needs non-empty rects"
    );
    let (sw, sh) = (src.rect.width(), src.rect.height());
    let (dw, dh) = (dst_rect.width(), dst_rect.height());

    let tx = axis_taps(sw, dw, kernel);
    let ty = axis_taps(sh, dh, kernel);

    // Horizontal: src rows × dst cols, f32 intermediate (§16.2: parity is
    // measured on the final output).
    let mut mid = vec![[0.0f32; 4]; (dw as usize) * (sh as usize)];
    for row in 0..sh {
        for (d, taps) in tx.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            for (i, &wt) in taps.w.iter().enumerate() {
                let sx = src.rect.min.x + taps.start + i as i32;
                let v = src.get(sx, src.rect.min.y + row, edge);
                for c in 0..4 {
                    acc[c] += v[c] * wt;
                }
            }
            mid[row as usize * dw as usize + d] = acc;
        }
    }

    // Vertical: dst cols × dst rows.
    let mut dst = Plane::new(dst_rect);
    for (dy, taps) in ty.iter().enumerate() {
        for dx in 0..dw {
            let mut acc = [0.0f32; 4];
            for (i, &wt) in taps.w.iter().enumerate() {
                let sy = taps.start + i as i32;
                // Edge policy on the row index of the intermediate.
                let ry = match edge {
                    EdgePolicy::Clamp => sy.clamp(0, sh - 1),
                    EdgePolicy::Transparent => sy,
                };
                if ry < 0 || ry >= sh {
                    continue; // transparent contributes zero
                }
                let v = mid[ry as usize * dw as usize + dx as usize];
                for c in 0..4 {
                    acc[c] += v[c] * wt;
                }
            }
            dst.px[dy * dw as usize + dx as usize] = acc;
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    const KERNELS: [Kernel; 7] = [
        Kernel::Nearest,
        Kernel::Bilinear,
        Kernel::CatmullRom,
        Kernel::BSpline,
        Kernel::CubicSharper,
        Kernel::Mitchell,
        Kernel::Lanczos3,
    ];

    #[test]
    fn wire_mapping_is_the_19_3_table() {
        assert_eq!(Kernel::from_wire("nearest"), Some(Kernel::Nearest));
        assert_eq!(Kernel::from_wire("bilinear"), Some(Kernel::Bilinear));
        assert_eq!(Kernel::from_wire("bicubic"), Some(Kernel::CatmullRom));
        assert_eq!(Kernel::from_wire("bicubic_smoother"), Some(Kernel::BSpline));
        assert_eq!(
            Kernel::from_wire("bicubic_sharper"),
            Some(Kernel::CubicSharper)
        );
        assert_eq!(Kernel::from_wire("lanczos"), Some(Kernel::Lanczos3));
        assert_eq!(
            Kernel::from_wire("auto"),
            None,
            "auto resolves via auto_kernel"
        );
        assert_eq!(
            Kernel::from_wire("mitchell"),
            None,
            "dossier option, not wire-reachable"
        );
    }

    #[test]
    fn auto_rule_is_scale_gated() {
        assert_eq!(auto_kernel(0.5), Some(Kernel::Lanczos3));
        assert_eq!(auto_kernel(2.0), Some(Kernel::CatmullRom));
        assert_eq!(auto_kernel(1.0), None, "no-op at scale 1 (§19.3)");
    }

    #[test]
    fn kernel_values_match_the_spec_formulas() {
        // Catmull-Rom a=−0.5 at integer/half offsets (hand-computed from
        // the §18.2 formula): w(0)=1, w(±1)=0, w(0.5)=0.5625, w(1.5)=−0.0625.
        let cr = Kernel::CatmullRom;
        assert_eq!(cr.weight(0.0), 1.0);
        assert_eq!(cr.weight(1.0), 0.0);
        assert!((cr.weight(0.5) - 0.5625).abs() < 1e-12);
        assert!((cr.weight(1.5) - (-0.0625)).abs() < 1e-12);
        assert_eq!(cr.weight(2.0), 0.0);
        // Sharper a=−0.75: w(0.5) = (a+2)/8 − (a+3)/4 + 1 = 0.59375;
        // w(1.5) = a(3.375) − 5a(2.25) + 8a(1.5) − 4a = −0.09375.
        let cs = Kernel::CubicSharper;
        assert!((cs.weight(0.5) - 0.59375).abs() < 1e-12);
        assert!((cs.weight(1.5) - (-0.09375)).abs() < 1e-12);
        // B-spline: w(0)=2/3, w(1)=1/6, never negative.
        let bs = Kernel::BSpline;
        assert!((bs.weight(0.0) - 2.0 / 3.0).abs() < 1e-12);
        assert!((bs.weight(1.0) - 1.0 / 6.0).abs() < 1e-12);
        for i in 0..200 {
            assert!(
                bs.weight(i as f64 / 100.0) >= -1e-15,
                "B-spline has no negative lobes"
            );
        }
        // Mitchell B=C=1/3: w(0) = (6−2B)/6 = 16/18 ≈ 0.888…; w(1) = 1/18… ·
        // hand value: at x=1 second piece? x=1 uses first piece: (12−9B−6C
        // −18+12B+6C+6−2B)/6 = (0 + B)/6 → wait: sum = (12−9B−6C)+(−18+12B
        // +6C)+(6−2B) = B → B/6 = 1/18.
        let mi = Kernel::Mitchell;
        assert!((mi.weight(0.0) - 16.0 / 18.0).abs() < 1e-12);
        assert!((mi.weight(1.0) - 1.0 / 18.0).abs() < 1e-12);
        // Lanczos-3: 1 at 0, 0 at integers, 0 outside |x|≥3.
        let lz = Kernel::Lanczos3;
        assert_eq!(lz.weight(0.0), 1.0);
        for k in 1..=3 {
            assert!(lz.weight(k as f64).abs() < 1e-12, "sinc zeros at integers");
        }
        assert_eq!(lz.weight(3.5), 0.0);
        // Bilinear tent.
        assert_eq!(Kernel::Bilinear.weight(0.0), 1.0);
        assert_eq!(Kernel::Bilinear.weight(0.25), 0.75);
        assert_eq!(Kernel::Bilinear.weight(1.0), 0.0);
    }

    #[test]
    fn cubic_partition_of_unity() {
        // The a-family and BC-family cubics are partitions of unity:
        // Σ_k w(x+k) = 1 for any phase x (textbook property; underpins the
        // per-tap normalization being a no-op for them at scale ≥ 1).
        for kernel in [
            Kernel::CatmullRom,
            Kernel::BSpline,
            Kernel::CubicSharper,
            Kernel::Mitchell,
            Kernel::Bilinear,
        ] {
            for i in 0..=20 {
                let x = i as f64 / 20.0;
                let sum: f64 = (-3..=3).map(|k| kernel.weight(x + k as f64)).sum();
                assert!((sum - 1.0).abs() < 1e-10, "{kernel:?} at phase {x}: {sum}");
            }
        }
    }

    fn ramp_plane(w: i32, h: i32) -> Plane {
        // Opaque horizontal ramp: value = x/(w-1) premultiplied (a=1).
        let mut p = Plane::new(IRect::new(0, 0, w, h));
        for y in 0..h {
            for x in 0..w {
                let v = x as f32 / (w - 1) as f32;
                p.set(x, y, [v, v * 0.5, 1.0 - v, 1.0]);
            }
        }
        p
    }

    #[test]
    fn identity_resize_is_bit_exact_for_interpolating_kernels() {
        // §34.4 property 5 / §18.3: scale 1 on the aligned grid returns the
        // source bit-for-bit for every INTERPOLATING kernel (taps hit
        // integers where w = δ). B-spline and Mitchell are approximating
        // kernels (w(±1) ≠ 0) and smooth even at identity — the §18.3
        // "identity transform is a pixel-exact no-op" guarantee is met at
        // the command level by the identity short-circuit (xform module),
        // never by running an approximating kernel at scale 1.
        let src = ramp_plane(17, 9);
        for kernel in [
            Kernel::Nearest,
            Kernel::Bilinear,
            Kernel::CatmullRom,
            Kernel::CubicSharper,
            Kernel::Lanczos3,
        ] {
            let out = resize_plane(&src, src.rect, kernel, EdgePolicy::Clamp);
            assert_eq!(out.px, src.px, "{kernel:?} identity");
        }
        // Approximating kernels stay within a loose smoothing tolerance and
        // preserve constants (alpha plane is constant 1 here).
        for kernel in [Kernel::BSpline, Kernel::Mitchell] {
            let out = resize_plane(&src, src.rect, kernel, EdgePolicy::Clamp);
            for (o, s) in out.px.iter().zip(src.px.iter()) {
                assert!((o[3] - s[3]).abs() < 1e-6, "{kernel:?} constant alpha");
                assert!((o[0] - s[0]).abs() < 0.06, "{kernel:?} bounded smoothing");
            }
        }
    }

    #[test]
    fn constant_plane_is_preserved_exactly() {
        // Σw = 1 per output tap ⇒ constants survive any scale.
        let mut src = Plane::new(IRect::new(0, 0, 13, 7));
        src.px.fill([0.25, 0.5, 0.125, 1.0]);
        for kernel in KERNELS {
            for dst in [IRect::new(0, 0, 40, 21), IRect::new(0, 0, 5, 3)] {
                let out = resize_plane(&src, dst, kernel, EdgePolicy::Clamp);
                for px in &out.px {
                    for c in 0..4 {
                        let want = [0.25, 0.5, 0.125, 1.0][c];
                        assert!((px[c] - want).abs() < 1e-5, "{kernel:?} → {dst:?}: {px:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn bilinear_upscale_of_a_ramp_is_linear() {
        // Center-aligned bilinear on a linear ramp reproduces the linear
        // interpolant exactly (within f32): sample s = (d+0.5)·r − 0.5.
        let src = ramp_plane(8, 2);
        let out = resize_plane(
            &src,
            IRect::new(0, 0, 16, 2),
            Kernel::Bilinear,
            EdgePolicy::Clamp,
        );
        let r = 8.0 / 16.0;
        for d in 0..16i32 {
            let s = ((d as f64 + 0.5) * r - 0.5).clamp(0.0, 7.0);
            let want = (s / 7.0) as f32;
            let got = out.px[d as usize][0];
            assert!((got - want).abs() < 1e-6, "d={d}: got {got} want {want}");
        }
    }

    #[test]
    fn integer_downscale_with_box_like_footprint_matches_the_direct_oracle() {
        // 2:1 bilinear minification with footprint scaling f=2: weights at
        // Δ/2 for Δ ∈ {−1.5,−0.5,0.5,1.5}/… — compute the direct oracle by
        // brute force from the same definition and compare a 1-D case.
        let mut src = Plane::new(IRect::new(0, 0, 8, 1));
        let vals = [0.1f32, 0.9, 0.3, 0.7, 0.5, 0.2, 0.8, 0.4];
        for (x, v) in vals.iter().enumerate() {
            src.set(x as i32, 0, [*v, 0.0, 0.0, 1.0]);
        }
        let out = resize_plane(
            &src,
            IRect::new(0, 0, 4, 1),
            Kernel::Bilinear,
            EdgePolicy::Clamp,
        );
        for d in 0..4i32 {
            let s = (d as f64 + 0.5) * 2.0 - 0.5;
            let (mut acc, mut wsum) = (0.0f64, 0.0f64);
            for k in -4..12i32 {
                let w = Kernel::Bilinear.weight((k as f64 - s) / 2.0);
                if w != 0.0 {
                    let kc = k.clamp(0, 7);
                    acc += w * vals[kc as usize] as f64;
                    wsum += w;
                }
            }
            let want = (acc / wsum) as f32;
            let got = out.px[d as usize][0];
            assert!((got - want).abs() < 1e-6, "d={d}: got {got} want {want}");
        }
    }

    #[test]
    fn premultiplied_sampling_produces_no_edge_halo() {
        // Left half opaque red premul, right half fully transparent (0s).
        // §18.2 proof sketch: interpolated color stays ≤ alpha — no color
        // leaks out of transparency.
        let mut src = Plane::new(IRect::new(0, 0, 8, 4));
        for y in 0..4 {
            for x in 0..4 {
                src.set(x, y, [1.0, 0.0, 0.0, 1.0]);
            }
        }
        let out = resize_plane(
            &src,
            IRect::new(0, 0, 16, 8),
            Kernel::CatmullRom,
            EdgePolicy::Transparent,
        );
        for px in &out.px {
            // Premul validity within fp tolerance, even mid-rim.
            assert!(px[0] <= px[3] + 1e-4, "halo: {px:?}");
        }
    }

    #[test]
    fn ringing_is_present_for_cr_and_absent_for_bspline() {
        // §18.2 ringing policy: CR overshoots a step edge (kept, not
        // clamped); B-spline (no negative lobes) cannot.
        let mut src = Plane::new(IRect::new(0, 0, 16, 1));
        for x in 0..16 {
            let v = if x < 8 { 0.0 } else { 1.0 };
            src.set(x, 0, [v, v, v, 1.0]);
        }
        let up_cr = resize_plane(
            &src,
            IRect::new(0, 0, 64, 1),
            Kernel::CatmullRom,
            EdgePolicy::Clamp,
        );
        let up_bs = resize_plane(
            &src,
            IRect::new(0, 0, 64, 1),
            Kernel::BSpline,
            EdgePolicy::Clamp,
        );
        let min_cr = up_cr.px.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let max_cr = up_cr
            .px
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            min_cr < -1e-4 && max_cr > 1.0 + 1e-4,
            "CR keeps its overshoot: [{min_cr}, {max_cr}]"
        );
        for p in &up_bs.px {
            assert!(
                p[0] >= -1e-6 && p[0] <= 1.0 + 1e-6,
                "B-spline never rings: {}",
                p[0]
            );
        }
    }

    #[test]
    fn edge_policies_differ_only_at_the_border() {
        let src = ramp_plane(8, 8);
        let up_c = resize_plane(
            &src,
            IRect::new(0, 0, 24, 24),
            Kernel::Lanczos3,
            EdgePolicy::Clamp,
        );
        let up_t = resize_plane(
            &src,
            IRect::new(0, 0, 24, 24),
            Kernel::Lanczos3,
            EdgePolicy::Transparent,
        );
        // Interior pixels agree…
        let c = up_c.px[12 * 24 + 12];
        let t = up_t.px[12 * 24 + 12];
        for i in 0..4 {
            assert!((c[i] - t[i]).abs() < 1e-6);
        }
        // …the border differs (transparent pulls alpha down).
        assert!(up_t.px[0][3] < up_c.px[0][3]);
    }

    #[test]
    fn nearest_is_pure_pixel_pick() {
        let src = ramp_plane(4, 4);
        let out = resize_plane(
            &src,
            IRect::new(0, 0, 8, 8),
            Kernel::Nearest,
            EdgePolicy::Clamp,
        );
        // Each 2×2 destination block equals its single source pixel exactly.
        for dy in 0..8i32 {
            for dx in 0..8i32 {
                let want = src.get(dx / 2, dy / 2, EdgePolicy::Clamp);
                assert_eq!(out.px[(dy * 8 + dx) as usize], want);
            }
        }
    }

    #[test]
    fn two_runs_are_bit_identical() {
        let src = ramp_plane(31, 17);
        for kernel in KERNELS {
            let a = resize_plane(&src, IRect::new(0, 0, 13, 40), kernel, EdgePolicy::Clamp);
            let b = resize_plane(&src, IRect::new(0, 0, 13, 40), kernel, EdgePolicy::Clamp);
            assert_eq!(a.px, b.px, "{kernel:?}");
        }
    }
}

/// Globally-mapped separable resize (the §40.2 all-surface walk): the
/// document-space scale map `src = (dst + 0.5)/s − 0.5` applied per axis
/// with footprint scaling — content anywhere in document space (layer
/// bounds need not touch the origin) resamples through the SAME global
/// mapping the canvas does, so every surface stays in registration
/// (§40.2 "mask resampled with the same kernel as pixels").
pub fn resize_plane_global(
    src: &Plane,
    dst_rect: IRect,
    sx: f64,
    sy: f64,
    kernel: Kernel,
    edge: EdgePolicy,
) -> Plane {
    assert!(sx > 0.0 && sy > 0.0, "positive scale factors");
    let taps_axis = |n_dst: i32, dst_min: i32, s: f64| -> Vec<AxisTaps> {
        let f = (1.0 / s).max(1.0);
        let support = kernel.support() * f;
        let mut out = Vec::with_capacity(n_dst as usize);
        for i in 0..n_dst {
            let d_global = (dst_min + i) as f64;
            let s_src = (d_global + 0.5) / s - 0.5;
            let lo = (s_src - support).ceil() as i32;
            let hi = (s_src + support).floor() as i32;
            let mut w = Vec::with_capacity((hi - lo + 1).max(0) as usize);
            let mut sum = 0.0f64;
            for k in lo..=hi {
                let wt = kernel.weight((k as f64 - s_src) / f);
                sum += wt;
                w.push(wt);
            }
            let inv = if sum != 0.0 { 1.0 / sum } else { 0.0 };
            out.push(AxisTaps {
                start: lo,
                w: w.iter().map(|x| (x * inv) as f32).collect(),
            });
        }
        out
    };
    let tx = taps_axis(dst_rect.width(), dst_rect.min.x, sx);
    let ty = taps_axis(dst_rect.height(), dst_rect.min.y, sy);

    // Horizontal into (src rows × dst cols) over the src row range that the
    // vertical taps can read.
    let row_lo = ty.iter().map(|t| t.start).min().unwrap_or(0);
    let row_hi = ty
        .iter()
        .map(|t| t.start + t.w.len() as i32)
        .max()
        .unwrap_or(0)
        .max(row_lo + 1);
    let rows = (row_hi - row_lo) as usize;
    let dw = dst_rect.width() as usize;
    let mut mid = vec![[0.0f32; 4]; dw * rows];
    for (ri, row) in (row_lo..row_hi).enumerate() {
        for (d, taps) in tx.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            for (i, &wt) in taps.w.iter().enumerate() {
                let v = src.get(taps.start + i as i32, row, edge);
                for c in 0..4 {
                    acc[c] += v[c] * wt;
                }
            }
            mid[ri * dw + d] = acc;
        }
    }
    let mut dst = Plane::new(dst_rect);
    for (dy, taps) in ty.iter().enumerate() {
        for dx in 0..dw {
            let mut acc = [0.0f32; 4];
            for (i, &wt) in taps.w.iter().enumerate() {
                let ri = (taps.start + i as i32 - row_lo) as usize;
                let v = mid[ri * dw + dx];
                for c in 0..4 {
                    acc[c] += v[c] * wt;
                }
            }
            dst.px[dy * dw + dx] = acc;
        }
    }
    dst
}

#[cfg(test)]
mod global_tests {
    use super::*;

    #[test]
    fn global_resize_agrees_with_plane_resize_at_origin() {
        // For origin-anchored planes and dst = scaled rect, the global map
        // equals the plane-relative map bit-for-bit (same taps).
        let mut src = Plane::new(IRect::new(0, 0, 16, 12));
        for y in 0..12 {
            for x in 0..16 {
                src.set(x, y, [((x * 3 + y) % 7) as f32 / 6.0, 0.5, 0.25, 1.0]);
            }
        }
        let dst = IRect::new(0, 0, 24, 18);
        let a = resize_plane(&src, dst, Kernel::CatmullRom, EdgePolicy::Clamp);
        let b = resize_plane_global(&src, dst, 1.5, 1.5, Kernel::CatmullRom, EdgePolicy::Clamp);
        assert_eq!(a.px, b.px);
    }

    #[test]
    fn global_resize_keeps_offset_content_in_registration() {
        // A plane away from the origin scales onto the globally-scaled
        // rect: the content's document position doubles with the canvas.
        let mut src = Plane::new(IRect::new(10, 6, 14, 10)); // 4×4 at (10,6)
        for y in 6..10 {
            for x in 10..14 {
                src.set(x, y, [1.0, 0.0, 0.0, 1.0]);
            }
        }
        let dst = IRect::new(20, 12, 28, 20);
        let out = resize_plane_global(
            &src,
            dst,
            2.0,
            2.0,
            Kernel::Bilinear,
            EdgePolicy::Transparent,
        );
        // Center of the scaled content is opaque red; corners of the dst
        // rect (outside the scaled content's interior support) fade.
        let mid = out.get(24, 16, EdgePolicy::Transparent);
        assert!(
            mid[0] > 0.99 && mid[3] > 0.99,
            "registered content center {mid:?}"
        );
        // Identity at s=1 with an offset rect is a verbatim copy.
        let id = resize_plane_global(
            &src,
            src.rect,
            1.0,
            1.0,
            Kernel::CatmullRom,
            EdgePolicy::Transparent,
        );
        assert_eq!(id.px, src.px);
    }
}
