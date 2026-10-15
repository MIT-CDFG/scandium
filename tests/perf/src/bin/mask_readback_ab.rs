// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §5 MASK-READBACK-PERF A/B probe (dev-only, bench-labeled; NOT a criterion
//! bench and NOT wired to `.claude/perf/baselines` — it re-baselines nothing).
//!
//! Drives the PUBLIC readback door `composite_plane_f32` (the same
//! reference tile-major walk `export_png_bytes` composites through) over
//! masked multi-layer documents built from public command doors, and reports
//! warm min/median/max over N reps. Compare two builds: HEAD (occlusion cull
//! ON) vs the cull disabled (`all_u8 = false` in png_io) to read the recovery.
//!
//! Scenarios:
//!   readback_2048_8L_occluded  — the finding's shape: 2048², 8 layers, 6
//!                                masked partial layers under an OPAQUE
//!                                unmasked top (occluder) ⇒ cull to 1/tile.
//!   readback_2048_8L_noocc     — same stack, top layer masked (NO opaque
//!                                occluder) ⇒ honest no-win control.
//!   readback_512_1L            — single opaque layer, 512² ⇒ overhead floor.

use appname_conformance::assembly_point;
use appname_core::{composite_plane_f32, Engine};
use serde_json::{json, Value};
use std::time::Instant;

fn engine() -> Engine {
    assembly_point::assembled_engine()
}

fn apply(e: &mut Engine, id: u32, kind: &str, target: Option<Value>, params: Value) {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("ab000000-0000-4000-8000-{id:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    e.apply_json(&v.to_string())
        .unwrap_or_else(|err| panic!("{kind} -> {:?} {}", err.code, err.message));
}

/// Build a `dim×dim` doc: a white bg (opaque Uniform) + `masked` partial
/// masked layers + (optionally) an opaque unmasked top occluder.
///
/// Layers are made with `layer.add` + `fill` (NOT `edit.paste`): paste
/// installs F32 layers, which trip the cull's `all_u8` gate off — so this
/// probe (and the cull) only apply to the all-U8 stacks it targets. Every
/// layer here inherits the doc's U8 depth.
fn build(dim: u32, masked: usize, opaque_top: bool) -> Engine {
    let mut e = engine();
    let mut id = 1u32;
    apply(
        &mut e,
        id,
        "doc.new",
        None,
        json!({"width": dim, "height": dim, "background": "white"}),
    );
    id += 1;

    let add_filled = |e: &mut Engine, id: &mut u32, rgb: [f32; 3]| -> String {
        let layer = format!("aabb0000-0000-4000-8000-{:012x}", *id);
        apply(
            e,
            *id,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": layer}),
        );
        *id += 1;
        apply(
            e,
            *id,
            "fill",
            Some(json!({"layerId": layer})),
            json!({"content": "color", "color": {"model": "rgb", "comps": [rgb[0], rgb[1], rgb[2], 1.0]}}),
        );
        *id += 1;
        layer
    };

    for i in 0..masked {
        let t = (i as f32 + 1.0) / (masked as f32 + 2.0);
        let layer = add_filled(&mut e, &mut id, [t, 1.0 - t, 0.5]);
        let mask = format!("aacc0000-0000-4000-8000-{:012x}", id);
        apply(
            &mut e,
            id,
            "mask.add",
            Some(json!({"layerId": layer})),
            json!({"kind": "raster", "from": "reveal_all", "newMaskId": mask}),
        );
        id += 1;
        // Partial coverage ⇒ genuinely non-occluding masked layers.
        apply(
            &mut e,
            id,
            "mask.edit",
            Some(json!({"layerId": layer, "maskId": mask})),
            json!({"edit": {"op": "fill", "value": 128}}),
        );
        id += 1;
    }

    if opaque_top {
        add_filled(&mut e, &mut id, [0.1, 0.2, 0.3]);
    } else if masked > 0 {
        // Top layer masked too ⇒ no opaque unmasked occluder anywhere.
        let layer = add_filled(&mut e, &mut id, [0.1, 0.2, 0.3]);
        let mask = format!("aacc0000-0000-4000-8000-{:012x}", id);
        apply(
            &mut e,
            id,
            "mask.add",
            Some(json!({"layerId": layer})),
            json!({"kind": "raster", "from": "reveal_all", "newMaskId": mask}),
        );
        id += 1;
        apply(
            &mut e,
            id,
            "mask.edit",
            Some(json!({"layerId": layer, "maskId": mask})),
            json!({"edit": {"op": "fill", "value": 128}}),
        );
    }
    e
}

fn time_readback(name: &str, e: &Engine, reps: usize) {
    // Warm.
    let (w, h, _) = composite_plane_f32(e).expect("readback");
    let mut ms: Vec<f64> = Vec::with_capacity(reps);
    for _ in 0..reps {
        let t = Instant::now();
        let out = composite_plane_f32(e).expect("readback");
        ms.push(t.elapsed().as_secs_f64() * 1e3);
        std::hint::black_box(&out);
    }
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = ms[ms.len() / 2];
    println!(
        "{name:32} {w}x{h}  min {:8.2} ms  median {:8.2} ms  max {:8.2} ms  (n={reps})",
        ms[0],
        median,
        ms[ms.len() - 1],
    );
}

fn main() {
    let reps: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    println!("== mask-readback A/B (composite_plane_f32) ==");
    time_readback("readback_2048_8L_occluded", &build(2048, 6, true), reps);
    time_readback("readback_2048_8L_noocc", &build(2048, 7, false), reps);
    time_readback("readback_512_1L", &build(512, 0, false), reps);
}
