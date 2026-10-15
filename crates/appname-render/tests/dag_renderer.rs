// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! DAG renderer integration suite (CP-0.5 slice C): the stateful
//! [`Renderer`] against the naive oracle, through core's public doors.
//!
//! Spec: ES §4.6 (cache correctness + [PERF] hit-rate contracts), §2.7
//! (content dedup), §4.5 (Dissolve never dedups across seeds), §34.3
//! (the oracle is the behavior authority — the DAG must be bit-equal for
//! ANY cache state: cold, warm, evicting, swept).

mod support;

use appname_core::IRect;
use appname_render::{composite_document, Renderer};
use support::{load_fixture, LayerSpec, PkgSpec, TileSpec};

/// Two-tile document (512×256): distinct bg tiles + a fractional paint
/// layer; `edited` changes ONLY paint's tile (1,0) — same uuids, so a
/// warm renderer sees a one-tile content delta.
fn two_tile_spec(edited: bool) -> PkgSpec {
    PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000e0",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.125, 0.125, 0.125, 1.0],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.25, 0.25, 0.25, 1.0],
                        },
                    ],
                )
            },
            LayerSpec {
                name: "paint",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000e1",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.25, 0.0, 0.0, 0.25],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: if edited {
                                [0.0, 0.5, 0.0, 0.5]
                            } else {
                                [0.25, 0.0, 0.0, 0.25]
                            },
                        },
                    ],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000e0",
            "00000000-0000-4000-8000-0000000000e1",
        ],
    }
}

/// bg + two structurally identical isolated groups (same params), each
/// holding one identical-content paint layer (distinct uuids — content
/// hashing must dedup them; Dissolve must NOT).
fn twin_groups_spec(blend: &'static str) -> PkgSpec {
    PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000f0",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.5, 0.5, 0.5, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "ga",
                blend,
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000f1",
                    vec!["00000000-0000-4000-8000-0000000000f2"],
                )
            },
            LayerSpec {
                name: "pa",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000f2",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.30, 0.30, 0.30, 0.50],
                    }],
                )
            },
            LayerSpec {
                name: "gb",
                blend,
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000f3",
                    vec!["00000000-0000-4000-8000-0000000000f4"],
                )
            },
            LayerSpec {
                name: "pb",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000f4",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.30, 0.30, 0.30, 0.50],
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000f0",
            "00000000-0000-4000-8000-0000000000f1",
            "00000000-0000-4000-8000-0000000000f3",
        ],
    }
}

fn assert_equals_oracle(doc: &appname_core::Document, roi: IRect, r: &mut Renderer) {
    let dag = r.render(doc, roi).expect("dag render");
    let oracle = composite_document(doc, roi).expect("oracle");
    assert_eq!(
        dag.region_rgba_f32(roi),
        oracle.region_rgba_f32(roi),
        "DAG output must be bit-equal to the oracle (§34.3 discipline)"
    );
}

/// The core invariant: DAG ≡ oracle bitwise for every fixture shape and
/// every cache state (cold, warm, warm-across-edit).
#[test]
fn renderer_equals_oracle_cold_and_warm() {
    let mut r = Renderer::new();
    for spec in [
        two_tile_spec(false),
        twin_groups_spec("multiply"),
        twin_groups_spec("dissolve"),
    ] {
        let engine = load_fixture(&spec);
        let doc = engine.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        assert_equals_oracle(doc, canvas, &mut r); // cold
        assert_equals_oracle(doc, canvas, &mut r); // warm (full hit)
    }
    // Warm across an edit: same renderer, changed document.
    let engine = load_fixture(&two_tile_spec(true));
    let doc = engine.document().unwrap();
    assert_equals_oracle(doc, doc.geometry().canvas_rect(), &mut r);
}

/// Warm full-hit render: zero kernel work, every tile served whole
/// (§4.6 [PERF] — unchanged documents cost lookups only).
#[test]
fn warm_render_runs_zero_kernels() {
    let engine = load_fixture(&two_tile_spec(false));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let mut r = Renderer::new();
    r.render(doc, canvas).unwrap();
    let cold = r.stats();
    assert_eq!(cold.root_misses, 2, "two distinct tiles evaluated cold");
    assert!(cold.leaf_kernel_runs >= 4, "two layers × two tiles cold");
    r.render(doc, canvas).unwrap();
    let warm = r.stats();
    assert_eq!(warm.root_hits, 2, "both tiles served whole");
    assert_eq!(warm.root_misses, 0);
    assert_eq!(
        warm.leaf_kernel_runs, 0,
        "no kernel ran on a full-hit render"
    );
}

/// A one-tile edit invalidates exactly that tile (per-tile content
/// hashing): the unchanged tile stays a whole-tile hit — the §4.6 [PERF]
/// "re-render only its dirty tiles up its ancestry" contract.
#[test]
fn one_tile_edit_recomputes_only_that_tile() {
    let before = load_fixture(&two_tile_spec(false));
    let after = load_fixture(&two_tile_spec(true));
    let (db, da) = (before.document().unwrap(), after.document().unwrap());
    let canvas = db.geometry().canvas_rect();
    let mut r = Renderer::new();
    r.render(db, canvas).unwrap();
    let out = r.render(da, canvas).unwrap();
    let warm = r.stats();
    assert_eq!(warm.root_hits, 1, "unchanged tile (0,0) served whole");
    assert_eq!(warm.root_misses, 1, "edited tile (1,0) recomputed");
    assert_eq!(
        warm.leaf_kernel_runs, 2,
        "bg + paint at the edited tile only"
    );
    // And the output is the oracle of the AFTER document.
    let oracle = composite_document(da, canvas).unwrap();
    assert_eq!(out.region_rgba_f32(canvas), oracle.region_rgba_f32(canvas));
}

/// Content-hash dedup (§2.7): two isolated groups with identical params
/// over identical-content children (distinct uuids) share ONE cache
/// entry — the second probe hits within the same cold render.
#[test]
fn identical_isolated_groups_dedup() {
    let engine = load_fixture(&twin_groups_spec("multiply"));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let mut r = Renderer::new();
    let out = r.render(doc, canvas).unwrap();
    let s = r.stats();
    assert_eq!(s.group_misses, 1, "first twin evaluated");
    assert_eq!(s.group_hits, 1, "second twin served from the first's entry");
    assert_eq!(
        s.group_composites, 2,
        "both still composite onto the backdrop"
    );
    let oracle = composite_document(doc, canvas).unwrap();
    assert_eq!(out.region_rgba_f32(canvas), oracle.region_rgba_f32(canvas));
}

/// Dissolve never dedups across seeds (§4.5 layer-decorrelation): the
/// same twin fixture with Dissolve groups misses twice — their hashes
/// fold the per-node offsets.
#[test]
fn dissolve_groups_never_dedup() {
    let engine = load_fixture(&twin_groups_spec("dissolve"));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    // Precondition: the two fixed group uuids decorrelate (they do; the
    // assert documents the dependency rather than trusting luck).
    let tree = appname_render::flatten_document(doc).unwrap();
    let (ga, gb) = (tree[1].layer_id(), tree[2].layer_id());
    assert_ne!(
        appname_render::bn_offsets(doc.id(), ga),
        appname_render::bn_offsets(doc.id(), gb),
        "fixture uuids must decorrelate for this test to be meaningful"
    );
    let mut r = Renderer::new();
    let out = r.render(doc, canvas).unwrap();
    let s = r.stats();
    assert_eq!(s.group_misses, 2, "distinct seeds → distinct entries");
    assert_eq!(s.group_hits, 0);
    let oracle = composite_document(doc, canvas).unwrap();
    assert_eq!(out.region_rgba_f32(canvas), oracle.region_rgba_f32(canvas));
}

/// Tiny budgets force eviction; zero budget disables insertion — output
/// stays bit-equal to the oracle either way (the cache can change cost,
/// never pixels).
#[test]
fn eviction_and_zero_budget_never_change_pixels() {
    for budget in [0usize, 2 * 1024 * 1024] {
        let mut r = Renderer::with_budget_bytes(budget);
        for edited in [false, true, false] {
            let engine = load_fixture(&two_tile_spec(edited));
            let doc = engine.document().unwrap();
            let canvas = doc.geometry().canvas_rect();
            assert_equals_oracle(doc, canvas, &mut r);
        }
        if budget == 0 {
            assert_eq!(r.stats().entries, 0, "zero budget stores nothing");
        }
    }
}

/// The generation sweep bounds residency: after two renders of a
/// different document, the first document's entries are gone; rendering
/// it again is correct (recompute, not resurrection).
#[test]
fn generation_sweep_drops_stale_entries() {
    let ea = load_fixture(&two_tile_spec(false));
    let eb = load_fixture(&twin_groups_spec("multiply"));
    let (da, db) = (ea.document().unwrap(), eb.document().unwrap());
    let (ca, cb) = (da.geometry().canvas_rect(), db.geometry().canvas_rect());
    let mut r = Renderer::new();
    r.render(da, ca).unwrap();
    let resident_a = r.stats().entries;
    assert!(resident_a >= 2);
    r.render(db, cb).unwrap();
    r.render(db, cb).unwrap();
    let s = r.stats();
    assert!(
        s.swept >= resident_a,
        "doc-A entries unreachable for 2 generations get swept"
    );
    assert_equals_oracle(da, ca, &mut r);
    let again = r.stats();
    assert_eq!(
        again.root_misses, 2,
        "swept entries recompute, never resurrect"
    );
}

/// §4.1 step-5 empty-need skip: a group whose only content sits in tile
/// (1,0) is never probed, hashed, or evaluated at tile (0,0) — and a
/// sub-ROI render of just tile (0,0) does no group work at all.
#[test]
fn roi_scheduling_skips_out_of_need_nodes() {
    let spec = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000100",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.125, 0.125, 0.125, 1.0],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.25, 0.25, 0.25, 1.0],
                        },
                    ],
                )
            },
            LayerSpec {
                name: "g",
                blend: "multiply",
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-000000000101",
                    vec!["00000000-0000-4000-8000-000000000102"],
                )
            },
            LayerSpec {
                name: "paint",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000102",
                    vec![TileSpec {
                        x: 1,
                        y: 0,
                        value: [0.25, 0.0, 0.0, 0.25],
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-000000000100",
            "00000000-0000-4000-8000-000000000101",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let mut r = Renderer::new();
    let out = r.render(doc, canvas).unwrap();
    let s = r.stats();
    assert_eq!(
        s.group_misses + s.group_hits,
        1,
        "group work only at its needed tile"
    );
    assert!(
        s.roi_skips >= 2,
        "paint + group skipped at tile (0,0): {s:?}"
    );
    let oracle = composite_document(doc, canvas).unwrap();
    assert_eq!(out.region_rgba_f32(canvas), oracle.region_rgba_f32(canvas));

    // Sub-ROI pull: only tile (0,0) requested → zero group work.
    let mut r2 = Renderer::new();
    let sub = IRect::new(0, 0, 256, 256);
    let out = r2.render(doc, sub).unwrap();
    let s2 = r2.stats();
    assert_eq!(
        s2.group_misses + s2.group_hits,
        0,
        "pull-driven: nothing pulled the group"
    );
    let oracle = composite_document(doc, sub).unwrap();
    assert_eq!(out.region_rgba_f32(sub), oracle.region_rgba_f32(sub));
}

/// TRIPWIRE (slice C+E review MINOR-2): the DAG cache keys are sound
/// only while core's §2.4 prune discipline holds — no dense tile whose
/// materialized bytes are uniform-valued survives a public door
/// (bitwise-zero → absent; uniform-valued dense → Uniform storage).
/// The key cannot tell a Uniform from its dense twin (equal content
/// hashes by core's design), but zero-working-alpha evaluation CAN
/// (normative uniform skip vs the dense kernel's 1-ulp round trip). If
/// core ever lets a uniform-valued DENSE tile through, this trips
/// before a warm cache can serve divergent bytes.
#[test]
fn prune_discipline_holds_through_public_doors() {
    // A uniform-valued chunk (dense on the wire) and a genuinely dense
    // control, through the loader.
    let spec = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![LayerSpec {
            name: "l",
            ..LayerSpec::raster(
                "00000000-0000-4000-8000-000000000110",
                vec![
                    TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.25, 0.5, 0.125, 0.5],
                    }, // uniform-valued
                    TileSpec {
                        x: 1,
                        y: 0,
                        value: [0.0, 0.0, 0.0, 0.0],
                    }, // bitwise zero
                ],
            )
        }],
        root_children: vec!["00000000-0000-4000-8000-000000000110"],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let id = appname_render::flatten_document(doc).unwrap()[0].layer_id();
    let buf = doc.layer(id).unwrap().pixels().unwrap();
    let t00 = buf
        .get_tile(appname_core::TileCoord::new(0, 0, 0))
        .expect("uniform-valued content persists");
    assert!(
        t00.is_uniform(),
        "§2.4 collapse: a uniform-valued dense chunk must load as Uniform storage — \
         the DAG cache-key soundness invariant (dag.rs leaf_tile_hash doc) is broken; \
         raise to O-INT/O-CORE before merging"
    );
    assert!(
        buf.get_tile(appname_core::TileCoord::new(0, 1, 0))
            .is_none(),
        "bitwise-zero chunk must prune to absent (§2.4)"
    );
}
