// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Blend-golden verification — ACTIVE regression gate (CP-0.3 slice A).
//!
//! Spec: ES §4.4/§4.5 (formulas + the spec-mandated worked example),
//! §34.3 (golden suite; CPU path = permanent parity oracle); plan §0.6
//! rule 6. Three independent nets on every `cargo test`:
//!
//! 1. BYTES — committed golden windows == fresh public-API production;
//! 2. ORACLES — the composited values match O-QA's hand-computed
//!    expectations derived from the formula TEXT (independent of
//!    appname-render's code and tests);
//! 3. CATALOG — since the CP-0.5 render companion, ALL 27 modes
//!    composite (the CP-0.3 19-refusal pin retired IN THE SAME TRAIN
//!    BATCH as the render merge, per the standing companion gate);
//!    Dissolve additionally satisfies its Appendix-A membership property
//!    (every pixel is either the hardened source or the untouched
//!    backdrop).

use appname_conformance::blend_goldens::{
    build_tile, cases, doc_walk_window, produce_window, TileDef, PROBES, PROBES_RGB, WIN,
};
use appname_conformance::paths;
use appname_core::BlendMode;
use appname_render::{blend_implemented, composite_tile, TileRgba, BLEND_SET_V1};

fn blend_dir() -> std::path::PathBuf {
    paths::repo_root().join("goldens").join("blend")
}

fn window_pixel(bytes: &[u8], wx: usize, wy: usize) -> [f32; 4] {
    let off = (wy * WIN + wx) * 16;
    let mut out = [0.0f32; 4];
    for (i, o) in out.iter_mut().enumerate() {
        let b = &bytes[off + i * 4..off + i * 4 + 4];
        *o = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    }
    out
}

#[test]
fn blend_goldens_match_fresh_production() {
    for case in cases() {
        let fresh = produce_window(&case);
        let committed = std::fs::read(blend_dir().join(format!("{}.bin", case.name)))
            .unwrap_or_else(|e| panic!("{}: golden missing: {e}", case.name));
        assert_eq!(
            fresh, committed,
            "{}: committed blend golden != fresh public-API production — \
             route the change through O-QA review",
            case.name
        );
    }
    let fresh = doc_walk_window();
    let committed = std::fs::read(blend_dir().join("doc-walk-import.bin")).unwrap();
    assert_eq!(fresh, committed, "doc-walk-import golden drifted");
}

#[test]
fn hand_computed_oracles_hold() {
    // Tolerance: the oracle is an f64 evaluation of the formula text; the
    // pipeline is f32. 2e-5 absolute covers the widest case (|Co| < 8).
    const TOL: f32 = 2e-5;
    let mut checked = 0;
    for case in cases() {
        let bytes = produce_window(&case);
        for (name, wx, wy, co, ao) in PROBES {
            if *name != case.name {
                continue;
            }
            let got = window_pixel(&bytes, *wx, *wy);
            for (ch, g) in got.iter().enumerate().take(3) {
                assert!(
                    (g - co).abs() <= TOL,
                    "{name} ({wx},{wy}) ch{ch}: oracle {co} vs composited \
                     {g} — formula drift between spec and compositor"
                );
            }
            assert!(
                (got[3] - ao).abs() <= TOL,
                "{name} ({wx},{wy}) alpha: oracle {ao} vs {}",
                got[3]
            );
            for v in got {
                assert!(v.is_finite(), "{name} ({wx},{wy}): non-finite scalar {v}");
            }
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        PROBES.len(),
        "every recorded oracle probe must resolve to a live case"
    );
    // alpha-zero: beyond its probes, the WHOLE window is exact zeros and
    // finite (the un-premultiply guard, §4.5)
    let zero_case = cases()
        .into_iter()
        .find(|c| c.name == "alpha-zero")
        .unwrap();
    for b in produce_window(&zero_case) {
        assert_eq!(b, 0, "alpha-zero window must be all zero bytes");
    }
}

/// The flipped pin (CP-0.5 render companion): every one of the 27 modes
/// composites through the public door and the composite MOVES the
/// destination for a non-trivial input; the refusal set is EMPTY.
#[test]
fn all_27_modes_composite() {
    let mut composited = 0;
    for mode in BlendMode::ALL {
        assert!(
            blend_implemented(mode),
            "{mode:?}: catalog must be complete since the render merge"
        );
        let mut dst = TileRgba::splat([0.225, 0.18, 0.405, 0.9]);
        let src = TileRgba::splat([0.4, 0.16, 0.2, 0.8]);
        let before = dst.clone();
        composite_tile(&mut dst, &src, mode, 1.0)
            .unwrap_or_else(|e| panic!("{mode:?}: refused: {e:?}"));
        assert!(
            dst != before,
            "{mode:?}: compositing a non-trivial source must move dst"
        );
        composited += 1;
    }
    assert_eq!(composited, 27, "the full Appendix-A catalog");
    assert_eq!(BLEND_SET_V1.len(), 8, "the historical v1 set stays pinned");
}

/// Per-channel oracles for the CP-0.5 chromatic cases.
#[test]
fn rgb_oracles_hold() {
    const TOL: f32 = 2e-5;
    let mut checked = 0;
    for case in cases() {
        let mut bytes = None;
        for (name, wx, wy, rgb, ao) in PROBES_RGB {
            if *name != case.name {
                continue;
            }
            let b = bytes.get_or_insert_with(|| produce_window(&case));
            let got = window_pixel(b, *wx, *wy);
            for ch in 0..3 {
                assert!(
                    (got[ch] - rgb[ch]).abs() <= TOL,
                    "{name} ({wx},{wy}) ch{ch}: oracle {} vs {}",
                    rgb[ch],
                    got[ch]
                );
            }
            assert!((got[3] - ao).abs() <= TOL, "{name}: alpha");
            checked += 1;
        }
    }
    assert_eq!(checked, PROBES_RGB.len(), "every RGB probe resolves");
}

/// Dissolve's Appendix-A membership property, independent of the
/// blue-noise pattern: every output pixel is EITHER the hardened source
/// (un-premultiplied Cs at alpha 1) OR the untouched backdrop pixel.
#[test]
fn dissolve_membership_property() {
    let case = cases()
        .into_iter()
        .find(|c| c.name == "mode2-dissolve")
        .unwrap();
    let bytes = produce_window(&case);
    let src = build_tile(TileDef::SrcRamp);
    let bd = build_tile(TileDef::BackdropRamp);
    let mut hardened = 0u32;
    for wy in 0..WIN {
        for wx in 0..WIN {
            let got = window_pixel(&bytes, wx, wy);
            let s = src.get(wx * 4, wy * 4);
            let b = bd.get(wx * 4, wy * 4);
            let sa = s[3];
            let source_pixel = if sa > 0.0 {
                [s[0] / sa, s[1] / sa, s[2] / sa, 1.0]
            } else {
                b // aS hardens to 0 at sa == 0: backdrop untouched
            };
            let eq = |a: [f32; 4], c: [f32; 4]| a.iter().zip(c).all(|(x, y)| (x - y).abs() <= 1e-6);
            assert!(
                eq(got, source_pixel) || eq(got, b),
                "dissolve ({wx},{wy}): pixel is neither hardened source                  nor backdrop: {got:?}"
            );
            if eq(got, source_pixel) && !eq(got, b) {
                hardened += 1;
            }
        }
    }
    assert!(
        hardened > 0,
        "the dissolve pattern must select SOME source pixels"
    );
}

#[test]
fn blend_manifest_passes_shasum() {
    let out = appname_conformance::shasum256()
        .args(["-c", "MANIFEST.sha256"])
        .current_dir(blend_dir())
        .output()
        .expect("shasum available");
    assert!(
        out.status.success(),
        "goldens/blend/MANIFEST.sha256 check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// CP-0.5 render companion: the DAG door (`Renderer::render`) is
/// ROI-scope bit-equal to the oracle (`composite_document`) on a real
/// mixed-mode document — QA's conformance angle on the §34.3 parity
/// contract, over the goldens' own stroked/pasted doc shape.
#[test]
fn dag_matches_oracle_on_roi_bit_exact() {
    use appname_conformance::assembly_point;
    use appname_core::IRect;
    use appname_render::{composite_document, Renderer};

    let mut engine = assembly_point::assembled_engine();
    engine
        .apply_bytes(
            br#"{"schema":"appname.command.v1","id":"da600000-0000-4000-8000-000000000001","kind":"doc.new","params":{"width":320,"height":200,"background":"white"}}"#,
        )
        .expect("doc.new");
    let receipt_layer = {
        let samples =
            engine.add_samples_asset(appname_conformance::stroke_goldens::fixed_samples_basic());
        let layer = engine
            .document()
            .unwrap()
            .z_order_walk()
            .into_iter()
            .find(|id| {
                engine
                    .document()
                    .unwrap()
                    .layer(*id)
                    .unwrap()
                    .pixels()
                    .is_some()
            })
            .unwrap();
        let env = format!(
            r#"{{"schema":"appname.command.v1","id":"da600000-0000-4000-8000-000000000002","kind":"brush.stroke","target":{{"layerId":"{layer}"}},"params":{{"brushPreset":"{preset}","color":{{"model":"rgb","comps":[1,0.2,0,0]}},"seed":11,"opacity":1}},"samples":"{samples}"}}"#,
            preset = assembly_point::preset_hard_round_16_id(),
        );
        engine.apply_bytes(env.as_bytes()).expect("stroke");
        layer
    };
    let _ = receipt_layer;

    let doc = engine.document().unwrap();
    // three fixed ROIs: tile-aligned, straddling, and edge-clipped
    for roi in [
        IRect::new(0, 0, 256, 200),
        IRect::new(37, 19, 291, 173),
        IRect::new(200, 100, 320, 200),
    ] {
        let oracle = composite_document(doc, roi).expect("oracle");
        let mut renderer = Renderer::new();
        let dag = renderer.render(doc, roi).expect("dag");
        let a = oracle.region_rgba_f32(roi);
        let b = dag.region_rgba_f32(roi);
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert!(
                x.to_bits() == y.to_bits(),
                "DAG != oracle at scalar {i} in {roi:?}: {x} vs {y}"
            );
        }
    }
}

/// The STORED Dissolve coverage mask (render-companion standing item,
/// closed CP-0.7 wave 2): committed bytes == fresh production, and the
/// pattern is a real screen door (both classes populated).
#[test]
fn dissolve_coverage_mask_matches_committed() {
    use appname_conformance::blend_goldens::dissolve_coverage_mask;
    let fresh = dissolve_coverage_mask();
    let committed = std::fs::read(blend_dir().join("dissolve-coverage.mask.bin"))
        .expect("dissolve-coverage.mask.bin committed");
    assert_eq!(committed.len(), 256 * 256);
    assert_eq!(
        fresh, committed,
        "dissolve coverage mask drifted from the committed golden \
         (goldens change only via O-QA review)"
    );
    let selected: usize = fresh.iter().map(|&m| usize::from(m)).sum();
    assert!(
        selected > 0 && selected < 256 * 256,
        "screen-door has both classes: {selected}/65536"
    );
}
