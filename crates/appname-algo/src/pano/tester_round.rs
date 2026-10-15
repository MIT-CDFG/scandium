// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Item-12 tester round — adversarial/edge tests for the pano classical
//! tier, exercising ONLY public pano APIs (plus `pub(crate)` fixtures
//! helpers). cfg(test)-only module; no implementation code here.

#![cfg(test)]

use super::gaussian_blur;
use super::homography::{
    apply_homography, dlt_homography, ransac_homography, Homography, RansacParams,
};
use super::matching::{match_hamming_ratio, match_l2_ratio};
use super::multiband::multiband_blend;
use super::orb::{orb_detect, OrbParams};
use super::sift::{sift_detect, SiftParams};
use crate::plane::{Plane, SplitMix64};

// ---------------------------------------------------------------- fixtures

fn no_double() -> SiftParams {
    SiftParams {
        double_input: false,
        ..SiftParams::default()
    }
}

/// Analytic Gaussian blob on a constant background.
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

/// Deterministic blob field confined to a central disc (radius `r`), so a
/// rotation about the image center keeps all content in frame.
fn central_blob_field(w: usize, h: usize, seed: u64, n: usize, r: f64) -> Plane {
    let mut rng = SplitMix64(seed);
    let (cx, cy) = ((w - 1) as f64 / 2.0, (h - 1) as f64 / 2.0);
    let blobs: Vec<(f64, f64, f64, f64)> = (0..n)
        .map(|_| {
            let ang = rng.next_f64() * 2.0 * std::f64::consts::PI;
            let rad = rng.next_f64().sqrt() * r;
            (
                cx + rad * ang.cos(),
                cy + rad * ang.sin(),
                1.8 + rng.next_f64() * 1.9,
                0.35 + rng.next_f64() * 0.55,
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

/// Bilinear sample with edge clamp (test-code resampler, same convention
/// as the crate's own `bilinear_resize` interior arithmetic).
fn sample_bilinear(img: &Plane, sx: f64, sy: f64) -> f64 {
    let x0 = sx.floor();
    let y0 = sy.floor();
    let ax = sx - x0;
    let ay = sy - y0;
    let (x0, y0) = (x0 as isize, y0 as isize);
    img.at_clamped(x0, y0) as f64 * (1.0 - ax) * (1.0 - ay)
        + img.at_clamped(x0 + 1, y0) as f64 * ax * (1.0 - ay)
        + img.at_clamped(x0, y0 + 1) as f64 * (1.0 - ax) * ay
        + img.at_clamped(x0 + 1, y0 + 1) as f64 * ax * ay
}

/// Rotate `img` by `theta` about its center (backward map + bilinear).
fn rotate_about_center(img: &Plane, theta: f64) -> Plane {
    let (c, s) = (theta.cos(), theta.sin());
    let (cx, cy) = ((img.w - 1) as f64 / 2.0, (img.h - 1) as f64 / 2.0);
    let mut out = Plane::new(img.w, img.h);
    for y in 0..img.h {
        for x in 0..img.w {
            // Backward map: rotate the output coordinate by −theta.
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let sx = c * dx + s * dy + cx;
            let sy = -s * dx + c * dy + cy;
            out.data[y * img.w + x] = sample_bilinear(img, sx, sy) as f32;
        }
    }
    out
}

/// Forward-map a point under the same center rotation.
fn rotate_point(x: f64, y: f64, cx: f64, cy: f64, theta: f64) -> (f64, f64) {
    let (c, s) = (theta.cos(), theta.sin());
    let dx = x - cx;
    let dy = y - cy;
    (c * dx - s * dy + cx, s * dx + c * dy + cy)
}

fn l2(a: &[f32; 128], b: &[f32; 128]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(p, q)| (*p as f64 - *q as f64).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Lossless 90° rotation: (x, y) -> (y, w−1−x).
fn rot90(img: &Plane) -> Plane {
    let mut out = Plane::new(img.h, img.w);
    for y in 0..img.h {
        for x in 0..img.w {
            out.data[(img.w - 1 - x) * out.w + y] = img.at(x, y);
        }
    }
    out
}

/// Axis-aligned bright rectangles (per-rect intensity) on a 0.1
/// background — corner-rich AND asymmetric (distinct patch contexts), so
/// steered descriptors stay discriminative.
fn rects_fixture(w: usize, h: usize, rects: &[(usize, usize, usize, usize, f32)]) -> Plane {
    let mut p = Plane::from_vec(w, h, vec![0.1; w * h]);
    for &(x0, y0, x1, y1, v) in rects {
        for y in y0..y1 {
            for x in x0..x1 {
                p.data[y * w + x] = v;
            }
        }
    }
    p
}

fn grid_points() -> Vec<(f64, f64)> {
    let mut pts = Vec::new();
    for gy in 0..6 {
        for gx in 0..6 {
            pts.push((10.0 + 20.0 * gx as f64, 8.0 + 18.0 * gy as f64));
        }
    }
    pts
}

/// Mild projective ground truth (distinct from the builder tests' gt()).
fn gt_h() -> Homography {
    [
        [1.03, 0.04, -4.0],
        [-0.03, 0.97, 5.5],
        [-1.2e-4, 0.9e-4, 1.0],
    ]
}

fn max_probe_deviation(ha: &Homography, hb: &Homography) -> f64 {
    let mut worst = 0.0f64;
    for gy in 0..5 {
        for gx in 0..5 {
            let p = (15.0 + 21.0 * gx as f64, 12.0 + 19.0 * gy as f64);
            let a = apply_homography(ha, p.0, p.1).unwrap();
            let b = apply_homography(hb, p.0, p.1).unwrap();
            worst = worst.max(((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt());
        }
    }
    worst
}

// -------------------------------------------------------------------- SIFT

#[test]
fn sift_rotation_30deg_descriptors_survive_with_low_match_distance() {
    // Rotation invariance at a NON-90° angle: 30°, synthesized with a
    // test-code bilinear resampler (the same convention as the crate's
    // own resize), content confined to a central disc so nothing leaves
    // the frame. Interpolation adds real blur/aliasing error, so the
    // bounds are honest rather than tight: matches must be geometrically
    // consistent under the known rotation, and matched descriptor
    // distances must sit well below the cross-pair noise floor (median
    // all-pairs L2 between the two feature sets).
    let theta = 30f64.to_radians();
    let img = central_blob_field(96, 96, 20260814, 12, 30.0);
    let rot = rotate_about_center(&img, theta);
    let fa = sift_detect(&img, &no_double()).unwrap();
    let fb = sift_detect(&rot, &no_double()).unwrap();
    assert!(
        fa.len() >= 5 && fb.len() >= 5,
        "{} / {}",
        fa.len(),
        fb.len()
    );
    let da: Vec<[f32; 128]> = fa.iter().map(|f| f.descriptor).collect();
    let db: Vec<[f32; 128]> = fb.iter().map(|f| f.descriptor).collect();
    let matches = match_l2_ratio(&da, &db, 0.8, true);
    assert!(matches.len() >= 5, "only {} matches", matches.len());
    // Noise floor: median distance over all cross pairs.
    let mut all: Vec<f64> = Vec::with_capacity(da.len() * db.len());
    for a in &da {
        for b in &db {
            all.push(l2(a, b));
        }
    }
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let floor = all[all.len() / 2];
    let (cx, cy) = (47.5, 47.5);
    let mut good = 0usize;
    let mut good_dist_max = 0.0f64;
    for m in &matches {
        let ka = &fa[m.query].keypoint;
        let kb = &fb[m.train].keypoint;
        let (rx, ry) = rotate_point(ka.x, ka.y, cx, cy, theta);
        let d = ((kb.x - rx).powi(2) + (kb.y - ry).powi(2)).sqrt();
        if d < 2.5 {
            good += 1;
            good_dist_max = good_dist_max.max(m.distance);
        }
    }
    assert!(
        good * 10 >= matches.len() * 8,
        "only {good}/{} matches consistent under the 30° rotation",
        matches.len()
    );
    // Honest descriptor-distance bound: every geometrically consistent
    // match must be well below the cross-pair noise floor.
    assert!(
        good_dist_max < 0.6 * floor,
        "matched distance {good_dist_max} not below 0.6x noise floor {floor}"
    );
}

#[test]
fn sift_scale_invariance_across_analytic_2x_pair() {
    // Scale invariance: the SAME structure rendered analytically at 1x
    // and 2x (blob sigma doubled, center coordinates doubled — no
    // resampling error at all). The detected characteristic scale must
    // double with the structure. Tolerance ±12.5% on the ratio covers
    // discrete scale sampling (levels are 2^(1/3) apart ≈ 26%) after
    // subscale interpolation.
    let (cx1, cy1, sb1) = (31.3, 30.6, 2.5);
    let img1 = blob(64, 64, cx1, cy1, sb1);
    let img2 = blob(128, 128, 2.0 * cx1, 2.0 * cy1, 2.0 * sb1);
    let f1 = sift_detect(&img1, &no_double()).unwrap();
    let f2 = sift_detect(&img2, &no_double()).unwrap();
    assert!(!f1.is_empty() && !f2.is_empty());
    let best = |fs: &[super::sift::SiftFeature]| -> super::sift::SiftKeypoint {
        fs.iter()
            .max_by(|a, b| {
                a.keypoint
                    .response
                    .partial_cmp(&b.keypoint.response)
                    .unwrap()
            })
            .unwrap()
            .keypoint
    };
    let k1 = best(&f1);
    let k2 = best(&f2);
    assert!(
        (k1.x - cx1).abs() < 1.0 && (k1.y - cy1).abs() < 1.0,
        "1x center missed: ({}, {})",
        k1.x,
        k1.y
    );
    assert!(
        (k2.x - 2.0 * cx1).abs() < 2.0 && (k2.y - 2.0 * cy1).abs() < 2.0,
        "2x center missed: ({}, {})",
        k2.x,
        k2.y
    );
    let ratio = k2.sigma / k1.sigma;
    assert!(
        (ratio - 2.0).abs() < 0.25,
        "scale ratio {ratio} vs expected 2.0 (sigma {} -> {})",
        k1.sigma,
        k2.sigma
    );
}

#[test]
fn sift_contrast_threshold_boundary_flips_detection() {
    // Exact boundary behavior of the low-contrast rejection: measure the
    // strongest interpolated response r on a single-blob fixture, then
    // re-run with the threshold nudged just below r (the blob must
    // survive) and just above r (every response <= r, so the output must
    // be empty). The threshold only filters — same scale space — so this
    // isolates the |D(x-hat)| >= threshold comparison itself.
    let img = blob(64, 64, 31.3, 30.6, 3.2);
    let feats = sift_detect(&img, &no_double()).unwrap();
    assert!(!feats.is_empty());
    let r = feats
        .iter()
        .map(|f| f.keypoint.response)
        .fold(0.0f64, f64::max);
    assert!(r > SiftParams::default().contrast_threshold);
    let below = SiftParams {
        contrast_threshold: r * 0.95,
        ..no_double()
    };
    let above = SiftParams {
        contrast_threshold: r * 1.05,
        ..no_double()
    };
    let fb = sift_detect(&img, &below).unwrap();
    assert!(
        !fb.is_empty(),
        "threshold just below the top response killed the blob"
    );
    assert!(
        fb.iter().all(|f| f.keypoint.response >= r * 0.95),
        "a surviving feature is below the raised threshold"
    );
    let fa = sift_detect(&img, &above).unwrap();
    assert!(
        fa.is_empty(),
        "threshold above the top response still yields {} features",
        fa.len()
    );
}

#[test]
fn sift_multi_orientation_emission_at_two_direction_keypoints() {
    // Paper §5: every histogram peak within 80% of the maximum spawns its
    // own keypoint. A blurred bright square's corners see two dominant
    // gradient directions (the two edge normals, ~90° apart, equal mass
    // by the corner's diagonal symmetry) — so at least one refined
    // (x, y, sigma) location must emit >= 2 features, and some such pair
    // must be separated by roughly a right angle. Same-location features
    // share exact (x, y, sigma) bits (orientation is assigned after
    // refinement), so exact grouping is sound.
    let sq = rects_fixture(64, 64, &[(20, 20, 44, 44, 0.9)]);
    let img = gaussian_blur(&sq, 1.5);
    let feats = sift_detect(&img, &no_double()).unwrap();
    assert!(!feats.is_empty(), "no keypoints on the blurred square");
    let mut multi = 0usize;
    let mut right_angle_pair = false;
    for i in 0..feats.len() {
        for j in i + 1..feats.len() {
            let (a, b) = (&feats[i].keypoint, &feats[j].keypoint);
            if a.x == b.x && a.y == b.y && a.sigma == b.sigma {
                multi += 1;
                let mut d = (a.orientation - b.orientation).abs();
                if d > std::f64::consts::PI {
                    d = 2.0 * std::f64::consts::PI - d;
                }
                assert!(d > 1e-6, "duplicate feature with identical orientation");
                if (d - std::f64::consts::FRAC_PI_2).abs() < 0.45 {
                    right_angle_pair = true;
                }
            }
        }
    }
    assert!(
        multi >= 1,
        "no keypoint emitted multiple orientations on the corner fixture"
    );
    assert!(
        right_angle_pair,
        "no multi-orientation pair near the corner's 90° separation"
    );
}

#[test]
fn sift_flat_image_and_minimum_size_boundary() {
    // Flat input: zero DoG everywhere -> Ok(empty), not an error and not
    // a panic (both with and without input doubling).
    let flat = Plane::from_vec(32, 32, vec![0.5; 32 * 32]);
    assert!(sift_detect(&flat, &SiftParams::default())
        .unwrap()
        .is_empty());
    assert!(sift_detect(&flat, &no_double()).unwrap().is_empty());
    // Documented minimum is 8x8: 7 px on either axis errors, exactly 8x8
    // succeeds (empty on flat input) on both doubling paths.
    let tiny = Plane::from_vec(7, 8, vec![0.5; 56]);
    assert!(sift_detect(&tiny, &SiftParams::default()).is_err());
    assert!(sift_detect(&tiny, &no_double()).is_err());
    let min = Plane::from_vec(8, 8, vec![0.5; 64]);
    assert!(sift_detect(&min, &SiftParams::default())
        .unwrap()
        .is_empty());
    assert!(sift_detect(&min, &no_double()).unwrap().is_empty());
}

// --------------------------------------------------------------------- ORB

#[test]
fn orb_rot90_descriptors_match_under_steering() {
    // Descriptor-level rotation invariance under an EXACT (lossless) 90°
    // rotation, on an asymmetric multi-rectangle fixture (distinct patch
    // contexts keep corners distinguishable after steering — a single
    // square's four corners are locally identical up to rotation and
    // would defeat the ratio test by construction). Surviving
    // ratio+cross-check matches must be geometrically consistent under
    // the known rotation, with Hamming distances far below the ~128
    // random-descriptor expectation.
    let img = rects_fixture(
        96,
        96,
        &[
            (10, 8, 30, 20, 0.9),
            (40, 30, 58, 50, 0.55),
            (12, 36, 24, 54, 0.35),
            (60, 62, 84, 78, 0.75),
        ],
    );
    let rimg = rot90(&img);
    let fa = orb_detect(&img, &OrbParams::default()).unwrap();
    let fb = orb_detect(&rimg, &OrbParams::default()).unwrap();
    assert!(
        fa.len() >= 8 && fb.len() >= 8,
        "{} / {}",
        fa.len(),
        fb.len()
    );
    let da: Vec<[u8; 32]> = fa.iter().map(|f| f.descriptor).collect();
    let db: Vec<[u8; 32]> = fb.iter().map(|f| f.descriptor).collect();
    let matches = match_hamming_ratio(&da, &db, 0.8, true);
    assert!(matches.len() >= 5, "only {} matches", matches.len());
    let w = img.w as f64;
    let mut good = 0usize;
    let mut good_dist_max = 0.0f64;
    for m in &matches {
        let a = &fa[m.query];
        let b = &fb[m.train];
        // rot90 maps (x, y) -> (y, w−1−x).
        let (rx, ry) = (a.y, w - 1.0 - a.x);
        let tol = 3.5 * 1.2f64.powi(a.level.max(b.level) as i32);
        if ((b.x - rx).powi(2) + (b.y - ry).powi(2)).sqrt() < tol {
            good += 1;
            good_dist_max = good_dist_max.max(m.distance);
        }
    }
    assert!(
        good * 10 >= matches.len() * 8,
        "only {good}/{} matches geometrically consistent under rot90",
        matches.len()
    );
    // Steering quantization is 12° and 90 is not a multiple of 12, so
    // exact-zero distance is NOT expected; 80 of 256 bits is an honest
    // bound (> 5 sigma below the 128 +/- 8 random expectation).
    assert!(
        good_dist_max <= 80.0,
        "consistent match Hamming distance {good_dist_max} above 80/256"
    );
}

#[test]
fn orb_fast_threshold_sensitivity_and_min_size() {
    // The fixture's contrast is 0.8 (0.9 on 0.1): a segment-test
    // threshold above it can never fire; the default (~0.078) detects
    // the corners; tightening the threshold never adds detections.
    let img = rects_fixture(64, 64, &[(24, 24, 44, 44, 0.9)]);
    let n_default = orb_detect(&img, &OrbParams::default()).unwrap().len();
    assert!(n_default > 0);
    let mid = OrbParams {
        fast_threshold: 0.4,
        ..OrbParams::default()
    };
    let n_mid = orb_detect(&img, &mid).unwrap().len();
    assert!(
        n_mid <= n_default,
        "raising the threshold added detections: {n_mid} > {n_default}"
    );
    let over = OrbParams {
        fast_threshold: 0.85,
        ..OrbParams::default()
    };
    assert!(
        orb_detect(&img, &over).unwrap().is_empty(),
        "threshold above the fixture contrast still fired"
    );
    // Documented minimum is 33x33 (2*15 + 3): 32 px errors, a flat 33x33
    // succeeds with zero features.
    let small = Plane::from_vec(32, 40, vec![0.5; 32 * 40]);
    assert!(orb_detect(&small, &OrbParams::default()).is_err());
    let min = Plane::from_vec(33, 33, vec![0.5; 33 * 33]);
    assert!(orb_detect(&min, &OrbParams::default()).unwrap().is_empty());
}

#[test]
fn orb_pattern_seed_changes_descriptors_not_detections() {
    // The BRIEF-pattern seed is a descriptor-only parameter: detection,
    // ranking, and orientation are computed before the pattern is ever
    // sampled, so two seeds must agree on every keypoint field and
    // differ somewhere in the descriptor bits.
    let img = rects_fixture(96, 96, &[(10, 8, 30, 20, 0.9), (40, 30, 58, 50, 0.55)]);
    let f1 = orb_detect(&img, &OrbParams::default()).unwrap();
    let alt = OrbParams {
        pattern_seed: 0xDEAD_BEEF_2026_0814,
        ..OrbParams::default()
    };
    let f2 = orb_detect(&img, &alt).unwrap();
    assert_eq!(f1.len(), f2.len(), "seed changed the detection count");
    let mut any_desc_diff = false;
    for (a, b) in f1.iter().zip(&f2) {
        assert_eq!(a.x.to_bits(), b.x.to_bits());
        assert_eq!(a.y.to_bits(), b.y.to_bits());
        assert_eq!(a.level, b.level);
        assert_eq!(a.response.to_bits(), b.response.to_bits());
        assert_eq!(a.orientation.to_bits(), b.orientation.to_bits());
        if a.descriptor != b.descriptor {
            any_desc_diff = true;
        }
    }
    assert!(
        any_desc_diff,
        "different seeds produced identical descriptors"
    );
}

// ---------------------------------------------------------------- matching

#[test]
fn ratio_test_rejects_near_identical_targets() {
    // Two NEAR-identical (not exactly equal) train descriptors: the
    // query's best/second-best ratio is ~0.97, which 0.8 must reject and
    // a 0.995 ratio accepts (matching the nearer one). This pins the
    // ratio test as an ambiguity filter, not an exact-duplicate filter.
    let mut q = [0f32; 128];
    q[0] = 1.0;
    let mut t0 = [0f32; 128];
    t0[0] = 1.0;
    t0[1] = 0.30;
    let mut t1 = [0f32; 128];
    t1[0] = 1.0;
    t1[2] = 0.31; // slightly farther: d ratio ~ 0.30/0.31 ~ 0.968
    let train = vec![t0, t1];
    let query = vec![q];
    let d0 = l2(&q, &t0);
    let d1 = l2(&q, &t1);
    let ratio = d0 / d1;
    assert!(ratio > 0.9 && ratio < 1.0, "fixture drifted: ratio {ratio}");
    assert!(
        match_l2_ratio(&query, &train, 0.8, false).is_empty(),
        "0.8 accepted an ambiguous (~{ratio:.3}) match"
    );
    let m = match_l2_ratio(&query, &train, 0.995, false);
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].query, m[0].train), (0, 0));
}

#[test]
fn cross_check_mutual_sets_are_mirror_images() {
    // Cross-check symmetry theorem: with the ratio test neutralized
    // (ratio 1.0 — only exact ties fail) and generic-position random
    // descriptors (no ties), mutual-nearest acceptance is symmetric, so
    // matching A->B and B->A must yield exactly mirrored pair sets.
    let mut rng = SplitMix64(0x5EED_2026);
    let mk = |rng: &mut SplitMix64| -> [f32; 128] {
        let mut d = [0f32; 128];
        for v in d.iter_mut() {
            *v = rng.next_f64() as f32;
        }
        d
    };
    let a: Vec<[f32; 128]> = (0..23).map(|_| mk(&mut rng)).collect();
    let b: Vec<[f32; 128]> = (0..29).map(|_| mk(&mut rng)).collect();
    let ab = match_l2_ratio(&a, &b, 1.0, true);
    let ba = match_l2_ratio(&b, &a, 1.0, true);
    assert!(!ab.is_empty(), "no mutual matches at all");
    let mut fwd: Vec<(usize, usize)> = ab.iter().map(|m| (m.query, m.train)).collect();
    let mut rev: Vec<(usize, usize)> = ba.iter().map(|m| (m.train, m.query)).collect();
    fwd.sort_unstable();
    rev.sort_unstable();
    assert_eq!(fwd, rev, "A->B and B->A mutual sets are not mirrors");
}

#[test]
fn matching_empty_inputs_and_nan_descriptor_policy() {
    // Empty inputs never panic and never match.
    let d: Vec<[f32; 128]> = vec![[0.1; 128], [0.2; 128]];
    assert!(match_l2_ratio(&[], &d, 0.8, true).is_empty());
    assert!(match_l2_ratio(&d, &[], 0.8, true).is_empty());
    let h: Vec<[u8; 32]> = vec![[0xAA; 32], [0x55; 32]];
    assert!(match_hamming_ratio(&[], &h, 0.8, true).is_empty());
    assert!(match_hamming_ratio(&h, &[], 0.8, true).is_empty());
    // Single train descriptor: ratio test impossible -> empty (L2 twin
    // of the existing Hamming pin).
    assert!(match_l2_ratio(&d, &d[..1], 0.8, false).is_empty());

    // NaN policy, pinned honestly (matching has no validation layer —
    // own detectors cannot produce NaN descriptors since they reject
    // non-finite input, so this documents raw-API behavior):
    // 1. a NaN QUERY descriptor is silently unmatched (all its distances
    //    are NaN, which never win a strict-less comparison); no panic;
    let nanq = [f32::NAN; 128];
    let mut clean = [0f32; 128];
    clean[3] = 1.0;
    let train = vec![clean, [0.5f32; 128], [0.25f32; 128]];
    let m = match_l2_ratio(&[nanq, clean], &train, 0.8, false);
    assert_eq!(m.len(), 1);
    assert_eq!((m[0].query, m[0].train), (1, 0));
    // 2. a NaN TRAIN descriptor is never selected — but note the quirk:
    //    with exactly 2 trains one of which is NaN, the second-best
    //    distance stays +inf and the ratio test passes vacuously (d1 <
    //    ratio·inf). Pinned as-is: NaN weakens the ratio denominator.
    let m2 = match_l2_ratio(&[clean], &[clean, nanq], 0.8, false);
    assert_eq!(m2.len(), 1, "NaN train made the clean pair unmatchable");
    assert_eq!((m2[0].query, m2[0].train), (0, 0));
    assert_eq!(m2[0].distance, 0.0);
}

// -------------------------------------------------------- homography/RANSAC

#[test]
fn identity_homography_recovered_exactly_from_noiseless_points() {
    // dst == src: normalized DLT must return the identity action to
    // numerical precision (tight bound — noiseless, well-conditioned),
    // and RANSAC must call every correspondence an inlier.
    let src = grid_points();
    let h = dlt_homography(&src, &src).unwrap();
    let eye: Homography = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let dev = max_probe_deviation(&h, &eye);
    assert!(dev < 1e-9, "identity deviation {dev}");
    let p = RansacParams {
        seed: 5,
        inlier_threshold: 0.5,
        ..RansacParams::default()
    };
    let r = ransac_homography(&src, &src, &p).unwrap();
    assert_eq!(
        r.inliers.len(),
        src.len(),
        "not all noiseless points inliers"
    );
    let dev = max_probe_deviation(&r.h, &eye);
    assert!(dev < 1e-9, "RANSAC identity deviation {dev}");
}

#[test]
fn ransac_outlier_fraction_sweep_10_to_50_percent() {
    // Outlier tolerance sweep on a 36-point noiseless grid: at 10/20/30/
    // 40/50% gross contamination the true model must be recovered (probe
    // deviation < 1e-4 px — inlier refit is noiseless) and every clean
    // correspondence must be an inlier. 50% still succeeds here because
    // w^4 = 6.25% at the fixture's inlier count keeps the adaptive bound
    // (99% confidence) well inside the 2000-hypothesis budget; the
    // honest failure point of THIS fixture was probed above the charter
    // range and lies beyond it (recovery still held at 55%; not asserted
    // — sample-probability arithmetic, not a contract).
    let src = grid_points();
    let h_true = gt_h();
    let clean: Vec<(f64, f64)> = src
        .iter()
        .map(|&(x, y)| apply_homography(&h_true, x, y).unwrap())
        .collect();
    for (fi, frac) in [0.10f64, 0.20, 0.30, 0.40, 0.50].iter().enumerate() {
        let n_out = (src.len() as f64 * frac).round() as usize;
        let mut dst = clean.clone();
        let mut rng = SplitMix64(1000 + fi as u64);
        // Corrupt n_out DISTINCT indices with gross outliers kept >= 10
        // px from the true target (so contamination is real, never an
        // accidental inlier).
        let mut corrupted: Vec<usize> = Vec::new();
        while corrupted.len() < n_out {
            let j = (rng.next_u64() % src.len() as u64) as usize;
            if corrupted.contains(&j) {
                continue;
            }
            loop {
                let cand = (rng.next_f64() * 220.0 - 10.0, rng.next_f64() * 170.0 - 10.0);
                let d = ((cand.0 - clean[j].0).powi(2) + (cand.1 - clean[j].1).powi(2)).sqrt();
                if d >= 10.0 {
                    dst[j] = cand;
                    break;
                }
            }
            corrupted.push(j);
        }
        let p = RansacParams {
            seed: 77,
            inlier_threshold: 1.0,
            ..RansacParams::default()
        };
        let r = ransac_homography(&src, &dst, &p)
            .unwrap_or_else(|e| panic!("{:.0}% outliers: {e}", frac * 100.0));
        for i in 0..src.len() {
            if !corrupted.contains(&i) {
                assert!(
                    r.inliers.contains(&i),
                    "{:.0}% outliers: clean point {i} not an inlier",
                    frac * 100.0
                );
            }
        }
        let dev = max_probe_deviation(&r.h, &h_true);
        assert!(
            dev < 1e-4,
            "{:.0}% outliers: probe deviation {dev}",
            frac * 100.0
        );
    }
}

#[test]
fn ransac_seed_insensitivity_on_clean_fixture() {
    // On a fixture with an unambiguous inlier structure (noiseless
    // inliers, 30% gross outliers), the SEED must not change the answer:
    // every seed converges to the same inlier set, and since the final
    // model is a deterministic refit of that set, the homographies are
    // bit-identical.
    let src = grid_points();
    let h_true = gt_h();
    let mut dst: Vec<(f64, f64)> = src
        .iter()
        .map(|&(x, y)| apply_homography(&h_true, x, y).unwrap())
        .collect();
    let mut rng = SplitMix64(4242);
    let mut corrupted = Vec::new();
    while corrupted.len() < 11 {
        let j = (rng.next_u64() % src.len() as u64) as usize;
        if corrupted.contains(&j) {
            continue;
        }
        let cand = (rng.next_f64() * 200.0, rng.next_f64() * 150.0);
        let d = ((cand.0 - dst[j].0).powi(2) + (cand.1 - dst[j].1).powi(2)).sqrt();
        if d < 10.0 {
            continue;
        }
        dst[j] = cand;
        corrupted.push(j);
    }
    let mut reference: Option<(Vec<usize>, Homography)> = None;
    for seed in [1u64, 7, 42, 999, 31337] {
        let p = RansacParams {
            seed,
            inlier_threshold: 1.0,
            ..RansacParams::default()
        };
        let r = ransac_homography(&src, &dst, &p).unwrap();
        match &reference {
            None => reference = Some((r.inliers.clone(), r.h)),
            Some((inl, h)) => {
                assert_eq!(&r.inliers, inl, "seed {seed} changed the inlier set");
                // Index loop kept: the assertion references H[i][j] by index pair.
                #[allow(clippy::needless_range_loop)]
                for i in 0..3 {
                    for j in 0..3 {
                        assert_eq!(
                            r.h[i][j].to_bits(),
                            h[i][j].to_bits(),
                            "seed {seed} changed H[{i}][{j}]"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn dlt_collinear_full_input_no_panic_pinned() {
    // All source AND destination points on straight lines. RANSAC
    // already rejects this via its per-sample degeneracy guard (builder
    // test); the raw DLT path has no such guard: the doc comment
    // promises an error on rank-deficient systems, but a collinear cloud
    // is not detected (the largest-entry check only fires on the exact
    // zero solution) and the call returns Ok with a non-unique model.
    // Pinned tolerantly: no panic, and EITHER an Err (a future guard
    // would satisfy the doc) OR an Ok model that at least reproduces the
    // constraint points it was built from. Reported in the lane log as a
    // doc-vs-behavior defect (implementation untouchable this round).
    let src: Vec<(f64, f64)> = (0..8)
        .map(|i| (i as f64 * 10.0, 3.0 + i as f64 * 2.0))
        .collect();
    let dst: Vec<(f64, f64)> = (0..8)
        .map(|i| (5.0 + i as f64 * 7.0, 40.0 - i as f64 * 3.0))
        .collect();
    match dlt_homography(&src, &dst) {
        Err(_) => {} // would match the doc's promise
        Ok(h) => {
            for (s, d) in src.iter().zip(&dst) {
                let Some((px, py)) = apply_homography(&h, s.0, s.1) else {
                    continue;
                };
                let e = ((px - d.0).powi(2) + (py - d.1).powi(2)).sqrt();
                assert!(
                    e < 1e-6,
                    "collinear DLT returned Ok but does not even fit its own points (err {e})"
                );
            }
        }
    }
}

#[test]
fn homography_nan_policy_pinned() {
    // No validation layer exists on this API (unlike the detectors).
    // Pinned honestly, tolerant of a future validation fix:
    // 1. DLT with a non-finite correspondence must not panic; today it
    //    returns Ok with a non-finite H (NaN flows through normalization
    //    and the eigensolver; every guard comparison is NaN-false).
    //    Reported in the lane log alongside the collinear doc gap.
    let mut src = grid_points();
    let dst = grid_points();
    src[3] = (f64::NAN, 12.0);
    match dlt_homography(&src, &dst) {
        Err(_) => {} // acceptable (stricter) future behavior
        Ok(h) => assert!(
            h.iter().flatten().any(|v| !v.is_finite()),
            "NaN input produced a silently finite model"
        ),
    }
    // 2. RANSAC treats non-finite correspondences as outliers (NaN
    //    reprojection error fails the strict-less inlier test): a clean
    //    majority still yields the right model and the NaN indices are
    //    never inliers.
    let src = grid_points();
    let h_true = gt_h();
    let mut dst: Vec<(f64, f64)> = src
        .iter()
        .map(|&(x, y)| apply_homography(&h_true, x, y).unwrap())
        .collect();
    let nan_idx = [2usize, 9, 17, 30];
    for &i in &nan_idx {
        dst[i] = (f64::NAN, f64::NAN);
    }
    let p = RansacParams {
        seed: 13,
        inlier_threshold: 1.0,
        ..RansacParams::default()
    };
    let r = ransac_homography(&src, &dst, &p).unwrap();
    for &i in &nan_idx {
        assert!(
            !r.inliers.contains(&i),
            "NaN correspondence {i} became an inlier"
        );
    }
    let dev = max_probe_deviation(&r.h, &h_true);
    assert!(dev < 1e-6, "NaN-contaminated RANSAC deviation {dev}");
    // 3. All correspondences non-finite: clean Err, no panic.
    let bad: Vec<(f64, f64)> = (0..8).map(|_| (f64::NAN, f64::NAN)).collect();
    assert!(ransac_homography(&src[..8], &bad, &p).is_err());
}

// --------------------------------------------------------------- multiband

#[test]
fn multiband_single_level_is_exact_linear_blend() {
    // levels = 1 is the degenerate pyramid: one Laplacian level (the
    // image itself) and one mask level (the mask itself), so the blend
    // must equal the pointwise lerp m·a + (1−m)·b exactly (same f32
    // arithmetic, no pyramid round trip at all).
    let (w, h) = (37, 22);
    let mut rng = SplitMix64(88);
    let mut a = Plane::new(w, h);
    let mut b = Plane::new(w, h);
    let mut m = Plane::new(w, h);
    for i in 0..w * h {
        a.data[i] = rng.next_f64() as f32;
        b.data[i] = rng.next_f64() as f32;
        m.data[i] = rng.next_f64() as f32;
    }
    let out = multiband_blend(&a, &b, &m, Some(1)).unwrap();
    for i in 0..w * h {
        let expect = (m.data[i] as f64 * a.data[i] as f64
            + (1.0 - m.data[i] as f64) * b.data[i] as f64) as f32;
        assert_eq!(
            out.data[i].to_bits(),
            expect.to_bits(),
            "pixel {i}: {} vs lerp {}",
            out.data[i],
            expect
        );
    }
}

#[test]
fn multiband_saturated_masks_select_single_source() {
    // Mask identically 1 (resp. 0): every mask-pyramid level stays a
    // constant plane, so the output is the Laplacian round trip of A
    // (resp. B) alone — bound 1e-5, the substrate's demonstrated
    // round-trip accuracy.
    let (w, h) = (48, 33);
    let mut rng = SplitMix64(7);
    let mut a = Plane::new(w, h);
    let mut b = Plane::new(w, h);
    for i in 0..w * h {
        a.data[i] = (0.2 + 0.6 * rng.next_f64()) as f32;
        b.data[i] = (0.2 + 0.6 * rng.next_f64()) as f32;
    }
    let ones = Plane::from_vec(w, h, vec![1.0; w * h]);
    let zeros = Plane::from_vec(w, h, vec![0.0; w * h]);
    let oa = multiband_blend(&a, &b, &ones, None).unwrap();
    let ob = multiband_blend(&a, &b, &zeros, None).unwrap();
    for i in 0..w * h {
        assert!(
            (oa.data[i] - a.data[i]).abs() < 1e-5,
            "mask=1 pixel {i}: {} vs {}",
            oa.data[i],
            a.data[i]
        );
        assert!(
            (ob.data[i] - b.data[i]).abs() < 1e-5,
            "mask=0 pixel {i}: {} vs {}",
            ob.data[i],
            b.data[i]
        );
    }
}

#[test]
fn multiband_quadrant_seam_blends_horizontal_and_vertical_together() {
    // A quadrant mask exercises a horizontal AND a vertical seam meeting
    // at a T-junction (three flat bands: A holds the top-left quadrant,
    // B the rest). Oracle bundle on flat 0.8-over-0.2 fields, 3 levels:
    // no ringing anywhere; the mid-row and mid-column transitions are
    // both monotone toward B; the four image corners hold their sources.
    let (w, h) = (64, 64);
    let a = Plane::from_vec(w, h, vec![0.8; w * h]);
    let b = Plane::from_vec(w, h, vec![0.2; w * h]);
    let mut mask = Plane::new(w, h);
    for y in 0..32 {
        for x in 0..32 {
            mask.data[y * w + x] = 1.0;
        }
    }
    let out = multiband_blend(&a, &b, &mask, Some(3)).unwrap();
    let eps = 0.02;
    for &v in &out.data {
        assert!(
            (0.2 - eps..=0.8 + eps).contains(&(v as f64)),
            "ringing: value {v}"
        );
    }
    // Corners.
    assert!(
        (out.at(1, 1) - 0.8).abs() < 0.02,
        "A corner: {}",
        out.at(1, 1)
    );
    assert!((out.at(w - 2, 1) - 0.2).abs() < 0.02);
    assert!((out.at(1, h - 2) - 0.2).abs() < 0.02);
    assert!((out.at(w - 2, h - 2) - 0.2).abs() < 0.02);
    // Mid-row (inside the top half): 0.8 -> 0.2 along +x, monotone.
    let row = 16;
    for x in 1..w {
        let step = out.at(x, row) as f64 - out.at(x - 1, row) as f64;
        assert!(step <= 1e-3, "row {row} non-monotone at x={x}: step {step}");
    }
    // Mid-column (inside the left half): 0.8 -> 0.2 along +y, monotone.
    let col = 16;
    for y in 1..h {
        let step = out.at(col, y) as f64 - out.at(col, y - 1) as f64;
        assert!(step <= 1e-3, "col {col} non-monotone at y={y}: step {step}");
    }
}

#[test]
fn multiband_feathered_mask_is_honored_not_binarized() {
    // A NON-binary (linear ramp) mask: the blend must track the feather
    // — monotone along x, near-source values at the ends, and the exact
    // 50/50 value at the ramp midpoint by symmetry. This pins that the
    // mask pyramid treats fractional weights as weights (nothing snaps
    // them to a seam). Levels capped at 3 for the same documented reason
    // as the builders' hard-seam test: at the natural depth 5 the widest
    // band reaches the borders (measured endpoints 0.269/0.651 here), so
    // 3 levels keeps the endpoints testable (measured 0.216/0.770;
    // bound 0.06). Monotonicity and the 0.5 midpoint hold at EVERY depth
    // probed (2..=5).
    let (w, h) = (64, 32);
    let a = Plane::from_vec(w, h, vec![0.8; w * h]);
    let b = Plane::from_vec(w, h, vec![0.2; w * h]);
    let mut mask = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            mask.data[y * w + x] = x as f32 / (w - 1) as f32;
        }
    }
    let out = multiband_blend(&a, &b, &mask, Some(3)).unwrap();
    let row = h / 2;
    for x in 1..w {
        let step = out.at(x, row) as f64 - out.at(x - 1, row) as f64;
        assert!(step >= -1e-3, "non-monotone at x={x}: step {step}");
    }
    assert!(
        (out.at(0, row) - 0.2).abs() < 0.06,
        "left end {}",
        out.at(0, row)
    );
    assert!(
        (out.at(w - 1, row) - 0.8).abs() < 0.06,
        "right end {}",
        out.at(w - 1, row)
    );
    // Ramp midpoint: mask 0.5 (between columns 31/32) -> value 0.5.
    let mid = 0.5 * (out.at(31, row) + out.at(32, row)) as f64;
    assert!((mid - 0.5).abs() < 0.02, "midpoint {mid} vs 0.5");
}

#[test]
fn multiband_nan_policy_pinned() {
    // Mask validation catches NaN for free (a NaN never satisfies the
    // [0,1] range test) — pinned as an Err. The IMAGE planes have no
    // such validation (unlike the detectors' posture): a non-finite
    // sample propagates through the pyramids into the output. Pinned
    // tolerantly (no panic; Err would also pass, so a future validation
    // layer does not break this test) and reported in the lane log as a
    // posture inconsistency.
    let (w, h) = (32, 24);
    let a = Plane::from_vec(w, h, vec![0.7; w * h]);
    let b = Plane::from_vec(w, h, vec![0.3; w * h]);
    let mut mask = Plane::new(w, h);
    for x in 0..w {
        for y in 0..h {
            mask.data[y * w + x] = if x < 16 { 1.0 } else { 0.0 };
        }
    }
    let mut nan_mask = mask.clone();
    nan_mask.data[5] = f32::NAN;
    assert!(
        multiband_blend(&a, &b, &nan_mask, None).is_err(),
        "NaN mask value passed the [0,1] validation"
    );
    let mut nan_a = a.clone();
    nan_a.data[10 * w + 10] = f32::NAN;
    match multiband_blend(&nan_a, &b, &mask, None) {
        Err(_) => {} // acceptable (stricter) future behavior
        Ok(out) => assert!(
            out.data.iter().any(|v| !v.is_finite()),
            "NaN image sample vanished silently"
        ),
    }
}
