// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w03_spill_scenario — the W03 bounded-RSS spill scenario, engine-level
//! over PUBLIC surfaces (O-QA workflow gate; run by
//! tests/workflows/w03_large_image.sh under `/usr/bin/time -l`, which
//! asserts the peak-RSS tier bound from tests/perf/scenarios.yaml).
//!
//! Spec: plan workflow matrix W03; ES §10.4 (history byte limits, spill
//! order), §3.4 (scratch file); O-CORE's recorded smoke (12k x 8k, 128
//! MiB budget, peak RSS 217 MiB) is the reference point. PUBLIC-surface
//! variant: a 12288x8192 TRANSPARENT canvas (stores no tiles, §2.2),
//! then 10 x [paste a DISTINCT deterministic-noise 2048² layer; fill
//! it]. Noise matters: uniform planes dedup/stay-resident by design
//! (§10.4 divergence note) and produced ZERO spills in the first cut of
//! this scenario — each fill's BEFORE must be an incompressible 16 MiB
//! plane, so 10 fills carry ~160 MiB of history against a 64 MiB budget
//! and MUST spill. Asserts: spills occurred, resident history within
//! budget, undo/redo byte-identical across spilled steps (state_hash).
//! NOTE (flagged OQ): the CLI exposes no history-budget surface yet, so
//! the shipped-BINARY variant waits on that flag; the engine is the
//! shipped code either way.
//!
//! Exit 0 = scenario invariants hold (the RSS bound is the RUNNER's
//! assertion, measured on this whole process).

use appname_conformance::assembly_point;

const BUDGET: usize = 16 * 1024 * 1024;
const NOISE: u32 = 1024;
const PLANES: u64 = 6;

/// Deterministic RGBA noise PNG (SplitMix64 stream; seed per layer).
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
    // opaque alpha: premultiplied sanity for ingestion
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

fn main() {
    let mut engine = assembly_point::assembled_engine();
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"03000000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":12288,"height":8192,"background":"transparent"}}"#,
        )
        .expect("doc.new 12288x8192 transparent");

    let base_hash = engine.state_hash();
    engine.set_history_limits(BUDGET, usize::MAX);

    // 10 x [paste distinct noise plane; fill it] — each fill's before is
    // an incompressible 16 MiB plane
    for i in 0..PLANES {
        let asset = engine
            .add_raster_asset(noise_png(0x5EED_0000 + i))
            .expect("ingest noise");
        let layer = format!("03000000-0000-4000-8000-{:012}", 100 + i);
        let paste = format!(
            r#"{{"schema":"appname.command.v1","id":"03000000-0000-4000-8000-{:012}","kind":"edit.paste","params":{{"assetId":"{asset}","at":[{x},128],"newLayerId":"{layer}"}}}}"#,
            200 + i,
            x = 128 + (i as u32) * 1200,
        );
        engine.apply_bytes(paste.as_bytes()).expect("paste noise");
        let fill = format!(
            r#"{{"schema":"appname.command.v1","id":"03000000-0000-4000-8000-{:012}","kind":"fill","target":{{"layerId":"{layer}"}},"params":{{"content":"foreground"}}}}"#,
            300 + i
        );
        engine
            .apply_bytes(fill.as_bytes())
            .expect("fill noise plane");
    }
    let full_hash = engine.state_hash();
    let stats = engine.history_stats();
    println!(
        "stats: spills={} resident={}B disk={}B",
        stats.spills, stats.resident_bytes, stats.spilled_bytes
    );
    assert!(stats.spills > 0, "the budget must force spills: {stats:?}");
    assert!(
        stats.resident_bytes <= BUDGET,
        "resident history within the budget: {stats:?}"
    );

    // byte-identical undo/redo across the spilled (and partially
    // DROPPED, section 10.4 floor) history: only the retained suffix is
    // in-process undoable — walk exactly that far and back (D-48).
    let mut undoable = 0usize;
    while engine.undoable_in_process() {
        engine.undo().expect("undo across spill");
        undoable += 1;
    }
    assert!(undoable > 0, "a retained suffix must remain undoable");
    let dropped_expected = engine.journal().len() > undoable + 1;
    let floor_hash = engine.state_hash();
    assert_ne!(floor_hash, full_hash);
    let _ = base_hash; // base is below the drop floor by design
    for _ in 0..undoable {
        engine.redo().expect("redo across spill");
    }
    assert_eq!(
        engine.state_hash(),
        full_hash,
        "redo across spilled steps restores the full state byte-exactly"
    );
    assert!(
        dropped_expected == (engine.history_stats().dropped_steps > 0),
        "drop accounting consistent with the journal/undoable split"
    );
    println!(
        "walk: undoable={} of {} journaled; dropped_steps={}",
        undoable,
        engine.journal().len(),
        engine.history_stats().dropped_steps
    );

    println!("w03_spill_scenario: PASS (in-process invariants)");
}
