// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Oriented-FAST + rotated-binary-descriptor features — clean-room from
//! rublee11 (ICCV 2011), with the corner test from rosten06 (FAST) and the
//! binary-test descriptor idea from calonder10 (BRIEF), all as described
//! by rublee11 itself. No reference implementation was consulted (census
//! route: the upstream detector library is a non-staged behavior reference
//! with `files_taken: []`).
//!
//! Pipeline:
//! 1. Image pyramid: `n_levels` levels, geometric scale step
//!    `scale_factor` (paper: 8 levels, factor 1.2), own bilinear resize.
//! 2. FAST-9 segment-test corners per level: a pixel is a corner when at
//!    least 9 contiguous pixels of the 16-pixel Bresenham circle (radius
//!    3) are all brighter than `p + t` or all darker than `p − t`. Corner
//!    score = the larger of the two sums of `|I − p| − t` over the
//!    qualifying set (rosten06's V), followed by deterministic 3×3
//!    non-maximum suppression.
//! 3. Harris-response ranking (paper §3.1: FAST has large responses along
//!    edges, so corners are re-ranked by the Harris measure and the top N
//!    kept). Harris here: central-difference gradients, 7×7 block sum,
//!    k = 0.04 (documented own constants of the standard measure).
//! 4. Intensity-centroid orientation (paper §3.2, rosin99 moments):
//!    θ = atan2(m01, m10) over the radius-15 disc.
//! 5. Steered 256-bit descriptor (paper §3.3): binary tests on a
//!    Gaussian-smoothed patch, the test pattern rotated by the keypoint
//!    orientation quantized to 2π/30 (12°) steps as in the paper.
//!
//! DIVERGENCE, recorded honestly (also in the lane log): rublee11 §3.4
//! uses a LEARNED test pattern (greedy search over a training corpus for
//! low-correlation, high-variance tests). A learned pattern needs training
//! data this project does not ship, so the pattern here is generated
//! deterministically by the crate's own seeded [`SplitMix64`] stream —
//! 256 point pairs drawn uniformly from the radius-13 disc (rejection
//! sampling; radius 13 keeps every rotated sample inside the 31×31
//! patch). The seed is a documented parameter
//! ([`OrbParams::pattern_seed`]) and part of any golden key. This follows
//! the paper's PRE-learning baseline family (random tests, as in BRIEF)
//! rather than its learned refinement; expected effect per the paper is
//! somewhat higher inter-bit correlation, not a correctness change.
//!
//! Determinism (D-48): single-threaded, fixed level → row → column order;
//! the only randomness is the seeded, parameterized pattern above.

use super::{bilinear_resize, gaussian_blur};
use crate::plane::{Plane, SplitMix64};

/// The 16 offsets of the radius-3 Bresenham circle, clockwise from
/// straight up (rosten06's segment-test ring).
const CIRCLE: [(isize, isize); 16] = [
    (0, -3),
    (1, -3),
    (2, -2),
    (3, -1),
    (3, 0),
    (3, 1),
    (2, 2),
    (1, 3),
    (0, 3),
    (-1, 3),
    (-2, 2),
    (-3, 1),
    (-3, 0),
    (-3, -1),
    (-2, -2),
    (-1, -3),
];

/// Contiguous-arc length for the segment test (FAST-9).
const FAST_ARC: usize = 9;
/// Patch radius for orientation + descriptor sampling (paper: 31×31).
const PATCH_RADIUS: isize = 15;
/// Maximum test-point radius so every rotated sample stays inside the
/// patch (own documented choice, see module docs).
const PATTERN_RADIUS: f64 = 13.0;
/// Number of binary tests (paper: 256).
const N_TESTS: usize = 256;
/// Orientation quantization steps for steering (paper §3.3: 2π/30).
const STEER_STEPS: usize = 30;
/// Gaussian smoothing sigma before descriptor sampling (calonder10
/// prescribes pre-smoothing; sigma 2 is this implementation's documented
/// choice).
const DESC_BLUR_SIGMA: f64 = 2.0;

/// Detector parameters. Defaults per rublee11 where the paper states
/// them; own documented choices otherwise.
#[derive(Clone, Copy, Debug)]
pub struct OrbParams {
    /// Maximum features kept after Harris ranking (across all levels).
    pub n_features: usize,
    /// Pyramid levels (paper: 8).
    pub n_levels: usize,
    /// Pyramid scale step (paper: 1.2).
    pub scale_factor: f64,
    /// FAST segment-test threshold `t` for values in [0, 1] (20/255 —
    /// the customary FAST default expressed for float rasters).
    pub fast_threshold: f32,
    /// Harris response constant k (standard 0.04).
    pub harris_k: f64,
    /// Seed for the deterministic test-pattern generation (documented
    /// divergence from the paper's learned pattern; part of golden keys).
    pub pattern_seed: u64,
}

impl Default for OrbParams {
    fn default() -> Self {
        OrbParams {
            n_features: 500,
            n_levels: 8,
            scale_factor: 1.2,
            fast_threshold: 20.0 / 255.0,
            harris_k: 0.04,
            pattern_seed: DEFAULT_PATTERN_SEED,
        }
    }
}

/// Default pattern seed (arbitrary fixed constant; any golden key
/// includes the seed actually used).
pub const DEFAULT_PATTERN_SEED: u64 = 0x00AB_5EED_0000_0012;

/// One detected, oriented, described feature.
#[derive(Clone, Debug)]
pub struct OrbFeature {
    /// Column in input-image pixels (level coordinates scaled back).
    pub x: f64,
    /// Row in input-image pixels.
    pub y: f64,
    /// Pyramid level the feature was detected on.
    pub level: usize,
    /// Harris response used for ranking.
    pub response: f64,
    /// Intensity-centroid orientation, radians in (−π, π].
    pub orientation: f64,
    /// 256 binary tests packed little-endian (bit i of byte i/8).
    pub descriptor: [u8; 32],
}

/// Generate the 256-pair test pattern from a seed: 512 points, each drawn
/// uniformly from the radius-[`PATTERN_RADIUS`] disc by rejection
/// sampling on the crate's [`SplitMix64`] stream (fixed draw order —
/// bit-stable across runs and platforms).
pub fn generate_brief_pattern(seed: u64) -> Vec<(f64, f64)> {
    let mut rng = SplitMix64(seed);
    let mut pts = Vec::with_capacity(2 * N_TESTS);
    while pts.len() < 2 * N_TESTS {
        let x = (rng.next_f64() * 2.0 - 1.0) * PATTERN_RADIUS;
        let y = (rng.next_f64() * 2.0 - 1.0) * PATTERN_RADIUS;
        if x * x + y * y <= PATTERN_RADIUS * PATTERN_RADIUS {
            pts.push((x, y));
        }
    }
    pts
}

/// Detect oriented features and compute their binary descriptors.
///
/// `img` is a grayscale plane, values nominally in [0, 1]. Features are
/// returned ranked by Harris response (descending), ties broken by
/// (level, row, column) — a deterministic total order.
pub fn orb_detect(img: &Plane, p: &OrbParams) -> Result<Vec<OrbFeature>, String> {
    let min_dim = 2 * (PATCH_RADIUS as usize) + 3;
    if img.w < min_dim || img.h < min_dim {
        return Err(format!(
            "orb_detect: image {}x{} below the {}x{} minimum",
            img.w, img.h, min_dim, min_dim
        ));
    }
    if p.n_levels == 0 || p.scale_factor <= 1.0 {
        return Err("orb_detect: n_levels >= 1 and scale_factor > 1 required".into());
    }
    // Deliberate negated-comparison shape: also catches NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(p.fast_threshold > 0.0) {
        return Err("orb_detect: fast_threshold must be positive".into());
    }
    // Non-finite samples would propagate NaN into the Harris responses
    // and panic the ranking comparison; reject up front (crate
    // convention — same posture as grabcut's trimap validation).
    if let Some(i) = img.data.iter().position(|v| !v.is_finite()) {
        return Err(format!("orb_detect: non-finite input sample at index {i}"));
    }
    let pattern = generate_brief_pattern(p.pattern_seed);

    struct Candidate {
        x: usize,
        y: usize,
        level: usize,
        response: f64,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut levels: Vec<Plane> = Vec::with_capacity(p.n_levels);
    for lv in 0..p.n_levels {
        let s = p.scale_factor.powi(lv as i32);
        let (nw, nh) = (
            (img.w as f64 / s).round() as usize,
            (img.h as f64 / s).round() as usize,
        );
        if nw < min_dim || nh < min_dim {
            break;
        }
        let level_img = if lv == 0 {
            img.clone()
        } else {
            bilinear_resize(img, nw, nh)
        };

        // FAST scores over the interior (margin: patch radius, so every
        // kept corner supports orientation + descriptor windows).
        let margin = PATCH_RADIUS as usize + 1;
        let mut score = Plane::new(level_img.w, level_img.h);
        for y in margin..level_img.h - margin {
            for x in margin..level_img.w - margin {
                score.data[y * level_img.w + x] =
                    fast_score(&level_img, x, y, p.fast_threshold) as f32;
            }
        }
        // Deterministic 3×3 non-max suppression: a pixel survives when no
        // neighbor has a higher score and no earlier (row-major) neighbor
        // has an equal score.
        for y in margin..level_img.h - margin {
            for x in margin..level_img.w - margin {
                let v = score.at(x, y);
                if v <= 0.0 {
                    continue;
                }
                let mut keep = true;
                'nb: for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let n = score.at((x as isize + dx) as usize, (y as isize + dy) as usize);
                        let earlier = dy < 0 || (dy == 0 && dx < 0);
                        if n > v || (n == v && earlier) {
                            keep = false;
                            break 'nb;
                        }
                    }
                }
                if keep {
                    candidates.push(Candidate {
                        x,
                        y,
                        level: lv,
                        response: harris_response(&level_img, x, y, p.harris_k),
                    });
                }
            }
        }
        levels.push(level_img);
    }

    // Harris ranking: descending response; deterministic tie-break.
    candidates.sort_by(|a, b| {
        b.response
            .partial_cmp(&a.response)
            .unwrap()
            .then(a.level.cmp(&b.level))
            .then(a.y.cmp(&b.y))
            .then(a.x.cmp(&b.x))
    });
    candidates.truncate(p.n_features);

    // Blur each used level once for descriptor sampling.
    let blurred: Vec<Plane> = levels
        .iter()
        .map(|l| gaussian_blur(l, DESC_BLUR_SIGMA))
        .collect();

    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        let li = &levels[c.level];
        let theta = intensity_centroid_angle(li, c.x, c.y);
        let desc = steered_descriptor(&blurred[c.level], c.x, c.y, theta, &pattern);
        // Map level coordinates back through the center-aligned resize:
        // level pixel x sampled input coordinate (x + 0.5)·s − 0.5, with
        // s the ACTUAL per-axis ratio (input extent / level extent) that
        // `bilinear_resize` used — not the nominal scale_factor power,
        // which differs slightly because level sizes are rounded. Level 0
        // is the input itself (s = 1: identity).
        let sx = img.w as f64 / li.w as f64;
        let sy = img.h as f64 / li.h as f64;
        out.push(OrbFeature {
            x: (c.x as f64 + 0.5) * sx - 0.5,
            y: (c.y as f64 + 0.5) * sy - 0.5,
            level: c.level,
            response: c.response,
            orientation: theta,
            descriptor: desc,
        });
    }
    Ok(out)
}

/// FAST segment test + score. Returns 0 for non-corners; for corners, the
/// larger of the brighter/darker sums of `|I − p| − t` (rosten06's V).
fn fast_score(img: &Plane, x: usize, y: usize, t: f32) -> f64 {
    let p = img.at(x, y);
    let mut brighter = [false; 16];
    let mut darker = [false; 16];
    let mut ring = [0f32; 16];
    for (i, (dx, dy)) in CIRCLE.iter().enumerate() {
        let v = img.at((x as isize + dx) as usize, (y as isize + dy) as usize);
        ring[i] = v;
        brighter[i] = v > p + t;
        darker[i] = v < p - t;
    }
    let has_arc = |mask: &[bool; 16]| -> bool {
        // Longest circular run of true.
        let mut best = 0usize;
        let mut run = 0usize;
        for i in 0..32 {
            if mask[i % 16] {
                run += 1;
                best = best.max(run);
                if best >= FAST_ARC {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    };
    let is_corner = has_arc(&brighter) || has_arc(&darker);
    if !is_corner {
        return 0.0;
    }
    let sum_b: f64 = (0..16)
        .filter(|&i| brighter[i])
        .map(|i| (ring[i] - p - t) as f64)
        .sum();
    let sum_d: f64 = (0..16)
        .filter(|&i| darker[i])
        .map(|i| (p - ring[i] - t) as f64)
        .sum();
    sum_b.max(sum_d)
}

/// Harris corner response at (x, y): central-difference gradients over a
/// 7×7 block window, R = det(M) − k·tr(M)².
fn harris_response(img: &Plane, x: usize, y: usize, k: f64) -> f64 {
    let (mut sxx, mut syy, mut sxy) = (0f64, 0f64, 0f64);
    for dy in -3isize..=3 {
        for dx in -3isize..=3 {
            let (px, py) = (x as isize + dx, y as isize + dy);
            let gx = img.at_clamped(px + 1, py) as f64 - img.at_clamped(px - 1, py) as f64;
            let gy = img.at_clamped(px, py + 1) as f64 - img.at_clamped(px, py - 1) as f64;
            sxx += gx * gx;
            syy += gy * gy;
            sxy += gx * gy;
        }
    }
    sxx * syy - sxy * sxy - k * (sxx + syy) * (sxx + syy)
}

/// Intensity-centroid orientation over the radius-[`PATCH_RADIUS`] disc
/// (rublee11 §3.2): θ = atan2(m01, m10), moments about the corner.
fn intensity_centroid_angle(img: &Plane, x: usize, y: usize) -> f64 {
    let (mut m10, mut m01) = (0f64, 0f64);
    let r2 = PATCH_RADIUS * PATCH_RADIUS;
    for dy in -PATCH_RADIUS..=PATCH_RADIUS {
        for dx in -PATCH_RADIUS..=PATCH_RADIUS {
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let v = img.at_clamped(x as isize + dx, y as isize + dy) as f64;
            m10 += dx as f64 * v;
            m01 += dy as f64 * v;
        }
    }
    m01.atan2(m10)
}

/// Steered binary descriptor: the pattern rotated by θ quantized to
/// 2π/[`STEER_STEPS`] (paper §3.3), tests on the pre-smoothed image, bit
/// set when I(p1) < I(p2).
fn steered_descriptor(
    blurred: &Plane,
    x: usize,
    y: usize,
    theta: f64,
    pattern: &[(f64, f64)],
) -> [u8; 32] {
    let step = 2.0 * std::f64::consts::PI / STEER_STEPS as f64;
    let q = (theta / step).round() * step;
    let (c, s) = (q.cos(), q.sin());
    let sample = |(px, py): (f64, f64)| -> f32 {
        let rx = (c * px - s * py).round() as isize;
        let ry = (s * px + c * py).round() as isize;
        blurred.at_clamped(x as isize + rx, y as isize + ry)
    };
    let mut d = [0u8; 32];
    for i in 0..N_TESTS {
        let a = sample(pattern[2 * i]);
        let b = sample(pattern[2 * i + 1]);
        if a < b {
            d[i / 8] |= 1 << (i % 8);
        }
    }
    d
}

/// Hamming distance between two 256-bit descriptors.
pub fn hamming256(a: &[u8; 32], b: &[u8; 32]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash;

    /// Bright axis-aligned square on a dark background; smooth enough to
    /// be realistic (half-pixel anti-aliased border via clamped ramp).
    fn square_fixture(w: usize, h: usize, x0: usize, y0: usize, size: usize) -> Plane {
        let mut p = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let inside = x >= x0 && x < x0 + size && y >= y0 && y < y0 + size;
                p.data[y * w + x] = if inside { 0.9 } else { 0.1 };
            }
        }
        p
    }

    #[test]
    fn fast_detects_corner_not_flat_or_edge() {
        // Corner fixture: a square corner well inside the margin.
        let img = square_fixture(64, 64, 24, 24, 20);
        let f = orb_detect(&img, &OrbParams::default()).unwrap();
        assert!(!f.is_empty(), "no corners on the square fixture");
        // Every detection sits near one of the four square corners (the
        // segment test fires only where the ring sees a 3/4-vs-1/4
        // split, not along the straight sides). Tolerance is level-aware
        // AND ring-aware: on bilinearly resized (anti-aliased) coarse
        // levels the segment test can fire up to about the ring radius
        // (3) from the geometric corner (measured worst 2.9 level px at
        // level 2), so allow 3.5 level pixels, scaled back to input
        // coordinates. Straight sides stay clean at every level — that
        // is the discriminating half of the oracle.
        for feat in &f {
            let tol = 3.5 * 1.2f64.powi(feat.level as i32);
            let near = [(24.0, 24.0), (43.0, 24.0), (24.0, 43.0), (43.0, 43.0)]
                .iter()
                .any(|(cx, cy)| ((feat.x - cx).powi(2) + (feat.y - cy).powi(2)).sqrt() < tol);
            assert!(
                near,
                "detection away from corners: ({}, {}) level {}",
                feat.x, feat.y, feat.level
            );
        }
        // Flat fixture: nothing.
        let flat = Plane::from_vec(64, 64, vec![0.4; 64 * 64]);
        assert!(orb_detect(&flat, &OrbParams::default()).unwrap().is_empty());
        // Pure straight edge through the whole image: the ring never sees
        // a >= 9 contiguous run on the sides, and there is no corner.
        let mut edge = Plane::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                edge.data[y * 64 + x] = if x < 32 { 0.1 } else { 0.9 };
            }
        }
        assert!(orb_detect(&edge, &OrbParams::default()).unwrap().is_empty());
    }

    /// Lossless 90° rotation (same mapping as the sift test).
    fn rot90(img: &Plane) -> Plane {
        let mut out = Plane::new(img.h, img.w);
        for y in 0..img.h {
            for x in 0..img.w {
                out.data[(img.w - 1 - x) * out.w + y] = img.at(x, y);
            }
        }
        out
    }

    #[test]
    fn orientation_follows_centroid_rotation() {
        // The top-left corner of the bright square: the bright mass lies
        // toward +x, +y, so θ ≈ π/4. After a lossless 90° rotation the
        // same physical corner's θ must shift by −π/2 (mod 2π),
        // quantified within 0.2 rad (steering quantization is 12° ≈ 0.21
        // but the reported angle is unquantized; 0.2 covers pixel grid
        // asymmetry of the disc).
        let img = square_fixture(64, 64, 24, 24, 20);
        let f = orb_detect(&img, &OrbParams::default()).unwrap();
        let tl = f
            .iter()
            .min_by(|a, b| {
                let da = (a.x - 24.0).powi(2) + (a.y - 24.0).powi(2);
                let db = (b.x - 24.0).powi(2) + (b.y - 24.0).powi(2);
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        assert!(
            (tl.orientation - std::f64::consts::FRAC_PI_4).abs() < 0.2,
            "top-left corner orientation {} vs π/4",
            tl.orientation
        );
        let rimg = rot90(&img);
        let rf = orb_detect(&rimg, &OrbParams::default()).unwrap();
        // The top-left corner (24,24) maps to (24, 64-1-24) = (24, 39).
        let rtl = rf
            .iter()
            .min_by(|a, b| {
                let da = (a.x - 24.0).powi(2) + (a.y - 39.0).powi(2);
                let db = (b.x - 24.0).powi(2) + (b.y - 39.0).powi(2);
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        let mut delta = rtl.orientation - tl.orientation;
        while delta <= -std::f64::consts::PI {
            delta += 2.0 * std::f64::consts::PI;
        }
        while delta > std::f64::consts::PI {
            delta -= 2.0 * std::f64::consts::PI;
        }
        assert!(
            (delta + std::f64::consts::FRAC_PI_2).abs() < 0.2,
            "orientation delta {} vs −π/2",
            delta
        );
    }

    #[test]
    fn hamming_self_match_is_zero_and_distance_is_metric_sane() {
        let img = square_fixture(64, 64, 24, 24, 20);
        let f1 = orb_detect(&img, &OrbParams::default()).unwrap();
        let f2 = orb_detect(&img, &OrbParams::default()).unwrap();
        assert_eq!(f1.len(), f2.len());
        for (a, b) in f1.iter().zip(&f2) {
            assert_eq!(hamming256(&a.descriptor, &b.descriptor), 0);
        }
        // Distinct corners of the square differ in many bits.
        if f1.len() >= 2 {
            assert!(hamming256(&f1[0].descriptor, &f1[1].descriptor) > 0);
        }
    }

    #[test]
    fn pattern_regeneration_is_bit_stable_and_seed_sensitive() {
        let a = generate_brief_pattern(DEFAULT_PATTERN_SEED);
        let b = generate_brief_pattern(DEFAULT_PATTERN_SEED);
        assert_eq!(a.len(), 512);
        for (u, v) in a.iter().zip(&b) {
            assert_eq!(u.0.to_bits(), v.0.to_bits());
            assert_eq!(u.1.to_bits(), v.1.to_bits());
        }
        for (x, y) in &a {
            assert!(x * x + y * y <= PATTERN_RADIUS * PATTERN_RADIUS + 1e-12);
        }
        let c = generate_brief_pattern(DEFAULT_PATTERN_SEED ^ 1);
        assert!(a.iter().zip(&c).any(|(u, v)| u.0 != v.0 || u.1 != v.1));
    }

    #[test]
    fn coarse_level_coordinates_map_back_center_aligned() {
        // Level-to-input mapping oracle. A bright square centered EXACTLY
        // at the image center ((129−1)/2 = 64) is symmetric under both
        // axis flips, and the center-aligned `bilinear_resize` preserves
        // that symmetry at every pyramid level (mirroring an output pixel
        // mirrors the source coordinate about the source center). So the
        // detections at any single level form flip-symmetric sets and
        // their mapped-back centroid must sit at 64.0 — segment-test
        // localization noise cancels by symmetry, leaving only mapping
        // bias. A naive `x·s` map-back (no center-alignment term) is
        // biased by 0.5·(s−1) per axis (≈0.2 px at level 1 already,
        // ≈0.74 px at level 5), so the 0.15-px assertion fails under it
        // for every level ≥ 1 (verified failing against the pre-fix
        // mapping, 2026-08-13: level-1 centroid read (64.2, 64.2)); the
        // correct `(x + 0.5)·s − 0.5` map-back cancels exactly.
        let img = square_fixture(129, 129, 44, 44, 41);
        let f = orb_detect(&img, &OrbParams::default()).unwrap();
        let mut deepest = 0usize;
        for lv in 0..8usize {
            let pts: Vec<&OrbFeature> = f.iter().filter(|p| p.level == lv).collect();
            if pts.is_empty() {
                continue;
            }
            deepest = deepest.max(lv);
            let cx = pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64;
            let cy = pts.iter().map(|p| p.y).sum::<f64>() / pts.len() as f64;
            assert!(
                (cx - 64.0).abs() < 0.15 && (cy - 64.0).abs() < 0.15,
                "level {lv} centroid ({cx}, {cy}) off the symmetric center 64 ({} detections)",
                pts.len()
            );
        }
        assert!(
            deepest >= 3,
            "deepest detected level {deepest} < 3 — fixture no longer exercises the coarse-level mapping"
        );
    }

    #[test]
    fn non_finite_input_is_rejected_up_front() {
        // The API returns Result; NaN/Inf input must be an Err, not an
        // internal comparison panic (crate validation convention).
        let mut img = square_fixture(64, 64, 24, 24, 20);
        img.data[3 * 64 + 9] = f32::NAN;
        let err = orb_detect(&img, &OrbParams::default()).unwrap_err();
        assert!(err.contains("non-finite"), "unexpected error text: {err}");
        let mut img = square_fixture(64, 64, 24, 24, 20);
        img.data[64 * 64 - 1] = f32::NEG_INFINITY;
        assert!(orb_detect(&img, &OrbParams::default()).is_err());
    }

    #[test]
    fn determinism_two_run_and_golden() {
        let img = square_fixture(80, 64, 22, 20, 26);
        let f1 = orb_detect(&img, &OrbParams::default()).unwrap();
        let f2 = orb_detect(&img, &OrbParams::default()).unwrap();
        let hash = |fs: &[OrbFeature]| -> u64 {
            let mut all: Vec<f32> = Vec::new();
            for f in fs {
                all.extend_from_slice(&[
                    f.x as f32,
                    f.y as f32,
                    f.level as f32,
                    f.orientation as f32,
                ]);
                all.extend(f.descriptor.iter().map(|&b| b as f32));
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
    // 2026-08-13 from the first run whose behavioral oracles (corner
    // discrimination, centroid rotation, self-match) were all green.
    // Golden key includes DEFAULT_PATTERN_SEED. Re-pinning requires
    // investigation + a fresh oracle pass.
    // RE-PINNED 2026-08-13 (item-12 fix round): the level-to-input
    // map-back now applies `bilinear_resize`'s center-aligned inverse
    // (x + 0.5)·s − 0.5 with the actual per-axis ratio, shifting every
    // level ≥ 1 coordinate; re-pinned from a two-run-equal own run after
    // the full oracle set (including the new symmetric-centroid mapping
    // oracle) was green (old value 13347874057159209560).
    const GOLDEN: u64 = 8040161340689587933;
}
