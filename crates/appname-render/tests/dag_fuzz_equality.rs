// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Seeded fuzz-equality soak (CP-0.5, O-RENDER): randomized documents —
//! nested groups, all 27 modes, alpha extremes, HDR values, uniform and
//! dense tiles — pinned DAG ≡ oracle within the requested ROI, cold,
//! warm, and warm-across-edit, plus oracle determinism across loads.
//!
//! Spec: ES §34.3/§34.4 (property discipline; the oracle is the
//! behavior authority), §4.6/§4.7 (what the DAG must reproduce).
//! Deterministic: SplitMix64 with fixed seeds — a failure reproduces
//! bit-identically everywhere; the failing case index and seed print in
//! the panic message.

mod support;

use appname_render::{composite_document, Renderer};
use support::{load_fixture, LayerSpec, NodeKind, PkgSpec, TileSpec};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn unit(&mut self) -> f32 {
        (self.next() % 4097) as f32 / 4096.0
    }
}

const BLENDS: [&str; 27] = [
    "normal",
    "dissolve",
    "darken",
    "multiply",
    "colorBurn",
    "linearBurn",
    "darkerColor",
    "lighten",
    "screen",
    "colorDodge",
    "linearDodge",
    "lighterColor",
    "overlay",
    "softLight",
    "hardLight",
    "vividLight",
    "linearLight",
    "pinLight",
    "hardMix",
    "difference",
    "exclusion",
    "subtract",
    "divide",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn fresh_id(rng: &mut Rng) -> &'static str {
    leak(format!(
        "00000000-0000-4000-8000-{:04x}{:08x}",
        rng.below(0xFFFF),
        rng.below(0xFFFF_FFFF)
    ))
}

/// One random premultiplied-valid tile value; occasionally exact-0/1
/// alpha, uniform-friendly dyadics, or HDR super-brights (§6.4).
fn tile_value(rng: &mut Rng) -> [f32; 4] {
    match rng.below(8) {
        0 => [0.0, 0.0, 0.0, 0.0], // bitwise zero (prunes)
        1 => {
            let a = rng.unit();
            [0.0, 0.0, 0.0, a] // black at random alpha
        }
        2 => [0.25, 0.5, 0.125, 1.0], // opaque dyadic
        3 => {
            let a = 0.5f32;
            [3.5 * a, 0.0, 1.25 * a, a] // HDR super-bright premult
        }
        _ => {
            let a = rng.unit();
            [rng.unit() * a, rng.unit() * a, rng.unit() * a, a]
        }
    }
}

/// A random raster layer: 0–3 tiles over a 2×1 tile canvas.
fn random_raster(rng: &mut Rng) -> LayerSpec {
    let mut tiles = Vec::new();
    for (tx, ty) in [(0i32, 0i32), (1, 0)] {
        if rng.below(4) < 3 {
            tiles.push(TileSpec {
                x: tx,
                y: ty,
                value: tile_value(rng),
            });
        }
    }
    let mut l = LayerSpec::raster(fresh_id(rng), tiles);
    l.name = leak(format!("r{}", rng.below(1_000_000)));
    l.blend = BLENDS[rng.below(27) as usize];
    l.opacity = [0.0, 0.25, 0.5, 1.0, 1.0, 1.0][rng.below(6) as usize];
    l.fill_opacity = [0.5, 1.0, 1.0][rng.below(3) as usize];
    l.visible = rng.below(8) != 0;
    l
}

/// A random document: up to 6 top-level items; groups nest ≤ 2 deep
/// with 1–3 children each. `edit_seed` perturbs exactly one raster
/// value so warm-across-edit runs see a content delta on stable uuids.
fn random_spec(seed: u64, edit_seed: u64) -> PkgSpec {
    let mut rng = Rng(seed);
    let mut layers: Vec<LayerSpec> = Vec::new();
    let mut root_children: Vec<&'static str> = Vec::new();

    fn build_item(rng: &mut Rng, layers: &mut Vec<LayerSpec>, depth: u32) -> &'static str {
        if depth < 2 && rng.below(4) == 0 {
            let n = 1 + rng.below(3);
            let children: Vec<&'static str> =
                (0..n).map(|_| build_item(rng, layers, depth + 1)).collect();
            let mut g = LayerSpec::group(fresh_id(rng), children);
            g.name = leak(format!("g{}", rng.below(1_000_000)));
            g.blend = BLENDS[rng.below(27) as usize];
            g.opacity = [0.0, 0.5, 1.0, 1.0][rng.below(4) as usize];
            g.fill_opacity = [0.5, 1.0][rng.below(2) as usize];
            g.visible = rng.below(8) != 0;
            let id = g.id;
            layers.push(g);
            id
        } else {
            let l = random_raster(rng);
            let id = l.id;
            layers.push(l);
            id
        }
    }

    let items = 1 + rng.below(6);
    for _ in 0..items {
        root_children.push(build_item(&mut rng, &mut layers, 0));
    }

    // The edit: overwrite the first raster's first tile with a value
    // drawn from edit_seed (edit_seed == 0 → no edit).
    if edit_seed != 0 {
        let mut erng = Rng(edit_seed);
        if let Some(l) = layers.iter_mut().find_map(|l| match &mut l.kind {
            NodeKind::Raster { tiles } if !tiles.is_empty() => Some(tiles),
            _ => None,
        }) {
            l[0].value = tile_value(&mut erng);
        }
    }

    PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers,
        root_children,
    }
}

fn run_cases(range: std::ops::Range<u64>) {
    let mut renderer = Renderer::new();
    for case in range {
        let seed = 0xF0220000 + case;
        let spec = random_spec(seed, 0);
        let engine = load_fixture(&spec);
        let doc = engine.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        let oracle = composite_document(doc, canvas).unwrap();

        for pass in ["cold", "warm"] {
            let dag = renderer.render(doc, canvas).unwrap();
            assert_eq!(
                dag.region_rgba_f32(canvas),
                oracle.region_rgba_f32(canvas),
                "case {case} (seed {seed:#x}) {pass}: DAG diverged from oracle"
            );
        }

        // Warm-across-edit on stable uuids.
        let edited = random_spec(seed, seed ^ 0xED17);
        let engine2 = load_fixture(&edited);
        let doc2 = engine2.document().unwrap();
        let dag2 = renderer.render(doc2, canvas).unwrap();
        let oracle2 = composite_document(doc2, canvas).unwrap();
        assert_eq!(
            dag2.region_rgba_f32(canvas),
            oracle2.region_rgba_f32(canvas),
            "case {case} (seed {seed:#x}) edited: DAG diverged from oracle"
        );

        // Oracle determinism across loads of identical bytes.
        let engine3 = load_fixture(&spec);
        let doc3 = engine3.document().unwrap();
        let oracle3 = composite_document(doc3, canvas).unwrap();
        assert_eq!(
            oracle.region_rgba_f32(canvas),
            oracle3.region_rgba_f32(canvas),
            "case {case} (seed {seed:#x}): oracle not load-stable"
        );
    }
}

/// Per-PR slice of the soak (~15 s): 10 seeded documents, each checked
/// DAG ≡ oracle (requested ROI) cold and warm on a SHARED renderer
/// (cross-document cache pollution is part of the test), then
/// warm-across-edit, then oracle determinism across loads.
#[test]
fn fuzz_dag_equals_oracle_across_random_documents() {
    run_cases(0..10);
}

/// The extended soak (60 cases, ~100 s) — `#[ignore]`d out of the
/// per-PR gate; run explicitly (`cargo test -- --ignored`) in
/// nightly/soak contexts. Executed green 2026-08-13 overnight
/// (lane-log record); O-QA wires it into the nightly tier at their
/// wave-2 harness pass.
#[test]
#[ignore = "extended soak — nightly tier (see doc)"]
fn fuzz_soak_extended() {
    run_cases(0..60);
}
