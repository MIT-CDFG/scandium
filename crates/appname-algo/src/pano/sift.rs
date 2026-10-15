// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Scale-invariant feature transform — clean-room from lowe04
//! ("Distinctive Image Features from Scale-Invariant Keypoints", IJCV
//! 60(2), 2004). No reference implementation was consulted (census route:
//! the upstream detector library is a non-staged behavior reference with
//! `files_taken: []`; this file works from the paper alone).
//!
//! Rights: contract `pano_align_stack_flow` `features.classical` row
//! `sift_cleared_US6711293`; patent P-6711293 verified in
//! `contracts/algorithms/patent-register.yaml` — status
//! "Expired - Lifetime", expiry 2020-03-06 (register read this session).
//!
//! Pipeline (paper section in parentheses; every parameter default is the
//! paper's, each documented on [`SiftParams`]):
//! 1. Gaussian scale space + difference-of-Gaussian pyramid (§3): base
//!    image optionally doubled (bilinear), pre-blurred to `sigma0`
//!    accounting for the assumed capture blur; `octave_scales + 3`
//!    Gaussians per octave; octave step decimates the `2·sigma0` image.
//! 2. 3-D extrema over the 26-neighborhood (§3.1), subpixel/subscale
//!    Taylor refinement with up to 5 re-centering steps (§4, brown-lowe
//!    interpolation as described by the paper), low-contrast rejection on
//!    the interpolated |D| (§4), and edge rejection via the 2×2 spatial
//!    Hessian trace²/det ratio test (§4.1).
//! 3. Orientation assignment (§5): Gaussian-weighted 36-bin gradient
//!    orientation histogram (window sigma = 1.5 × keypoint scale), peak +
//!    parabolic 3-point interpolation, extra keypoints for every local
//!    peak within 80% of the maximum.
//! 4. Descriptor (§6): 4×4 spatial subregions × 8 orientation bins =
//!    128-d, samples rotated to the keypoint orientation, trilinear
//!    binning, Gaussian weighting (sigma = half the window width),
//!    normalize → clamp at 0.2 → renormalize.
//!
//! Determinism (D-48): no RNG anywhere in this engine; iteration order is
//! octave → scale → row → column, so the output ordering is itself
//! deterministic. Pure function over a planar f32 raster.

use super::{bilinear_resize, decimate2, gaussian_blur};
use crate::plane::Plane;

/// Detector/descriptor parameters. Defaults are the paper's (lowe04).
#[derive(Clone, Copy, Debug)]
pub struct SiftParams {
    /// Scales per octave `S` (paper §3.1: S = 3 sampled scales gave the
    /// best repeatability; `S + 3` Gaussian images are built per octave).
    pub octave_scales: usize,
    /// Base blur `sigma0` of scale-space level 0 (paper §3.1: 1.6).
    pub sigma0: f64,
    /// Blur assumed already present in the input (paper §3.1: 0.5; doubles
    /// to 1.0 when the input is upsampled 2×).
    pub assumed_blur: f64,
    /// Low-contrast rejection threshold on the interpolated |D(x̂)| for
    /// image values in [0, 1] (paper §4: 0.03).
    pub contrast_threshold: f64,
    /// Edge rejection ratio `r` (paper §4.1: r = 10; keypoints with
    /// tr²/det ≥ (r+1)²/r on the 2×2 spatial Hessian are rejected).
    pub edge_ratio: f64,
    /// Double the input with bilinear interpolation before building the
    /// scale space (paper §3.1: doubling roughly quadruples the number of
    /// stable keypoints). Detection coordinates are reported in input
    /// pixels either way.
    pub double_input: bool,
    /// Hard cap on octave count (the natural limit — smallest dimension
    /// falling under 8 px — applies first).
    pub max_octaves: usize,
}

impl Default for SiftParams {
    fn default() -> Self {
        SiftParams {
            octave_scales: 3,
            sigma0: 1.6,
            assumed_blur: 0.5,
            contrast_threshold: 0.03,
            edge_ratio: 10.0,
            double_input: true,
            max_octaves: 8,
        }
    }
}

/// A refined scale-space keypoint (coordinates in input-image pixels).
#[derive(Clone, Copy, Debug)]
pub struct SiftKeypoint {
    /// Column, input-image pixels.
    pub x: f64,
    /// Row, input-image pixels.
    pub y: f64,
    /// Absolute scale (Gaussian sigma in input-image pixels).
    pub sigma: f64,
    /// Interpolated |D(x̂)| contrast response.
    pub response: f64,
    /// Assigned orientation, radians in (−π, π].
    pub orientation: f64,
}

/// Keypoint + 128-d descriptor.
#[derive(Clone, Debug)]
pub struct SiftFeature {
    /// The refined, oriented keypoint.
    pub keypoint: SiftKeypoint,
    /// 4×4×8 gradient-orientation descriptor, normalized, clamped at 0.2,
    /// renormalized (paper §6).
    pub descriptor: [f32; 128],
}

/// Number of orientation-histogram bins (paper §5: 36).
const ORI_BINS: usize = 36;
/// Descriptor grid width (paper §6: 4×4 subregions).
const DESC_WIDTH: usize = 4;
/// Descriptor orientation bins per subregion (paper §6: 8).
const DESC_BINS: usize = 8;
/// Orientation window sigma multiplier (paper §5: 1.5 × scale).
const ORI_SIGMA_FACTOR: f64 = 1.5;
/// Orientation window radius in sigmas (own documented truncation: 3σ of
/// the weighting Gaussian, the paper's "Gaussian-weighted circular
/// window" with the weight below 1.2% of peak outside it).
const ORI_RADIUS_FACTOR: f64 = 3.0;
/// Descriptor subregion width in units of keypoint scale (paper §6 uses a
/// 16×16 sample array around the keypoint at descriptor scale; the
/// customary formulation is 3·sigma per histogram column).
const DESC_SCALE_FACTOR: f64 = 3.0;
/// Secondary-peak acceptance ratio for multiple orientations (paper §5:
/// 80%).
const ORI_PEAK_RATIO: f64 = 0.8;
/// Maximum Taylor re-centering steps (paper §4 cites Brown's convergence
/// in a few steps; 5 is the customary cap, documented as such).
const MAX_INTERP_STEPS: usize = 5;

/// Detect scale-space features and compute their descriptors.
///
/// `img` is a grayscale plane with values nominally in [0, 1] (the
/// contrast threshold is calibrated to that range). Returns features in
/// deterministic octave → scale → row → column → orientation order.
pub fn sift_detect(img: &Plane, p: &SiftParams) -> Result<Vec<SiftFeature>, String> {
    if img.w < 8 || img.h < 8 {
        return Err(format!(
            "sift_detect: image {}x{} below the 8x8 minimum",
            img.w, img.h
        ));
    }
    if p.octave_scales == 0 {
        return Err("sift_detect: octave_scales must be >= 1".into());
    }
    if p.sigma0 <= p.assumed_blur {
        return Err("sift_detect: sigma0 must exceed assumed_blur".into());
    }
    // Deliberate negated-comparison shape: also catches NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(p.edge_ratio > 0.0) || !(p.contrast_threshold > 0.0) {
        return Err("sift_detect: thresholds must be positive".into());
    }
    // Non-finite samples would propagate NaN into the orientation
    // histogram and panic an internal comparison; reject up front
    // (crate convention — same posture as grabcut's trimap validation).
    if let Some(i) = img.data.iter().position(|v| !v.is_finite()) {
        return Err(format!("sift_detect: non-finite input sample at index {i}"));
    }

    // ---- 1. Scale space -------------------------------------------------
    // Base image: optional 2x upsample (which doubles the assumed capture
    // blur), then bring the blur up to sigma0.
    let (base, first_octave_scale) = if p.double_input {
        (bilinear_resize(img, img.w * 2, img.h * 2), 0.5)
    } else {
        (img.clone(), 1.0)
    };
    let assumed = p.assumed_blur * if p.double_input { 2.0 } else { 1.0 };
    let delta = (p.sigma0 * p.sigma0 - assumed * assumed).sqrt();
    let base = gaussian_blur(&base, delta);

    let s = p.octave_scales;
    let n_gauss = s + 3;
    // sigma of scale-space level i (octave-relative).
    let rel_sigma = |i: usize| p.sigma0 * 2f64.powf(i as f64 / s as f64);
    // Incremental blur i-1 -> i.
    let mut inc = vec![0.0; n_gauss];
    for (i, v) in inc.iter_mut().enumerate().skip(1) {
        let (a, b) = (rel_sigma(i - 1), rel_sigma(i));
        *v = (b * b - a * a).sqrt();
    }
    let natural = {
        let m = base.w.min(base.h);
        // Keep octaves while the smallest dimension is at least 8.
        let mut n = 0usize;
        let mut d = m;
        while d >= 8 {
            n += 1;
            d /= 2;
        }
        n.max(1)
    };
    let n_oct = natural.min(p.max_octaves);

    let mut gauss: Vec<Vec<Plane>> = Vec::with_capacity(n_oct);
    for o in 0..n_oct {
        let mut level: Vec<Plane> = Vec::with_capacity(n_gauss);
        let first = if o == 0 {
            base.clone()
        } else {
            // The image at octave-relative sigma 2*sigma0 is index S.
            decimate2(&gauss[o - 1][s])
        };
        level.push(first);
        for i in 1..n_gauss {
            let next = gaussian_blur(&level[i - 1], inc[i]);
            level.push(next);
        }
        gauss.push(level);
    }
    let dog: Vec<Vec<Plane>> = gauss
        .iter()
        .map(|lev| {
            (0..n_gauss - 1)
                .map(|i| {
                    let mut d = Plane::new(lev[i].w, lev[i].h);
                    for j in 0..d.data.len() {
                        d.data[j] = lev[i + 1].data[j] - lev[i].data[j];
                    }
                    d
                })
                .collect()
        })
        .collect();

    // ---- 2..4. Extrema -> refine -> orient -> describe ------------------
    // Pre-filter below half the final contrast threshold (own documented
    // pre-check; the binding test is the interpolated |D(x̂)| >= threshold).
    let prefilter = 0.5 * p.contrast_threshold;
    let mut out: Vec<SiftFeature> = Vec::new();
    for (o, dlev) in dog.iter().enumerate() {
        let (w, h) = (dlev[0].w, dlev[0].h);
        for si in 1..=s {
            for y in 1..h.saturating_sub(1) {
                for x in 1..w.saturating_sub(1) {
                    let v = dlev[si].at(x, y);
                    if (v.abs() as f64) < prefilter {
                        continue;
                    }
                    if !is_extremum(dlev, si, x, y, v) {
                        continue;
                    }
                    let Some(rk) = refine(dlev, si, x, y, s, p) else {
                        continue;
                    };
                    // Absolute coordinates / scale in input pixels.
                    // Octave decimation (`decimate2` keeps even indices)
                    // is offset-free, so base coordinates are x·2^o; the
                    // doubled base was built by the center-aligned
                    // `bilinear_resize` (base pixel x samples input
                    // coordinate (x + 0.5)·0.5 − 0.5 = x/2 − 1/4), so
                    // undoing the doubling subtracts 1/4 after the pure
                    // scale. Without doubling the map is the pure scale.
                    let oct_scale = first_octave_scale * 2f64.powi(o as i32);
                    let center_off = if p.double_input { 0.25 } else { 0.0 };
                    let kx = rk.x * oct_scale - center_off;
                    let ky = rk.y * oct_scale - center_off;
                    let sigma_abs = p.sigma0 * 2f64.powf(rk.s / s as f64) * oct_scale;
                    let sigma_rel = p.sigma0 * 2f64.powf(rk.s / s as f64);
                    // Gradients come from the Gaussian image nearest the
                    // refined scale.
                    let gi = (rk.s.round() as isize).clamp(0, n_gauss as isize - 1) as usize;
                    let gimg = &gauss[o][gi];
                    for ori in orientations(gimg, rk.x, rk.y, sigma_rel) {
                        let kp = SiftKeypoint {
                            x: kx,
                            y: ky,
                            sigma: sigma_abs,
                            response: rk.response,
                            orientation: ori,
                        };
                        if let Some(desc) = describe(gimg, rk.x, rk.y, sigma_rel, ori) {
                            out.push(SiftFeature {
                                keypoint: kp,
                                descriptor: desc,
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Strict 26-neighborhood extremum test in the DoG stack.
fn is_extremum(dog: &[Plane], si: usize, x: usize, y: usize, v: f32) -> bool {
    let cmp_gt = v > 0.0;
    for ds in -1isize..=1 {
        let pl = &dog[(si as isize + ds) as usize];
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                if ds == 0 && dy == 0 && dx == 0 {
                    continue;
                }
                let n = pl.at((x as isize + dx) as usize, (y as isize + dy) as usize);
                if cmp_gt {
                    if v <= n {
                        return false;
                    }
                } else if v >= n {
                    return false;
                }
            }
        }
    }
    true
}

struct Refined {
    x: f64,
    y: f64,
    /// Refined scale-space level index (fractional, in Gaussian-level
    /// units).
    s: f64,
    response: f64,
}

/// Taylor subpixel/subscale refinement + contrast and edge rejection
/// (paper §4/§4.1). Returns None when the candidate is rejected.
fn refine(
    dog: &[Plane],
    si0: usize,
    x0: usize,
    y0: usize,
    s: usize,
    p: &SiftParams,
) -> Option<Refined> {
    let (w, h) = (dog[0].w, dog[0].h);
    let (mut si, mut x, mut y) = (si0 as isize, x0 as isize, y0 as isize);
    let mut offset = [0f64; 3];
    let mut converged = false;
    for _ in 0..MAX_INTERP_STEPS {
        if si < 1 || si > s as isize || x < 1 || x > w as isize - 2 || y < 1 || y > h as isize - 2 {
            return None;
        }
        let d = |ds: isize, dy: isize, dx: isize| -> f64 {
            dog[(si + ds) as usize].at((x + dx) as usize, (y + dy) as usize) as f64
        };
        // Gradient (x, y, s order).
        let g = [
            (d(0, 0, 1) - d(0, 0, -1)) / 2.0,
            (d(0, 1, 0) - d(0, -1, 0)) / 2.0,
            (d(1, 0, 0) - d(-1, 0, 0)) / 2.0,
        ];
        let c = d(0, 0, 0);
        // Hessian, finite differences.
        let dxx = d(0, 0, 1) + d(0, 0, -1) - 2.0 * c;
        let dyy = d(0, 1, 0) + d(0, -1, 0) - 2.0 * c;
        let dss = d(1, 0, 0) + d(-1, 0, 0) - 2.0 * c;
        let dxy = (d(0, 1, 1) - d(0, 1, -1) - d(0, -1, 1) + d(0, -1, -1)) / 4.0;
        let dxs = (d(1, 0, 1) - d(1, 0, -1) - d(-1, 0, 1) + d(-1, 0, -1)) / 4.0;
        let dys = (d(1, 1, 0) - d(1, -1, 0) - d(-1, 1, 0) + d(-1, -1, 0)) / 4.0;
        let hess = [[dxx, dxy, dxs], [dxy, dyy, dys], [dxs, dys, dss]];
        offset = solve3(&hess, &[-g[0], -g[1], -g[2]])?;
        if offset[0].abs() < 0.5 && offset[1].abs() < 0.5 && offset[2].abs() < 0.5 {
            converged = true;
            break;
        }
        x += offset[0].round() as isize;
        y += offset[1].round() as isize;
        si += offset[2].round() as isize;
    }
    if !converged {
        return None;
    }
    // Interpolated contrast |D(x̂)| = D + ½ ∇Dᵀ·x̂ (paper eq. 3).
    let d = |ds: isize, dy: isize, dx: isize| -> f64 {
        dog[(si + ds) as usize].at((x + dx) as usize, (y + dy) as usize) as f64
    };
    let g = [
        (d(0, 0, 1) - d(0, 0, -1)) / 2.0,
        (d(0, 1, 0) - d(0, -1, 0)) / 2.0,
        (d(1, 0, 0) - d(-1, 0, 0)) / 2.0,
    ];
    let d_hat = d(0, 0, 0) + 0.5 * (g[0] * offset[0] + g[1] * offset[1] + g[2] * offset[2]);
    if d_hat.abs() < p.contrast_threshold {
        return None;
    }
    // Edge rejection on the 2x2 spatial Hessian (paper §4.1).
    let c = d(0, 0, 0);
    let dxx = d(0, 0, 1) + d(0, 0, -1) - 2.0 * c;
    let dyy = d(0, 1, 0) + d(0, -1, 0) - 2.0 * c;
    let dxy = (d(0, 1, 1) - d(0, 1, -1) - d(0, -1, 1) + d(0, -1, -1)) / 4.0;
    let tr = dxx + dyy;
    let det = dxx * dyy - dxy * dxy;
    let r = p.edge_ratio;
    if det <= 0.0 || tr * tr * r >= (r + 1.0) * (r + 1.0) * det {
        return None;
    }
    Some(Refined {
        x: x as f64 + offset[0],
        y: y as f64 + offset[1],
        s: si as f64 + offset[2],
        response: d_hat.abs(),
    })
}

/// Solve a symmetric 3×3 linear system by Gaussian elimination with
/// partial pivoting; None on (near-)singular systems.
fn solve3(a: &[[f64; 3]; 3], b: &[f64; 3]) -> Option<[f64; 3]> {
    let mut m = [
        [a[0][0], a[0][1], a[0][2], b[0]],
        [a[1][0], a[1][1], a[1][2], b[1]],
        [a[2][0], a[2][1], a[2][2], b[2]],
    ];
    for col in 0..3 {
        // Partial pivot (deterministic: first maximal row wins).
        let mut piv = col;
        for row in col + 1..3 {
            if m[row][col].abs() > m[piv][col].abs() {
                piv = row;
            }
        }
        if m[piv][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, piv);
        for row in col + 1..3 {
            let f = m[row][col] / m[col][col];
            // Row operation reads pivot row `col` while writing row `row`:
            // two rows of `m` at once, the index math is load-bearing.
            #[allow(clippy::needless_range_loop)]
            for k in col..4 {
                m[row][k] -= f * m[col][k];
            }
        }
    }
    let mut x = [0f64; 3];
    for row in (0..3).rev() {
        let mut s = m[row][3];
        for k in row + 1..3 {
            s -= m[row][k] * x[k];
        }
        x[row] = s / m[row][row];
    }
    Some(x)
}

/// Gradient magnitude/orientation at integer coordinates of a Gaussian
/// image (central differences; None at the 1-px border).
#[inline]
fn gradient(img: &Plane, x: isize, y: isize) -> Option<(f64, f64)> {
    if x < 1 || y < 1 || x >= img.w as isize - 1 || y >= img.h as isize - 1 {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    let dx = img.at(x + 1, y) as f64 - img.at(x - 1, y) as f64;
    let dy = img.at(x, y + 1) as f64 - img.at(x, y - 1) as f64;
    Some(((dx * dx + dy * dy).sqrt(), dy.atan2(dx)))
}

/// Orientation assignment (paper §5): returns every accepted orientation
/// for the keypoint, dominant peak first, then remaining accepted peaks in
/// bin order (deterministic).
fn orientations(img: &Plane, x: f64, y: f64, sigma_rel: f64) -> Vec<f64> {
    let sw = ORI_SIGMA_FACTOR * sigma_rel;
    let radius = (ORI_RADIUS_FACTOR * sw).round().max(1.0) as isize;
    let (cx, cy) = (x.round() as isize, y.round() as isize);
    let mut hist = [0f64; ORI_BINS];
    let denom = 2.0 * sw * sw;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let Some((mag, ang)) = gradient(img, cx + dx, cy + dy) else {
                continue;
            };
            let wgt = (-((dx * dx + dy * dy) as f64) / denom).exp();
            // Map (−π, π] onto [0, ORI_BINS).
            let mut b =
                (ang + std::f64::consts::PI) / (2.0 * std::f64::consts::PI) * ORI_BINS as f64;
            if b >= ORI_BINS as f64 {
                b -= ORI_BINS as f64;
            }
            hist[b as usize % ORI_BINS] += mag * wgt;
        }
    }
    let max = hist.iter().cloned().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    // Dominant bin first (first maximal bin wins — deterministic), then
    // the remaining accepted peaks in ascending bin order.
    let dominant = (0..ORI_BINS)
        .max_by(|&a, &b| hist[a].partial_cmp(&hist[b]).unwrap().then(b.cmp(&a)))
        .unwrap();
    let mut order: Vec<usize> = vec![dominant];
    order.extend((0..ORI_BINS).filter(|&i| i != dominant));
    for i in order {
        let l = hist[(i + ORI_BINS - 1) % ORI_BINS];
        let c = hist[i];
        let r = hist[(i + 1) % ORI_BINS];
        if c < ORI_PEAK_RATIO * max || c <= l || c <= r {
            continue;
        }
        // Parabolic 3-point interpolation of the peak (paper §5).
        let denom_p = l - 2.0 * c + r;
        let off = if denom_p.abs() < 1e-12 {
            0.0
        } else {
            0.5 * (l - r) / denom_p
        };
        let binf = i as f64 + off;
        let mut ang = binf / ORI_BINS as f64 * 2.0 * std::f64::consts::PI - std::f64::consts::PI;
        if ang <= -std::f64::consts::PI {
            ang += 2.0 * std::f64::consts::PI;
        }
        out.push(ang);
    }
    out
}

/// Descriptor extraction (paper §6): 4×4×8 with trilinear binning; None
/// when the sample window falls entirely outside the gradient-valid area.
fn describe(img: &Plane, x: f64, y: f64, sigma_rel: f64, ori: f64) -> Option<[f32; 128]> {
    let hist_width = DESC_SCALE_FACTOR * sigma_rel;
    // Window radius: the rotated 4×4 grid's half-diagonal (customary
    // formulation of the paper's 16×16 sample array at descriptor scale).
    let radius =
        (hist_width * (DESC_WIDTH as f64 + 1.0) * std::f64::consts::SQRT_2 / 2.0).round() as isize;
    let (cos_o, sin_o) = (ori.cos(), ori.sin());
    let (cx, cy) = (x.round() as isize, y.round() as isize);
    let mut hist = [0f64; DESC_WIDTH * DESC_WIDTH * DESC_BINS];
    let weight_denom = 2.0 * (0.5 * DESC_WIDTH as f64) * (0.5 * DESC_WIDTH as f64);
    let mut any = false;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            // Rotate the offset into the keypoint frame.
            let rx = (cos_o * dx as f64 + sin_o * dy as f64) / hist_width;
            let ry = (-sin_o * dx as f64 + cos_o * dy as f64) / hist_width;
            // Bin coordinates in [0, 4): + width/2 − 0.5 centers the grid.
            let bx = rx + DESC_WIDTH as f64 / 2.0 - 0.5;
            let by = ry + DESC_WIDTH as f64 / 2.0 - 0.5;
            if bx <= -1.0 || bx >= DESC_WIDTH as f64 || by <= -1.0 || by >= DESC_WIDTH as f64 {
                continue;
            }
            let Some((mag, ang)) = gradient(img, cx + dx, cy + dy) else {
                continue;
            };
            any = true;
            let wgt = (-(rx * rx + ry * ry) / weight_denom).exp();
            let mut oa = ang - ori;
            while oa < 0.0 {
                oa += 2.0 * std::f64::consts::PI;
            }
            while oa >= 2.0 * std::f64::consts::PI {
                oa -= 2.0 * std::f64::consts::PI;
            }
            let ob = oa / (2.0 * std::f64::consts::PI) * DESC_BINS as f64;
            trilinear_accumulate(&mut hist, bx, by, ob, mag * wgt);
        }
    }
    if !any {
        return None;
    }
    // Normalize -> clamp 0.2 -> renormalize (paper §6.1).
    let mut v: Vec<f64> = hist.to_vec();
    let norm = v.iter().map(|a| a * a).sum::<f64>().sqrt();
    if norm <= 0.0 {
        return None;
    }
    for a in v.iter_mut() {
        *a = (*a / norm).min(0.2);
    }
    let norm2 = v.iter().map(|a| a * a).sum::<f64>().sqrt();
    if norm2 <= 0.0 {
        return None;
    }
    let mut out = [0f32; 128];
    for (i, a) in v.iter().enumerate() {
        out[i] = (*a / norm2) as f32;
    }
    Some(out)
}

/// Trilinear soft-binning of one weighted sample into the 4×4×8 histogram
/// (paper §6: each entry multiplied by 1−d for each dimension).
fn trilinear_accumulate(hist: &mut [f64], bx: f64, by: f64, ob: f64, w: f64) {
    let x0 = bx.floor() as isize;
    let y0 = by.floor() as isize;
    let o0 = ob.floor() as isize;
    let fx = bx - x0 as f64;
    let fy = by - y0 as f64;
    let fo = ob - o0 as f64;
    for (dy, wy) in [(0isize, 1.0 - fy), (1, fy)] {
        let yy = y0 + dy;
        if yy < 0 || yy >= DESC_WIDTH as isize {
            continue;
        }
        for (dx, wx) in [(0isize, 1.0 - fx), (1, fx)] {
            let xx = x0 + dx;
            if xx < 0 || xx >= DESC_WIDTH as isize {
                continue;
            }
            for (do_, wo) in [(0isize, 1.0 - fo), (1, fo)] {
                let oo =
                    ((o0 + do_) % DESC_BINS as isize + DESC_BINS as isize) % DESC_BINS as isize;
                hist[(yy as usize * DESC_WIDTH + xx as usize) * DESC_BINS + oo as usize] +=
                    w * wy * wx * wo;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash;

    /// Analytic Gaussian blob, amplitude 1, on a zero background.
    fn blob(w: usize, h: usize, cx: f64, cy: f64, sigma: f64) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                p.data[y * w + x] = (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp() as f32;
            }
        }
        p
    }

    fn no_double() -> SiftParams {
        SiftParams {
            double_input: false,
            ..SiftParams::default()
        }
    }

    #[test]
    fn gaussian_blob_found_at_known_scale_and_position() {
        // Oracle (derived in-code): for image G(sigma_b), scale-space level
        // value at the center is ∝ sigma_b²/(sigma_b² + sigma²), so the
        // DoG magnitude across sigma maximizes at sigma = sigma_b/sqrt(k),
        // k = 2^(1/S). With sigma_b = 3.2, S = 3: sigma* ≈ 2.85. The
        // detected keypoint must sit within 1 px of the center and within
        // ±35% of sigma* (discrete scale sampling + the assumed input
        // blur of 0.5 px shift the refined value). The center is placed
        // OFF the half-integer lattice (a blob centered exactly between
        // pixels puts the true extremum on the ±0.5 refinement boundary,
        // where the Taylor step legitimately oscillates).
        let sb = 3.2;
        let (cx, cy) = (31.3, 30.6);
        let img = blob(64, 64, cx, cy, sb);
        let feats = sift_detect(&img, &no_double()).unwrap();
        assert!(!feats.is_empty(), "no keypoints on the blob fixture");
        // Strongest response.
        let best = feats
            .iter()
            .max_by(|a, b| {
                a.keypoint
                    .response
                    .partial_cmp(&b.keypoint.response)
                    .unwrap()
            })
            .unwrap();
        let k = &best.keypoint;
        assert!(
            (k.x - cx).abs() < 1.0 && (k.y - cy).abs() < 1.0,
            "blob center missed: ({}, {})",
            k.x,
            k.y
        );
        let sigma_star = sb / 2f64.powf(1.0 / 3.0).sqrt();
        assert!(
            (k.sigma - sigma_star).abs() < 0.35 * sigma_star,
            "scale {} vs expected {}",
            k.sigma,
            sigma_star
        );
    }

    #[test]
    fn pure_edge_fixture_is_rejected() {
        // A smooth straight step edge has high principal curvature ratio:
        // the §4.1 edge test must leave zero keypoints. The blob fixture
        // (same size, same pipeline) keeps at least one — so the rejection
        // is doing the discriminating, not the thresholds.
        let (w, h) = (64, 64);
        let mut edge = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // Smooth vertical step (tanh profile, width ~3 px; center
                // off the half-integer lattice so refinement can
                // converge — rejection must come from the edge test, not
                // from non-convergence).
                let t = (x as f64 - 31.3) / 3.0;
                edge.data[y * w + x] = (0.5 + 0.5 * t.tanh()) as f32;
            }
        }
        let ef = sift_detect(&edge, &no_double()).unwrap();
        assert!(
            ef.is_empty(),
            "edge fixture produced {} keypoints",
            ef.len()
        );
        let bf = sift_detect(&blob(64, 64, 31.3, 30.6, 3.2), &no_double()).unwrap();
        assert!(!bf.is_empty());
    }

    /// Lossless 90° counter-clockwise rotation: (x, y) -> (y, w-1-x).
    fn rot90(img: &Plane) -> Plane {
        let mut out = Plane::new(img.h, img.w);
        for y in 0..img.h {
            for x in 0..img.w {
                // Destination coordinates in the rotated frame.
                let nx = y;
                let ny = img.w - 1 - x;
                out.data[ny * out.w + nx] = img.at(x, y);
            }
        }
        out
    }

    /// Smooth deterministic feature-rich fixture: overlapping Gaussian
    /// blobs of varied position/scale/amplitude (overlaps make local
    /// neighborhoods asymmetric, so orientations are meaningful).
    fn asym_fixture() -> Plane {
        let mut rng = crate::plane::SplitMix64(2026);
        let (w, h) = (96usize, 96usize);
        let blobs: Vec<(f64, f64, f64, f64)> = (0..14)
            .map(|_| {
                (
                    12.0 + rng.next_f64() * (w as f64 - 24.0),
                    12.0 + rng.next_f64() * (h as f64 - 24.0),
                    1.8 + rng.next_f64() * 2.6,
                    0.35 + rng.next_f64() * 0.6,
                )
            })
            .collect();
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let mut v = 0.05f64;
                for &(bx, by, bs, ba) in &blobs {
                    let dx = x as f64 - bx;
                    let dy = y as f64 - by;
                    v += ba * (-(dx * dx + dy * dy) / (2.0 * bs * bs)).exp();
                }
                p.data[y * w + x] = v.min(1.0) as f32;
            }
        }
        p
    }

    #[test]
    fn descriptors_match_under_90_degree_rotation() {
        // Rotation-invariance oracle, run operationally: detect on a
        // feature-rich fixture and its LOSSLESS 90° rotation, match with
        // the ratio test + cross-check (which only accepts a pair whose
        // distance is well below the cross-match noise floor of all
        // competing descriptors — that is what the ratio denominator IS),
        // then require the surviving matches to be geometrically correct
        // under the known rotation. Orientation assignment must undo the
        // 90° turn for any of this to survive.
        let a = asym_fixture();
        let b = rot90(&a);
        let fa = sift_detect(&a, &no_double()).unwrap();
        let fb = sift_detect(&b, &no_double()).unwrap();
        assert!(
            fa.len() >= 5 && fb.len() >= 5,
            "{} / {}",
            fa.len(),
            fb.len()
        );
        let da: Vec<[f32; 128]> = fa.iter().map(|f| f.descriptor).collect();
        let db: Vec<[f32; 128]> = fb.iter().map(|f| f.descriptor).collect();
        let matches = crate::pano::matching::match_l2_ratio(&da, &db, 0.8, true);
        assert!(
            matches.len() >= 5,
            "only {} ratio-test matches",
            matches.len()
        );
        let mut good = 0usize;
        for m in &matches {
            let ka = &fa[m.query].keypoint;
            let kb = &fb[m.train].keypoint;
            // rot90 maps (x, y) -> (y, w−1−x).
            let (rx, ry) = (ka.y, (a.w - 1) as f64 - ka.x);
            let d = ((kb.x - rx).powi(2) + (kb.y - ry).powi(2)).sqrt();
            if d < 2.0 {
                good += 1;
            }
        }
        assert!(
            good * 10 >= matches.len() * 8,
            "only {good}/{} matches geometrically consistent",
            matches.len()
        );
    }

    #[test]
    fn non_finite_input_is_rejected_up_front() {
        // The API returns Result; NaN/Inf input must be an Err, not an
        // internal comparison panic (crate validation convention).
        let mut img = blob(64, 64, 31.3, 30.6, 3.2);
        img.data[5 * 64 + 7] = f32::NAN;
        let err = sift_detect(&img, &no_double()).unwrap_err();
        assert!(err.contains("non-finite"), "unexpected error text: {err}");
        let mut img = blob(64, 64, 31.3, 30.6, 3.2);
        img.data[0] = f32::INFINITY;
        assert!(sift_detect(&img, &SiftParams::default()).is_err());
    }

    #[test]
    fn determinism_two_run_and_golden() {
        let img = asym_fixture();
        let f1 = sift_detect(&img, &SiftParams::default()).unwrap();
        let f2 = sift_detect(&img, &SiftParams::default()).unwrap();
        assert_eq!(f1.len(), f2.len());
        let hash = |fs: &[SiftFeature]| -> u64 {
            let mut all: Vec<f32> = Vec::new();
            for f in fs {
                all.extend_from_slice(&[
                    f.keypoint.x as f32,
                    f.keypoint.y as f32,
                    f.keypoint.sigma as f32,
                    f.keypoint.orientation as f32,
                ]);
                all.extend_from_slice(&f.descriptor);
            }
            plane_hash(&all)
        };
        let (h1, h2) = (hash(&f1), hash(&f2));
        assert_eq!(h1, h2, "detector must replay bit-identically");
        assert_eq!(
            h1, GOLDEN,
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Self-pinned golden (see determinism_two_run_and_golden): pinned
    // 2026-08-13 from the first run whose behavioral oracles (blob
    // scale/position, edge rejection, rotation invariance) were all green.
    // Re-pinning requires investigation + a fresh oracle pass.
    // RE-PINNED 2026-08-13 (item-12 fix round): the doubled-input
    // map-back now carries the center-alignment −0.25 term of
    // `bilinear_resize`, shifting every default-params coordinate by
    // exactly −0.25 px; re-pinned from a two-run-equal own run after the
    // full oracle set was green (old value 16771707226855784093).
    const GOLDEN: u64 = 4737850577093922512;
}
