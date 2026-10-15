// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w25_low_memory_spill — the W25 low-memory spill scenario at its
//! CP-0.7 subset, engine-level over PUBLIC surfaces (run by
//! tests/workflows/w25_low_memory_spill.sh, which adds the shipped-CLI
//! `--history-budget` leg and the peak-RSS reading).
//!
//! Spec: plan §2.4 workflow row (W25 low-memory spill — "the CP-0.5
//! spill machinery under the OpNode graph"); the spec artifact
//! W25-low-memory-spill.yaml (its full form — env_lowram8 cap, stall
//! histogram, disk-full injection — is the 1.0 gate; the pieces that
//! exist at 0.7 are exercised here and the rest is recorded pending in
//! the lane log).
//!
//! Two engines run the IDENTICAL deterministic command stream — pastes +
//! fills of incompressible noise planes, a smartobject.convert and
//! history.snapshot IN the stream so the spill machinery runs under the
//! CP-0.7 OpNode-era graph — one unbounded, one under an 8 MiB history
//! budget. Asserts:
//!   spills > 0 AND revives > 0 (the spill path genuinely exercised in
//!     BOTH directions — W25's distinguishing check over W03)
//!   resident history within budget
//!   LOSSLESS: budgeted-engine state_hash == unbounded-engine state_hash
//!     at the final state AND after a full undo/redo walk of the
//!     retained suffix (spilling changes nothing the document can see)
//!
//! Exit 0 = invariants hold.

use appname_conformance::assembly_point;

const BUDGET: usize = 8 * 1024 * 1024;
const NOISE: u32 = 1024;
const PLANES: u64 = 5;

fn noise_png(seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut data = vec![0u8; (NOISE * NOISE * 4) as usize];
    for chunk in data.chunks_exact_mut(8) {
        chunk.copy_from_slice(&next().to_le_bytes());
    }
    for px in data.chunks_exact_mut(4) {
        px[3] = 255;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, NOISE, NOISE);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&data).expect("png data");
    }
    out
}

/// The deterministic W25 stream, applied to one engine.
fn run_stream(engine: &mut appname_core::Engine) {
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"25000000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":4096,"height":4096,"background":"transparent"}}"#,
        )
        .expect("doc.new");
    for i in 0..PLANES {
        let asset = engine
            .add_raster_asset(noise_png(0x25_5EED + i))
            .expect("ingest noise");
        let layer = format!("25000000-0000-4000-8000-{:012}", 100 + i);
        let paste = format!(
            r#"{{"schema":"appname.command.v1","id":"25000000-0000-4000-8000-{:012}","kind":"edit.paste","params":{{"assetId":"{asset}","at":[{x},64],"newLayerId":"{layer}"}}}}"#,
            200 + i,
            x = 64 + (i as u32) * 600,
        );
        engine.apply_bytes(paste.as_bytes()).expect("paste");
        let fill = format!(
            r#"{{"schema":"appname.command.v1","id":"25000000-0000-4000-8000-{:012}","kind":"fill","target":{{"layerId":"{layer}"}},"params":{{"content":"foreground"}}}}"#,
            300 + i
        );
        engine.apply_bytes(fill.as_bytes()).expect("fill");
    }
    // OpNode-era graph members: a smart object + a named snapshot ride the
    // stream so spill/undo machinery runs under the CP-0.7 document graph.
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"25000000-0000-4000-8000-000000000401","kind":"smartobject.convert","params":{"layerIds":["25000000-0000-4000-8000-000000000100"],"newLayerId":"25000000-0000-4000-8000-000000000410"}}"#,
        )
        .expect("smartobject.convert");
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"25000000-0000-4000-8000-000000000402","kind":"history.snapshot","params":{"name":"w25 checkpoint"}}"#,
        )
        .expect("history.snapshot");
    // one more heavy step after the snapshot
    let fill = br#"{"schema":"appname.command.v1","id":"25000000-0000-4000-8000-000000000403","kind":"fill","target":{"layerId":"25000000-0000-4000-8000-000000000101"},"params":{"content":"background"}}"#;
    engine.apply_bytes(fill).expect("post-snapshot fill");
}

fn main() {
    // Unbounded reference run.
    let mut reference = assembly_point::assembled_engine();
    run_stream(&mut reference);
    let ref_hash = reference.state_hash();

    // Budgeted run: identical stream under an 8 MiB history budget.
    let mut budgeted = assembly_point::assembled_engine();
    budgeted.set_history_limits(BUDGET, usize::MAX);
    run_stream(&mut budgeted);
    let full_hash = budgeted.state_hash();

    assert_eq!(
        full_hash, ref_hash,
        "LOSSLESS: budgeted final state == unbounded final state"
    );

    let stats = budgeted.history_stats();
    println!(
        "stats: spills={} revives={} resident={}B disk={}B dropped={}",
        stats.spills, stats.revives, stats.resident_bytes, stats.spilled_bytes, stats.dropped_steps
    );
    assert!(stats.spills > 0, "budget must force spills: {stats:?}");
    assert!(
        stats.resident_bytes <= BUDGET,
        "resident history within budget: {stats:?}"
    );

    // Undo across spilled steps (forces revives), then redo back.
    let mut undoable = 0usize;
    while budgeted.undoable_in_process() {
        budgeted.undo().expect("undo across spill");
        undoable += 1;
    }
    assert!(undoable > 0, "a retained suffix stays undoable");
    let stats = budgeted.history_stats();
    assert!(
        stats.revives > 0,
        "undo across spilled steps must revive from disk: {stats:?}"
    );
    for _ in 0..undoable {
        budgeted.redo().expect("redo across spill");
    }
    assert_eq!(
        budgeted.state_hash(),
        ref_hash,
        "LOSSLESS after the full revive walk: state_hash equals the unbounded run"
    );
    println!(
        "walk: undoable={} of {} journaled; revives={}",
        undoable,
        budgeted.journal().len(),
        budgeted.history_stats().revives
    );
    println!("w25_low_memory_spill: PASS (in-process invariants)");
}
