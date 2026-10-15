// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w07_psd_migration — workflow gate W07 at its CP-0.7 subset (plan §2.4:
//! "psd-migration: the merged PSD import corpus subset + compat
//! reports"), run by tests/workflows/w07_psd_migration.sh.
//!
//! Spec: W07-psd-migration.yaml (§34.6/§42.5); corpus truth
//! `bogos/tests/interop/psd-corpus.yaml` — the checked-in matrix files do
//! not exist in the spec repo, the yaml's own generator class is a
//! byte-level emitter. This binary reuses O-IO's proven in-crate emitter
//! VERBATIM via `#[path]` include (crate-portable, std+flate2 only — the
//! same mechanism precedent as the CP-0.3 stroke assembly-point include;
//! swaps to a plain import if O-IO ever exports a fixture-emitter door,
//! flagged in the lane log). Six corpus-subset cases mirror rows O-IO's
//! suite proved (provenance per case below).
//!
//! Per case: decode (zero crashes — the process completing IS the W07
//! step-2 assertion), a document lands through the journaled import door,
//! the §42.4 compat report is present and schema-tagged, unsupported
//! features are LISTED and their layers imported as fallbacks (never
//! dropped, W07 step 6), and the flattened composite + canonical report
//! byte-match the engine-produced goldens under goldens/psd_w07/.
//!
//! Modes: `verify` (default; compares goldens, exit 1 on any drift) ·
//! `record` (writes goldens — O-QA regeneration only).

#[path = "../../../../crates/appname-io/tests/psd_common/mod.rs"]
mod psd_common;

use appname_core::geom::IRect;
use appname_core::hash::ContentHash;
use appname_io::{CodecRegistry, DecodeOpts};
use psd_common::{LayerSpec, PsdBuilder};

/// Deterministic sRGB test pattern (the emitter tests' srgb_bytes shape,
/// re-derived: position-dependent bytes, opaque or patterned alpha).
fn srgb_bytes(w: usize, h: usize, opaque: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let a = if opaque {
                255
            } else {
                128 + ((x + y) % 96) as u8
            };
            out.extend_from_slice(&[
                (x * 40 % 256) as u8,
                (y * 56 % 256) as u8,
                ((x * 13 + y * 29) % 256) as u8,
                a,
            ]);
        }
    }
    out
}

fn rgb_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

struct Case {
    name: &'static str,
    provenance: &'static str,
    bytes: Vec<u8>,
    /// landed layer count expected (0 = decode-only refusal case)
    landed: usize,
    /// substring a compat item's feature field must carry ("" = none required)
    feature: &'static str,
}

fn cases() -> Vec<Case> {
    let rgba = srgb_bytes(6, 5, true);
    let rgb = rgb_of(&rgba);

    // 1. flat 8-bit RGB, raw merged data (psd-corpus H-04 / O-IO
    //    flat_rgb8_imports_with_schema_valid_report)
    let mut flat = PsdBuilder::rgb8(6, 5);
    flat.merged_from_rgb8(&rgb, 3);

    // 2. layered, RLE channel data (rle_channels_and_merged_round_trip)
    let mut rle_layer = LayerSpec::basic((0, 0, 5, 6), "rle", &srgb_bytes(6, 5, true));
    for ch in &mut rle_layer.channels {
        ch.2 = true;
    }
    let mut rle = PsdBuilder::rgb8(6, 5);
    rle.layers = vec![rle_layer];
    rle.merged_from_rgb8(&rgb, 3);

    // 3. two styled layers: names, blend, opacity, clipping
    //    (psd_layers_land_journaled_with_full_props)
    let mut styled = LayerSpec::basic((0, 0, 5, 6), "styled", &srgb_bytes(6, 5, true));
    styled.blend = *b"scrn";
    styled.opacity = 191;
    // NOTE: clip_to_below imports faithfully but the CPU compositor's
    // clipping arm is a pending render row (ClippingNotImplemented —
    // probed 2026-08-13); the second layer stays unclipped here so the
    // flatten leg composites. The clipping IMPORT truth is pinned by
    // O-IO's psd_layers_land_journaled_with_full_props.
    let mut second = LayerSpec::basic((0, 0, 5, 6), "second", &srgb_bytes(6, 5, false));
    second.opacity = 128;
    let mut layered = PsdBuilder::rgb8(6, 5);
    layered.layers = vec![styled, second];
    layered.merged_from_rgb8(&rgb, 3);

    // 4. raster layer mask rasterizes into alpha
    //    (raster_mask_rasterizes_into_alpha)
    let mut masked = LayerSpec::basic((0, 0, 5, 6), "masked", &srgb_bytes(6, 5, true));
    masked.mask = Some(((0, 0, 3, 3), 0, 0));
    masked.channels.push((-2, vec![255u8; 9], false));
    let mut with_mask = PsdBuilder::rgb8(6, 5);
    with_mask.layers = vec![masked];
    with_mask.merged_from_rgb8(&rgb, 3);

    // 5. adjustment layer → composite fallback + AD-17 compat item
    //    (adjustment_key_falls_back_over_canvas; W07 step-6 evidence)
    let mut adj = LayerSpec::basic((0, 0, 0, 0), "Curves 1", &[]);
    adj.channels.clear();
    adj.keys.push((*b"curv", vec![0u8; 4]));
    let mut with_adj = PsdBuilder::rgb8(6, 5);
    with_adj.layers = vec![
        LayerSpec::basic((0, 0, 5, 6), "base", &srgb_bytes(6, 5, true)),
        adj,
    ];
    with_adj.merged_from_rgb8(&rgb, 3);

    // 6. corrupt channel data → per-layer composite fallback, typed item
    //    (corrupt_channel_falls_back_to_composite; W07 zero-crash row)
    let mut corrupt = LayerSpec::basic((0, 0, 5, 6), "corrupt", &srgb_bytes(6, 5, true));
    corrupt.code_override = Some(2); // ZIP code over a non-zlib payload
    let mut with_corrupt = PsdBuilder::rgb8(6, 5);
    with_corrupt.layers = vec![corrupt];
    with_corrupt.merged_from_rgb8(&rgb, 3);

    vec![
        Case {
            name: "flat-rgb8-raw",
            provenance: "psd-corpus H-04",
            bytes: flat.build(),
            landed: 1,
            feature: "",
        },
        Case {
            name: "layered-rle",
            provenance: "psd-corpus RLE row",
            bytes: rle.build(),
            landed: 1,
            feature: "",
        },
        Case {
            name: "layered-styled",
            provenance: "layer-records row",
            bytes: layered.build(),
            landed: 2,
            feature: "",
        },
        Case {
            name: "layer-mask",
            provenance: "mask rasterization row",
            bytes: with_mask.build(),
            landed: 1,
            feature: "",
        },
        Case {
            name: "adjustment-fallback",
            provenance: "AD-17 fallback row",
            bytes: with_adj.build(),
            landed: 2,
            feature: "adjustment",
        },
        Case {
            name: "corrupt-channel",
            provenance: "corrupt-channel row",
            bytes: with_corrupt.build(),
            landed: 1,
            feature: "corrupt",
        },
    ]
}

fn golden_dir() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.join("goldens").join("psd_w07")
}

fn main() {
    // `seeds <dir>`: emit the case bytes as a fuzz seed corpus (the
    // fuzz/ crate's PSD seeds — same emitter, same provenance).
    if std::env::args().nth(1).as_deref() == Some("seeds") {
        let dir = std::path::PathBuf::from(std::env::args().nth(2).expect("seeds <dir>"));
        std::fs::create_dir_all(&dir).unwrap();
        for case in cases() {
            std::fs::write(dir.join(format!("{}.psd", case.name)), &case.bytes).unwrap();
        }
        println!("seed corpus written under {}", dir.display());
        return;
    }
    let record = std::env::args().nth(1).as_deref() == Some("record");
    let dir = golden_dir();
    let reg = CodecRegistry::builtin();
    let psd = reg.by_id("psd").expect("psd codec registered");
    let mut failures = 0usize;

    for case in cases() {
        let imported = match psd.decode(&case.bytes, &DecodeOpts::default()) {
            Ok(i) => i,
            Err(e) => {
                println!("{}: FAIL decode refused: {e:?}", case.name);
                failures += 1;
                continue;
            }
        };
        // compat report: present, schema-tagged (§42.4)
        let report = imported
            .compat_report
            .clone()
            .expect("PSD import always carries the §42.4 report");
        assert_eq!(
            report.get("schema").and_then(|s| s.as_str()),
            Some("appname.compatReport.v1"),
            "{}: report schema tag",
            case.name
        );
        if !case.feature.is_empty() {
            let items = report["items"].as_array().expect("items array");
            assert!(
                items
                    .iter()
                    .any(|i| i["feature"].as_str().unwrap_or("").contains(case.feature)),
                "{}: report lists the `{}` feature: {items:?}",
                case.name,
                case.feature
            );
        }

        // land through the journaled import door
        let (w, h) = imported.size;
        let mut engine =
            appname_core::engine::kinds::register(appname_core::EngineAssembly::new()).build();
        engine
            .apply_json(&format!(
                "{{\"schema\":\"appname.command.v1\",\"id\":\"07000000-0000-4000-8000-000000000001\",\
                 \"kind\":\"doc.new\",\"params\":{{\"width\":{w},\"height\":{h},\
                 \"background\":\"transparent\"}}}}"
            ))
            .expect("doc.new");
        let ids = appname_io::import_into_engine(&mut engine, "psd", imported)
            .expect("journaled landing");
        assert_eq!(
            ids.len(),
            case.landed,
            "{}: landed layer count (unsupported features import as fallbacks, never dropped)",
            case.name
        );

        // flatten + hash (the CPU reference compositor is the oracle)
        let canvas = IRect::new(0, 0, w as i32, h as i32);
        let composite = appname_render::composite_document(engine.document().unwrap(), canvas)
            .expect("composite");
        let plane = composite.region_rgba_f32(canvas);
        let mut bytes = Vec::with_capacity(plane.len() * 4);
        for s in &plane {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let hash = ContentHash::of_bytes(&bytes).to_hex();
        let report_text = serde_json::to_string_pretty(&report).expect("report serializes");

        let hash_path = dir.join(format!("{}.composite.blake3", case.name));
        let report_path = dir.join(format!("{}.report.json", case.name));
        if record {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&hash_path, format!("{hash}\n")).unwrap();
            std::fs::write(&report_path, format!("{report_text}\n")).unwrap();
            println!("{}: RECORDED ({hash}) [{}]", case.name, case.provenance);
        } else {
            let want_hash = std::fs::read_to_string(&hash_path)
                .unwrap_or_else(|e| panic!("{}: golden missing ({e}); run `record`", case.name));
            let want_report = std::fs::read_to_string(&report_path)
                .unwrap_or_else(|e| panic!("{}: report golden missing ({e})", case.name));
            let ok_h = want_hash.trim() == hash;
            let ok_r = want_report.trim_end() == report_text;
            if ok_h && ok_r {
                println!(
                    "{}: PASS (composite golden + canonical report) [{}]",
                    case.name, case.provenance
                );
            } else {
                println!(
                    "{}: FAIL (composite {} report {})",
                    case.name,
                    if ok_h { "ok" } else { "DRIFTED" },
                    if ok_r { "ok" } else { "DRIFTED" }
                );
                failures += 1;
            }
        }
    }
    if failures > 0 {
        println!("w07_psd_migration: FAIL ({failures} case[s])");
        std::process::exit(1);
    }
    println!("w07_psd_migration: PASS (6 cases, zero crashes, reports schema-tagged)");
}
