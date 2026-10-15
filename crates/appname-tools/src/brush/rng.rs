// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §15.4 — dynamics jitter and the `Random` input draw from a
//! **PCG32** stream "[LEGAL: O'Neill 2014]", seeded per stroke with the
//! command-recorded `seed`, sub-streamed per output index (PCG sequence id =
//! output ordinal) so evaluation order is irrelevant; replay is bit-identical
//! (§34.4).
//!
//! Implemented from the published algorithm description (M.E. O'Neill,
//! "PCG: A Family of Simple Fast Space-Efficient Statistically Good
//! Algorithms for Random Number Generation", 2014 — the pcg32 /
//! XSH-RR 64/32 member with the standard `srandom` initialization). Pure
//! integer math: bit-identical on every platform.

/// PCG32 (XSH-RR 64/32): 64-bit state, 63-bit selectable stream, 32-bit
/// output.
///
/// Public beyond the brush module since CP-0.7: the §16.3 stochastic-filter
/// convention (`Pcg32::new(seed ^ blake3_64(tile), stream: pass)`) reuses
/// this one implementation across the lane's crates (appname-filters).
#[derive(Clone, Debug)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

const PCG32_MULT: u64 = 6364136223846793005;

impl Pcg32 {
    /// The reference `pcg32_srandom(initstate, initseq)` initialization:
    /// `state = 0; inc = (seq << 1) | 1; step; state += seed; step`.
    pub fn new(seed: u64, seq: u64) -> Pcg32 {
        let mut r = Pcg32 {
            state: 0,
            inc: (seq << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    /// One 32-bit output (XSH-RR output permutation over the pre-step state).
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(PCG32_MULT).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform draw in `[0, 1)`: top 24 bits scaled by 2⁻²⁴ (exact in f32).
    pub fn u01(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// The §15.4 `U(−1, 1)` jitter draw: `2·u01 − 1` (range `[−1, 1)`).
    pub fn sym(&mut self) -> f32 {
        self.u01() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference check vector: the first outputs of the published pcg32
    /// demo for `srandom(42, 54)` (O'Neill 2014 reference distribution's
    /// documented "Round 1" values). Pins this implementation to the
    /// algorithm the spec names.
    #[test]
    fn reference_vector_seed42_seq54() {
        let mut r = Pcg32::new(42, 54);
        let got: Vec<u32> = (0..6).map(|_| r.next_u32()).collect();
        assert_eq!(
            got,
            vec![0xa15c02b7, 0x7b47f409, 0xba1d3330, 0x83d2f293, 0xbfa4784b, 0xcbed606e]
        );
    }

    #[test]
    fn streams_are_independent_and_replayable() {
        let a1: Vec<u32> = {
            let mut r = Pcg32::new(7, 0);
            (0..8).map(|_| r.next_u32()).collect()
        };
        let a2: Vec<u32> = {
            let mut r = Pcg32::new(7, 0);
            (0..8).map(|_| r.next_u32()).collect()
        };
        let b: Vec<u32> = {
            let mut r = Pcg32::new(7, 1);
            (0..8).map(|_| r.next_u32()).collect()
        };
        assert_eq!(a1, a2, "same (seed, seq) replays bit-identically");
        assert_ne!(a1, b, "different sequence ids yield different streams");
    }

    #[test]
    fn unit_draws_are_in_range() {
        let mut r = Pcg32::new(123, 5);
        for _ in 0..10_000 {
            let u = r.u01();
            assert!((0.0..1.0).contains(&u));
            let s = r.sym();
            assert!((-1.0..1.0).contains(&s));
        }
    }
}
