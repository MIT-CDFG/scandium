// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Brute-force descriptor matching with the Lowe ratio test (lowe04 §7.1)
//! and optional mutual cross-check. Deterministic: exhaustive scans in
//! index order, strict-less comparisons, so ties keep the earliest train
//! index.
//!
//! Two entry points: L2 over 128-d float descriptors (the scale-invariant
//! detector) and Hamming over 256-bit binary descriptors (the oriented
//! binary detector; rublee11 evaluates matching by nearest Hamming
//! neighbor).

use super::orb::hamming256;

/// One accepted correspondence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    /// Index into the query descriptor set.
    pub query: usize,
    /// Index into the train descriptor set.
    pub train: usize,
    /// Distance under the metric used (L2 or Hamming).
    pub distance: f64,
}

/// Nearest + second-nearest under a caller-supplied metric; ties keep the
/// earliest index (strict-less updates).
fn best_two<F: Fn(usize) -> f64>(n: usize, dist: F) -> Option<(usize, f64, f64)> {
    let mut best = usize::MAX;
    let mut d1 = f64::INFINITY;
    let mut d2 = f64::INFINITY;
    for j in 0..n {
        let d = dist(j);
        if d < d1 {
            d2 = d1;
            d1 = d;
            best = j;
        } else if d < d2 {
            d2 = d;
        }
    }
    (best != usize::MAX).then_some((best, d1, d2))
}

/// Generic ratio-test matcher over an arbitrary metric.
///
/// `ratio` is the Lowe ratio (paper: 0.8 rejects 90% of false matches
/// while discarding under 5% of correct ones); a query is accepted when
/// `d1 < ratio · d2`. With fewer than 2 train descriptors the ratio test
/// cannot run and the query is rejected (honest failure, no special
/// case). `cross_check` additionally requires the train's own nearest
/// query to be this query (mutual consistency).
fn match_generic<F: Fn(usize, usize) -> f64>(
    nq: usize,
    nt: usize,
    dist: F,
    ratio: f64,
    cross_check: bool,
) -> Vec<Match> {
    let mut out = Vec::new();
    if nt < 2 {
        return out;
    }
    for q in 0..nq {
        let Some((j, d1, d2)) = best_two(nt, |j| dist(q, j)) else {
            continue;
        };
        // Deliberate negated-comparison shape: also catches NaN.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(d1 < ratio * d2) {
            continue;
        }
        if cross_check {
            let Some((qq, _, _)) = best_two(nq, |i| dist(i, j)) else {
                continue;
            };
            if qq != q {
                continue;
            }
        }
        out.push(Match {
            query: q,
            train: j,
            distance: d1,
        });
    }
    out
}

/// Brute-force L2 matching of 128-d float descriptors with ratio test.
pub fn match_l2_ratio(
    query: &[[f32; 128]],
    train: &[[f32; 128]],
    ratio: f64,
    cross_check: bool,
) -> Vec<Match> {
    let d = |q: usize, t: usize| -> f64 {
        query[q]
            .iter()
            .zip(&train[t])
            .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    match_generic(query.len(), train.len(), d, ratio, cross_check)
}

/// Brute-force Hamming matching of 256-bit descriptors with ratio test.
pub fn match_hamming_ratio(
    query: &[[u8; 32]],
    train: &[[u8; 32]],
    ratio: f64,
    cross_check: bool,
) -> Vec<Match> {
    let d = |q: usize, t: usize| -> f64 { hamming256(&query[q], &train[t]) as f64 };
    match_generic(query.len(), train.len(), d, ratio, cross_check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(seed: usize) -> [f32; 128] {
        // Deterministic unit-norm synthetic descriptors: one-hot-ish
        // bumps at seed-dependent positions.
        let mut d = [0f32; 128];
        d[seed % 128] = 1.0;
        d[(seed * 7 + 3) % 128] = 0.5;
        let n = (1.0f32 + 0.25).sqrt();
        for v in d.iter_mut() {
            *v /= n;
        }
        d
    }

    #[test]
    fn ratio_test_accepts_distinct_and_rejects_ambiguous() {
        let train = vec![desc(1), desc(40), desc(40)]; // 2nd/3rd identical
        let query = vec![desc(1), desc(40)];
        let m = match_l2_ratio(&query, &train, 0.8, false);
        // Query 0 matches train 0 uniquely; query 1 is ambiguous between
        // the identical trains (d1 == d2 fails d1 < 0.8·d2).
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].query, m[0].train), (0, 0));
        assert!(m[0].distance < 1e-6);
    }

    #[test]
    fn cross_check_requires_mutual_nearest() {
        // Two queries close to the same train: only the mutual pair
        // survives the cross-check.
        let train = vec![desc(5), desc(90)];
        let mut near5 = desc(5);
        near5[64] = 0.05; // slightly farther from train 0 than query 0 is
        let query = vec![desc(5), near5];
        let with = match_l2_ratio(&query, &train, 0.95, true);
        assert!(with.iter().any(|m| m.query == 0 && m.train == 0));
        assert!(!with.iter().any(|m| m.query == 1 && m.train == 0));
    }

    #[test]
    fn hamming_matcher_exact_and_tie_break_deterministic() {
        let a: [u8; 32] = [0b1010_1010; 32];
        let mut b = a;
        b[0] ^= 0xFF; // distance 8
        let c: [u8; 32] = [0; 32];
        let train = vec![b, a, c];
        let query = vec![a];
        let m = match_hamming_ratio(&query, &train, 0.8, false);
        assert_eq!(m.len(), 1);
        // Exact copy at index 1 wins with distance 0.
        assert_eq!((m[0].query, m[0].train), (0, 1));
        assert_eq!(m[0].distance, 0.0);
        // Fewer than 2 train descriptors: no ratio test possible.
        assert!(match_hamming_ratio(&query, &train[..1], 0.8, false).is_empty());
    }
}
