// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Property suites — LIVE through the PUBLIC engine API (wave-4 activation
//! of the slice-1 skeletons; formerly property_pending.rs, all
//! `#[ignore]`s dropped; the fixed seeds recorded there carry over
//! unchanged).
//!
//! Spec: ES §34.4 (property 7 round-trip, property 9 undo restoration),
//! §10.4 (history/undo), §3.3 (tiles are `Arc<TileStorage>`; undo states
//! and snapshots share them — COW aliasing safety); §40.2 (doc.new handle
//! inverse); determinism spine D-48; plan §2.1 "Test harness v1".
//!
//! At CP-0.1 exactly one kind is implemented (doc.new, 1/102), so the
//! command variety comes from seeded doc.new parameter draws, and the
//! live walks use FIXED depths (3 undo/redo cycles in the round-trip
//! property, 4 in the sharing property — a 1-entry journal has no deeper
//! walk). The recorded seed plans feed only the generator-determinism
//! pin below; they become real walk schedules when multi-command journals
//! arrive with the pixel-writing kinds (CP-0.3), without renaming.

use appname_conformance::expect::is_uuid_shaped;
use appname_core::{Engine, TileCoord, TileStorage};
use std::sync::Arc;

/// Fixed seeds — recorded beside the tests they drive (slice-1 values,
/// unchanged; goldens/README.md rule 2 applies the same discipline).
pub const PROPERTY_SEEDS: [u64; 4] = [
    0x0061_7070_6e61_6d65, // "appname" tail, seed 1
    0x5150_2d30_2e31_0001, // CP-0.1 seed 2
    0x5150_2d30_2e31_0002, // CP-0.1 seed 3
    0x5150_2d30_2e31_0003, // CP-0.1 seed 4
];

/// SplitMix64 — tiny deterministic stream for case generation (slice-1
/// generator, unchanged). NOT a quality RNG; never engine behavior.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

/// Deterministic undo/redo walk plans (slice-1 generator, unchanged —
/// the pinned first triple keeps guarding generator drift).
pub fn undo_redo_plans(seed: u64, n: usize) -> Vec<(u64, u64, u64)> {
    let mut rng = SplitMix64(seed);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let applies = 1 + rng.below(16);
        let undos = rng.below(applies + 1);
        let redos = rng.below(undos + 1);
        out.push((applies, undos, redos));
    }
    out
}

/// Deterministic doc.new envelope from a seed draw. `bg_pool` narrows the
/// background choice (COW tile properties need tile-storing backgrounds).
fn doc_new_envelope(rng: &mut SplitMix64, seed_no: usize, iter: usize, bg_pool: &[&str]) -> String {
    let width = 1 + rng.below(256);
    let height = 1 + rng.below(256);
    let bg = bg_pool[rng.below(bg_pool.len() as u64) as usize];
    let id = format!("5eed0000-{seed_no:04x}-4000-8000-{iter:012x}",);
    assert!(is_uuid_shaped(&id), "seed-id scheme drifted: {id}");
    format!(
        r#"{{"schema":"appname.command.v1","id":"{id}","kind":"doc.new","params":{{"width":{width},"height":{height},"background":"{bg}"}}}}"#
    )
}

/// Background raster tiles of the current document:
/// (coord, held Arc, content hash at capture time).
fn background_tiles(
    engine: &Engine,
) -> Vec<(TileCoord, Arc<TileStorage>, appname_core::ContentHash)> {
    let doc = engine.document().expect("initialized");
    let mut out = Vec::new();
    for id in doc.z_order_walk() {
        let node = doc.layer(id).expect("walk resolves");
        if let Some(buf) = node.pixels() {
            for (coord, arc) in buf.tiles().iter() {
                out.push((coord, Arc::clone(arc), buf.tiles().content_hash_at(coord)));
            }
        }
    }
    out
}

const ITERS: usize = 16;

/// ES §34.4 property 9 / §10.4: undo(1) returns to the byte-exact empty
/// pre-init state, redo(1) restores the byte-exact document — across
/// repeated cycles, state_hash identical every time.
#[test]
fn prop_undo_redo_roundtrip_byte_identity() {
    for (s, seed) in PROPERTY_SEEDS.iter().enumerate() {
        let mut rng = SplitMix64(*seed);
        for i in 0..ITERS {
            let env = doc_new_envelope(&mut rng, s, i, &["transparent", "white", "black"]);
            let mut engine = Engine::new();
            let h_empty = engine.state_hash();
            engine
                .apply_bytes(env.as_bytes())
                .unwrap_or_else(|e| panic!("seed {s} iter {i}: {e}"));
            let h_doc = engine.state_hash();
            assert_ne!(h_doc, h_empty, "doc state must differ from empty");
            for cycle in 0..3 {
                engine.undo().expect("undo doc.new");
                assert_eq!(
                    engine.state_hash(),
                    h_empty,
                    "seed {s} iter {i} cycle {cycle}: undo != empty state"
                );
                engine.redo().expect("redo doc.new");
                assert_eq!(
                    engine.state_hash(),
                    h_doc,
                    "seed {s} iter {i} cycle {cycle}: redo != original"
                );
            }
        }
    }
}

/// Journal cursor invariants (proto D03..D06 analogues at engine level):
/// walking past either end rejects with the exact history codes and
/// mutates nothing; cursor/length stay in envelope.
#[test]
fn prop_journal_cursor_invariants() {
    for (s, seed) in PROPERTY_SEEDS.iter().enumerate() {
        let mut rng = SplitMix64(*seed);
        for i in 0..ITERS {
            let env = doc_new_envelope(&mut rng, s, i, &["white", "black"]);
            let mut engine = Engine::new();

            // empty engine: both directions reject, nothing changes
            let e = engine.undo().unwrap_err();
            assert_eq!(e.code.as_str(), "error.history.nothing_to_undo");
            let e = engine.redo().unwrap_err();
            assert_eq!(e.code.as_str(), "error.history.nothing_to_redo");
            assert_eq!((engine.journal().len(), engine.journal_cursor()), (0, 0));

            engine.apply_bytes(env.as_bytes()).expect("apply");
            assert_eq!((engine.journal().len(), engine.journal_cursor()), (1, 1));
            let h1 = engine.state_hash();

            let e = engine.redo().unwrap_err();
            assert_eq!(e.code.as_str(), "error.history.nothing_to_redo");
            assert_eq!(engine.state_hash(), h1, "failed redo must not mutate");

            engine.undo().expect("undo");
            assert_eq!((engine.journal().len(), engine.journal_cursor()), (1, 0));
            let e = engine.undo().unwrap_err();
            assert_eq!(e.code.as_str(), "error.history.nothing_to_undo");

            engine.redo().expect("redo");
            assert_eq!((engine.journal().len(), engine.journal_cursor()), (1, 1));
            assert_eq!(engine.state_hash(), h1);
        }
    }
}

/// D-48 replay spine at the API layer (ES §7.8/§10.6): replaying the
/// journal into a fresh engine reproduces state_hash byte-identically —
/// and replay of the replay does too.
#[test]
fn prop_replay_byte_identity() {
    for (s, seed) in PROPERTY_SEEDS.iter().enumerate() {
        let mut rng = SplitMix64(*seed);
        for i in 0..ITERS {
            let env = doc_new_envelope(&mut rng, s, i, &["transparent", "white", "black"]);
            let mut engine = Engine::new();
            engine.apply_bytes(env.as_bytes()).expect("apply");
            let h = engine.state_hash();

            let r1 = engine.replay_assembled(engine.journal()).expect("replay 1");
            assert_eq!(r1.state_hash(), h, "seed {s} iter {i}: replay");
            let r2 = r1.replay_assembled(r1.journal()).expect("replay 2");
            assert_eq!(r2.state_hash(), h, "seed {s} iter {i}: replay^2");
            assert_eq!(r1.journal().len(), engine.journal().len());
        }
    }
}

/// COW aliasing safety (ES §3.3): Arcs held by an outside observer stay
/// byte-stable while the document is undone (stashed) and are the SAME
/// allocations again after redo — the §40.2 handle inverse copies nothing
/// and never writes back through shared tiles.
#[test]
fn prop_cow_snapshot_isolation() {
    for (s, seed) in PROPERTY_SEEDS.iter().enumerate() {
        let mut rng = SplitMix64(*seed);
        for i in 0..ITERS {
            // tile-storing backgrounds only (transparent stores no tiles)
            let env = doc_new_envelope(&mut rng, s, i, &["white", "black"]);
            let mut engine = Engine::new();
            engine.apply_bytes(env.as_bytes()).expect("apply");

            let doc = engine.document().expect("initialized");
            let (kind, model) = {
                let mut km = None;
                for id in doc.z_order_walk() {
                    if let Some(buf) = doc.layer(id).unwrap().pixels() {
                        km = Some((buf.kind(), buf.model()));
                    }
                }
                km.expect("background buffer exists")
            };
            let held = background_tiles(&engine);
            assert!(
                !held.is_empty(),
                "seed {s} iter {i}: white/black background stores tiles"
            );

            engine.undo().expect("undo");
            // document is stashed; held Arcs must still read the same bytes
            for (coord, arc, hash) in &held {
                assert_eq!(
                    arc.content_hash(kind, model),
                    *hash,
                    "seed {s} iter {i} {coord:?}: held tile mutated while \
                     stashed (writeback through a shared Arc)"
                );
            }

            engine.redo().expect("redo");
            let doc = engine.document().expect("re-initialized");
            let mut restored = 0;
            for id in doc.z_order_walk() {
                if let Some(buf) = doc.layer(id).unwrap().pixels() {
                    for (coord, arc, hash) in &held {
                        let now = buf
                            .get_tile(*coord)
                            .unwrap_or_else(|| panic!("tile {coord:?} lost"));
                        assert!(
                            Arc::ptr_eq(now, arc),
                            "seed {s} iter {i} {coord:?}: redo copied the \
                             tile (handle inverse must move, not copy)"
                        );
                        assert_eq!(buf.tiles().content_hash_at(*coord), *hash);
                        restored += 1;
                    }
                }
            }
            assert_eq!(restored, held.len());
        }
    }
}

/// COW sharing economy: repeated undo/redo cycles never reallocate the
/// background tiles (ptr-identical across every cycle — zero copy
/// amplification), and the §2.2 contract "transparent stores no tiles"
/// holds under seeded geometry draws.
#[test]
fn prop_cow_unchanged_tiles_stay_shared() {
    for (s, seed) in PROPERTY_SEEDS.iter().enumerate() {
        let mut rng = SplitMix64(*seed);
        for i in 0..ITERS {
            let env = doc_new_envelope(&mut rng, s, i, &["white", "black"]);
            let mut engine = Engine::new();
            engine.apply_bytes(env.as_bytes()).expect("apply");
            let held = background_tiles(&engine);
            assert!(!held.is_empty());

            for _cycle in 0..4 {
                engine.undo().expect("undo");
                engine.redo().expect("redo");
            }
            let after = background_tiles(&engine);
            assert_eq!(after.len(), held.len());
            for ((c0, a0, h0), (c1, a1, h1)) in held.iter().zip(&after) {
                assert_eq!(c0, c1);
                assert!(
                    Arc::ptr_eq(a0, a1),
                    "seed {s} iter {i} {c0:?}: tile reallocated across \
                     undo/redo cycles"
                );
                assert_eq!(h0, h1);
            }

            // transparent branch of the §2.2 contract, same seed stream
            let env_t = doc_new_envelope(&mut rng, s, i + ITERS, &["transparent"]);
            let mut engine_t = Engine::new();
            engine_t.apply_bytes(env_t.as_bytes()).expect("apply");
            assert!(
                background_tiles(&engine_t).is_empty(),
                "seed {s} iter {i}: transparent background stored tiles"
            );
        }
    }
}

mod generator_determinism {
    //! Spec: D-48 discipline for the seeded generator itself (slice-1
    //! test, unchanged: the pinned literal keeps guarding drift).
    use super::{undo_redo_plans, PROPERTY_SEEDS};

    #[test]
    fn plans_are_deterministic_and_in_envelope() {
        for seed in PROPERTY_SEEDS {
            let a = undo_redo_plans(seed, 64);
            let b = undo_redo_plans(seed, 64);
            assert_eq!(a, b, "seeded generator must be run-stable");
            for (applies, undos, redos) in a {
                assert!((1..=16).contains(&applies));
                assert!(undos <= applies);
                assert!(redos <= undos);
            }
        }
        // pin the first plan of seed 1 so silent generator drift is loud.
        // The literal is MECHANICALLY DERIVED, not hand-invented (rule 7):
        // extracted 2026-08-11 by running this committed generator itself —
        // undo_redo_plans(PROPERTY_SEEDS[0], 1)[0] printed via a temporary
        // eprintln under `cargo test -- --nocapture` (O-INT slice-1 review,
        // MINOR 1). Any change to SplitMix64 or the plan envelope must
        // re-derive it the same way and note the re-derivation here.
        assert_eq!(
            undo_redo_plans(PROPERTY_SEEDS[0], 1)[0],
            (1, 0, 0),
            "seeded plan generator drifted: seed-1 first triple changed"
        );
    }
}
