// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Panorama classical tier — clean-room from the published papers.
//!
//! Spec: ALGORITHMS.md §22 (22.B [NORMATIVE] selected baseline); dossier
//! `pano-align-stack-flow`; contract `contracts/algorithms/algorithms.yaml`
//! area `pano_align_stack_flow` — `features.classical:
//! [sift_cleared_US6711293, orb]`, `align.sparse: sift_orb`, `geometry:
//! {ransac_magsac: L0, bundle_adjustment: ceres_bsd, model: brown_lowe07}`,
//! `compositing: {seams: graphcut_cleared, blend: burt_adelson_multiband}`.
//!
//! SPEC-ANCHOR CAVEAT (verified against current ES text this session, per
//! the lane's stale-anchor discipline): the contract row's `spec_refs`
//! `"§24.2"`/`"§24.4"` do NOT match current ENGINEERING-SPEC.md headings
//! (ES Part 24 is "Canvas viewport and navigation"; §24.2 = rulers/guides,
//! §24.4 = high-DPI display). The ES feature matrix carries `photo.merge`
//! ("HDR multi-exposure merge; panorama stitch") as Post-1.0/plugin; the
//! governing build plan (AGENT-ASSIGNMENTS-full-build §2.5, CP-1.0 "Ports
//! wave 2" row) schedules "pano classical tier (SIFT/ORB own impls + ceres
//! link record + Burt–Adelson multiband own) + Photomerge v1 (W12)". This
//! module cites ALGORITHMS.md §22 + the dossier as its behavior anchors and
//! flags the spec_refs pair for the fold-in list.
//!
//! Rights route (census, INTEGRATION-NOTES §6 item 12a): the census row
//! `B2-opencv-sift-orb.yaml` records the upstream detector library as an
//! Apache-2.0 behavior/porting REFERENCE with `files_taken: []` and its
//! tree NOT staged — nothing from it was fetched or consulted this build.
//! Every engine in this directory is implemented STRICTLY from the
//! published papers: lowe04 (scale-invariant features), rublee11 (oriented
//! FAST + rotated binary descriptor), hartley04 (normalized DLT), and
//! burt83 (multiresolution spline blending). The GPL blending tool named in
//! INTEGRATION-NOTES §5.5 was never fetched or consulted (no memo exists;
//! multiband here is own-implemented from burt83). The SIFT patent was
//! verified in `contracts/algorithms/patent-register.yaml`: P-6711293,
//! status "Expired - Lifetime", expiry 2020-03-06. The area's banned
//! detectors (the 2029-date-gated detector; the research-only learned
//! detector) are not implemented, referenced, or approximated here.
//!
//! Modules:
//! - [`sift`] — Gaussian scale-space DoG detector + 128-d gradient
//!   histogram descriptor (lowe04).
//! - [`orb`] — FAST-9 corners, Harris ranking, intensity-centroid
//!   orientation, steered 256-bit binary descriptor (rublee11; the
//!   sampling-pattern divergence is documented in the module).
//! - [`matching`] — brute-force descriptor matching: L2 + Lowe ratio test,
//!   Hamming for binary descriptors, optional mutual cross-check.
//! - [`homography`] — normalized DLT (hartley04) + deterministic seeded
//!   RANSAC (fischler81 class; the contract's `ransac_magsac` L0 row —
//!   plain RANSAC as v1, the MAGSAC upgrade is a named remainder).
//! - [`multiband`] — Burt–Adelson Laplacian-pyramid seam blending
//!   (burt83) on the crate's own [`crate::fusion`] pyramid substrate.
//!
//! NOT this item (recorded remainders for CP-1.0 integration):
//! - `align.refine: ecc_lk_pyramid` (evangelidis08) — contract row exists;
//!   plan §2.5 scopes item 12 to detectors + homography + multiband.
//! - `compositing.seams: graphcut_cleared` — the item-8 min-cut engine
//!   ([`crate::graphcut`]) is the sanctioned seam solver; the pano seam
//!   graph construction over overlap regions is integration work, not a
//!   new engine.
//! - `geometry.bundle_adjustment: ceres_bsd` — LINKED dependency
//!   (Part 48 record; 12b DRAFT in the lane log). No solver code here.
//! - Learned tier (packs) and dense flow — INTEGRATION-NOTES order 24.
//!
//! Determinism (D-48): pure functions over planar f32 rasters, f64
//! internal accumulation, single-threaded fixed iteration order. The only
//! randomness is the crate's own seeded [`crate::plane::SplitMix64`]
//! generator, used exactly twice, both as documented parameters: the
//! binary-descriptor sampling pattern ([`orb::OrbParams::pattern_seed`])
//! and RANSAC sampling ([`homography::RansacParams::seed`]).
//!
//! ADAPTER SEAM (brokered): the CP-1.0 Photomerge v1 flow (O-TOOLS, W12)
//! adapts source layers into gray [`Plane`]s (linear working space, values
//! nominally [0, 1]), runs detect → match → RANSAC per image pair, elects
//! the geometry model, solves global geometry (the linked bundle-adjustment
//! dependency, 12b), finds seams with the item-8 min-cut engine, blends
//! with [`multiband::multiband_blend`], and lands every mutation through
//! the `appname.command.v1` envelope spine. This crate never touches
//! documents.

pub mod homography;
pub mod matching;
pub mod multiband;
pub mod orb;
pub mod sift;
#[cfg(test)]
mod tester_round;

use crate::plane::Plane;

/// Symmetric separable Gaussian kernel for standard deviation `sigma`,
/// radius `ceil(4·sigma)` (own truncation choice, documented: 4σ keeps the
/// truncated tail below ~3e-5 of the mass, comfortably under the f32
/// raster's resolution), normalized to sum 1 in f64.
pub(crate) fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    assert!(sigma > 0.0, "gaussian_kernel: sigma must be positive");
    let radius = (4.0 * sigma).ceil().max(1.0) as usize;
    let mut k = Vec::with_capacity(2 * radius + 1);
    let s2 = 2.0 * sigma * sigma;
    for i in 0..=2 * radius {
        let d = i as f64 - radius as f64;
        k.push((-d * d / s2).exp());
    }
    let sum: f64 = k.iter().sum();
    for v in k.iter_mut() {
        *v /= sum;
    }
    k
}

/// Separable Gaussian blur, replicate border, f64 accumulation.
pub(crate) fn gaussian_blur(src: &Plane, sigma: f64) -> Plane {
    let k = gaussian_kernel(sigma);
    let radius = k.len() / 2;
    let (w, h) = (src.w, src.h);
    // Horizontal pass into an f64 buffer, then vertical.
    let mut tmp = vec![0f64; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (t, &f) in k.iter().enumerate() {
                let xi = x as isize + t as isize - radius as isize;
                s += f * src.at_clamped(xi, y as isize) as f64;
            }
            tmp[y * w + x] = s;
        }
    }
    let mut out = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (t, &f) in k.iter().enumerate() {
                let yi = y as isize + t as isize - radius as isize;
                let yc = yi.clamp(0, h as isize - 1) as usize;
                s += f * tmp[yc * w + x];
            }
            out.data[y * w + x] = s as f32;
        }
    }
    out
}

/// Bilinear resize with pixel-center alignment: output sample (x, y) reads
/// source coordinate `(x + 0.5)·scale − 0.5` (clamped), the resampling
/// convention that keeps image centers fixed under scaling.
pub(crate) fn bilinear_resize(src: &Plane, nw: usize, nh: usize) -> Plane {
    assert!(nw > 0 && nh > 0, "bilinear_resize: empty target");
    let sx = src.w as f64 / nw as f64;
    let sy = src.h as f64 / nh as f64;
    let mut out = Plane::new(nw, nh);
    for y in 0..nh {
        for x in 0..nw {
            let fx = (x as f64 + 0.5) * sx - 0.5;
            let fy = (y as f64 + 0.5) * sy - 0.5;
            let x0 = fx.floor();
            let y0 = fy.floor();
            let ax = fx - x0;
            let ay = fy - y0;
            let (x0, y0) = (x0 as isize, y0 as isize);
            let v00 = src.at_clamped(x0, y0) as f64;
            let v10 = src.at_clamped(x0 + 1, y0) as f64;
            let v01 = src.at_clamped(x0, y0 + 1) as f64;
            let v11 = src.at_clamped(x0 + 1, y0 + 1) as f64;
            let v = v00 * (1.0 - ax) * (1.0 - ay)
                + v10 * ax * (1.0 - ay)
                + v01 * (1.0 - ax) * ay
                + v11 * ax * ay;
            out.data[y * nw + x] = v as f32;
        }
    }
    out
}

/// Decimate by 2 (keep even indices; odd sizes keep the last covered
/// sample) — the scale-space octave step (the source is already low-passed
/// to twice the base blur before this is called, per lowe04 §3).
pub(crate) fn decimate2(src: &Plane) -> Plane {
    let dw = src.w.div_ceil(2);
    let dh = src.h.div_ceil(2);
    let mut out = Plane::new(dw, dh);
    for y in 0..dh {
        for x in 0..dw {
            out.data[y * dw + x] = src.at((x * 2).min(src.w - 1), (y * 2).min(src.h - 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::homography::{apply_homography, ransac_homography, Homography, RansacParams};
    use super::matching::match_l2_ratio;
    use super::sift::{sift_detect, SiftParams};
    use super::*;
    use crate::plane::SplitMix64;

    #[test]
    fn gaussian_kernel_is_normalized_and_symmetric() {
        for &s in &[0.5, 1.6, 3.2] {
            let k = gaussian_kernel(s);
            let sum: f64 = k.iter().sum();
            assert!((sum - 1.0).abs() < 1e-12, "sigma {s}: sum {sum}");
            for i in 0..k.len() / 2 {
                assert_eq!(k[i], k[k.len() - 1 - i], "sigma {s}: asymmetry at {i}");
            }
        }
    }

    #[test]
    fn blur_preserves_constant_and_resize_identity() {
        let p = Plane::from_vec(9, 7, vec![0.375; 63]);
        let b = gaussian_blur(&p, 2.0);
        for &v in &b.data {
            assert!((v - 0.375).abs() < 1e-6);
        }
        // Identity-size resize reproduces the input exactly (ax = ay = 0).
        let mut q = Plane::new(8, 6);
        for (i, v) in q.data.iter_mut().enumerate() {
            *v = (i % 13) as f32 / 13.0;
        }
        let r = bilinear_resize(&q, 8, 6);
        assert_eq!(q.data, r.data);
    }

    /// Deterministic blob-field fixture: N Gaussian blobs of varied
    /// position/scale/amplitude — feature-rich, smooth, seedable.
    fn blob_field(w: usize, h: usize, seed: u64, n: usize) -> Plane {
        let mut rng = SplitMix64(seed);
        let blobs: Vec<(f64, f64, f64, f64)> = (0..n)
            .map(|_| {
                (
                    12.0 + rng.next_f64() * (w as f64 - 24.0),
                    12.0 + rng.next_f64() * (h as f64 - 24.0),
                    1.8 + rng.next_f64() * 3.0,
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

    /// Invert a 3×3 homography (adjugate / determinant).
    fn invert_h(h: &Homography) -> Homography {
        let m = h;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let adj = [
            [
                m[1][1] * m[2][2] - m[1][2] * m[2][1],
                m[0][2] * m[2][1] - m[0][1] * m[2][2],
                m[0][1] * m[1][2] - m[0][2] * m[1][1],
            ],
            [
                m[1][2] * m[2][0] - m[1][0] * m[2][2],
                m[0][0] * m[2][2] - m[0][2] * m[2][0],
                m[0][2] * m[1][0] - m[0][0] * m[1][2],
            ],
            [
                m[1][0] * m[2][1] - m[1][1] * m[2][0],
                m[0][1] * m[2][0] - m[0][0] * m[2][1],
                m[0][0] * m[1][1] - m[0][1] * m[1][0],
            ],
        ];
        let mut out = [[0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                out[i][j] = adj[i][j] / det;
            }
        }
        out
    }

    /// Backward-map warp under `h` (bilinear, edge-clamped).
    fn warp(img: &Plane, h: &Homography) -> Plane {
        let hi = invert_h(h);
        let mut out = Plane::new(img.w, img.h);
        for y in 0..img.h {
            for x in 0..img.w {
                let (sx, sy) = apply_homography(&hi, x as f64, y as f64).unwrap();
                let x0 = sx.floor();
                let y0 = sy.floor();
                let ax = sx - x0;
                let ay = sy - y0;
                let (x0, y0) = (x0 as isize, y0 as isize);
                let v = img.at_clamped(x0, y0) as f64 * (1.0 - ax) * (1.0 - ay)
                    + img.at_clamped(x0 + 1, y0) as f64 * ax * (1.0 - ay)
                    + img.at_clamped(x0, y0 + 1) as f64 * (1.0 - ax) * ay
                    + img.at_clamped(x0 + 1, y0 + 1) as f64 * ax * ay;
                out.data[y * img.w + x] = v as f32;
            }
        }
        out
    }

    #[test]
    fn end_to_end_detect_match_estimate_recovers_the_warp() {
        // Full classical-tier pipeline oracle: synthetic image pair under
        // a KNOWN rigid-ish homography (4° rotation about the center +
        // (7.5, −4.25) translation) → detect → ratio-test match →
        // seeded RANSAC → the estimated H must agree with ground truth
        // within 1.5 px over the central probe grid (detector subpixel
        // noise + warp resampling set the floor).
        let img = blob_field(128, 128, 1234, 14);
        let (c, s) = (4f64.to_radians().cos(), 4f64.to_radians().sin());
        let (cx, cy) = (64.0, 64.0);
        let h_true: Homography = [
            [c, -s, cx - c * cx + s * cy + 7.5],
            [s, c, cy - s * cx - c * cy - 4.25],
            [0.0, 0.0, 1.0],
        ];
        let warped = warp(&img, &h_true);
        let p = SiftParams::default();
        let fa = sift_detect(&img, &p).unwrap();
        let fb = sift_detect(&warped, &p).unwrap();
        assert!(
            fa.len() >= 8 && fb.len() >= 8,
            "{} / {}",
            fa.len(),
            fb.len()
        );
        let da: Vec<[f32; 128]> = fa.iter().map(|f| f.descriptor).collect();
        let db: Vec<[f32; 128]> = fb.iter().map(|f| f.descriptor).collect();
        let matches = match_l2_ratio(&da, &db, 0.8, true);
        assert!(matches.len() >= 6, "only {} matches", matches.len());
        let src: Vec<(f64, f64)> = matches
            .iter()
            .map(|m| (fa[m.query].keypoint.x, fa[m.query].keypoint.y))
            .collect();
        let dst: Vec<(f64, f64)> = matches
            .iter()
            .map(|m| (fb[m.train].keypoint.x, fb[m.train].keypoint.y))
            .collect();
        let rp = RansacParams {
            seed: 99,
            inlier_threshold: 2.0,
            ..RansacParams::default()
        };
        let r = ransac_homography(&src, &dst, &rp).unwrap();
        assert!(r.inliers.len() >= 6, "only {} inliers", r.inliers.len());
        for gy in 0..4 {
            for gx in 0..4 {
                let pt = (34.0 + 20.0 * gx as f64, 34.0 + 20.0 * gy as f64);
                let a = apply_homography(&h_true, pt.0, pt.1).unwrap();
                let b = apply_homography(&r.h, pt.0, pt.1).unwrap();
                let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                assert!(d < 1.5, "probe {:?}: deviation {d}", pt);
            }
        }
    }
}
