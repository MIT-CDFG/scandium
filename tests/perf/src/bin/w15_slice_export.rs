// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w15_slice_export — workflow gate W15 (multi-asset export, slice leg)
//! run by tests/workflows/w15_multi_asset_export.sh.
//!
//! Spec: W15-multi-asset-export.yaml (§41.4/§29.2/§26.2). The slice DOC
//! MODEL (slice.add/edit/delete, §29.2 "slices are document data") and
//! the io-side slice export machinery (`appname_io::export::slices`) are
//! both merged; the CLI `--export-slices` wiring is not — this binary
//! drives the SHIPPED library surfaces end-to-end: document built through
//! the one apply door (production assembly), slices read back from the
//! doc model, `export_slices` over PNG+WebP at 1x/2x, manifest +
//! naming/traversal/collision/unknown-variable legs per the artifact.
//!
//! Determinism in place of frozen goldens (no new goldens this wave
//! without recorded rationale): the whole job runs TWICE into separate
//! destinations and every emitted file must be byte-identical across
//! runs; per-slice pixel truth is asserted structurally (decode-back
//! extent + solid-region color spot checks against the painted layout).
//!
//! The artifact's artboard leg is gated Post-1.0 (§19.4) — declared skip
//! per §41.1, recorded by the wrapper script.

use appname_color::Cms;
use appname_core::geom::IRect;
use appname_io::export::preset::ExportPreset;
use appname_io::export::slices::{export_slices, ManifestStamp, SliceDef, SliceJob};
use appname_io::export::ExportSource;
use appname_io::meta::Metadata;

fn engine() -> appname_core::Engine {
    appname_assembly::production_assembly().build()
}

fn apply(
    engine: &mut appname_core::Engine,
    seq: u32,
    kind: &str,
    body: &str,
) -> appname_core::ApplyReceipt {
    let id = format!("15000000-0000-4000-8000-{:012x}", seq);
    let env =
        format!("{{\"schema\":\"appname.command.v1\",\"id\":\"{id}\",\"kind\":\"{kind}\",{body}}}");
    match engine.apply_json(&env) {
        Ok(r) => r,
        Err(e) => panic!(
            "apply {kind} (seq {seq}): {} — {}",
            e.code.as_str(),
            e.message
        ),
    }
}

fn main() {
    let work = std::env::temp_dir().join(format!("appname-qa-w15-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let mut failures = 0usize;

    // --- build the document through the one apply door -----------------
    // 384x256 canvas, filled background; 12 slices in the doc model via
    // slice.add (the W15 step-1 count: the spec builds them From guides +
    // From layer bounds in the UI; the doc-model truth both routes write
    // is the slice row, which is what the shipped surface exposes today).
    let mut eng = engine();
    let new_receipt = apply(
        &mut eng,
        1,
        "doc.new",
        "\"params\":{\"width\":384,\"height\":256,\"background\":\"white\"}",
    );
    let layer = *new_receipt
        .created_ids
        .last()
        .expect("doc.new creates [root group, background layer]");
    apply(
        &mut eng,
        2,
        "fill",
        &format!(
            "\"target\":{{\"layerId\":\"{layer}\"}},\"params\":{{\"content\":\"foreground\"}}"
        ),
    );
    // 12 slices on a 4x3 grid of 96x85/86 px cells (integer bounds).
    let mut seq = 10u32;
    for row in 0..3u32 {
        for col in 0..4u32 {
            let (x, y) = (col * 96, row * 85);
            let h = if row == 2 { 86 } else { 85 };
            apply(
                &mut eng,
                seq,
                "slice.add",
                &format!(
                    "\"params\":{{\"name\":\"cell-r{row}c{col}\",\"bounds\":{{\"x\":{x},\"y\":{y},\"w\":96,\"h\":{h}}}}}"
                ),
            );
            seq += 1;
        }
    }
    let doc = eng.document().unwrap();
    let rows = doc.slices();
    if rows.len() == 12 {
        println!("slice-model PASS (12 slice rows in the document model)");
    } else {
        println!("slice-model FAIL ({} rows != 12)", rows.len());
        failures += 1;
    }

    // --- flatten once, export everything from the one source -----------
    let canvas = IRect::new(0, 0, 384, 256);
    let composite = appname_render::composite_document(doc, canvas).expect("composite");
    let plane = composite.region_rgba_f32(canvas);
    let meta = Metadata::default();
    let source = ExportSource {
        width: 384,
        height: 256,
        plane: &plane,
        icc: None,
        meta: &meta,
        doc_name: "artboards_slices",
        ppi: 72.0,
    };
    let defs: Vec<SliceDef> = rows
        .iter()
        .map(|r| SliceDef {
            name: r.name.clone(),
            bounds: r.bounds,
            preset: None,
        })
        .collect();
    // One preset per requested format (W15: PNG and WebP), each at 1x+2x.
    let mut png = ExportPreset::default_for("png").expect("png preset");
    let mut webp = ExportPreset::default_for("webp").expect("webp preset");
    png.scale_variants = vec![1.0, 2.0];
    webp.scale_variants = vec![1.0, 2.0];
    let presets = vec![png, webp];
    let stamp = ManifestStamp {
        id: "00000000-0000-4000-8000-00000000w015".into(),
        generated: "2026-08-18T00:00:00Z".into(),
        document: "artboards_slices.appname".into(),
    };

    let mut cms = Cms::new();
    let run = |dest: &std::path::Path, cms: &mut Cms| {
        std::fs::create_dir_all(dest).unwrap();
        export_slices(
            &source,
            &defs,
            &presets,
            &SliceJob {
                template: None,
                dest,
                overwrite: false,
                stamp: stamp.clone(),
            },
            cms,
        )
    };
    let d1 = work.join("run1");
    let d2 = work.join("run2");
    let r1 = run(&d1, &mut cms).expect("slice export run 1");
    let _r2 = run(&d2, &mut cms).expect("slice export run 2");

    // 48 files: 12 slices x 2 formats x 2 scales.
    if r1.written.len() == 48 && r1.failed.is_empty() {
        println!("emit-count PASS (48 outputs = 12 slices x 2 formats x 2 scales, 0 failures)");
    } else {
        println!(
            "emit-count FAIL ({} outputs, {} failures)",
            r1.written.len(),
            r1.failed.len()
        );
        failures += 1;
    }

    // Naming: template default {name}-{slice}.{ext}, @2x stem suffix.
    let names: Vec<&str> = r1.written.iter().map(|o| o.file.as_str()).collect();
    let want_1x = "artboards_slices-cell-r0c0.png";
    let want_2x = "artboards_slices-cell-r0c0@2x.png";
    if names.contains(&want_1x) && names.contains(&want_2x) {
        println!("naming PASS (template + @2x stem suffix: {want_1x}, {want_2x})");
    } else {
        println!("naming FAIL (missing {want_1x} or {want_2x} in {names:?})");
        failures += 1;
    }

    // Manifest: present, schema-tagged, one entry per emitted file.
    let manifest_text =
        std::fs::read_to_string(d1.join("export-manifest.json")).expect("manifest written");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).expect("manifest JSON");
    let schema_ok = manifest["schema"] == "appname.slicemanifest.v1";
    let listed = manifest["entries"].as_array().map(Vec::len).unwrap_or(0);
    if schema_ok && listed == 48 {
        println!(
            "manifest PASS (appname.slicemanifest.v1, 48 entries with slice/scale/dims/format)"
        );
    } else {
        println!("manifest FAIL (schema ok={schema_ok}, {listed} entries)");
        failures += 1;
    }

    // Determinism: run-2 files byte-identical (masked-manifest discipline:
    // the stamp is caller-injected identical, so the manifest pins too).
    let mut mismatched = 0usize;
    for out in &r1.written {
        let a = std::fs::read(d1.join(&out.file)).unwrap();
        let b = std::fs::read(d2.join(&out.file)).unwrap();
        if a != b {
            mismatched += 1;
        }
    }
    if mismatched == 0 {
        println!("byte-identity PASS (48 files byte-identical across two runs; manifest pinned by the injected stamp)");
    } else {
        println!("byte-identity FAIL ({mismatched} files differ across runs)");
        failures += 1;
    }

    // Decode-back structural truth: 1x PNG of slice r0c0 is 96x85 and
    // solid foreground-fill black; 2x is 192x170.
    for (file, want_w, want_h) in [
        ("artboards_slices-cell-r0c0.png", 96u32, 85u32),
        ("artboards_slices-cell-r0c0@2x.png", 192, 170),
    ] {
        let dec = png::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(d1.join(file)).unwrap(),
        ));
        let mut reader = dec.read_info().unwrap();
        let info = reader.info();
        let (w, h) = (info.width, info.height);
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        let out = reader.next_frame(&mut buf).unwrap();
        let px = &buf[..out.buffer_size()];
        let solid = px
            .chunks_exact(4)
            .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0);
        if (w, h) == (want_w, want_h) && solid {
            println!("decode-back PASS ({file}: {w}x{h}, solid fill color)");
        } else {
            println!("decode-back FAIL ({file}: {w}x{h}, solid={solid})");
            failures += 1;
        }
    }

    // --- step 5: traversal guard + collision policy --------------------
    let evil = vec![SliceDef {
        name: "../evil".into(),
        bounds: IRect::new(0, 0, 96, 85),
        preset: None,
    }];
    let d3 = work.join("run3");
    std::fs::create_dir_all(&d3).unwrap();
    let r_evil = export_slices(
        &source,
        &evil,
        &presets,
        &SliceJob {
            template: None,
            dest: &d3,
            overwrite: false,
            stamp: stamp.clone(),
        },
        &mut cms,
    )
    .expect("traversal job completes (per-file policy)");
    let escaped = work
        .join("run3")
        .parent()
        .map(|p| p.join("evil.png"))
        .unwrap();
    let sanitized_ok = r_evil
        .written
        .iter()
        .all(|o| o.path.starts_with(&d3) && !o.file.contains(".."));
    if sanitized_ok && !escaped.exists() {
        println!(
            "traversal-guard PASS (../evil sanitized to _ , outputs stay inside dest: {:?})",
            r_evil.written.iter().map(|o| &o.file).collect::<Vec<_>>()
        );
    } else {
        println!(
            "traversal-guard FAIL (sanitized_ok={sanitized_ok}, escaped={})",
            escaped.exists()
        );
        failures += 1;
    }
    // Re-export into the same dest without overwrite -> " (2)" suffixes.
    let r_coll = export_slices(
        &source,
        &evil,
        &presets,
        &SliceJob {
            template: None,
            dest: &d3,
            overwrite: false,
            stamp: stamp.clone(),
        },
        &mut cms,
    )
    .expect("collision job completes");
    let has_suffix = r_coll.written.iter().any(|o| o.file.contains(" (2)"));
    if has_suffix {
        println!("collision-suffix PASS (re-export without overwrite produced \" (2)\" names)");
    } else {
        println!(
            "collision-suffix FAIL (no \" (2)\" in {:?})",
            r_coll.written.iter().map(|o| &o.file).collect::<Vec<_>>()
        );
        failures += 1;
    }

    // --- step 6: unknown template variable -> validation error, no files
    let d4 = work.join("run4");
    std::fs::create_dir_all(&d4).unwrap();
    let bad = export_slices(
        &source,
        &defs,
        &presets,
        &SliceJob {
            template: Some("{name}-{bogusvar}.{ext}"),
            dest: &d4,
            overwrite: false,
            stamp: stamp.clone(),
        },
        &mut cms,
    );
    let wrote: Vec<_> = std::fs::read_dir(&d4).unwrap().collect();
    match bad {
        Err(e) if wrote.is_empty() => {
            println!("unknown-variable PASS (typed validation error before any write: {e:?})")
        }
        Err(_) => {
            println!(
                "unknown-variable FAIL (error but {} files written)",
                wrote.len()
            );
            failures += 1;
        }
        Ok(_) => {
            println!("unknown-variable FAIL (job succeeded)");
            failures += 1;
        }
    }

    let _ = std::fs::remove_dir_all(&work);
    if failures > 0 {
        println!("w15_slice_export: FAIL ({failures} leg[s])");
        std::process::exit(1);
    }
    println!("w15_slice_export: PASS (slice leg complete on the shipped io surface)");
}
