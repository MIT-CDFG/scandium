// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Select workflow smoke — seed → refine → output modes, over what the
//! CP-1.0 consolidated O-TOOLS round actually made runnable (O-QA
//! ratification batch, W-additions item).
//!
//! Surface honesty (the reason this is a conformance test and not a
//! workflow script): the quick arm is WIRE-LIVE
//! (`selection.set{source:"quick"}` — smoked end-to-end through the
//! shipped CLI in `tests/workflows/quick_select_smoke.sh`), but the
//! GrabCut refine tier and the §13.4 Select&Mask pipeline have NO wire
//! kinds yet (recorded escalations: refine-tier addressing; the stage-9
//! layer-creation door). Their runnable surface today is the
//! engine-backed callable one (`appname_tools::select::{grabcut_refine,
//! select_mask}`), so this smoke drives seed → refine → Select&Mask
//! against the ASSEMBLED engine's real document state and pins
//! determinism across independent runs. When the wire kinds land, the
//! workflow-script translation follows (flagged in the log).
//!
//! Spec: ES §13.2/§13.3 (quick + §8.B refine tier), §13.4 (Select&Mask
//! stages, stage-8 forced-output rule, stage-9 output table); contracts
//! selection.set.yaml; D-48.

use appname_conformance::assembly_point::assembled_engine;
use appname_core::{Engine, IRect, IVec2};
use appname_tools::select::grabcut_refine::{grabcut_refine, GrabcutRefineParams};
use appname_tools::select::select_mask::{select_and_mask, OutputMode, SelectMaskParams};
use appname_tools::select::MaskPlane;

const W: i32 = 96;
const H: i32 = 64;

fn env(n: u32, kind: &str, layer: Option<&str>, params: &str) -> String {
    let target = layer
        .map(|l| format!(r#","target":{{"layerId":"{l}"}}"#))
        .unwrap_or_default();
    format!(
        r#"{{"schema":"appname.command.v1","id":"5e1ec000-0000-4000-8000-0001{n:08x}","kind":"{kind}"{target},"params":{params}}}"#
    )
}

/// Two-tone doc (left half red, right half white) with a LIVE quick
/// selection seeded on the red surface — the wire-live first stage.
fn seeded_engine() -> (Engine, appname_core::LayerId) {
    let mut e = assembled_engine();
    e.apply_json(&env(
        1,
        "doc.new",
        None,
        r#"{"width":96,"height":64,"background":"white"}"#,
    ))
    .unwrap();
    let layer = e.document().unwrap().z_order_walk()[0];
    let l = layer.to_string();
    e.apply_json(&env(
        2,
        "selection.set",
        None,
        r#"{"source":"rect","rect":{"x":0,"y":0,"w":48,"h":64},"antialias":false}"#,
    ))
    .unwrap();
    e.apply_json(&env(
        3,
        "fill",
        Some(&l),
        r#"{"content":"color","color":{"model":"rgb","comps":[1.0,0.0,0.0,1.0]}}"#,
    ))
    .unwrap();
    e.apply_json(&env(4, "selection.none", None, "{}")).unwrap();
    e.apply_json(&env(
        5,
        "selection.set",
        None,
        r#"{"source":"quick","seed":[10,32]}"#,
    ))
    .unwrap();
    (e, layer)
}

/// The engine's live selection surface copied into a tools-side plane
/// (the same read the workspace consumer performs).
fn selection_plane(e: &Engine) -> (MaskPlane, IRect) {
    let doc = e.document().unwrap();
    let bounds = doc.selection().bounds();
    let canvas = IRect::new(0, 0, W, H);
    let mut m = MaskPlane::new(canvas);
    for y in 0..H {
        for x in 0..W {
            let v = doc.selection().mask().read_pixel_f32(IVec2::new(x, y))[0];
            m.set(x, y, v);
        }
    }
    (m, bounds)
}

#[test]
fn seed_refine_select_mask_pipeline_smokes_and_is_deterministic() {
    let (e, layer) = seeded_engine();
    let canvas = IRect::new(0, 0, W, H);
    let (sel, sel_bounds) = selection_plane(&e);
    assert!(
        sel.get(10, 32) > 0.99,
        "quick seed selected the red surface"
    );
    assert_eq!(sel.get(70, 32), 0.0, "white surface stays out");
    assert!(!sel_bounds.is_empty(), "live selection bounds");

    // Stage 2: §8.B GrabCut refine tier over the target layer (the
    // engine-backed seam; no wire kind addresses it yet — recorded).
    let doc = e.document().unwrap();
    let buf = doc.layer(layer).unwrap().pixels().unwrap();
    let params = GrabcutRefineParams::default();
    let r1 = grabcut_refine(buf, true, canvas, &sel, sel_bounds, &params).unwrap();
    let r2 = grabcut_refine(buf, true, canvas, &sel, sel_bounds, &params).unwrap();
    assert_eq!(r1.rect, r2.rect, "refine region deterministic");
    assert_eq!(r1.v, r2.v, "refine matte bit-exact across runs");
    // Definite labels reproduce the thresholded input away from the band:
    // the seeded interior stays selected, the far white surface stays out.
    assert!(r1.get(10, 32) > 0.99, "refine keeps the definite interior");
    assert_eq!(r1.get(90, 5), 0.0, "refine leaves far white unselected");
    for v in &r1.v {
        assert!((0.0..=1.0).contains(v), "coverage invariant (§34.4 p13)");
    }

    // Stage 3: §13.4 Select&Mask over the refined matte — guidance
    // planes from the layer's straight working RGB (opaque doc:
    // straight == premultiplied).
    let mut rgb = [
        appname_algo::plane::Plane::new(W as usize, H as usize),
        appname_algo::plane::Plane::new(W as usize, H as usize),
        appname_algo::plane::Plane::new(W as usize, H as usize),
    ];
    for y in 0..H {
        for x in 0..W {
            let px = buf.read_pixel_f32(IVec2::new(x, y));
            for c in 0..3 {
                *rgb[c].at_mut(x as usize, y as usize) = px[c];
            }
        }
    }
    let sm_params = SelectMaskParams {
        radius: 4.0,
        decontaminate: true,
        ..Default::default()
    };
    let s1 = select_and_mask(&rgb, canvas, &r1, &sm_params).unwrap();
    let s2 = select_and_mask(&rgb, canvas, &r1, &sm_params).unwrap();
    assert_eq!(s1.matte.rect, s2.matte.rect);
    assert_eq!(s1.matte.v, s2.matte.v, "Select&Mask matte bit-exact");
    assert!(
        s1.matte.get(10, 32) > 0.9,
        "matte keeps the selected interior"
    );
    assert!(
        s1.matte.get(90, 5) < 0.1,
        "matte leaves the far white surface out"
    );
    for v in &s1.matte.v {
        assert!((0.0..=1.0).contains(v), "matte coverage in [0,1]");
    }

    // Stage-8/9 output-mode contract (the "output modes that are live"):
    // decontaminate ran, so its planes exist and the output table forces
    // the new-layer modes — Selection/Mask are NOT allowed with
    // decontaminate; the new-layer modes are (their command emission
    // waits on the layer-creation door, recorded escalation).
    let d1 = s1.decontaminated.as_ref().expect("stage 8 ran");
    let d2 = s2.decontaminated.as_ref().unwrap();
    for c in 0..3 {
        assert_eq!(d1[c].data, d2[c].data, "decontaminated plane {c} bit-exact");
    }
    assert!(!OutputMode::Selection.allowed_with_decontaminate());
    assert!(!OutputMode::Mask.allowed_with_decontaminate());
    assert!(OutputMode::NewLayer.allowed_with_decontaminate());
    assert!(OutputMode::NewLayerWithMask.allowed_with_decontaminate());
}

/// Without decontaminate every §13.4 stage-9 output mode is available,
/// and the selection-mode result is engine-installable TODAY: committing
/// the matte back through `selection.set{source:"rect"}`-class doors is
/// the workspace's job, but the SELECTION output needs no new door —
/// pin that the matte is a valid §13.1 coverage surface over the canvas.
#[test]
fn select_mask_without_decontaminate_keeps_all_output_modes() {
    let (e, layer) = seeded_engine();
    let canvas = IRect::new(0, 0, W, H);
    let (sel, _) = selection_plane(&e);
    let doc = e.document().unwrap();
    let buf = doc.layer(layer).unwrap().pixels().unwrap();
    let mut rgb = [
        appname_algo::plane::Plane::new(W as usize, H as usize),
        appname_algo::plane::Plane::new(W as usize, H as usize),
        appname_algo::plane::Plane::new(W as usize, H as usize),
    ];
    for y in 0..H {
        for x in 0..W {
            let px = buf.read_pixel_f32(IVec2::new(x, y));
            for c in 0..3 {
                *rgb[c].at_mut(x as usize, y as usize) = px[c];
            }
        }
    }
    let s = select_and_mask(&rgb, canvas, &sel, &SelectMaskParams::default()).unwrap();
    assert!(s.decontaminated.is_none(), "stage 8 skipped by default");
    // The stage-8 forcing rule constrains ONLY the decontaminate path —
    // with it off, the Selection/Mask modes remain legitimate outputs
    // (their `allowed_with_decontaminate` gate is the sole §13.4
    // restriction in the output table).
    assert!(!OutputMode::Selection.allowed_with_decontaminate());
    assert!(!OutputMode::Mask.allowed_with_decontaminate());
    for v in &s.matte.v {
        assert!((0.0..=1.0).contains(v), "§13.1 coverage surface");
    }
}
