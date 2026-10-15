// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Engine/compositor benches — DETERMINISTIC INPUTS ONLY (the
//! deps/criterion.yaml comparability requirement): fixed envelopes, the
//! generator-pinned samples fixture, fixed dirty rects. Four benches over
//! public surfaces per the slice-B mandate:
//!
//!   doc_new_apply       Engine::apply_bytes(doc.new) on a fresh engine
//!   stroke_apply        brush.stroke through the ASSEMBLED engine
//!                       (per-dab-ish: the 5-sample fixed stroke)
//!   recomposite_dirty   dirty-rect recomposition over the stroked doc
//!   export_png          export path on the stroked doc
//!
//! Spec: plan §3.14; baselines land under .claude/perf/baselines/ and
//! the over-15% wave-gate compares mean point estimates
//! (.claude/perf/compare_criterion.py).

use appname_conformance::assembly_point;
use appname_conformance::stroke_goldens::fixed_samples_basic;
use appname_core::{export_png_bytes, Engine, IRect};
use appname_render::{composite_document, recomposite_dirty, DirtyTracker};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::hint::black_box;

const DOC_NEW: &str = r#"{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":256,"height":192,"background":"white"}}"#;

/// 2048×2048 = 4.19 MP canvas for the retouch heal-commit scenario
/// (`perf_heal_4mp`, heal.stroke §45.1: "≤ 400 ms Poisson commit ≤ 4 MP
/// on ci-linux-x86"). The heal footprint below (sizePx 2500) blankets
/// the canvas, so the measured work is a genuine ~4 MP Poisson membrane
/// solve — not a token dab.
const DOC_NEW_4MP: &str = r#"{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-00000000000b","kind":"doc.new","params":{"width":2048,"height":2048,"background":"white"}}"#;

/// Assembled engine with the fixed doc + samples asset; returns the
/// ready-to-apply stroke envelope too (content-derived ids — identical
/// bytes every run).
fn stroked_setup() -> (Engine, String) {
    let mut engine = assembly_point::assembled_engine();
    let samples_id = engine.add_samples_asset(fixed_samples_basic());
    let receipt = engine
        .apply_bytes(DOC_NEW.as_bytes())
        .expect("bench doc.new");
    let layer = receipt.created_ids.last().copied().expect("bg layer");
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000002","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.2,0,0]}},"seed":11,"opacity":1}},"samples":"{samples_id}"}}"#,
        preset = assembly_point::preset_hard_round_16_id(),
    );
    (engine, env)
}

fn bench_doc_new(c: &mut Criterion) {
    c.bench_function("doc_new_apply", |b| {
        b.iter_batched(
            Engine::new,
            |mut engine| {
                engine
                    .apply_bytes(black_box(DOC_NEW.as_bytes()))
                    .expect("doc.new");
                engine
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_stroke(c: &mut Criterion) {
    c.bench_function("stroke_apply", |b| {
        b.iter_batched(
            stroked_setup,
            |(mut engine, env)| {
                engine
                    .apply_bytes(black_box(env.as_bytes()))
                    .expect("stroke");
                engine
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_recomposite(c: &mut Criterion) {
    let (mut engine, env) = stroked_setup();
    engine.apply_bytes(env.as_bytes()).expect("stroke");
    let doc_rect = IRect::new(0, 0, 256, 192);
    // fixed dirty rect across two tile columns (the stroke's band)
    let dirty_rect = IRect::new(8, 8, 120, 96);
    c.bench_function("recomposite_dirty", |b| {
        b.iter_batched(
            || {
                let doc = engine.document().expect("doc");
                let comp = composite_document(doc, doc_rect).expect("composite");
                let mut tracker = DirtyTracker::new();
                tracker.mark(doc.root(), dirty_rect);
                (comp, tracker.take())
            },
            |(mut comp, dirty)| {
                let doc = engine.document().expect("doc");
                let stats = recomposite_dirty(&mut comp, doc, &dirty).expect("recomposite");
                black_box(stats.tiles_recomposited);
                comp
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_export(c: &mut Criterion) {
    let (mut engine, env) = stroked_setup();
    engine.apply_bytes(env.as_bytes()).expect("stroke");
    c.bench_function("export_png", |b| {
        b.iter(|| black_box(export_png_bytes(&engine).expect("export")))
    });
}

/// CP-0.5 wave 2: fill + paste benches (assembled engine, fixed inputs).
/// A spill-under-budget bench joins when O-CORE's wave-2 spill mechanism
/// lands (noted; the wave-gate list grows then, with its own baseline).
fn fill_setup() -> (Engine, String) {
    let mut engine = assembly_point::assembled_engine();
    let receipt = engine
        .apply_bytes(DOC_NEW.as_bytes())
        .expect("bench doc.new");
    let layer = receipt.created_ids.last().copied().expect("bg layer");
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000003","kind":"fill","target":{{"layerId":"{layer}"}},"params":{{"content":"foreground"}}}}"#
    );
    (engine, env)
}

fn paste_setup() -> (Engine, String) {
    let mut engine = assembly_point::assembled_engine();
    engine
        .apply_bytes(DOC_NEW.as_bytes())
        .expect("bench doc.new");
    let asset = engine
        .add_raster_asset(appname_conformance::edit_goldens::paste_source_png())
        .expect("ingest");
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000004","kind":"edit.paste","params":{{"assetId":"{asset}","at":[16,12],"newLayerId":"be9c0000-0000-4000-8000-000000000005"}}}}"#
    );
    (engine, env)
}

fn bench_fill(c: &mut Criterion) {
    c.bench_function("fill_apply", |b| {
        b.iter_batched(
            fill_setup,
            |(mut engine, env)| {
                engine.apply_bytes(black_box(env.as_bytes())).expect("fill");
                engine
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_paste(c: &mut Criterion) {
    c.bench_function("paste_apply", |b| {
        b.iter_batched(
            paste_setup,
            |(mut engine, env)| {
                engine
                    .apply_bytes(black_box(env.as_bytes()))
                    .expect("paste");
                engine
            },
            BatchSize::SmallInput,
        )
    });
}

/// CP-0.5 wave 3: convert + spill paths (new baselines only; the
/// original trees stay untouched per the refresh policy).
fn saturated_setup() -> Engine {
    let mut engine = assembly_point::assembled_engine();
    let receipt = engine
        .apply_bytes(DOC_NEW.as_bytes())
        .expect("bench doc.new");
    let layer = receipt.created_ids.last().copied().expect("bg layer");
    let samples =
        engine.add_samples_asset(appname_conformance::stroke_goldens::fixed_samples_basic());
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000006","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.2,0,0]}},"seed":11,"opacity":1}},"samples":"{samples}"}}"#,
        preset = assembly_point::preset_hard_round_16_id(),
    );
    engine.apply_bytes(env.as_bytes()).expect("stroke");
    engine
}

fn bench_convert(c: &mut Criterion) {
    let convert = r#"{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000007","kind":"doc.convert_profile","params":{"profile":"ee12374b-5f25-51fd-a1c8-f45790a61d92"}}"#;
    let mut g = c.benchmark_group("color");
    g.sample_size(20);
    g.bench_function("convert_apply", |b| {
        b.iter_batched(
            saturated_setup,
            |mut engine| {
                engine
                    .apply_bytes(black_box(convert.as_bytes()))
                    .expect("convert");
                engine
            },
            BatchSize::SmallInput,
        )
    });
    g.finish();
}

fn noise_png_256(seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut data = vec![0u8; 256 * 256 * 4];
    for chunk in data.chunks_exact_mut(8) {
        chunk.copy_from_slice(&next().to_le_bytes());
    }
    for px in data.chunks_exact_mut(4) {
        px[3] = 255;
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 256, 256);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&data).expect("png data");
    }
    out
}

fn bench_spill(c: &mut Criterion) {
    let mut g = c.benchmark_group("history");
    g.sample_size(20);
    g.bench_function("fill_apply_with_spill", |b| {
        b.iter_batched(
            || {
                // noise plane under a tiny budget: the measured apply
                // carries §10.4 enforcement (compress + spill)
                let mut engine = assembly_point::assembled_engine();
                engine
                    .apply_bytes(DOC_NEW.as_bytes())
                    .expect("bench doc.new");
                let asset = engine
                    .add_raster_asset(noise_png_256(0xbe9c_5eed))
                    .expect("ingest");
                let paste = format!(
                    r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000008","kind":"edit.paste","params":{{"assetId":"{asset}","at":[0,0],"newLayerId":"be9c0000-0000-4000-8000-000000000009"}}}}"#
                );
                engine.apply_bytes(paste.as_bytes()).expect("paste");
                engine.set_history_limits(64 * 1024, usize::MAX);
                let fill = r#"{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-00000000000a","kind":"fill","target":{"layerId":"be9c0000-0000-4000-8000-000000000009"},"params":{"content":"foreground"}}"#;
                (engine, fill)
            },
            |(mut engine, fill)| {
                engine
                    .apply_bytes(black_box(fill.as_bytes()))
                    .expect("fill under budget");
                engine
            },
            BatchSize::SmallInput,
        )
    });
    g.finish();
}

/// CP-1.0 retouch row (O-TOOLS flagged `perf_heal_4mp` to O-QA): the
/// §45.1 Poisson heal COMMIT over a ~4 MP footprint, deterministic inputs
/// (the generator-pinned samples fixture + a large sizePx that blankets
/// the 2048² canvas). Healing mode with an explicit self-source + offset;
/// off-canvas source portions take `g = dest` (§45.2 edge rule), so the
/// full footprint solves. Own group + small sample_size (the iteration is
/// a real megapixel solve). Baseline lands under
/// `.claude/perf/baselines/<host-tag>/`; the ≤ 400 ms budget is the
/// ci-linux-x86 reference (scenarios.yaml) — this Mac records its tier.
fn heal_4mp_setup() -> (Engine, String) {
    let mut engine = assembly_point::assembled_engine();
    engine
        .apply_bytes(DOC_NEW_4MP.as_bytes())
        .expect("bench doc.new 4mp");
    let layer = engine
        .document()
        .expect("doc")
        .z_order_walk()
        .first()
        .copied()
        .expect("bg layer")
        .to_string();
    let samples = engine.add_samples_asset(fixed_samples_basic());
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-00000000000c","kind":"heal.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","mode":"healing","source":{{"layerId":"{layer}"}},"offset":[64,-32],"sizePx":2500}},"samples":"{samples}"}}"#,
        preset = assembly_point::preset_hard_round_16_id(),
    );
    (engine, env)
}

fn bench_heal_4mp(c: &mut Criterion) {
    let mut g = c.benchmark_group("retouch");
    g.sample_size(10);
    g.bench_function("heal_poisson_4mp", |b| {
        b.iter_batched(
            heal_4mp_setup,
            |(mut engine, env)| {
                engine
                    .apply_bytes(black_box(env.as_bytes()))
                    .expect("heal 4mp commit");
                engine
            },
            BatchSize::SmallInput,
        )
    });
    g.finish();
}

/// Q-1 bench-delta guard (the fifth door): filter.apply — gaussian blur
/// radius 4 through a FEATHERED selection on the full-canvas background,
/// the exact q-scenario cell shape whose bounds maintenance the Q-1 class
/// fix moved from the tile-granular shrink to the loader's pixel-exact
/// recompute. Pins the cost of that recompute against the filter's own
/// kernel work.
fn filter_setup() -> (Engine, String) {
    let mut engine = assembly_point::assembled_engine();
    let receipt = engine
        .apply_bytes(DOC_NEW.as_bytes())
        .expect("bench doc.new");
    let layer = receipt.created_ids.last().copied().expect("bg layer");
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000007","kind":"selection.set","params":{"source":"rect","rect":{"x":20,"y":15,"w":100,"h":60}}}"#,
        )
        .expect("selection.set");
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000008","kind":"selection.modify","params":{"feather":8.0}}"#,
        )
        .expect("selection.modify feather");
    let env = format!(
        r#"{{"schema":"appname.command.v1","id":"be9c0000-0000-4000-8000-000000000009","kind":"filter.apply","target":{{"layerId":"{layer}"}},"params":{{"filterId":"filter.gaussian_blur","params":{{"radius":4.0}}}}}}"#
    );
    (engine, env)
}

fn bench_filter(c: &mut Criterion) {
    c.bench_function("filter_apply_feathered", |b| {
        b.iter_batched(
            filter_setup,
            |(mut engine, env)| {
                engine
                    .apply_bytes(black_box(env.as_bytes()))
                    .expect("filter");
                engine
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    benches,
    bench_doc_new,
    bench_stroke,
    bench_recomposite,
    bench_export,
    bench_fill,
    bench_paste,
    bench_convert,
    bench_spill,
    bench_heal_4mp,
    bench_filter
);
criterion_main!(benches);
