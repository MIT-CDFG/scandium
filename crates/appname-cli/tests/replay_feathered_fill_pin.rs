// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Regression pin for aud-replay F1 (CRITICAL, D-48 determinism spine):
// a fill through a FEATHERED selection that covers LESS than the full
// canvas must reproduce `state_hash` across save -> reopen, so the
// replay-as-recovery gate keeps undo/redo reachable.
//
// The break: the fill's eager bounds growth counted the feather fringe
// (sub-quantum coverage that stores as transparent) as content, so the
// in-memory layer extent ran up to 1px proud of the persisted extent.
// The package loader recomputes bounds pixel-exact over the quantized
// plane, so `region_hash` (which folds bounds) — and thus
// `state_hash_persistent` — forked live-vs-reloaded, and the CLI's
// replay-as-recovery gate refused undo/redo on the reopened package.
// The fix re-tightens the fill's bounds pixel-exact, matching the loader.
//
// Sweep mirrors the audit's isolation (feather=0 always passed;
// feather>0 on a partial rect must now pass too; full-canvas feather
// passed and must still pass).

use appname_core::{load_package_bytes_with, save_package_bytes, Engine, EngineAssembly};

fn assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

/// doc.new (96x64 white) -> selection.set(rect, feather) -> layer.add
/// raster -> fill background on the NEW (empty) layer. The empty raster
/// starts with empty bounds, so the fill alone establishes the extent —
/// the reduced repro from the audit probe.
fn build(feather: f64, rect: (i32, i32, i32, i32)) -> Engine {
    let mut e = assembly().build();
    let doc_new_id = "5e1f7e57-0001-4000-8000-000000000001";
    e.apply_json(&format!(
        r#"{{"schema":"appname.command.v1","id":"{doc_new_id}","kind":"doc.new","params":{{"width":96,"height":64,"background":"white"}}}}"#
    ))
    .expect("doc.new");
    let (x, y, w, h) = rect;
    e.apply_json(&format!(
        r#"{{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000002","kind":"selection.set","params":{{"source":"rect","rect":{{"x":{x},"y":{y},"w":{w},"h":{h}}},"feather":{feather}}}}}"#
    ))
    .expect("selection.set");
    let newl = "5cf74f35-5b58-510b-9c02-908f07df3db1";
    e.apply_json(&format!(
        r#"{{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000003","kind":"layer.add","params":{{"kind":"raster","index":0,"name":"L4","newLayerId":"{newl}"}}}}"#
    ))
    .expect("layer.add");
    e.apply_json(&format!(
        r#"{{"schema":"appname.command.v1","id":"5e1f7e57-0001-4000-8000-000000000004","kind":"fill","target":{{"layerId":"{newl}"}},"params":{{"content":"background","opacity":1.0}}}}"#
    ))
    .expect("fill");
    e
}

/// The replay-as-recovery gate exactly as the CLI runs it (main.rs): a
/// reopened package can only offer undo/redo if replaying its journal
/// from doc.new reproduces the persisted document state. Returns the
/// rebuilt engine rewound to the persisted cursor on success.
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
         document state (aud-replay F1)"
    );
    rebuilt
}

fn roundtrip_reproduces_state(tag: &str, feather: f64, rect: (i32, i32, i32, i32)) {
    let e = build(feather, rect);
    let before = e.state_hash_persistent();

    let bytes = save_package_bytes(&e).expect("save");
    let loaded = load_package_bytes_with(&assembly(), &bytes).expect("load");

    // The F1 invariant: persisted state survives the round-trip bit-for-bit.
    assert_eq!(
        before,
        loaded.engine.state_hash_persistent(),
        "[{tag}] state_hash_persistent diverged across save->reopen"
    );

    // The replay-as-recovery gate must pass — this is what F1 broke and
    // what disabled undo/redo on the reopened package.
    let mut rebuilt = recover_or_panic(&loaded.engine, tag);

    // Undo/redo must be reachable after reopen: the recovered engine
    // walks the history in both directions.
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

// A partial rect (does NOT cover the full 96x64 canvas) so the feather
// spreads a sub-canvas fringe — the isolated F1 trigger.
const PARTIAL: (i32, i32, i32, i32) = (70, 30, 13, 10);
const FULL: (i32, i32, i32, i32) = (0, 0, 96, 64);

#[test]
fn feather_zero_partial_fill_replays() {
    // Control: feather=0 always reproduced (hard-edged selection).
    roundtrip_reproduces_state("feather=0 partial", 0.0, PARTIAL);
}

#[test]
fn feathered_partial_fill_replays() {
    // The break: feather>0 on a partial rect. The audit's isolated case.
    roundtrip_reproduces_state("feather=4.34 partial", 4.34, PARTIAL);
}

#[test]
fn feathered_partial_fill_replays_second_feather() {
    // A second feather magnitude, still sub-canvas — breadth on the swept axis.
    roundtrip_reproduces_state("feather=2.0 partial", 2.0, PARTIAL);
}

#[test]
fn feathered_full_canvas_fill_replays() {
    // Full-canvas feather reaches every edge: no sub-canvas fringe, passed
    // before and must still pass.
    roundtrip_reproduces_state("feather=4.34 full", 4.34, FULL);
}
