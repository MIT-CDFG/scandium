// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.5 ("PSD readers are continuously fuzzed", §34.2) — the
//! CP-0.7 WAVE-1 FLOOR: a seed corpus + deterministic mutation harness in
//! plain tests. cargo-fuzz proper needs a deps record (requested from
//! O-BUILD in the lane log); when it lands (wave 2), these seeds and
//! mutators move into `fuzz_targets/` unchanged.
//!
//! Bar (the D-series discipline): every mutated input either imports Ok
//! or returns a TYPED `CodecError` mapped into the §8.1 taxonomy — never
//! a panic, never an abort, and the §8.1 bomb gates hold. Determinism: a
//! fixed xorshift64* PRNG (seeds pinned) — identical runs mutate
//! identical bytes.

mod common;
mod psd_common;

use appname_io::{CodecRegistry, DecodeOpts};
use common::srgb_bytes;
use psd_common::{LayerSpec, PsdBuilder};

/// Pinned xorshift64* — no external RNG dep, identical across runs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The seed corpus: one representative per structural family.
fn seeds() -> Vec<Vec<u8>> {
    let rgba = srgb_bytes(6, 5, false);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();

    // 1. Flat RGB.
    let mut flat = PsdBuilder::rgb8(6, 5);
    flat.merged_from_rgb8(&rgb, 3);

    // 2. Layered with groups, mask, keys, RLE.
    let mut opener = LayerSpec::basic((0, 0, 0, 0), "open", &[]);
    opener.channels.clear();
    opener.keys.push((*b"lsct", 3u32.to_be_bytes().to_vec()));
    let mut masked = LayerSpec::basic((0, 0, 5, 6), "masked", &rgba);
    masked.mask = Some(((0, 0, 2, 2), 0, 0));
    masked.channels.push((-2, vec![255, 128, 64, 0], false));
    masked.keys.push((*b"lfx2", vec![0u8; 6]));
    masked.keys.push((*b"iOpa", vec![128]));
    for ch in &mut masked.channels {
        ch.2 = true;
    }
    let mut folder = LayerSpec::basic((0, 0, 0, 0), "G", &[]);
    folder.channels.clear();
    folder.keys.push((*b"lsct", 1u32.to_be_bytes().to_vec()));
    let mut layered = PsdBuilder::rgb8(6, 5);
    layered.layers = vec![opener, masked, folder];
    layered.merged_from_rgb8(&rgb, 3);

    // 3. 16-bit gray via Lr16 (PSB).
    let mut deep_layer = LayerSpec::basic((0, 0, 1, 1), "deep", &[0, 0, 0, 255]);
    deep_layer.channels = vec![(0, 0x8081u16.to_be_bytes().to_vec(), false)];
    let mut deep = PsdBuilder::rgb8(1, 1);
    deep.psb = true;
    deep.mode = 1;
    deep.depth = 16;
    deep.channels = 1;
    deep.via_lr16 = true;
    deep.layers = vec![deep_layer];
    deep.merged = vec![0x8081u16.to_be_bytes().to_vec()];

    // 4. Indexed with palette + resources.
    let mut indexed = PsdBuilder::rgb8(3, 1);
    indexed.mode = 2;
    indexed.channels = 1;
    indexed.color_mode_data = vec![7u8; 768];
    indexed.resources.push((1047, 1u16.to_be_bytes().to_vec()));
    indexed.resources.push((1039, b"icc".to_vec()));
    indexed.merged = vec![vec![0, 1, 2]];

    vec![flat.build(), layered.build(), deep.build(), indexed.build()]
}

fn assert_typed_or_ok(bytes: &[u8], ctx: &str) {
    let reg = CodecRegistry::builtin();
    // Tight limits double as the bomb-gate probe under mutation.
    let opts = DecodeOpts {
        max_pixels: 1 << 20,
        ..DecodeOpts::default()
    };
    match reg.by_id("psd").unwrap().decode(bytes, &opts) {
        Ok(_) => {}
        Err(e) => {
            let mid = e.message_id();
            assert!(
                mid.starts_with("error.codec.") || mid.starts_with("error.io."),
                "{ctx}: unmapped error {mid}"
            );
        }
    }
}

/// ~1600 single/multi-byte flips across the corpus: no panics, typed
/// errors only. (A panic aborts the test process — the assertion IS the
/// completed run.)
#[test]
fn byte_flips_never_panic() {
    let mut rng = Rng(0x1BAD_5EED_0BAD_CAFE);
    for (si, seed) in seeds().iter().enumerate() {
        for round in 0..400 {
            let mut fuzzed = seed.clone();
            let flips = 1 + rng.below(4);
            for _ in 0..flips {
                let at = rng.below(fuzzed.len());
                fuzzed[at] ^= (rng.next() & 0xFF) as u8;
            }
            assert_typed_or_ok(&fuzzed, &format!("seed {si} round {round}"));
        }
    }
}

/// Truncations at every eighth boundary + random cuts: typed, no panics.
#[test]
fn truncations_never_panic() {
    let mut rng = Rng(0xFEED_FACE_CAFE_F00D);
    for (si, seed) in seeds().iter().enumerate() {
        for denom in 1..8usize {
            let cut = seed.len() * denom / 8;
            assert_typed_or_ok(&seed[..cut], &format!("seed {si} cut {denom}/8"));
        }
        for round in 0..100 {
            let cut = rng.below(seed.len());
            assert_typed_or_ok(&seed[..cut], &format!("seed {si} random cut {round}"));
        }
    }
}

/// Length-field scribbles: overwrite aligned 32-bit words with hostile
/// values (0, MAX, off-by-one patterns) — the classic parser killers.
#[test]
fn length_field_scribbles_never_panic() {
    let hostile: [u32; 6] = [0, u32::MAX, 0x7FFF_FFFF, 0x8000_0000, 1, 0x0001_0000];
    let mut rng = Rng(0x00DD_BA11_5EED_C0DE);
    for (si, seed) in seeds().iter().enumerate() {
        for round in 0..200 {
            let mut fuzzed = seed.clone();
            let at = rng.below(fuzzed.len().saturating_sub(4));
            let v = hostile[rng.below(hostile.len())];
            fuzzed[at..at + 4].copy_from_slice(&v.to_be_bytes());
            assert_typed_or_ok(&fuzzed, &format!("seed {si} scribble {round}"));
        }
    }
}

/// Seed sanity: the unmutated corpus imports Ok (the mutations above are
/// exercising real parse paths, not dead ones).
#[test]
fn seeds_import_clean() {
    let reg = CodecRegistry::builtin();
    for (si, seed) in seeds().iter().enumerate() {
        let imported = reg
            .by_id("psd")
            .unwrap()
            .decode(seed, &DecodeOpts::default())
            .unwrap_or_else(|e| panic!("seed {si} must import: {e}"));
        assert!(imported.compat_report.is_some());
    }
}
