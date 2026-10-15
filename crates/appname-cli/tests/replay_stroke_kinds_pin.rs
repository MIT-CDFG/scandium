// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// CLASS-KILLING pin for CRITICAL Q-1 (D-48 determinism spine): the
// aud-replay F1 class — WRITES-THROUGH-SOFT-COVERAGE — reached from every
// door q-scenario can reach, not just the four kinds that first showed it.
//
// The class: a raster write door whose per-pixel change test runs
// PRE-quantization counts the soft coverage fringe (sub-quantum values that
// store as exactly transparent), so the eagerly grown in-memory extent runs
// up to 1px proud of the persisted plane; dually, an alpha-REDUCING write
// re-tightened with the §2.4 TILE-granular shrink keeps a proud extent
// whenever the receded content edge sits inside a still-occupied tile. The
// package loader recomputes bounds PIXEL-exact on reload; `bounds` folds
// into `region_hash` → `state_hash_persistent`, so either slack forks
// live-vs-reloaded and the CLI's replay-as-recovery gate refuses undo/redo
// on the reopened package (error.command.bad_target) — undo history
// permanently unreachable.
//
// Doors covered here (each swept per SELECTION STATE {none, feathered,
// rect} where the op consumes a selection — O-WATCH: the failure surface is
// NOT a clean axis product, heal/blur passed rect×partial while failing
// none/feathered, so no extrapolating from one arm):
//   - every registered `*.stroke` kind + `stroke_path`  (finish_write_bounds)
//   - filter.apply            (appname-filters — the FIFTH door, the 24
//                              OP-filter × S-feathered × C-partial cells)
//   - edit.cut                (core: feathered/rect cut shrink)
//   - mask.apply              (core: §11.2 bake shrink)
//   - layer.merge_down        (core: receiver grow + bake_own_mask shrink)
//   - content_aware_fill      (tools: soft mask blend)
//   - doc.canvas_resize       (tools: opaque write — safe by construction,
//                              probed with the feathered arm regardless)
//   - transform.puppet        (identity arm writes nothing today; its
//                              selection arm is routed structurally)
// `clone.stroke` is unbuildable from its own contract (P-1, known/queued);
// it rides the same can_erase `install_commit` door proven by smudge.
//
// The census guard (`bounds_door_census.rs`, sibling test) REDS when a new
// bounds-mutating call site appears outside the audited set, so a future
// door cannot silently reintroduce the class.

use appname_core::{load_package_bytes_with, save_package_bytes, Engine, EngineAssembly};
use appname_tools::brush::{encode_samples, InputSample};
use appname_tools::kind_impls::preset_hard_round_16_id;
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

const W: u32 = 200;
const H: u32 = 120;
const L: &str = "99999999-0000-4000-8000-000000000001";
const U: &str = "99999999-0000-4000-8000-000000000002";
const MASK: &str = "77777777-0000-4000-8000-000000000001";
const PATH_ID: &str = "88888888-0000-4000-8000-000000000001";

/// Envelope applier with content-derived ids; panics on refusal so a probe
/// can never silently measure an empty journal (the q1_battery lesson:
/// a refused scene answers a DIFFERENT question).
struct Scene {
    e: Engine,
    n: u32,
    tag: String,
}

impl Scene {
    fn new(tag: &str) -> Scene {
        let mut s = Scene {
            e: assembly().build(),
            n: 0,
            tag: tag.to_string(),
        };
        s.apply(json!({"kind": "doc.new",
                       "params": {"width": W, "height": H, "background": "white"}}));
        s
    }

    fn apply(&mut self, v: Value) {
        self.n += 1;
        let mut env = v;
        env["schema"] = json!("appname.command.v1");
        env["id"] = json!(format!("5e1f7e57-0001-4000-8000-0000{:08x}", self.n));
        let tag = &self.tag;
        self.e
            .apply_json(&env.to_string())
            .unwrap_or_else(|err| panic!("[{tag}] setup `{}` refused: {err}", env["kind"]));
    }

    /// layer.add raster L; `partial` lays a HARD-edged opaque rect
    /// (30,20)-(130,80) on it, so L carries pixel-exact, non-full-canvas
    /// content — the extent-slack precondition.
    fn add_layer(&mut self, id: &str, partial: bool) {
        self.apply(json!({"kind": "layer.add",
                          "params": {"kind": "raster", "index": 0, "newLayerId": id}}));
        if partial {
            self.apply(json!({"kind": "selection.set",
                              "params": {"source": "rect",
                                         "rect": {"x": 30, "y": 20, "w": 100, "h": 60}}}));
            self.apply(json!({"kind": "fill", "target": {"layerId": id},
                              "params": {"content": "color",
                                         "color": {"model": "rgba",
                                                   "comps": [0.0, 0.4, 1.0, 1.0]}}}));
            self.apply(json!({"kind": "selection.none"}));
        }
    }

    /// The swept selection states — the EXACT q-scenario matrix
    /// construction (reports/q-scenario-harness/matrix.py): rect
    /// (20,15)-(120,75) pokes OUT of the partial-content rect on the
    /// top-left, and the feathered arm is `selection.set` + a separate
    /// `selection.modify {feather: 8}`. Geometry matters: the Q-1
    /// sub-quantum band for filter.apply lives where the blur tail and the
    /// feather fringe overlap OUTSIDE stored content (the diagonal corner
    /// band) — a nearly-identical probe with an inline feather 4.5 rect at
    /// (55,35) missed it and read the fifth door as green.
    fn selection(&mut self, sel: &str) {
        match sel {
            "none" => {}
            "feathered" => {
                self.apply(json!({"kind": "selection.set",
                    "params": {"source": "rect",
                               "rect": {"x": 20, "y": 15, "w": 100, "h": 60}}}));
                self.apply(json!({"kind": "selection.modify",
                    "params": {"feather": 8.0}}));
            }
            "rect" => self.apply(json!({"kind": "selection.set",
                "params": {"source": "rect",
                           "rect": {"x": 20, "y": 15, "w": 100, "h": 60}}})),
            other => panic!("unknown selection state {other}"),
        }
    }

    fn samples(&mut self, pts: &[(f64, f64, f64)]) -> String {
        let samples: Vec<InputSample> = pts
            .iter()
            .map(|(x, y, t)| InputSample {
                x: *x,
                y: *y,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: *t,
            })
            .collect();
        self.e
            .add_samples_asset(encode_samples(&samples).unwrap())
            .to_string()
    }
}

/// A stroke that STARTS inside the partial-content rect and DRAGS out past
/// its bottom-right edge: starting on content gives the content-consuming
/// kinds (smudge/heal/blur) pixels to work on; EXITING lays the soft fringe
/// at a NEW extent frontier (a stroke wholly inside the existing extent
/// grows no bounds and cannot expose the class).
const STROKE: &[(f64, f64, f64)] = &[(40.0, 30.0, 0.0), (110.0, 68.0, 0.05), (175.0, 100.0, 0.10)];

/// The buildable stroke kinds (kind name -> params beside brushPreset +
/// samples). `clone.stroke` is deliberately absent — P-1 (see file header).
fn stroke_kind_params() -> Vec<(&'static str, Value)> {
    vec![
        (
            "brush.stroke",
            json!({"color": {"model": "rgb", "comps": [1, 0, 0, 0]}, "seed": 7}),
        ),
        ("eraser.stroke", json!({})),
        ("heal.stroke", json!({"mode": "spot"})),
        ("dodge_burn.stroke", json!({"mode": "dodge"})),
        ("smudge.stroke", json!({"strength": 0.5})),
        ("blur_sharpen.stroke", json!({"mode": "blur"})),
    ]
}

const CLONE_UNBUILDABLE: &str = "clone.stroke";

/// The replay-as-recovery gate exactly as the CLI runs it (main.rs): a
/// reopened package can offer undo/redo only if replaying its journal from
/// doc.new reproduces the persisted document state.
fn recover_or_panic(loaded: &Engine, tag: &str) -> Engine {
    let cursor = loaded.journal_cursor();
    let mut rebuilt = loaded.fresh_like();
    for (i, env) in loaded.journal().iter().enumerate() {
        rebuilt.apply_envelope(env.clone()).unwrap_or_else(|e| {
            panic!(
                "[{tag}] journal entry {i} ({}) failed to re-apply: {e}",
                env.kind
            )
        });
    }
    while rebuilt.journal_cursor() > cursor {
        rebuilt.undo().expect("rewind to persisted cursor");
    }
    assert_eq!(
        rebuilt.state_hash_persistent(),
        loaded.state_hash_persistent(),
        "[{tag}] replay-as-recovery: the replayed journal must reproduce the persisted \
         document state (Q-1 / aud-replay F1 class)"
    );
    rebuilt
}

/// The full Q-1 invariant for one built scene: state survives save->reopen
/// bit-for-bit, the replay-as-recovery gate holds, and undo/redo are
/// reachable on the reopened package.
fn assert_roundtrip(tag: &str, e: Engine) {
    let before = e.state_hash_persistent();

    let bytes = save_package_bytes(&e).expect("save");
    let loaded = load_package_bytes_with(&assembly(), &bytes).expect("load");

    assert_eq!(
        before,
        loaded.engine.state_hash_persistent(),
        "[{tag}] state_hash_persistent forked across save->reopen (Q-1)"
    );

    let mut rebuilt = recover_or_panic(&loaded.engine, tag);

    assert!(
        rebuilt.undoable_in_process(),
        "[{tag}] undo unavailable after reopen"
    );
    let pre_undo = rebuilt.state_hash_persistent();
    rebuilt.undo().expect("undo after reopen");
    assert!(
        rebuilt.redoable_in_process(),
        "[{tag}] redo unavailable after reopen"
    );
    rebuilt.redo().expect("redo after reopen");
    assert_eq!(
        pre_undo,
        rebuilt.state_hash_persistent(),
        "[{tag}] undo->redo did not return to the persisted state"
    );
}

/// One stroke-kind cell: content ∈ {empty, partial}, sel ∈ {none,
/// feathered, rect} — the selection is ACTIVE during the stroke.
fn stroke_cell(kind: &str, params: &Value, content: &str, sel: &str) {
    let tag = format!("{kind}/{content}/sel-{sel}");
    let mut s = Scene::new(&tag);
    s.add_layer(L, content == "partial");
    s.selection(sel);
    let samples = s.samples(STROKE);
    let mut p = params.clone();
    p["brushPreset"] = json!(preset_hard_round_16_id().to_string());
    p["sizePx"] = json!(14.0);
    s.apply(json!({"kind": kind, "target": {"layerId": L}, "params": p,
                   "samples": samples}));
    assert_roundtrip(&tag, s.e);
}

fn stroke_path_cell(content: &str, sel: &str) {
    let tag = format!("stroke_path/{content}/sel-{sel}");
    let mut s = Scene::new(&tag);
    s.add_layer(L, content == "partial");
    s.selection(sel);
    s.apply(json!({"kind": "vector.path_new",
        "params": {"path": {"subpaths": [{"segments": [{"p0": [40, 30], "c1": [60, 40],
                                                        "c2": [90, 60], "p1": [175, 100]}],
                                          "closed": false}], "fillRule": "nonzero"},
                   "newPathId": PATH_ID}}));
    s.apply(json!({"kind": "stroke_path", "target": {"layerId": L},
                   "params": {"source": {"pathId": PATH_ID},
                              "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                              "widthPx": 6}}));
    assert_roundtrip(&tag, s.e);
}

fn sweep_strokes(sel: &str) {
    for (kind, params) in stroke_kind_params() {
        for content in ["empty", "partial"] {
            stroke_cell(kind, &params, content, sel);
        }
    }
    for content in ["empty", "partial"] {
        stroke_path_cell(content, sel);
    }
}

#[test]
fn stroke_kinds_replay_selection_none() {
    sweep_strokes("none");
}

#[test]
fn stroke_kinds_replay_selection_feathered() {
    sweep_strokes("feathered");
}

#[test]
fn stroke_kinds_replay_selection_rect() {
    sweep_strokes("rect");
}

// ---------------------------------------------------------------------------
// Soft-coverage DOOR probes beyond the stroke kinds (the evidenced census:
// every bounds-grow site from the door sweep, probed WITH the feathered arm).
// ---------------------------------------------------------------------------

/// The FIFTH door (q-scenario A3's remaining 24 cells): filter.apply on
/// partial content, per selection state.
#[test]
fn filter_apply_replays_per_selection_state() {
    for sel in ["none", "feathered", "rect"] {
        let tag = format!("filter.apply/partial/sel-{sel}");
        let mut s = Scene::new(&tag);
        s.add_layer(L, true);
        s.selection(sel);
        s.apply(json!({"kind": "filter.apply", "target": {"layerId": L},
                       "params": {"filterId": "filter.gaussian_blur",
                                  "params": {"radius": 4.0}}}));
        assert_roundtrip(&tag, s.e);
    }
}

/// edit.cut erases through selection coverage: a feathered cut leaves a
/// sub-quantum-erased fringe, a hard rect cut recedes the content edge
/// inside a still-occupied tile — both defeated the tile-granular shrink.
/// GEOMETRY MATTERS (probe hardening): the selection must span a full
/// content EDGE band — a selection that leaves a surviving content strip
/// along the old extent preserves the bounds and reads the door as green
/// (this probe's first geometry did exactly that).
#[test]
fn edit_cut_replays_per_selection_state() {
    for feather in [0.0, 8.0] {
        let sel = if feather > 0.0 { "feathered" } else { "rect" };
        let tag = format!("edit.cut/partial/sel-{sel}-full-height-band");
        let mut s = Scene::new(&tag);
        s.add_layer(L, true);
        // Full-height band over the content's right region: content right
        // edge (x 90..130, all y) is entirely erased, the new true edge
        // (hard: x=80; feathered: the quantize-to-zero ramp near x~86)
        // sits INSIDE the occupied tile [64,128).
        s.apply(json!({"kind": "selection.set",
                       "params": {"source": "rect",
                                  "rect": {"x": 80, "y": 0, "w": 120, "h": 120}}}));
        if feather > 0.0 {
            s.apply(json!({"kind": "selection.modify",
                           "params": {"feather": feather}}));
        }
        s.apply(json!({"kind": "edit.cut", "target": {"layerId": L}}));
        assert_roundtrip(&tag, s.e);
    }
}

/// mask.apply bakes §11.2 coverage into alpha. The from-selection mask edge
/// (x=93) sits off the tile grid, so the bake pulls the true content edge
/// inside a still-occupied tile — the tile-granular shrink kept the proud
/// pre-bake extent.
#[test]
fn mask_apply_replays() {
    let tag = "mask.apply/partial/from-offgrid-selection";
    let mut s = Scene::new(tag);
    s.add_layer(L, true);
    s.apply(json!({"kind": "selection.set",
                   "params": {"source": "rect",
                              "rect": {"x": 30, "y": 20, "w": 63, "h": 60},
                              "antialias": false}}));
    s.apply(json!({"kind": "mask.add", "target": {"layerId": L},
                   "params": {"kind": "raster", "newMaskId": MASK,
                              "from": "selection"}}));
    s.apply(json!({"kind": "selection.none"}));
    s.apply(json!({"kind": "mask.apply",
                   "target": {"layerId": L, "maskId": MASK}}));
    assert_roundtrip(tag, s.e);
}

/// layer.merge_down: the receiver grew by the SOURCES' whole bounds rect,
/// but a soft-fringed source's tight rect has transparent corners whose
/// composite over an empty receiver stores nothing — proud receiver extent.
#[test]
fn merge_down_replays_soft_source() {
    let tag = "layer.merge_down/soft-source-over-partial-receiver";
    let mut s = Scene::new(tag);
    s.add_layer(L, true); // receiver: partial content
                          // Insert the source DIRECTLY ABOVE the receiver (index counts from the
                          // bottom of the z-order; index 0 would land below it).
    s.apply(json!({"kind": "layer.add",
                   "params": {"kind": "raster", "index": 1, "newLayerId": U}}));
    let samples = s.samples(STROKE);
    s.apply(json!({"kind": "brush.stroke", "target": {"layerId": U},
                   "params": {"brushPreset": preset_hard_round_16_id().to_string(),
                              "sizePx": 14.0,
                              "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                              "seed": 7},
                   "samples": samples}));
    s.apply(json!({"kind": "layer.merge_down", "target": {"layerId": U},
                   "params": {}}));
    assert_roundtrip(tag, s.e);
}

/// layer.merge_down with a MASKED receiver: merge first bakes the
/// receiver's own §11.2 coverage (`bake_own_mask`), whose off-grid mask
/// edge recedes the receiver's content inside a still-occupied tile — the
/// same shrink class at the merge pre-bake door.
#[test]
fn merge_down_replays_masked_receiver() {
    let tag = "layer.merge_down/masked-receiver";
    let mut s = Scene::new(tag);
    s.add_layer(L, true); // receiver: partial content 30..130 x 20..80
                          // Off-grid from-selection mask (edge x=93 inside tile [64,128)): the
                          // merge's own-mask pre-bake recedes the receiver's true content edge
                          // sub-tile. The SOURCE stroke below is CONFINED to x<90 so its merged
                          // extent cannot swallow the receiver's proud band (the first geometry
                          // of this probe crossed x=128 and read the door as green).
    s.apply(json!({"kind": "selection.set",
                   "params": {"source": "rect",
                              "rect": {"x": 30, "y": 20, "w": 63, "h": 60},
                              "antialias": false}}));
    s.apply(json!({"kind": "mask.add", "target": {"layerId": L},
                   "params": {"kind": "raster", "newMaskId": MASK,
                              "from": "selection"}}));
    s.apply(json!({"kind": "selection.none"}));
    // Insert the source DIRECTLY ABOVE the receiver (index counts from the
    // bottom of the z-order; index 0 would land below it).
    s.apply(json!({"kind": "layer.add",
                   "params": {"kind": "raster", "index": 1, "newLayerId": U}}));
    let samples = s.samples(&[(40.0, 30.0, 0.0), (70.0, 50.0, 0.05)]);
    s.apply(json!({"kind": "brush.stroke", "target": {"layerId": U},
                   "params": {"brushPreset": preset_hard_round_16_id().to_string(),
                              "sizePx": 10.0,
                              "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                              "seed": 7},
                   "samples": samples}));
    s.apply(json!({"kind": "layer.merge_down", "target": {"layerId": U},
                   "params": {}}));
    assert_roundtrip(tag, s.e);
}

/// layer.merge_down with a MASKED SOURCE: the receiver grew by the source's
/// UNMASKED bounds rect, but the composite is gated by `mask_coverage`, so
/// nothing lands beyond the mask edge — the receiver extent ran proud of
/// its stored plane.
#[test]
fn merge_down_replays_masked_source() {
    let tag = "layer.merge_down/masked-source";
    let mut s = Scene::new(tag);
    s.add_layer(L, false); // receiver: EMPTY (no extent to hide the slack)
    s.apply(json!({"kind": "layer.add",
                   "params": {"kind": "raster", "index": 1, "newLayerId": U}}));
    // Source: partial content 30..130, then an off-grid mask 30..93.
    s.apply(json!({"kind": "selection.set",
                   "params": {"source": "rect",
                              "rect": {"x": 30, "y": 20, "w": 100, "h": 60}}}));
    s.apply(json!({"kind": "fill", "target": {"layerId": U},
                   "params": {"content": "color",
                              "color": {"model": "rgba",
                                        "comps": [0.0, 0.4, 1.0, 1.0]}}}));
    s.apply(json!({"kind": "selection.set",
                   "params": {"source": "rect",
                              "rect": {"x": 30, "y": 20, "w": 63, "h": 60}}}));
    s.apply(json!({"kind": "mask.add", "target": {"layerId": U},
                   "params": {"kind": "raster", "newMaskId": MASK,
                              "from": "selection"}}));
    s.apply(json!({"kind": "selection.none"}));
    s.apply(json!({"kind": "layer.merge_down", "target": {"layerId": U},
                   "params": {}}));
    assert_roundtrip(tag, s.e);
}

/// content_aware_fill blends synthesized content by mask/selection coverage
/// (sub-quantum feather fringe) and can synthesize MORE transparent than
/// the original — both slack directions at one door.
#[test]
fn content_aware_fill_replays_feathered() {
    let tag = "content_aware_fill/partial/sel-feathered";
    let mut s = Scene::new(tag);
    s.add_layer(L, true);
    s.selection("feathered");
    s.apply(
        json!({"kind": "content_aware_fill", "target": {"layerId": L},
                   "params": {"output": "current_layer"}}),
    );
    assert_roundtrip(tag, s.e);
}

/// doc.canvas_resize writes exact OPAQUE pixels (alpha 1.0), so every
/// counted pixel stores nonzero and the extent cannot run proud — safe by
/// construction. Probed with the feathered arm anyway (the census bar:
/// verdicts are evidence, not assertion).
#[test]
fn canvas_resize_replays_with_feathered_selection() {
    let tag = "doc.canvas_resize/partial/sel-feathered";
    let mut s = Scene::new(tag);
    s.add_layer(L, true);
    s.selection("feathered");
    s.apply(json!({"kind": "doc.canvas_resize",
                   "params": {"width": 240, "height": 150}}));
    assert_roundtrip(tag, s.e);
}

/// transform.puppet: only the §45.5 identity solve is live, and identity
/// writes nothing (bit-exact no-op) — through BOTH arms. The selection arm
/// is routed through the class door structurally so the ARAP arm cannot
/// land on the forked pair later; this probe pins that the identity commit
/// (selection active, feathered) stays replay-clean.
#[test]
fn transform_puppet_identity_replays_feathered() {
    let tag = "transform.puppet/partial/sel-feathered/identity";
    let mut s = Scene::new(tag);
    s.add_layer(L, true);
    s.selection("feathered");
    s.apply(json!({"kind": "transform.puppet", "target": {"layerId": L},
    "params": {"pins": [
        {"pos": [40.0, 40.0]},
        {"pos": [50.0, 40.0], "depth": 1},
        {"pos": [45.0, 42.0]},
    ]}}));
    assert_roundtrip(tag, s.e);
}

// ---------------------------------------------------------------------------
// Adversarial rounds.
// ---------------------------------------------------------------------------

/// Adversarial round 1: a SEQUENCE of strokes. Each write re-tightens the
/// extent to the loader-canonical rect, so the next grows from a tight base
/// — the inductive invariant that keeps the scoped grow exact.
#[test]
fn sequential_strokes_stay_loader_tight() {
    let tag = "sequential-brush";
    let mut s = Scene::new(tag);
    s.add_layer(L, false);
    let preset = preset_hard_round_16_id().to_string();
    let s1 = s.samples(&[(60.0, 50.0, 0.0), (80.0, 60.0, 0.05)]);
    s.apply(json!({"kind": "brush.stroke", "target": {"layerId": L},
                   "params": {"brushPreset": preset, "sizePx": 12.0,
                              "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                              "seed": 1},
                   "samples": s1}));
    let s2 = s.samples(&[(80.0, 60.0, 0.0), (150.0, 95.0, 0.05)]);
    s.apply(json!({"kind": "brush.stroke", "target": {"layerId": L},
                   "params": {"brushPreset": preset, "sizePx": 12.0,
                              "color": {"model": "rgb", "comps": [0, 1, 0, 0]},
                              "seed": 2},
                   "samples": s2}));
    assert_roundtrip(tag, s.e);
}

/// Adversarial round 2: a FAINT stroke (opacity/flow 0.001) whose f32 delta
/// is nonzero everywhere it lands but quantizes to no stored change — the
/// eager grow would set a non-empty extent over a plane the loader reads as
/// empty. The pixel-exact grow reads the STORED plane.
#[test]
fn faint_stroke_matches_loader_extent() {
    let tag = "faint-brush";
    let mut s = Scene::new(tag);
    s.add_layer(L, false);
    let samples = s.samples(STROKE);
    s.apply(json!({"kind": "brush.stroke", "target": {"layerId": L},
                   "params": {"brushPreset": preset_hard_round_16_id().to_string(),
                              "sizePx": 14.0,
                              "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                              "opacity": 0.001, "flow": 0.001, "seed": 5},
                   "samples": samples}));
    assert_roundtrip(tag, s.e);
}

// ---------------------------------------------------------------------------
// Census guards.
// ---------------------------------------------------------------------------

#[test]
fn every_registered_stroke_kind_is_covered() {
    // Structural anti-latency guard: the `*.stroke` kinds this pin
    // exercises, plus the documented unbuildable one, MUST equal the
    // assembly's registered `*.stroke` set. A newly registered stroke kind
    // lands here first, before it can silently reintroduce the Q-1 fork.
    let mut covered: BTreeSet<&str> = stroke_kind_params().iter().map(|(k, _)| *k).collect();
    covered.insert(CLONE_UNBUILDABLE);

    let registered: BTreeSet<&str> = assembly()
        .kinds()
        .filter(|k| k.ends_with(".stroke"))
        .collect();

    assert_eq!(
        covered,
        registered,
        "the set of registered `*.stroke` kinds changed — extend this pin's kind table so the \
         Q-1 class-killing invariant covers the new sibling (missing from pin: {:?}; extra in \
         pin: {:?})",
        registered.difference(&covered).collect::<Vec<_>>(),
        covered.difference(&registered).collect::<Vec<_>>(),
    );
    // `stroke_path` is a stroke kind without the `.stroke` suffix; it is
    // covered explicitly by the sweep tests above. Non-stroke doors are
    // covered by the door probes above + the bounds_door_census pin.
}
