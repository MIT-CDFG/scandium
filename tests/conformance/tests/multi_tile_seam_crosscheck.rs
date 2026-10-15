// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Multi-tile SEAM cross-checks — the render reviewer's MINOR-1 closure
//! promoted into the standing suite (reopened-CP-0.7 goldens round).
//!
//! Spec: ES §12.1 (chain formula; per-tile evaluation with aprons at
//! the render door vs whole-plane evaluation at core's export door —
//! the tile SEAMS are exactly where the two implementations could
//! diverge), §16.3 (kernel reach / clamp policy), §17.1/§11.4
//! (adjustment segment pipeline over the accumulated backdrop),
//! §34.3 (render-vs-core bit-strict discipline; the
//! `parametric_composite.rs` cross-check contract, which is
//! single-tile — this suite is its multi-tile completion).
//!
//! Provenance: the §12.1 render review (INTEGRATION-log 2026-08-16,
//! "render review PASS") closed the single-tile gap empirically with
//! four multi-tile seam probes and handed the four cases to O-QA as
//! the spec. Encoded here: 320×320 documents (2×2 tiles of 256; seams
//! at x = 256 and y = 256), content patch straddling BOTH seams:
//!   1. gaussian chain across the seams (non-dyadic taps — apron reads
//!      must reproduce the whole-plane neighborhood exactly);
//!   2. dyadic single-tap shift `filter.custom` kernel across the
//!      seams (ROI ≠ identity: shifted-in content crosses tile edges);
//!   3. adjustment over a multi-tile backdrop with NON-ALIGNED internal
//!      rects and SEEDED-NOISE content (segment pipeline: the kept-rect
//!      extraction runs over rects that align with nothing);
//!   4. stacked chain + adjustment (both pipelines in one document).
//!
//! Each leg is BYTE-STRICT render-vs-core after the shared §6.4/§8.2
//! quantization, plus a premise pin that the composite really spans
//! 2×2 tiles and the patch really crosses both seams.

use appname_conformance::assembly_point::assembled_engine;
use appname_core::io::srgb::{srgb_decode, srgb_encode, unpremul};
use appname_core::{decode_png_to_buffer, export_png_bytes, Engine, IVec2, ProfileId};
use appname_render::composite_document_with;

fn apply(e: &mut Engine, raw: &str) {
    e.apply_bytes(raw.as_bytes())
        .unwrap_or_else(|err| panic!("{raw} -> {:?} {}", err.code, err.message));
}

fn env(n: u32, kind: &str, target_layer: Option<&str>, params: &str) -> String {
    let target = match target_layer {
        Some(l) => format!(r#","target":{{"layerId":"{l}"}}"#),
        None => String::new(),
    };
    format!(
        r#"{{"schema":"appname.command.v1","id":"5ea20000-0000-4000-8000-{n:012x}","kind":"{kind}"{target},"params":{params}}}"#
    )
}

const L: &str = "5ea20000-0000-4000-8000-00000000bbb1";
const S: &str = "5ea20000-0000-4000-8000-00000000bbb2";

/// 320×320 white doc + raster layer with a red patch straddling BOTH
/// tile seams: rect (200,200)–(312,312) crosses x = 256 and y = 256.
fn multi_tile_doc_with_patch() -> Engine {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            r#"{"width":320,"height":320,"background":"white"}"#,
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "layer.add",
            None,
            &format!(r#"{{"kind":"raster","newLayerId":"{L}","name":"patch"}}"#),
        ),
    );
    apply(
        &mut e,
        &env(
            3,
            "selection.set",
            None,
            r#"{"source":"rect","rect":{"x":200,"y":200,"w":112,"h":112},"antialias":false}"#,
        ),
    );
    apply(
        &mut e,
        &env(
            4,
            "fill",
            Some(L),
            r#"{"content":"color","color":{"model":"rgb","comps":[1.0,0.0,0.0,1.0]}}"#,
        ),
    );
    apply(&mut e, &env(5, "selection.none", None, r#"{}"#));
    e
}

/// Convert the patch layer to a smart object (proxy = the captured
/// patch raster; instance bounds cross both seams).
fn convert_patch(e: &mut Engine, n: u32) {
    apply(
        e,
        &env(
            n,
            "smartobject.convert",
            None,
            &format!(r#"{{"layerIds":["{L}"],"newLayerId":"{S}"}}"#),
        ),
    );
}

/// The §34.3 render-vs-core cross-check (parametric_composite.rs
/// contract, verbatim discipline): composite through the RENDER door
/// with the engine's own op registry, quantize with core's public
/// §6.4/§8.2 helpers exactly as `export_png_bytes` does, and compare
/// bit-strict against core's actual export decoded back through core's
/// own decoder. Also pins the multi-tile premise: 2×2 tiles.
fn crosscheck_vs_core_export(engine: &Engine) {
    let doc = engine.document().expect("initialized");
    let canvas = doc.geometry().canvas_rect();

    let png = export_png_bytes(engine).expect("core export serves");
    let (core_buf, w, h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    assert_eq!((w as i32, h as i32), (canvas.width(), canvas.height()));

    let comp =
        composite_document_with(doc, engine.assembly(), canvas).expect("render composite serves");
    assert_eq!(
        comp.tiles().count(),
        4,
        "premise: the 320×320 composite spans 2×2 tiles (seams at 256)"
    );

    let mut mismatches = 0usize;
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let p = IVec2::new(x, y);
            let s = unpremul(comp.pixel(p));
            let bytes = [
                (srgb_encode(s[0].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (srgb_encode(s[1].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (srgb_encode(s[2].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (s[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ];
            let a = bytes[3] as f32 / 255.0;
            let predicted = [
                srgb_decode(bytes[0] as f32 / 255.0) * a,
                srgb_decode(bytes[1] as f32 / 255.0) * a,
                srgb_decode(bytes[2] as f32 / 255.0) * a,
                a,
            ];
            let got = core_buf.read_pixel_f32(p);
            if got != predicted {
                mismatches += 1;
                assert!(
                    mismatches <= 1,
                    "render-vs-core mismatch at ({x},{y}) [and more]: core-export \
                     pixel {got:?} vs render-path prediction {predicted:?}"
                );
            }
        }
    }
    assert_eq!(mismatches, 0, "byte-strict render-vs-core");
}

/// Premise pin shared by the chain legs: the patch is live across both
/// seams at the render door (a red-dominant pixel on each side of each
/// seam inside the patch).
fn assert_patch_crosses_seams(engine: &Engine) {
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, engine.assembly(), canvas).unwrap();
    for p in [
        IVec2::new(250, 250), // NW tile, inside patch
        IVec2::new(262, 250), // NE tile, across the x seam
        IVec2::new(250, 262), // SW tile, across the y seam
        IVec2::new(262, 262), // SE tile, across both
    ] {
        let px = comp.pixel(p);
        assert!(
            px[0] > px[1] && px[0] > px[2],
            "patch content present across the seams at {p:?}: {px:?}"
        );
    }
}

/// Case 1 (reviewer probe 1): gaussian smart-filter chain evaluated
/// across both tile seams — non-dyadic taps, so the per-tile apron
/// reads are load-bearing; byte-strict against core's whole-plane
/// evaluation.
#[test]
fn seam_gaussian_chain_crosscheck() {
    let mut e = multi_tile_doc_with_patch();
    convert_patch(&mut e, 6);
    apply(
        &mut e,
        &env(
            7,
            "filter.smart_add",
            Some(S),
            r#"{"filterId":"filter.gaussian_blur","params":{"radius":4.0},"newOpId":"5ea20000-0000-4000-8000-00000000cc01"}"#,
        ),
    );
    assert_patch_crosses_seams(&e);
    crosscheck_vs_core_export(&e);
}

/// Case 2 (reviewer probe 2): dyadic single-tap (+2,0) shift
/// `filter.custom` kernel across the seams — ROI ≠ identity, shifted-in
/// content crosses tile edges exactly where a fabricated apron edge
/// would show.
#[test]
fn seam_dyadic_shift_kernel_crosscheck() {
    let mut e = multi_tile_doc_with_patch();
    convert_patch(&mut e, 6);
    apply(
        &mut e,
        &env(
            7,
            "filter.smart_add",
            Some(S),
            r#"{"filterId":"filter.custom","params":{"kernel":[[0,0,0,0,0],[0,0,0,0,0],[0,0,0,0,1],[0,0,0,0,0],[0,0,0,0,0]],"divisor":1,"offset":0.0},"newOpId":"5ea20000-0000-4000-8000-00000000cc02"}"#,
        ),
    );
    assert_patch_crosses_seams(&e);
    crosscheck_vs_core_export(&e);
}

/// Case 3 (reviewer probe 3): adjustment over a multi-tile backdrop —
/// the SEGMENT pipeline with NON-ALIGNED internal rects (content at
/// (37,41)–(283,299): aligned with nothing) and SEEDED-NOISE content
/// (filter.add_noise, fixed seed, baked destructively) so every pixel
/// is distinct and any segment-extraction misregistration shows.
#[test]
fn seam_adjustment_over_multitile_backdrop_crosscheck() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            r#"{"width":320,"height":320,"background":"white"}"#,
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "layer.add",
            None,
            &format!(r#"{{"kind":"raster","newLayerId":"{L}","name":"noise"}}"#),
        ),
    );
    apply(
        &mut e,
        &env(
            3,
            "selection.set",
            None,
            r#"{"source":"rect","rect":{"x":37,"y":41,"w":246,"h":258},"antialias":false}"#,
        ),
    );
    apply(
        &mut e,
        &env(
            4,
            "fill",
            Some(L),
            r#"{"content":"color","color":{"model":"rgb","comps":[0.5,0.5,0.5,1.0]}}"#,
        ),
    );
    apply(&mut e, &env(5, "selection.none", None, r#"{}"#));
    apply(
        &mut e,
        &env(
            6,
            "filter.apply",
            Some(L),
            r#"{"filterId":"filter.add_noise","params":{"amount":0.5,"distribution":"uniform","seed":424242}}"#,
        ),
    );
    apply(
        &mut e,
        &env(
            7,
            "adjustment.add",
            None,
            r#"{"opKind":"adjustment.invert","newLayerId":"5ea20000-0000-4000-8000-00000000ee01","newOpId":"5ea20000-0000-4000-8000-00000000ee02","params":{}}"#,
        ),
    );
    crosscheck_vs_core_export(&e);
}

/// Case 4 (reviewer probe 4): STACKED chain + adjustment — the smart
/// chain evaluates across the seams, then the adjustment's segment
/// pipeline consumes that accumulated backdrop, all in one document;
/// byte-strict end-to-end.
#[test]
fn seam_stacked_chain_and_adjustment_crosscheck() {
    let mut e = multi_tile_doc_with_patch();
    convert_patch(&mut e, 6);
    apply(
        &mut e,
        &env(
            7,
            "filter.smart_add",
            Some(S),
            r#"{"filterId":"filter.gaussian_blur","params":{"radius":4.0},"newOpId":"5ea20000-0000-4000-8000-00000000cc04"}"#,
        ),
    );
    apply(
        &mut e,
        &env(
            8,
            "adjustment.add",
            None,
            r#"{"opKind":"adjustment.brightness_contrast","newLayerId":"5ea20000-0000-4000-8000-00000000ee03","newOpId":"5ea20000-0000-4000-8000-00000000ee04","params":{"brightness":-0.25,"contrast":0.0}}"#,
        ),
    );
    crosscheck_vs_core_export(&e);
}
