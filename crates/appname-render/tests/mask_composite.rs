// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §11.2 Sub-slice A mask-compositing integration suite (O-RENDER
//! v1.0.1) — the aud-parity **T13**/**T05** regression, wired to the
//! serving behaviour that REPLACES aud-replay F2's uniform refusal.
//!
//! Spec: ES §11.2 (effective coverage — density → invert, the O-INT-ruled
//! order), §4.4/§4.6 (compositing), §34.3 (one compositing truth — the
//! render door and the CPU reference export must AGREE bit-strict on the
//! shared serve set, the cross-check below), QA-F8 (serve or refuse typed,
//! never silently drop). Documents are built through PUBLIC command doors
//! (doc.new / edit.paste over ingested solid rasters / mask.add /
//! mask.edit / mask.set_props) — the same doors the harness replays.
//!
//! **T13** (`t13-composite.jsonl`, hard round brush on a mask + mask.edit
//! fill:0, NO feather) is fully Sub-slice A: a mask filled to 0 on a top
//! layer REVEALS the layer below, and the render composite equals the CPU
//! reference export byte-for-byte. **T05** (`t05-masks.jsonl`) serves its
//! non-feather portion (reveal_all / hide_all / density) here, AND its
//! feathered headline (step 8, feather:18) — Sub-slice B, served on all
//! three compositing doors and cross-checked bit-strict between them.

use appname_core::{
    composite_plane_f32, decode_png_to_buffer, encode_composite_png, export_png_bytes,
    io::srgb::srgb_decode, io::srgb::srgb_encode, io::srgb::unpremul, Engine, EngineAssembly,
    IVec2, ProfileId,
};
use appname_render::composite_document_with;
use serde_json::{json, Value};

/// Core-kinds assembly (masks are raster — no filter/adjust/text seams
/// needed): doc.new / layer.add / edit.paste / mask.* / fill are all core
/// command kinds.
fn engine() -> Engine {
    appname_core::engine::kinds::register(EngineAssembly::new()).build()
}

fn apply(e: &mut Engine, id: u32, kind: &str, target: Option<Value>, params: Value) {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("11a50000-0000-4000-8000-{id:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    e.apply_json(&v.to_string())
        .unwrap_or_else(|err| panic!("{kind} -> {:?} {}", err.code, err.message));
}

/// A solid `[r,g,b,a]` (linear premultiplied) `w×h` PNG asset — encoded
/// through core's own §6.4/§8.2 quantizer so the ingested u8 storage is
/// exactly what the goldens bar uses.
fn solid_png(rgba: [f32; 4], w: u32, h: u32) -> Vec<u8> {
    let plane: Vec<f32> = std::iter::repeat(rgba)
        .take((w * h) as usize)
        .flatten()
        .collect();
    encode_composite_png(w, h, &plane).expect("encode solid asset")
}

/// The render-vs-core cross-check (§34.3 one compositing truth): the render
/// composite, quantized with core's public helpers exactly as
/// `export_png_bytes` does, must equal core's actual export decoded back —
/// bit-strict. Masked docs now ride this same contract.
fn crosscheck_vs_core_export(engine: &Engine) {
    let doc = engine.document().expect("initialized");
    let canvas = doc.geometry().canvas_rect();
    let png = export_png_bytes(engine).expect("core export serves the masked doc");
    let (core_buf, w, h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    assert_eq!((w as i32, h as i32), (canvas.width(), canvas.height()));
    let comp = composite_document_with(doc, engine.assembly(), canvas)
        .expect("render composite serves the masked doc");
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
            assert_eq!(
                core_buf.read_pixel_f32(p),
                predicted,
                "render-vs-core mismatch at ({x},{y}) — the mask must composite one way"
            );
        }
    }
}

const RED_ID: &str = "11a50000-0000-4000-8000-0000000000a1";
const BLUE_ID: &str = "11a50000-0000-4000-8000-0000000000a2";
const MASK_ID: &str = "11a50000-0000-4000-8000-0000000000b1";

/// A two-layer doc: an opaque RED bottom layer and an opaque BLUE top
/// layer (32×32, fully overlapping), plus a `reveal_all` raster mask on
/// the BLUE top layer. Returns the engine (mask id = [`MASK_ID`]).
fn red_under_blue_with_top_mask() -> Engine {
    let mut e = engine();
    apply(
        &mut e,
        1,
        "doc.new",
        None,
        json!({"width": 32, "height": 32, "background": "transparent"}),
    );
    let red = e
        .add_raster_asset(solid_png([1.0, 0.0, 0.0, 1.0], 32, 32))
        .expect("red asset");
    apply(
        &mut e,
        2,
        "edit.paste",
        None,
        json!({"assetId": red.to_string(), "at": [0, 0], "newLayerId": RED_ID}),
    );
    let blue = e
        .add_raster_asset(solid_png([0.0, 0.0, 1.0, 1.0], 32, 32))
        .expect("blue asset");
    apply(
        &mut e,
        3,
        "edit.paste",
        None,
        json!({"assetId": blue.to_string(), "at": [0, 0], "newLayerId": BLUE_ID}),
    );
    apply(
        &mut e,
        4,
        "mask.add",
        Some(json!({"layerId": BLUE_ID})),
        json!({"kind": "raster", "from": "reveal_all", "newMaskId": MASK_ID}),
    );
    e
}

fn center_pixel(e: &Engine) -> [f32; 4] {
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    composite_document_with(doc, e.assembly(), canvas)
        .expect("serves")
        .pixel(IVec2::new(16, 16))
}

fn is_red(px: [f32; 4]) -> bool {
    px[0] > 0.9 && px[2] < 0.1 && px[3] > 0.9
}
fn is_blue(px: [f32; 4]) -> bool {
    px[2] > 0.9 && px[0] < 0.1 && px[3] > 0.9
}

/// **T13**: a `reveal_all` mask on the top layer is a no-op (the top BLUE
/// layer wins); a `mask.edit fill:0` on that mask hides the top layer and
/// REVEALS the RED layer below (the ≥2-layer direct target — a mask
/// reveals the layers beneath). Render and core-export agree bit-strict,
/// and every reference export door serves.
#[test]
fn t13_mask_edit_fill_zero_reveals_the_layer_below() {
    let mut e = red_under_blue_with_top_mask();

    // reveal_all ⇒ the mask is a no-op ⇒ the top BLUE layer shows.
    assert!(
        is_blue(center_pixel(&e)),
        "reveal_all mask is a no-op — top layer wins"
    );

    // mask.edit fill:0 ⇒ the top layer's coverage is 0 everywhere ⇒ it
    // masks fully OUT ⇒ the RED layer below is revealed.
    apply(
        &mut e,
        5,
        "mask.edit",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"edit": {"op": "fill", "value": 0}}),
    );
    assert!(
        is_red(center_pixel(&e)),
        "mask.edit fill:0 must reveal the layer BELOW (the ≥2-layer target)"
    );

    // One compositing truth: the render composite equals the CPU reference
    // export byte-for-byte, and every reference door serves (PNG + f32).
    crosscheck_vs_core_export(&e);
    assert!(
        export_png_bytes(&e).is_ok(),
        "PNG export serves the masked doc"
    );
    assert!(
        composite_plane_f32(&e).is_ok(),
        "f32 readback serves the masked doc"
    );
}

/// **T05** (non-feather portion): `hide_all` masks the top layer fully out
/// (reveals RED), a partial `density` lerps toward full coverage, and a
/// `reveal_all` mask leaves the top layer intact — each served, and each
/// agreeing render-vs-core.
#[test]
fn t05_hide_all_and_density_serve_and_agree() {
    // hide_all on the top layer ⇒ RED revealed.
    let mut e = red_under_blue_with_top_mask();
    apply(
        &mut e,
        5,
        "mask.edit",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"edit": {"op": "fill", "value": 0}}),
    );
    assert!(is_red(center_pixel(&e)), "hide (fill:0) reveals RED below");
    crosscheck_vs_core_export(&e);

    // density 0.9 on a reveal_all (E=1) mask: m' = 1 − 0.9·(1 − 1) = 1 —
    // still a no-op at full coverage (density only bites where coverage <
    // 1). The top BLUE layer stays; the doors agree.
    let mut e2 = red_under_blue_with_top_mask();
    apply(
        &mut e2,
        5,
        "mask.set_props",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"density": 0.9}),
    );
    assert!(
        is_blue(center_pixel(&e2)),
        "density on full coverage stays full"
    );
    crosscheck_vs_core_export(&e2);

    // density 0.5 on a HIDE (E=0) mask: m' = 1 − 0.5·(1 − 0) = 0.5 — the
    // top BLUE composites at half coverage over RED, so neither pure blue
    // nor pure red: a genuine partial reveal. The doors agree bit-strict.
    let mut e3 = red_under_blue_with_top_mask();
    apply(
        &mut e3,
        5,
        "mask.edit",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"edit": {"op": "fill", "value": 0}}),
    );
    apply(
        &mut e3,
        6,
        "mask.set_props",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"density": 0.5}),
    );
    let px = center_pixel(&e3);
    assert!(
        px[2] > 0.1 && px[0] > 0.1,
        "density 0.5 over a hide gives a partial reveal (blue over red): {px:?}"
    );
    crosscheck_vs_core_export(&e3);
}

/// **T05** feathered headline (step 8, feather:18) — **Sub-slice B, now
/// SERVED**. This is the COMPOSITE-DOOR proof, not a command-door one:
/// the `mask.set_props feather` command was always accepted; what changed
/// is that all THREE compositing doors now produce feathered pixels
/// instead of refusing typed.
///
/// The signal is real, not merely non-erroring. A `reveal_all` mask's
/// stored coverage is exactly the canvas rect, so it has a hard edge at
/// the canvas boundary; feathering it ramps the coverage inward, and the
/// composite blends the BLUE top layer into the RED one underneath along
/// that border. Unfeathered, the same border pixel is pure BLUE — the
/// test asserts both, so it cannot pass on a silently-unfeathered serve.
#[test]
fn t05_feathered_mask_serves_at_every_composite_door() {
    // Baseline: the SAME document without feather composites pure BLUE at
    // the border. If this and the feathered case ever agree, feather is
    // being dropped silently — the failure mode QA-F8 exists to prevent.
    let plain = red_under_blue_with_top_mask();
    let border_unfeathered = {
        let doc = plain.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        composite_document_with(doc, plain.assembly(), canvas)
            .expect("unfeathered serves")
            .pixel(IVec2::new(0, 16))
    };
    assert!(
        is_blue(border_unfeathered),
        "precondition: unfeathered, the border is pure BLUE: {border_unfeathered:?}"
    );

    let mut e = red_under_blue_with_top_mask();
    apply(
        &mut e,
        5,
        "mask.set_props",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"feather": 18.0}),
    );

    // Both CPU reference doors SERVE (they used to refuse typed).
    export_png_bytes(&e).expect("PNG export serves the feathered mask");
    composite_plane_f32(&e).expect("f32 composite serves the feathered mask");

    // The render compositor SERVES the same construct.
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render serves the feathered mask");

    // ...and the pixels are actually FEATHERED. Blue-over-red with
    // coverage E composites to [1−E, 0, E, 1] (premultiplied linear), so
    // the border pixel is a genuine blend, strictly between the two
    // layers, and strictly different from the unfeathered baseline.
    let border = comp.pixel(IVec2::new(0, 16));
    assert!(
        border[0] > 0.05 && border[2] > 0.05,
        "the feathered border must be a RED/BLUE blend, not either layer: {border:?}"
    );
    assert!(
        border != border_unfeathered,
        "feathered and unfeathered borders are identical — feather was dropped"
    );
    assert!(
        border[2] < border_unfeathered[2],
        "feathering can only REDUCE coverage at a content edge: {border:?}"
    );

    // One compositing truth (§34.3): render and the CPU reference export
    // agree bit-strict on the feathered document too. This is the check
    // that would catch the two surfaces feathering DIFFERENTLY — the
    // serve↔serve analogue of the serve↔refuse split.
    crosscheck_vs_core_export(&e);
}

/// The feathered edge PROFILE at the composite door, checked against
/// properties DERIVED from the kernel rather than fitted to its output.
///
/// Setup: 64×64 canvas, `reveal_all` mask (stored coverage = the canvas
/// rect, hard edge at the boundary), `feather = 9` ⇒ σ = 3, Rmask = 9.
/// Blue-over-red compositing makes the coverage `E` directly readable:
/// `px = [1−E, 0, E, 1]`.
///
/// Derived expectations:
///   1. **Interior is EXACTLY full coverage.** More than Rmask inside the
///      edge every kernel tap lands on stored coverage 1.0, and the kernel
///      is normalized (Σ = 1), so the result is 1.0 — mass preservation,
///      not an approximation.
///   2. **Monotone non-decreasing inward.** More of the kernel's support
///      lies inside the content as you move in.
///   3. **The boundary column sits near one half.** By symmetry of the
///      1-D kernel, a half-plane edge evaluates to `(1 + w₀)/2`, and
///      `w₀ = 1/(σ√(2π)) ≈ 0.133` at σ = 3 — so the value is bracketed
///      well inside `(0.4, 0.7)` without pinning a fitted constant.
#[test]
fn feathered_edge_profile_is_monotone_and_full_inside_rmask() {
    let mut e = engine();
    apply(
        &mut e,
        1,
        "doc.new",
        None,
        json!({"width": 64, "height": 64, "background": "transparent"}),
    );
    let red = e
        .add_raster_asset(solid_png([1.0, 0.0, 0.0, 1.0], 64, 64))
        .expect("red asset");
    apply(
        &mut e,
        2,
        "edit.paste",
        None,
        json!({"assetId": red.to_string(), "at": [0, 0], "newLayerId": RED_ID}),
    );
    let blue = e
        .add_raster_asset(solid_png([0.0, 0.0, 1.0, 1.0], 64, 64))
        .expect("blue asset");
    apply(
        &mut e,
        3,
        "edit.paste",
        None,
        json!({"assetId": blue.to_string(), "at": [0, 0], "newLayerId": BLUE_ID}),
    );
    apply(
        &mut e,
        4,
        "mask.add",
        Some(json!({"layerId": BLUE_ID})),
        json!({"kind": "raster", "from": "reveal_all", "newMaskId": MASK_ID}),
    );
    apply(
        &mut e,
        5,
        "mask.set_props",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"feather": 9.0}),
    );

    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render serves the feathered mask");

    // E along a horizontal scan through the middle row.
    let row = 32;
    let e_at = |x: i32| {
        let px = comp.pixel(IVec2::new(x, row));
        // Premultiplied blue-over-red: px[2] IS the coverage, and the red
        // channel is its complement. Assert the identity so a change in
        // the compositing arithmetic cannot quietly reinterpret this scan.
        assert!(
            (px[0] + px[2] - 1.0).abs() < 1e-5,
            "blue-over-red must satisfy px[0] + px[2] == 1 at x={x}: {px:?}"
        );
        px[2]
    };

    // 1. Interior (≥ Rmask = 9 from both edges) is EXACTLY full coverage.
    for x in 9..=54 {
        assert_eq!(
            e_at(x),
            1.0,
            "interior coverage must be exactly 1.0 (normalized kernel over a \
             constant region) at x={x}"
        );
    }

    // 2. Monotone non-decreasing from the boundary inward.
    for x in 0..9 {
        assert!(
            e_at(x) <= e_at(x + 1),
            "coverage must not decrease inward: E({x})={} > E({})={}",
            e_at(x),
            x + 1,
            e_at(x + 1)
        );
    }

    // 3. The boundary column is the half-plane value (1 + w₀)/2.
    let edge = e_at(0);
    assert!(
        edge > 0.4 && edge < 0.7,
        "a half-plane edge through a symmetric normalized kernel sits near \
         one half, got {edge}"
    );

    // 4. The ramp is a genuine gradient, not a step: the boundary and the
    //    first interior column differ, and the ramp actually rises.
    assert!(
        e_at(8) > edge,
        "the feather ramp must rise across Rmask: E(0)={edge}, E(8)={}",
        e_at(8)
    );

    // And the two doors agree bit-strict on the whole 64×64 frame.
    crosscheck_vs_core_export(&e);
}

/// Requirement 2 (the correctness-critical one): the WARM DAG cache MUST
/// invalidate on a mask edit. The per-(node,tile) leaf hash folds mask
/// coverage identity (density/inverted + coverage-tile hash), so a
/// `mask.edit fill:0` on a warm renderer recomputes the tile — never
/// returns the STALE pre-edit tile (silent-wrong, worse than the refusal).
#[test]
fn warm_dag_cache_invalidates_on_mask_edit() {
    use appname_render::Renderer;

    let mut e = red_under_blue_with_top_mask(); // reveal_all mask on BLUE top
    let canvas = e.document().unwrap().geometry().canvas_rect();
    let mut renderer = Renderer::new();

    // Cold render: reveal_all is a no-op ⇒ the warm cache holds BLUE.
    let warm = renderer.render(e.document().unwrap(), canvas).unwrap();
    assert!(is_blue(warm.pixel(IVec2::new(16, 16))), "reveal_all ⇒ blue");

    // Edit the mask to hide the top layer.
    apply(
        &mut e,
        5,
        "mask.edit",
        Some(json!({"layerId": BLUE_ID, "maskId": MASK_ID})),
        json!({"edit": {"op": "fill", "value": 0}}),
    );

    // Warm re-render: the leaf hash changed (coverage folded) ⇒ cache
    // MISS ⇒ recompute ⇒ RED revealed. A stale hit would wrongly keep BLUE.
    let doc = e.document().unwrap();
    let after = renderer.render(doc, canvas).unwrap();
    assert!(
        is_red(after.pixel(IVec2::new(16, 16))),
        "warm cache must invalidate on mask.edit — a stale hit would keep BLUE"
    );

    // And the warm render equals the oracle (no stale tiles anywhere).
    let oracle = composite_document_with(doc, e.assembly(), canvas).unwrap();
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let p = IVec2::new(x, y);
            assert_eq!(
                after.pixel(p),
                oracle.pixel(p),
                "warm DAG != oracle at ({x},{y}) after a mask edit — stale cache"
            );
        }
    }
}
