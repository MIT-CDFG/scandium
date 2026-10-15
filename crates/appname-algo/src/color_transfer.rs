// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Clean-room implementation from the published method (E. Reinhard,
// M. Ashikhmin, B. Gooch, P. Shirley, "Color Transfer between Images",
// IEEE CG&A 2001 — census paper key reinhard01; the RGB<->LMS<->lαβ
// matrices below are the paper's published constants). The census row
// (B2-color-transfer-reinhard.yaml, MIT at pin 6724ccf9, files_taken: [])
// routes exactly this: "the engine op is a direct clean-room
// implementation from the paper with this repo as a licensed cross-check
// for goldens"; the repo is not staged locally and was not consulted.
// Histogram matching is the textbook CDF remap (census: "needs no
// reference").

//! Spec: ALGORITHMS.md §17 (see §20 note) `color_transfer` — primary
//! `reinhard01_lab_plus_histogram_match` (level L2); INTEGRATION-NOTES §6
//! order 6c.
//!
//! Statistical color transfer in Ruderman lαβ space: the input image's
//! per-channel mean/std in the decorrelated log space are moved onto the
//! reference image's, then transformed back. Plus per-channel histogram
//! matching (CDF remap) — the two halves of the contract's primary.
//!
//! The Pitié MKL/IDT quality option (`pitie_mkl_idt`, `fto_confirm: true`
//! in the contract) is NOT implemented this wave — its register confirm is
//! a Workstream C item per the census row.
//!
//! ADAPTER SEAM (brokered): O-TOOLS adjustment op `color_transfer` feeds
//! linear-RGB planes of the working space (values >= 0; log space guards
//! zeros internally) and lands results via `adjustment.add`/`filter.apply`
//! envelopes (D-48).

use crate::plane::Plane;

/// reinhard01 RGB -> LMS matrix (paper eq. 4).
const RGB_TO_LMS: [[f64; 3]; 3] = [
    [0.3811, 0.5783, 0.0402],
    [0.1967, 0.7244, 0.0782],
    [0.0241, 0.1288, 0.8444],
];

/// reinhard01 LMS -> RGB matrix (paper eq. 9).
const LMS_TO_RGB: [[f64; 3]; 3] = [
    [4.4679, -3.5873, 0.1193],
    [-1.2186, 2.3809, -0.1624],
    [0.0497, -0.2439, 1.2045],
];

/// Log-LMS floor (log10 domain guard for zero pixels).
const LOG_EPS: f64 = 1e-6;

#[inline]
fn mat_mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// RGB -> lαβ (paper eqs. 4–6): LMS, log10, then the √3/√6/√2-scaled
/// opponent decomposition.
fn rgb_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    let lms = mat_mul(&RGB_TO_LMS, rgb);
    let l = lms[0].max(LOG_EPS).log10();
    let m = lms[1].max(LOG_EPS).log10();
    let s = lms[2].max(LOG_EPS).log10();
    let s3 = 1.0 / 3f64.sqrt();
    let s6 = 1.0 / 6f64.sqrt();
    let s2 = 1.0 / 2f64.sqrt();
    [s3 * (l + m + s), s6 * (l + m - 2.0 * s), s2 * (l - m)]
}

/// lαβ -> RGB (paper eqs. 8–9).
fn lab_to_rgb(lab: [f64; 3]) -> [f64; 3] {
    let s3 = 3f64.sqrt() / 3.0;
    let s6 = 6f64.sqrt() / 6.0;
    let s2 = 2f64.sqrt() / 2.0;
    let l = lab[0] * s3 + lab[1] * s6 + lab[2] * s2;
    let m = lab[0] * s3 + lab[1] * s6 - lab[2] * s2;
    let s = lab[0] * s3 - 2.0 * lab[1] * s6;
    let lms = [10f64.powf(l), 10f64.powf(m), 10f64.powf(s)];
    mat_mul(&LMS_TO_RGB, lms)
}

/// Per-channel mean and standard deviation of the lαβ representation.
fn lab_stats(rgb: &[Plane; 3]) -> ([f64; 3], [f64; 3]) {
    let n = rgb[0].data.len();
    let mut mean = [0f64; 3];
    let mut var = [0f64; 3];
    let mut labs = Vec::with_capacity(n);
    for i in 0..n {
        let lab = rgb_to_lab([
            rgb[0].data[i] as f64,
            rgb[1].data[i] as f64,
            rgb[2].data[i] as f64,
        ]);
        for c in 0..3 {
            mean[c] += lab[c];
        }
        labs.push(lab);
    }
    for m in &mut mean {
        *m /= n as f64;
    }
    for lab in &labs {
        for c in 0..3 {
            var[c] += (lab[c] - mean[c]).powi(2);
        }
    }
    let std = [
        (var[0] / n as f64).sqrt(),
        (var[1] / n as f64).sqrt(),
        (var[2] / n as f64).sqrt(),
    ];
    (mean, std)
}

/// Reinhard statistics transfer: recolor `input` so its lαβ channel
/// statistics match `reference` (paper §"Statistics and color correction":
/// `l' = (l − μ_in) σ_ref/σ_in + μ_ref` per channel).
pub fn reinhard_transfer(input: &[Plane; 3], reference: &[Plane; 3]) -> [Plane; 3] {
    let (mu_in, sd_in) = lab_stats(input);
    let (mu_ref, sd_ref) = lab_stats(reference);
    let mut scale = [1.0f64; 3];
    for c in 0..3 {
        // Degenerate (constant) input channel: keep scale 1, the mean
        // shift still applies.
        if sd_in[c] > 1e-9 {
            scale[c] = sd_ref[c] / sd_in[c];
        }
    }
    let w = input[0].w;
    let h = input[0].h;
    let mut out = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
    for i in 0..w * h {
        let lab = rgb_to_lab([
            input[0].data[i] as f64,
            input[1].data[i] as f64,
            input[2].data[i] as f64,
        ]);
        let mut t = [0f64; 3];
        for c in 0..3 {
            t[c] = (lab[c] - mu_in[c]) * scale[c] + mu_ref[c];
        }
        let rgb = lab_to_rgb(t);
        for c in 0..3 {
            out[c].data[i] = rgb[c] as f32;
        }
    }
    out
}

/// Textbook per-channel histogram matching: remap `input` values so their
/// empirical CDF matches `reference`'s (256-bin CDFs, linear inverse
/// interpolation; input range taken from the joint min/max).
pub fn histogram_match(input: &Plane, reference: &Plane) -> Plane {
    const BINS: usize = 256;
    let lo = input
        .data
        .iter()
        .chain(&reference.data)
        .fold(f32::INFINITY, |m, &v| m.min(v)) as f64;
    let hi = input
        .data
        .iter()
        .chain(&reference.data)
        .fold(f32::NEG_INFINITY, |m, &v| m.max(v)) as f64;
    // Deliberate negated-comparison shape: also catches NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(hi > lo) {
        return input.clone(); // constant everything: nothing to match
    }
    let bin_of = |v: f64| -> usize {
        (((v - lo) / (hi - lo)) * (BINS - 1) as f64)
            .round()
            .clamp(0.0, (BINS - 1) as f64) as usize
    };
    let cdf = |p: &Plane| -> Vec<f64> {
        let mut hist = vec![0f64; BINS];
        for &v in &p.data {
            hist[bin_of(v as f64)] += 1.0;
        }
        let mut acc = 0.0;
        let mut c = vec![0f64; BINS];
        for (i, hv) in hist.iter().enumerate() {
            acc += hv;
            c[i] = acc;
        }
        for v in c.iter_mut() {
            *v /= acc;
        }
        c
    };
    let c_in = cdf(input);
    let c_ref = cdf(reference);
    // Inverse reference CDF via linear scan + interpolation.
    let inv_ref = |q: f64| -> f64 {
        let mut i = 0usize;
        while i < BINS - 1 && c_ref[i] < q {
            i += 1;
        }
        let x1 = i as f64;
        if i == 0 {
            return lo + (hi - lo) * (x1 / (BINS - 1) as f64);
        }
        let (q0, q1) = (c_ref[i - 1], c_ref[i]);
        let x = if q1 > q0 {
            (i - 1) as f64 + (q - q0) / (q1 - q0)
        } else {
            x1
        };
        lo + (hi - lo) * (x / (BINS - 1) as f64)
    };
    let mut out = Plane::new(input.w, input.h);
    for (o, &v) in out.data.iter_mut().zip(&input.data) {
        let q = c_in[bin_of(v as f64)];
        *o = inv_ref(q) as f32;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::{plane_hash, SplitMix64};

    fn scene(w: usize, h: usize, seed: u64, tint: [f32; 3]) -> [Plane; 3] {
        let mut rng = SplitMix64(seed);
        let mut out = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
        for i in 0..w * h {
            let base = 0.25 + 0.5 * rng.next_f64() as f32;
            for c in 0..3 {
                out[c].data[i] = (base * tint[c]).clamp(0.01, 1.0);
            }
        }
        out
    }

    #[test]
    fn lab_round_trip_is_stable() {
        // The paper publishes the matrices to 4 decimals; their product is
        // only approximately the identity, so ~7e-3 per-channel round-trip
        // error is inherent to reinhard01's own constants (kept verbatim on
        // purpose — the cross-check repo uses the same values).
        for rgb in [[0.5f64, 0.3, 0.7], [0.9, 0.9, 0.1], [0.02, 0.5, 0.4]] {
            let back = lab_to_rgb(rgb_to_lab(rgb));
            for c in 0..3 {
                assert!(
                    (back[c] - rgb[c]).abs() < 1e-2,
                    "{rgb:?} -> {back:?} channel {c}"
                );
            }
        }
    }

    #[test]
    fn self_transfer_is_identity() {
        let img = scene(24, 18, 3, [1.0, 0.9, 0.8]);
        let out = reinhard_transfer(&img, &img);
        for c in 0..3 {
            for i in 0..img[c].data.len() {
                assert!(
                    (out[c].data[i] - img[c].data[i]).abs() < 5e-3,
                    "c{c} pixel {i}: {} vs {}",
                    out[c].data[i],
                    img[c].data[i]
                );
            }
        }
    }

    #[test]
    fn output_statistics_match_the_reference() {
        // Warm input, cool reference: the transferred image's lαβ
        // statistics must land on the reference's.
        let input = scene(28, 20, 7, [1.0, 0.7, 0.5]);
        let reference = scene(28, 20, 11, [0.5, 0.7, 1.0]);
        let out = reinhard_transfer(&input, &reference);
        let (mu_out, sd_out) = lab_stats(&out);
        let (mu_ref, sd_ref) = lab_stats(&reference);
        for c in 0..3 {
            assert!(
                (mu_out[c] - mu_ref[c]).abs() < 0.02,
                "mean c{c}: {} vs {}",
                mu_out[c],
                mu_ref[c]
            );
            assert!(
                (sd_out[c] - sd_ref[c]).abs() < 0.02,
                "std c{c}: {} vs {}",
                sd_out[c],
                sd_ref[c]
            );
        }
        // And the recolor direction is right: blue channel mean rises.
        let mean = |p: &Plane| p.data.iter().map(|&v| v as f64).sum::<f64>() / p.data.len() as f64;
        assert!(
            mean(&out[2]) > mean(&input[2]),
            "cool reference must raise blue"
        );
    }

    #[test]
    fn histogram_match_aligns_quantiles() {
        let mut rng = SplitMix64(21);
        let mut input = Plane::new(32, 16);
        let mut reference = Plane::new(32, 16);
        for i in 0..32 * 16 {
            input.data[i] = (0.2 + 0.3 * rng.next_f64()) as f32;
            reference.data[i] = (0.5 + 0.4 * rng.next_f64()) as f32;
        }
        let out = histogram_match(&input, &reference);
        // Self-match is identity within bin resolution.
        let self_matched = histogram_match(&input, &input);
        for i in 0..input.data.len() {
            assert!(
                (self_matched.data[i] - input.data[i]).abs() < 0.01,
                "self-match pixel {i}"
            );
        }
        // Matched medians align within bin resolution.
        let median = |p: &Plane| {
            let mut v = p.data.clone();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        assert!(
            (median(&out) - median(&reference)).abs() < 0.02,
            "median {} vs {}",
            median(&out),
            median(&reference)
        );
    }

    #[test]
    fn transfer_golden_and_determinism() {
        let input = scene(24, 24, 42, [1.0, 0.8, 0.6]);
        let reference = scene(24, 24, 43, [0.6, 0.8, 1.0]);
        let a = reinhard_transfer(&input, &reference);
        let b = reinhard_transfer(&input, &reference);
        let ha: Vec<u64> = a.iter().map(|p| plane_hash(&p.data)).collect();
        let hb: Vec<u64> = b.iter().map(|p| plane_hash(&p.data)).collect();
        assert_eq!(ha, hb);
        assert_eq!(
            ha,
            vec![GOLDEN_R, GOLDEN_G, GOLDEN_B],
            "golden hash drift — investigate before re-pinning (O-QA rule)"
        );
    }

    // Pinned 2026-08-13 after the behavioral oracles above were green
    // (round-trip within the paper-constant bound, self-transfer identity,
    // statistics matching, quantile alignment).
    const GOLDEN_R: u64 = 13949794555635008495;
    const GOLDEN_G: u64 = 15662110056077782374;
    const GOLDEN_B: u64 = 13528708959041817083;
}
