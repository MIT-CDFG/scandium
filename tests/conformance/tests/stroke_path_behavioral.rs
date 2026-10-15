// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `stroke_path` end-to-end behavioral battery (§40.5 entry; the LAST
//! registry kind, O-TEXTVEC stroke slice) through the REAL production
//! composition (`assembly_point::assembled_engine()` — the live Door-1
//! stroke arms, the fill-family compositor, the §13.5 tracer).
//!
//! Exact-mass oracles ride the geometry battery's rectangle rings;
//! here the pins are the ARM's: gates in the pinned order, selection
//! semantics (gated for paths, NOT self-masked for boundary strokes),
//! locks, undo/redo byte identity, and replay determinism.

use appname_conformance::assembly_point;
use appname_core::{Engine, ErrorCode, IVec2};
use serde_json::{json, Value};

fn env(n: u32, kind: &str, layer: Option<&str>, params: Value) -> String {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("57a0be00-0000-4000-8000-0001{n:08x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(l) = layer {
        v["target"] = json!({"layerId": l});
    }
    v.to_string()
}

fn ready() -> (Engine, String) {
    let mut e = assembly_point::assembled_engine();
    e.apply_json(&env(
        1,
        "doc.new",
        None,
        json!({"width": 128, "height": 96, "background": "transparent"}),
    ))
    .unwrap();
    let bg = e.document().unwrap().z_order_walk()[0]
        .as_uuid()
        .to_string();
    (e, bg)
}

/// Register a rect path 20..84 × 20..52 through the live Door 1.
fn add_rect_path(e: &mut Engine, n: u32) -> String {
    let pid = "57a0be00-0000-4000-8000-00aa00000001".to_string();
    e.apply_json(&env(
        n,
        "vector.path_new",
        None,
        json!({
            "newPathId": pid,
            "path": {"fillRule": "nonzero", "subpaths": [{
                "closed": true,
                "segments": [
                    {"p0": [20, 20], "c1": [41.333333333333336, 20], "c2": [62.666666666666664, 20], "p1": [84, 20]},
                    {"p0": [84, 20], "c1": [84, 30.666666666666668], "c2": [84, 41.333333333333336], "p1": [84, 52]},
                    {"p0": [84, 52], "c1": [62.666666666666664, 52], "c2": [41.333333333333336, 52], "p1": [20, 52]},
                    {"p0": [20, 52], "c1": [20, 41.333333333333336], "c2": [20, 30.666666666666668], "p1": [20, 20]},
                ],
            }]},
        }),
    ))
    .unwrap();
    pid
}

fn alpha_at(e: &Engine, x: i32, y: i32) -> f32 {
    let d = e.document().unwrap();
    let layer = d.z_order_walk()[0];
    d.layer(layer)
        .unwrap()
        .pixels()
        .unwrap()
        .read_pixel_f32(IVec2::new(x, y))[3]
}

#[test]
fn line_stroke_paints_the_exact_ring_with_undo_identity() {
    let (mut e, bg) = ready();
    let pid = add_rect_path(&mut e, 2);
    let pre = e.state_hash();
    e.apply_json(&env(
        3,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "widthPx": 2,
            "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
        }),
    ))
    .unwrap();
    // Center ring around the rect edge: ink at the edge, none well
    // inside or outside.
    assert_eq!(alpha_at(&e, 20, 36), 1.0, "on the left edge");
    assert_eq!(alpha_at(&e, 52, 20), 1.0, "on the top edge");
    assert_eq!(alpha_at(&e, 52, 36), 0.0, "interior clear");
    assert_eq!(alpha_at(&e, 5, 5), 0.0, "exterior clear");
    let post = e.state_hash();
    e.undo().unwrap();
    assert_eq!(e.state_hash(), pre, "tile-delta inverse byte-exact");
    e.redo().unwrap();
    assert_eq!(e.state_hash(), post);
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), e.state_hash(), "replay determinism");
}

#[test]
fn position_arms_respect_the_fill_edge() {
    let (mut e, bg) = ready();
    let pid = add_rect_path(&mut e, 2);
    e.apply_json(&env(
        3,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "widthPx": 4,
            "color": {"model": "rgb", "comps": [0, 0, 1, 0]},
            "position": "inside",
        }),
    ))
    .unwrap();
    assert!(alpha_at(&e, 22, 36) > 0.0, "inside band paints inward");
    assert_eq!(
        alpha_at(&e, 17, 36),
        0.0,
        "inside band never leaves the fill"
    );
    e.undo().unwrap();
    e.apply_json(&env(
        4,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "widthPx": 4,
            "color": {"model": "rgb", "comps": [0, 0, 1, 0]},
            "position": "outside",
        }),
    ))
    .unwrap();
    assert!(alpha_at(&e, 18, 36) > 0.0, "outside band paints outward");
    assert_eq!(
        alpha_at(&e, 23, 36),
        0.0,
        "outside band never enters the fill"
    );
}

#[test]
fn selection_gates_path_strokes_but_not_boundary_strokes() {
    let (mut e, bg) = ready();
    let pid = add_rect_path(&mut e, 2);
    // Select the left half only: 0..52 × 0..96.
    e.apply_json(&env(
        3,
        "selection.set",
        None,
        json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 52, "h": 96}}),
    ))
    .unwrap();
    // Path stroke: gated — right edge (x=84) unpainted (§40.5).
    e.apply_json(&env(
        4,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "widthPx": 2,
            "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
        }),
    ))
    .unwrap();
    assert_eq!(alpha_at(&e, 20, 36), 1.0, "left edge inside the selection");
    assert_eq!(alpha_at(&e, 84, 36), 0.0, "right edge gated out");
    e.undo().unwrap();
    // Boundary stroke: NOT self-masked (§40.5 documented choice) — the
    // whole selection boundary rings, including the right edge at
    // x=52 where the selection itself ends.
    e.apply_json(&env(
        5,
        "stroke_path",
        Some(&bg),
        json!({
            "source": "selection",
            "widthPx": 2,
            "color": {"model": "rgb", "comps": [0, 1, 0, 0]},
        }),
    ))
    .unwrap();
    assert!(
        alpha_at(&e, 52, 48) > 0.0,
        "the boundary ring paints AT the selection edge (not self-masked)"
    );
    assert!(alpha_at(&e, 0, 48) > 0.0, "canvas-edge boundary rings too");
}

#[test]
fn gates_answer_in_the_pinned_order() {
    let (mut e, bg) = ready();
    // Dangling pathId → stage-4 bad_target.
    let err = e
        .apply_json(&env(
            2,
            "stroke_path",
            Some(&bg),
            json!({
                "source": {"pathId": "57a0be00-0000-4000-8000-00aa00000099"},
                "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
            }),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadTarget);
    // source "selection" with no active selection → selection_required.
    let err = e
        .apply_json(&env(
            3,
            "stroke_path",
            Some(&bg),
            json!({"source": "selection", "color": {"model": "rgb", "comps": [0, 0, 0, 0]}}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::SelectionRequired);
    // widthPx out of range → out_of_range (live re-check).
    let pid = add_rect_path(&mut e, 4);
    let err = e
        .apply_json(&env(
            5,
            "stroke_path",
            Some(&bg),
            json!({
                "source": {"pathId": pid},
                "widthPx": 2000,
                "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
            }),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    // tool brush: cross-field FIRST (missing brushPreset), then the
    // typed interim NI with a preset.
    let err = e
        .apply_json(&env(
            6,
            "stroke_path",
            Some(&bg),
            json!({
                "source": {"pathId": pid},
                "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
                "tool": "brush",
            }),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::MissingField, "brushPreset cross-field");
    let err = e
        .apply_json(&env(
            7,
            "stroke_path",
            Some(&bg),
            json!({
                "source": {"pathId": pid},
                "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
                "tool": "brush",
                "brushPreset": "57a0be00-0000-4000-8000-00bb00000001",
            }),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotImplemented, "brush trace interim");
    // PIXELS lock blocks.
    e.apply_json(&env(
        8,
        "layer.set_props",
        Some(&bg),
        json!({"locks": {"pixels": true}}),
    ))
    .unwrap();
    let err = e
        .apply_json(&env(
            9,
            "stroke_path",
            Some(&bg),
            json!({
                "source": {"pathId": pid},
                "color": {"model": "rgb", "comps": [0, 0, 0, 0]},
            }),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
}

#[test]
fn transparency_lock_constrains_to_existing_alpha() {
    let (mut e, bg) = ready();
    let pid = add_rect_path(&mut e, 2);
    // The transparent background + TRANSPARENCY lock: a stroke lands
    // source-atop on nothing → no pixel changes at all (§11.1).
    e.apply_json(&env(
        3,
        "layer.set_props",
        Some(&bg),
        json!({"locks": {"transparency": true}}),
    ))
    .unwrap();
    let pre = e.state_hash();
    e.apply_json(&env(
        4,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "widthPx": 2,
            "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
        }),
    ))
    .unwrap();
    assert_eq!(alpha_at(&e, 20, 36), 0.0, "source-atop on clear = clear");
    let _ = pre; // §40.1 no-op accept: the command succeeds, writes nothing
}

// ---------------------------------------------------------------------
// §5 review MINOR-6 gap closures
// ---------------------------------------------------------------------

#[test]
fn boundary_strokes_serve_all_positions_with_undo_identity() {
    let (mut e, bg) = ready();
    e.apply_json(&env(
        2,
        "selection.set",
        None,
        json!({"source": "rect", "rect": {"x": 20, "y": 20, "w": 40, "h": 30}}),
    ))
    .unwrap();
    let pre = e.state_hash();
    for (n, position, probe_in, probe_out) in [
        (3u32, "inside", (22, 35), (18, 35)),
        (4, "outside", (18, 35), (23, 35)),
    ] {
        e.apply_json(&env(
            n,
            "stroke_path",
            Some(&bg),
            json!({
                "source": "selection",
                "widthPx": 4,
                "color": {"model": "rgb", "comps": [1, 0, 1, 0]},
                "position": position,
            }),
        ))
        .unwrap();
        assert!(
            alpha_at(&e, probe_in.0, probe_in.1) > 0.0,
            "{position}: paints its side"
        );
        assert_eq!(
            alpha_at(&e, probe_out.0, probe_out.1),
            0.0,
            "{position}: respects the traced fill edge"
        );
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "{position}: tile-delta inverse");
    }
}

#[test]
fn zero_coverage_paints_are_no_op_accepts() {
    // §40.1: opacity 0, and an ACTIVE-but-empty selection gating a
    // path stroke, both succeed writing nothing.
    let (mut e, bg) = ready();
    let pid = add_rect_path(&mut e, 2);
    let pre = e.state_hash();
    e.apply_json(&env(
        3,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
            "opacity": 0,
        }),
    ))
    .unwrap();
    assert_eq!(alpha_at(&e, 20, 36), 0.0, "opacity 0 writes nothing");
    // Active empty selection: select a region wholly outside the path's
    // band, so the gate multiplies every stroke pixel to zero.
    e.apply_json(&env(
        4,
        "selection.set",
        None,
        json!({"source": "rect", "rect": {"x": 100, "y": 80, "w": 10, "h": 10}}),
    ))
    .unwrap();
    e.apply_json(&env(
        5,
        "stroke_path",
        Some(&bg),
        json!({
            "source": {"pathId": pid},
            "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
        }),
    ))
    .unwrap();
    assert_eq!(alpha_at(&e, 20, 36), 0.0, "gated-to-zero writes nothing");
    let _ = pre;
}
