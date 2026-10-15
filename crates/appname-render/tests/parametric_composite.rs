// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §12.1 PARAMETRIC RE-RENDER integration suite (reopened CP-0.7,
//! O-RENDER slice): the compositor serves what core's export door serves.
//!
//! Spec: ES §12.1 (chain formula), §12.2 (smart-filter chains over the
//! shared source), §12.4 (disabled == never-applied, bitwise), §11.4/
//! §17.1 (adjustment = n = 1 chain over the backdrop, pass-through
//! stacking), §11.5 (smart-object proxies, DD ⑨/D-48 value capture),
//! §4.7 (isolation rows named in refusals), §34.3 (parity discipline —
//! the render-vs-core cross-check below is THIS slice's proof leg:
//! core's `convert-preserves-look` W16 leg proves core-vs-reference,
//! these tests prove render-vs-core); QA-F8 (refuse-never-omit).
//!
//! **Cross-check contract** (the document_composite.rs precedent,
//! extended to parametric content): on the SHARED serve set the two
//! doors must agree bit-strict after the shared §6.4/§8.2 sRGB
//! quantization. Fixtures follow the established dyadic discipline where
//! the two accumulation orders must both be exact; the §12.1 evaluation
//! itself is core's own exported `ChainEvaluator` — one code path, so
//! chain math cannot drift between the doors by construction.
//!
//! All documents are built through PUBLIC doors only (envelopes through
//! the SHIPPED assembly composition: core kinds + appname-filters +
//! appname-adjust — the same seam the CLI/conformance assembly points
//! carry).

use appname_core::io::srgb::{srgb_decode, srgb_encode, unpremul};
use appname_core::{
    composite_plane_f32, decode_png_to_buffer, encode_composite_png, export_png_bytes,
    has_visible_parametric_content, Engine, EngineAssembly, IVec2, ProfileId,
};
use appname_render::{
    composite_document, composite_document_with, recomposite_dirty_with, CompositeError, DirtyMap,
    DirtyRecord, Renderer,
};
use serde_json::{json, Value};

/// DELIBERATE-PARTIAL rig (never claims production parity): filter +
/// adjust kinds/ops over BARE core — no tools/vector wraps; fixtures
/// use `doc.new` backgrounds, and the §12.1 op-eval legs need exactly
/// the op registries. ALLOWLISTED by exact path in appname-assembly's
/// `no_hand_rolled_chain_anywhere_in_the_workspace` guard (the ALLOW
/// entry names this file) — a comment required by that guard's rule.
/// [Cross-fence comment-only touch, flagged for O-RENDER ratification.]
fn assembled_engine() -> Engine {
    appname_adjust::register(appname_filters::register(
        appname_core::engine::kinds::register(EngineAssembly::new()),
    ))
    .build()
}

fn apply(e: &mut Engine, v: &Value) -> appname_core::ApplyReceipt {
    e.apply_json(&v.to_string())
        .unwrap_or_else(|err| panic!("{v} -> {:?} {}", err.code, err.message))
}

/// THE fully-seamed production assembly (core kinds + tools/filters/adjust +
/// the Door-1 vector / Door-6 text / fill-math seams) via the single-source
/// constructor — the assembly the CLI/cabi ship. The composite-legs slice
/// needs the text/vector/fill seams live, so the shape/text fixtures below
/// build on THIS (the bare `assembled_engine` above has no text/vector).
/// [Cross-fence dev-dep: appname-assembly, already a test-only path dep of
/// this crate — flagged for O-RENDER ratification alongside adjust/filters.]
fn seamed_engine() -> Engine {
    appname_assembly::production_assembly().build()
}

/// A closed axis-aligned rectangle as a `vector.shape_add` `path` param
/// (contiguous cubic subpath — the §20.1 wire form; mirrors the conformance
/// suite's `rect_vector_path`).
fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Value {
    let seg = |ax: f64, ay: f64, bx: f64, by: f64| {
        json!({
            "p0": [ax, ay],
            "c1": [ax + (bx - ax) / 3.0, ay + (by - ay) / 3.0],
            "c2": [ax + 2.0 * (bx - ax) / 3.0, ay + 2.0 * (by - ay) / 3.0],
            "p1": [bx, by],
        })
    };
    json!({
        "fillRule": "nonzero",
        "subpaths": [{"closed": true, "segments": [
            seg(x, y, x + w, y),
            seg(x + w, y, x + w, y + h),
            seg(x + w, y + h, x, y + h),
            seg(x, y + h, x, y),
        ]}],
    })
}

fn env(id: u32, kind: &str, target: Option<Value>, params: Value) -> Value {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("0e140000-0000-4000-8000-{id:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    v
}

/// White-background doc + `smartobject.convert` of the background: the
/// canonical smart-object fixture (proxy = the captured white raster).
/// Returns (engine, smart layer uuid string).
fn smart_doc(w: u32, h: u32) -> (Engine, String) {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": w, "height": h, "background": "white"}),
        ),
    );
    let bg = e.document().unwrap().z_order_walk()[0];
    let smart = "0e140000-0000-4000-8000-0000000000b1".to_string();
    apply(
        &mut e,
        &env(
            2,
            "smartobject.convert",
            None,
            json!({
                "layerIds": [bg.as_uuid().to_string()],
                "newLayerId": smart,
            }),
        ),
    );
    (e, smart)
}

/// The render-vs-core cross-check (document_composite.rs contract,
/// §34.3): composite through the RENDER door with the engine's own op
/// registry, quantize with core's public §6.4/§8.2 helpers exactly as
/// `export_png_bytes` does, and compare bit-strict against core's actual
/// export decoded back through core's own decoder.
fn crosscheck_vs_core_export(engine: &Engine) {
    let doc = engine.document().expect("initialized");
    let canvas = doc.geometry().canvas_rect();

    let png = export_png_bytes(engine).expect("core export serves");
    let (core_buf, w, h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    assert_eq!((w as i32, h as i32), (canvas.width(), canvas.height()));

    let comp =
        composite_document_with(doc, engine.assembly(), canvas).expect("render composite serves");

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
            assert_eq!(
                got, predicted,
                "render-vs-core mismatch at ({x},{y}): core-export pixel {got:?} vs \
                 render-path prediction {predicted:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------
// Serve legs + cross-check (§34.3 render-vs-core).
// ---------------------------------------------------------------------

/// Plain proxy (no chain): the smart layer composites its value-captured
/// proxy exactly like a raster surface — and matches core's export
/// byte-for-byte (core's W16 leg proved convert-preserves-look against
/// the pre-convert bytes; this proves render-vs-core on the same state).
#[test]
fn crosscheck_smart_proxy_plain() {
    let (e, _) = smart_doc(64, 48);
    crosscheck_vs_core_export(&e);
}

/// Enabled adjustment layer (production `adjustment.invert` through the
/// shipped bridges): §17.1 n = 1 chain over the composite below, both
/// doors bit-strict; `op.toggle` off restores the pre-add render bytes
/// exactly (§12.4 at the render door).
#[test]
fn crosscheck_adjustment_invert_over_backdrop_and_toggle_off() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 64, "height": 48, "background": "white"}),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let pre = composite_document_with(doc, e.assembly(), canvas).unwrap();

    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": "0e140000-0000-4000-8000-0000000000a1",
                "newOpId": "0e140000-0000-4000-8000-0000000000a2",
                "params": {},
            }),
        ),
    );
    crosscheck_vs_core_export(&e);

    // The adjustment BIT: white inverts to black at the served door.
    let doc = e.document().unwrap();
    let served = composite_document_with(doc, e.assembly(), canvas).unwrap();
    let px = served.pixel(IVec2::new(8, 8));
    assert!(
        px[0] < 0.5 && px[3] == 1.0,
        "invert over white backdrop must darken: {px:?}"
    );

    // §12.4 at the render door: toggle off == never applied, bitwise.
    apply(
        &mut e,
        &env(
            3,
            "op.toggle",
            None,
            json!({"opId": "0e140000-0000-4000-8000-0000000000a2", "enabled": false}),
        ),
    );
    let doc = e.document().unwrap();
    let off = composite_document_with(doc, e.assembly(), canvas).unwrap();
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let p = IVec2::new(x, y);
            assert_eq!(
                off.pixel(p).map(f32::to_bits),
                pre.pixel(p).map(f32::to_bits),
                "toggle-off must restore pre-add bytes at ({x},{y})"
            );
        }
    }
}

/// **§12.1 coverage-LERP at an ADJUSTMENT layer (n = 1), through the
/// op mask.** The normative rule is ES §12.1 "Evaluation order within a
/// node" [NORMATIVE] — `mᵢ = opᵢ.mask coverage per §11.2`, then
/// `inᵢ = lerp(inᵢ₋₁, blend(inᵢ₋₁, fᵢ, opᵢ.blend), opᵢ.opacity · mᵢ)` —
/// NOT §11.4 (which is "Groups and pass-through" and carries only the
/// adjustment's SCOPE). This pins the lerp at the composite door, where
/// no test reached it before: `mask_composite.rs` never mentions an
/// adjustment, and `op_eval.rs`'s unit leg stops at the evaluator.
///
/// A FEATHERED SELECTION baked into the op mask (`op.set_mask
/// from:'selection'`) gives genuine fractional coverage while the mask's
/// own `feather` prop stays 0 — so this serves rather than tripping the
/// §11.2 Sub-slice B refusal. The band pixels are the proof that this is
/// a LERP and not a binary gate: over a white backdrop `adjustment.invert`
/// produces 0, so the served pixel must equal `1 − m` EXACTLY at every
/// pixel, for the `m` the shared §11.2 routine reports.
///
/// **Declared limitation (§5 self-review, recorded not hidden):** the
/// expected `m` here comes from the SAME `resolve_effective_coverage` the
/// evaluator uses, so this test cannot catch a defect INSIDE the §11.2
/// order itself — that order is pinned separately by `mask_eval.rs`'s unit
/// legs (`density_then_invert_order`,
/// `zero_density_inverted_is_fully_masked_not_noop`). What this test does
/// pin, and what nothing pinned before it, is that the composite door
/// APPLIES that coverage as a §12.1 lerp. Perturbation evidence: swapping
/// the lerp for a `t >= 0.5` gate reds it at a fractional pixel, and
/// dropping the mask from the fade weight reds it likewise.
#[test]
fn adjustment_op_mask_coverage_lerps_the_effect_over_the_backdrop() {
    use appname_core::{resolve_effective_coverage, OpId};

    const OP: &str = "0e140000-0000-4000-8000-0000000000b2";
    // The SEAMED production assembly: `selection.set`/`selection.modify`
    // are tools-implemented (the bare rig registers them unimplemented).
    let mut e = seamed_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 64, "height": 48, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": "0e140000-0000-4000-8000-0000000000b1",
                "newOpId": OP,
                "params": {},
            }),
        ),
    );
    // Left half, softened: the feather band carries STRICTLY fractional
    // coverage — the only input on which lerp and gate disagree.
    apply(
        &mut e,
        &env(
            3,
            "selection.set",
            None,
            json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 32, "h": 48},
                   "antialias": false}),
        ),
    );
    apply(
        &mut e,
        &env(4, "selection.modify", None, json!({"feather": 8.0})),
    );
    apply(
        &mut e,
        &env(
            5,
            "op.set_mask",
            None,
            json!({"opId": OP, "from": "selection"}),
        ),
    );

    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let mask = doc
        .mask(
            doc.op(OpId::from_uuid(OP.parse().unwrap()))
                .expect("op resolves")
                .mask()
                .expect("op.set_mask installed the mask"),
        )
        .expect("mask node resolves");
    let served = composite_document_with(doc, e.assembly(), canvas)
        .expect("§12.1 op-masked adjustment serves at the render door");

    // EXACT lerp identity at every pixel: white (1.0) inverted is 0.0, so
    // lerp(1, 0, m) = 1 − m. Equality, not a tolerance band.
    let (mut full, mut none, mut fractional) = (0usize, 0usize, 0usize);
    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let p = IVec2::new(x, y);
            let m = resolve_effective_coverage(mask, p);
            // Regime tally only (the lerp assertion below stays EXACT):
            // a Gaussian-feathered plane need not reach a bit-exact 1.0,
            // so the non-vacuity guards use bands, never the equality.
            match m {
                _ if m > 0.999 => full += 1,
                _ if m < 0.001 => none += 1,
                _ => fractional += 1,
            }
            let px = served.pixel(p);
            assert_eq!(
                px[0],
                1.0 - m,
                "§12.1 lerp broken at ({x},{y}): coverage {m}, served {px:?}"
            );
            assert_eq!(px[3], 1.0, "adjustment must not disturb backdrop alpha");
        }
    }
    // All three coverage regimes must be present, or the assertion above
    // is vacuous: `full` and `none` alone are satisfied by a binary GATE;
    // only `fractional` separates a lerp from a gate.
    assert!(
        full > 0,
        "no fully-covered pixel: the adjustment never applies"
    );
    assert!(none > 0, "no zero-coverage pixel: the mask never masks out");
    assert!(
        fractional > 0,
        "the feathered op mask must produce STRICTLY fractional coverage — \
         without it this test cannot tell a lerp from a gate"
    );
    crosscheck_vs_core_export(&e);
}

/// Enabled smart-filter chain, ROI ≠ identity (production `filter.custom`
/// with a dyadic single-tap shift kernel through the shipped bridges):
/// the §12.1 apron path serves bit-strict against core, and the shifted
/// content proves the chain actually evaluated over the proxy.
#[test]
fn crosscheck_smart_chain_roi_shift_kernel() {
    let (mut e, smart) = smart_doc(64, 48);
    // 5×5 kernel with the single tap at (+2, 0): out(p) = src(p + (2,0))
    // — dyadic-exact, roi_out_to_in inflates by the kernel reach, so the
    // apron discipline is load-bearing (the shifted-in column crosses
    // what would be a fabricated edge without it).
    let mut kernel = [[0i32; 5]; 5];
    kernel[2][4] = 1;
    apply(
        &mut e,
        &env(
            3,
            "filter.smart_add",
            Some(json!({"layerId": smart})),
            json!({
                "filterId": "filter.custom",
                "newOpId": "0e140000-0000-4000-8000-0000000000c1",
                "params": {"kernel": kernel, "divisor": 1, "offset": 0.0},
            }),
        ),
    );
    crosscheck_vs_core_export(&e);

    // Shift semantics at the served door, per the §16.3 contract: alpha
    // passes through UNTOUCHED (both edges stay opaque), while the RGB
    // convolution shifts — one canvas-edge band reads taps beyond the
    // captured content (unpremultiplied 0 through the apron's real
    // absent-tile reads) and goes black; the interior stays white.
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas).unwrap();
    let inner = comp.pixel(IVec2::new(8, 8));
    assert_eq!(inner, [1.0, 1.0, 1.0, 1.0], "interior stays opaque white");
    let left = comp.pixel(IVec2::new(0, 8));
    let right = comp.pixel(IVec2::new(63, 8));
    assert_eq!(
        (left[3], right[3]),
        (1.0, 1.0),
        "§16.3: alpha passes through untouched at both edges"
    );
    assert!(
        (left[0] == 0.0) ^ (right[0] == 0.0),
        "exactly one edge band reads shifted-in absent content through the \
         §12.1 apron (RGB 0 under intact alpha): left {left:?} right {right:?}"
    );
}

/// Deterministic two-run identity on a non-dyadic chain (production
/// gaussian blur): the §12.1 serve is a pure function of (params, inputs)
/// — same document, same bytes, run to run (D-48 discipline at the
/// render door).
#[test]
fn smart_chain_gaussian_blur_serves_deterministically() {
    let (mut e, smart) = smart_doc(64, 48);
    apply(
        &mut e,
        &env(
            3,
            "filter.smart_add",
            Some(json!({"layerId": smart})),
            json!({
                "filterId": "filter.gaussian_blur",
                "newOpId": "0e140000-0000-4000-8000-0000000000c2",
                "params": {"radius": 2.0},
            }),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let ops = e.assembly();
    let a = composite_document_with(doc, ops, canvas).unwrap();
    let b = composite_document_with(doc, ops, canvas).unwrap();
    for ((ka, ta), (kb, tb)) in a.tiles().zip(b.tiles()) {
        assert_eq!(ka, kb);
        assert!(ta == tb, "two-run byte-identity at tile {ka:?}");
    }
    // Blur bites: the canvas-edge alpha ramps down (taps read absent
    // tiles beyond the captured content), the interior stays opaque.
    assert_eq!(a.pixel(IVec2::new(32, 24))[3], 1.0);
    assert!(a.pixel(IVec2::new(0, 0))[3] < 1.0, "edge alpha ramps");
}

// ---------------------------------------------------------------------
// Serve/refuse matrix (mirrors core's arms; §4.7 rows named).
// ---------------------------------------------------------------------

/// The BARE-registry door (`composite_document`, no ops): a plain proxy
/// serves (§12.4 identity needs no kernels — the tripwire-firing
/// behavior); an ENABLED chain entry refuses typed, never omits.
#[test]
fn bare_registry_serves_plain_proxy_and_refuses_enabled_chain() {
    let (mut e, smart) = smart_doc(64, 48);
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    assert!(
        composite_document(doc, canvas).is_ok(),
        "plain proxy serves through the bare door (this is what fires O-QA's tripwire)"
    );

    apply(
        &mut e,
        &env(
            3,
            "filter.smart_add",
            Some(json!({"layerId": smart})),
            json!({
                "filterId": "filter.gaussian_blur",
                "newOpId": "0e140000-0000-4000-8000-0000000000c3",
                "params": {"radius": 1.0},
            }),
        ),
    );
    let doc = e.document().unwrap();
    let err = composite_document(doc, canvas).unwrap_err();
    assert!(
        matches!(err, CompositeError::ParametricNotImplemented { .. }),
        "enabled op under a bare registry refuses typed: {err}"
    );
    // The ASSEMBLED door serves the same document.
    assert!(composite_document_with(doc, e.assembly(), canvas).is_ok());
}

/// Adjustment inside an isolating group refuses typed with the §4.7 row
/// named — at BOTH doors (render mirrors core's serve/refuse split).
#[test]
fn adjustment_inside_isolating_group_refuses_with_row_named() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 32, "height": 32, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": "0e140000-0000-4000-8000-0000000000a1",
                "newOpId": "0e140000-0000-4000-8000-0000000000a2",
                "params": {},
            }),
        ),
    );
    apply(
        &mut e,
        &env(
            3,
            "layer.group",
            None,
            json!({
                "layerIds": ["0e140000-0000-4000-8000-0000000000a1"],
                "newGroupId": "0e140000-0000-4000-8000-0000000000e1",
            }),
        ),
    );
    // Row 3: opacity < 1.0 isolates the group.
    apply(
        &mut e,
        &env(
            4,
            "layer.set_props",
            Some(json!({"layerId": "0e140000-0000-4000-8000-0000000000e1"})),
            json!({"opacity": 0.5}),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let err = composite_document_with(doc, e.assembly(), canvas).unwrap_err();
    match &err {
        CompositeError::ParametricNotImplemented { detail, .. } => {
            assert!(
                detail.contains("row 3"),
                "the isolating §4.7 row is named: {detail}"
            );
        }
        other => panic!("expected ParametricNotImplemented, got {other}"),
    }
    // Core's export door refuses the same document (serve/refuse parity).
    let core_err = export_png_bytes(&e).unwrap_err();
    assert_eq!(
        core_err.code,
        appname_core::ErrorCode::NotImplemented,
        "core refuses the isolating group too: {}",
        core_err.message
    );
}

/// Non-default compositing props on the adjustment LAYER refuse typed at
/// both doors (the §11.4 layer-level fade is not defined by this slice);
/// opacity 0 is the exact-identity skip at both doors, mirroring core.
#[test]
fn adjustment_layer_props_refuse_and_opacity_zero_skips() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 32, "height": 32, "background": "white"}),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let pre = composite_document_with(doc, e.assembly(), canvas).unwrap();
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": "0e140000-0000-4000-8000-0000000000a1",
                "newOpId": "0e140000-0000-4000-8000-0000000000a2",
                "params": {},
            }),
        ),
    );
    // FI-13 [NORMATIVE] (:5956-5962): node.opacity now SERVES at both doors
    // — it folds into the §12.1 lerp weight. Covered in full by
    // `fi13_node_opacity_folds_into_the_lerp_weight` below; here we only
    // pin that it no longer refuses, so this test keeps its subject (the
    // props that DO still refuse).
    apply(
        &mut e,
        &env(
            3,
            "layer.set_props",
            Some(json!({"layerId": "0e140000-0000-4000-8000-0000000000a1"})),
            json!({"opacity": 0.5}),
        ),
    );
    let doc = e.document().unwrap();
    composite_document_with(doc, e.assembly(), canvas)
        .expect("FI-13: a faded adjustment SERVES at the render door");
    export_png_bytes(&e).expect("FI-13: and at the reference export door");

    // RULING 2 (2026-08-28): node.blend now SERVES at both doors as stage 2a of
    // the engine-binding interpretation. What still refuses is DISSOLVE, which
    // routes to neither kernel arm and is a coverage threshold, not a colour op.
    apply(
        &mut e,
        &env(
            4,
            "layer.set_props",
            Some(json!({"layerId": "0e140000-0000-4000-8000-0000000000a1"})),
            json!({"opacity": 1.0, "blend": "dissolve"}),
        ),
    );
    let doc = e.document().unwrap();
    let err = composite_document_with(doc, e.assembly(), canvas).unwrap_err();
    assert!(
        matches!(err, CompositeError::ParametricNotImplemented { .. }),
        "Dissolve-blended adjustment layer refuses typed: {err}"
    );
    assert_eq!(
        export_png_bytes(&e).unwrap_err().code,
        appname_core::ErrorCode::NotImplemented,
        "never a serve↔refuse split"
    );
    apply(
        &mut e,
        &env(
            5,
            "layer.set_props",
            Some(json!({"layerId": "0e140000-0000-4000-8000-0000000000a1"})),
            json!({"blend": "normal"}),
        ),
    );
    // opacity 0 → exact skip (serves; bytes == pre-add).
    apply(
        &mut e,
        &env(
            6,
            "layer.set_props",
            Some(json!({"layerId": "0e140000-0000-4000-8000-0000000000a1"})),
            json!({"opacity": 0.0}),
        ),
    );
    let doc = e.document().unwrap();
    let skipped = composite_document_with(doc, e.assembly(), canvas).unwrap();
    for (k, t) in skipped.tiles() {
        let (_, pre_t) = pre.tiles().find(|(pk, _)| *pk == k).unwrap();
        assert!(
            t == pre_t,
            "opacity-0 adjustment is the exact identity at {k:?}"
        );
    }
}

/// Smart-object layers composite like raster surfaces: layer-level §4.5
/// blend serves at the RENDER door (render owns those kernels — this is
/// §4.5-defined semantics, not §12.1 invention; core's reference arm
/// refuses the same document, which is core's own recorded limitation).
#[test]
fn smart_proxy_with_blend_serves_at_render_door() {
    let (mut e, smart) = smart_doc(64, 48);
    apply(
        &mut e,
        &env(
            3,
            "layer.set_props",
            Some(json!({"layerId": smart})),
            json!({"blend": "multiply", "opacity": 0.5}),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("raster-surface semantics serve non-default props");
    // Multiply over transparent backdrop at opacity 0.5 halves coverage.
    assert!(comp.pixel(IVec2::new(8, 8))[3] > 0.0);
    // Core's reference arm (source-over only) refuses — recorded
    // asymmetry: render is the §4.5 owner, core's arm is not.
    assert_eq!(
        export_png_bytes(&e).unwrap_err().code,
        appname_core::ErrorCode::NotImplemented
    );
}

// ---------------------------------------------------------------------
// Dirty consumption (item 4: door-level rects → ROI-correct recompose).
// ---------------------------------------------------------------------

/// Receipt-driven recomposition on a parametric document: apply a chain
/// edit through the door, feed the RECEIPT's dirty rects (R1 ruling
/// shapes: full canvas for the unmasked adjustment; ROI-inflated
/// instance bounds for the chain member) into the render tracker, and
/// pin `recomposite_dirty_with` byte-equal to a fresh full composite.
#[test]
fn receipt_dirty_rects_drive_exact_parametric_recomposition() {
    // Leg 1: unmasked adjustment — R1 full-canvas dirty.
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 320, "height": 280, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.brightness_contrast",
                "newLayerId": "0e140000-0000-4000-8000-0000000000a1",
                "newOpId": "0e140000-0000-4000-8000-0000000000a2",
                "params": {"brightness": 0.0, "contrast": 0.0},
            }),
        ),
    );
    let canvas = e.document().unwrap().geometry().canvas_rect();
    let mut comp = composite_document_with(e.document().unwrap(), e.assembly(), canvas).unwrap();
    let receipt = apply(
        &mut e,
        &env(
            3,
            "op.set_params",
            None,
            json!({
                "opId": "0e140000-0000-4000-8000-0000000000a2",
                "params": {"brightness": -0.25, "contrast": 0.0},
            }),
        ),
    );
    assert!(
        !receipt.dirty.is_empty(),
        "chain edits emit door-level dirty rects (the tripwire-handoff flip)"
    );
    assert!(
        receipt.dirty.iter().any(|d| d.rect == canvas),
        "unmasked adjustment dirties the FULL CANVAS (R1 ruling): {:?}",
        receipt.dirty
    );
    let records: Vec<DirtyRecord> = receipt
        .dirty
        .iter()
        .map(|d| DirtyRecord {
            layer: d.layer,
            rect: d.rect,
        })
        .collect();
    let stats = recomposite_dirty_with(
        &mut comp,
        e.document().unwrap(),
        e.assembly(),
        &DirtyMap::from_records(&records),
    )
    .unwrap();
    assert_eq!(
        stats.tiles_recomposited, stats.tiles_in_composite,
        "full-canvas dirty recomposites every tile"
    );
    let fresh = composite_document_with(e.document().unwrap(), e.assembly(), canvas).unwrap();
    for ((k, t), (fk, ft)) in comp.tiles().zip(fresh.tiles()) {
        assert_eq!(k, fk);
        assert!(t == ft, "recomposited tile {k:?} == fresh full composite");
    }

    // Leg 2: smart-chain member edit — ROI-inflated instance bounds.
    let (mut e2, smart) = smart_doc(320, 280);
    apply(
        &mut e2,
        &env(
            3,
            "filter.smart_add",
            Some(json!({"layerId": smart})),
            json!({
                "filterId": "filter.gaussian_blur",
                "newOpId": "0e140000-0000-4000-8000-0000000000c4",
                "params": {"radius": 1.0},
            }),
        ),
    );
    let canvas2 = e2.document().unwrap().geometry().canvas_rect();
    let mut comp2 =
        composite_document_with(e2.document().unwrap(), e2.assembly(), canvas2).unwrap();
    let receipt2 = apply(
        &mut e2,
        &env(
            4,
            "op.set_params",
            None,
            json!({
                "opId": "0e140000-0000-4000-8000-0000000000c4",
                "params": {"radius": 3.0},
            }),
        ),
    );
    assert!(
        !receipt2.dirty.is_empty(),
        "smart-chain edits emit dirty rects too"
    );
    for d in &receipt2.dirty {
        assert!(
            d.rect.min.x <= 0 && d.rect.min.y <= 0,
            "chain-member dirty is the ROI-INFLATED instance bounds: {:?}",
            d.rect
        );
    }
    let records2: Vec<DirtyRecord> = receipt2
        .dirty
        .iter()
        .map(|d| DirtyRecord {
            layer: d.layer,
            rect: d.rect,
        })
        .collect();
    recomposite_dirty_with(
        &mut comp2,
        e2.document().unwrap(),
        e2.assembly(),
        &DirtyMap::from_records(&records2),
    )
    .unwrap();
    let fresh2 = composite_document_with(e2.document().unwrap(), e2.assembly(), canvas2).unwrap();
    for ((k, t), (fk, ft)) in comp2.tiles().zip(fresh2.tiles()) {
        assert_eq!(k, fk);
        assert!(t == ft, "recomposited tile {k:?} == fresh full composite");
    }
}

// ---------------------------------------------------------------------
// DAG routing (§4.6 engine: delegate parametric, cache plain proxies).
// ---------------------------------------------------------------------

/// The production DAG engine delegates parametric documents to the
/// oracle (bit-equal by construction — pinned anyway), and its stats
/// record the recorded deferral honestly (nothing probed, nothing
/// cached: forced-CPU, uncached).
#[test]
fn dag_renderer_delegates_parametric_bit_equal_and_uncached() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 300, "height": 260, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": "0e140000-0000-4000-8000-0000000000a1",
                "newOpId": "0e140000-0000-4000-8000-0000000000a2",
                "params": {},
            }),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let ops = e.assembly();
    let oracle = composite_document_with(doc, ops, canvas).unwrap();
    let mut r = Renderer::new();
    for _ in 0..2 {
        let dag = r.render_with(doc, ops, canvas).unwrap();
        for ((k, t), (ok, ot)) in dag.tiles().zip(oracle.tiles()) {
            assert_eq!(k, ok);
            assert!(t == ot, "DAG delegation == oracle at {k:?}");
        }
        let s = r.stats();
        assert_eq!(
            (s.root_hits, s.root_misses, s.entries),
            (0, 0, 0),
            "parametric render is uncached (recorded deferral): {s:?}"
        );
    }
}

/// A smart layer with NO enabled chain entries is a plain proxy leaf:
/// the DAG caches it normally (§12.4 structural identity ⇒ ordinary
/// content-hash keys), warm renders hit, bytes stay oracle-equal.
#[test]
fn dag_caches_plain_smart_proxy_leaves() {
    let (e, _) = smart_doc(300, 260);
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let ops = e.assembly();
    let oracle = composite_document_with(doc, ops, canvas).unwrap();
    let mut r = Renderer::new();
    let cold = r.render_with(doc, ops, canvas).unwrap();
    assert!(r.stats().root_misses > 0, "cold render evaluates");
    let warm = r.render_with(doc, ops, canvas).unwrap();
    assert!(
        r.stats().root_hits > 0 && r.stats().root_misses == 0,
        "warm render serves the proxy from cache: {:?}",
        r.stats()
    );
    for (((k, c), (_, w)), (_, o)) in cold.tiles().zip(warm.tiles()).zip(oracle.tiles()) {
        assert!(c == w && w == o, "cached == fresh == oracle at {k:?}");
    }
}

// ---------------------------------------------------------------------
// PARAMETRIC SHAPE/TEXT serve legs (composite-legs slice, O-INT-brokered):
// render gained LayerKind::Shape/Text arms consuming core's resolved
// ComposedPage (Door 1 coverage × FillConvert / Door 6 text page). These
// prove the fix at the render/core level; the shipped-surface probe and
// the single-truth cross-path pin live below.
// ---------------------------------------------------------------------

/// A `doc.new` white doc plus one served shape layer (solid fill, NO visible
/// stroke — the fill-only v1 serve set). Returns (engine, shape uuid).
fn shape_doc(w: u32, h: u32) -> (Engine, String) {
    let mut e = seamed_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": w, "height": h, "background": "white"}),
        ),
    );
    let shape = "0e140000-0000-4000-8000-0000000005a1".to_string();
    apply(
        &mut e,
        &env(
            2,
            "vector.shape_add",
            None,
            json!({
                "path": rect_path(8.0, 8.0, 24.0, 16.0),
                "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0.0, 0.0, 0.0, 1.0]}},
                "stroke": {"widthPx": 0.0},
                "newLayerId": shape,
            }),
        ),
    );
    (e, shape)
}

/// A `doc.new` white doc plus one point-layout text layer (the Door-6
/// serve; missing fonts never fail — §44.2 fallback). Returns (engine, uuid).
fn text_doc(w: u32, h: u32) -> (Engine, String) {
    let mut e = seamed_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": w, "height": h, "background": "white"}),
        ),
    );
    let text = "0e140000-0000-4000-8000-0000000007a1".to_string();
    apply(
        &mut e,
        &env(
            2,
            "text.add",
            None,
            json!({
                "layout": "point",
                "origin": [10.0, 20.0],
                "text": "Ag",
                "newLayerId": text,
            }),
        ),
    );
    (e, text)
}

/// SINGLE-TRUTH CROSS-PATH PIN (the diagnosis' single-truth guarantee):
/// render's parametric composite == core's `composite_plane_f32` on the
/// golden shape + text content. Two levels, both asserted:
///   (a) SHIPPED BYTES byte-identical — render's composite quantized through
///       core's own §6.4/§8.2 helpers equals core's export decoded back
///       (`crosscheck_vs_core_export`, the established render-vs-core pin);
///   (b) the raw linear-premultiplied f32 planes agree within the §34.3
///       ≤1/255 oracle tolerance (the two paths coexist until the follow-on
///       unification and CANNOT diverge past parity — render composites in
///       straight color through its §4.5 kernels, core in premultiplied
///       source-over; the sub-ULP gap is bounded, pinned here).
fn crosspath_pin(engine: &Engine) {
    // (a) shipped-bytes byte-identity (render == core export == the golden).
    crosscheck_vs_core_export(engine);

    // (b) raw f32 plane agreement within the §34.3 gate.
    let doc = engine.document().expect("initialized");
    let canvas = doc.geometry().canvas_rect();
    let (w, h, core_plane) = composite_plane_f32(engine).expect("core plane serves");
    assert_eq!((w as i32, h as i32), (canvas.width(), canvas.height()));
    let comp = composite_document_with(doc, engine.assembly(), canvas).expect("render serves");
    let render_plane = comp.region_rgba_f32(canvas);
    assert_eq!(render_plane.len(), core_plane.len());
    let mut worst = 0.0f32;
    for (r, c) in render_plane.iter().zip(&core_plane) {
        worst = worst.max((r - c).abs() * 255.0);
    }
    assert!(
        worst <= 1.0,
        "render vs core composite_plane_f32 diverged by {worst}/255 (> the §34.3 gate)"
    );
}

/// Shape layer: render's compositor SERVES it (was the frame-12 wall), and
/// the single-truth cross-path pin holds against core.
#[test]
fn shape_layer_serves_and_pins_to_core() {
    let (e, _) = shape_doc(64, 48);
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render composite SERVES the shape layer (composite-legs slice)");
    // The filled rect region is non-transparent over the white backdrop.
    assert_eq!(
        comp.pixel(IVec2::new(16, 14))[3],
        1.0,
        "filled shape is opaque"
    );
    crosspath_pin(&e);
}

/// Text layer: render's compositor SERVES it (was the frame-12 wall — the
/// "layer kind text has no CP-0.5 composite path" refusal), and pins to core.
#[test]
fn text_layer_serves_and_pins_to_core() {
    let (e, _) = text_doc(80, 48);
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    composite_document_with(doc, e.assembly(), canvas)
        .expect("render composite SERVES the text layer (composite-legs slice)");
    crosspath_pin(&e);
}

/// O-UI's EXACT probe re-run (INTEGRATION-log @ fe7c736 / O-UI-log @
/// 2c2e708), on the SHIPPED compositor + export doors: BEFORE the slice a
/// post-shape_add doc refused on BOTH readback and export, and a post-text
/// doc SPLIT them (export served via core, readback refused via render).
/// AFTER: render's composite door (the cabi readback path) AND core's export
/// door BOTH SERVE, for shape AND text — the three-way disagreement is
/// collapsed, readback is pixel-identical to export (crosspath_pin above).
#[test]
fn oui_probe_shape_and_text_serve_on_both_doors() {
    for (label, (e, _)) in [("shape", shape_doc(64, 48)), ("text", text_doc(80, 48))] {
        let doc = e.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        // The readback path (cabi composite.rs calls composite_document_with).
        assert!(
            composite_document_with(doc, e.assembly(), canvas).is_ok(),
            "{label}: the render/readback door now SERVES (was the frame-12 wall)"
        );
        // The export door (core composite_plane_f32 / export_png_bytes).
        assert!(
            has_visible_parametric_content(doc),
            "{label} layer is parametric content"
        );
        assert!(
            export_png_bytes(&e).is_ok(),
            "{label}: the export door SERVES (no readback-vs-export split)"
        );
    }
}

/// FAST-PATH BYTE-EQUALITY PIN (O-INT sharpened step 7): the pure-raster
/// PNG fast path (core `export_png_bytes`) that cabi/cli keep as an
/// optimization is BYTE-IDENTICAL to the render path — at BOTH raster
/// counts. At 1 raster the fast path FIRES in the shipped export doors; at
/// 2 rasters those doors route through render, and core's own walk must
/// still agree. So keeping the fast path can never diverge the shipped
/// export from the compositor truth, and the raster-count dispatch branch
/// is provably a no-op on the exported bytes (the old serve↔refuse flip is
/// gone — the same bytes come out whichever branch is taken).
#[test]
fn pure_raster_fast_path_pins_byte_equal_to_render_at_both_counts() {
    for two in [false, true] {
        let mut e = seamed_engine();
        apply(
            &mut e,
            &env(
                1,
                "doc.new",
                None,
                json!({"width": 64, "height": 48, "background": "white"}),
            ),
        );
        if two {
            apply(
                &mut e,
                &env(
                    2,
                    "layer.add",
                    None,
                    json!({"kind": "raster", "newLayerId": "0e140000-0000-4000-8000-0000000000f2"}),
                ),
            );
        }
        let doc = e.document().unwrap();
        assert!(
            !has_visible_parametric_content(doc),
            "pure-raster fixture (the fast-path domain)"
        );
        let canvas = doc.geometry().canvas_rect();
        // What the shipped export doors run at ≤1 raster (core fast path).
        let core_png = export_png_bytes(&e).unwrap();
        // What they run at ≥2 rasters (render → encode_composite_png), the
        // exact tail the CLI/cabi general path uses.
        let comp = composite_document_with(doc, e.assembly(), canvas).unwrap();
        let plane = comp.region_rgba_f32(canvas);
        let render_png =
            encode_composite_png(canvas.width() as u32, canvas.height() as u32, &plane).unwrap();
        assert_eq!(
            core_png,
            render_png,
            "fast-path (core) == render PNG bytes at {} raster(s)",
            if two { 2 } else { 1 }
        );
    }
}

/// ★ MAJOR-34 PIN — `OpNode::blend` and `OpNode::opacity` are UNREACHABLE
/// through every published command, and the engine refuses them TYPED.
///
/// ES §12.2 says an op's `blend` "blends the op's output over its input",
/// and the spec's `OpNode` field-semantics table attributes both `blend`
/// and `opacity` to `op.set_params`. **`op.set_params` cannot express
/// either**: its `params` is a FULL replacement re-validated against the
/// OP's registered schema, which carries no such keys, and the envelope
/// sets `additionalProperties: false`. No `op.*` command's schema mentions
/// either field — so the fields named by §12.2 have no door at all.
///
/// This pin asserts the REFUSAL, so it REDS the day the gap closes. That is
/// the point: the measurement behind MAJOR-34 otherwise lives only in desk
/// prose, and nothing would announce the change.
///
/// ⚠ IF THIS TEST FAILS, THE FEATURE GAP HAS CLOSED. Update MAJOR-34 in
/// reports/INTEGRATION-log.md and INVERT this test to assert the new
/// behaviour. Do NOT delete it.
#[test]
fn op_blend_and_opacity_have_no_door_major_34() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 32, "height": 32, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": "0e140000-0000-4000-8000-0000000000b1",
                "newOpId": "0e140000-0000-4000-8000-0000000000b2",
                "params": {},
            }),
        ),
    );

    // LIVENESS CONTROL: the door itself must work, or the two refusals
    // below prove nothing about `blend`/`opacity` specifically.
    apply(
        &mut e,
        &env(
            3,
            "op.set_params",
            None,
            json!({"opId": "0e140000-0000-4000-8000-0000000000b2", "params": {}}),
        ),
    );

    for (id, key, value) in [
        (4u32, "blend", json!("multiply")),
        (5u32, "opacity", json!(0.5)),
    ] {
        let err = e
            .apply_json(
                &env(
                    id,
                    "op.set_params",
                    None,
                    json!({
                        "opId": "0e140000-0000-4000-8000-0000000000b2",
                        "params": {key: value},
                    }),
                )
                .to_string(),
            )
            .unwrap_err();
        assert_eq!(
            err.code,
            appname_core::ErrorCode::OutOfRange,
            "§12.2 `{key}` must be refused TYPED by op.set_params, not silently dropped"
        );
        assert!(
            err.message.contains(key),
            "the refusal must name the unreachable field: {key} absent from {:?}",
            err.message
        );
    }
}

/// Build a 32×32 white document carrying one `adjustment.invert` layer.
fn inverted_white_doc() -> (Engine, &'static str) {
    const ADJ: &str = "0e140000-0000-4000-8000-0000000000b1";
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 32, "height": 32, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": ADJ,
                "newOpId": "0e140000-0000-4000-8000-0000000000b2",
                "params": {},
            }),
        ),
    );
    (e, ADJ)
}

fn center(e: &Engine) -> [f32; 4] {
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    composite_document_with(doc, e.assembly(), canvas)
        .expect("serves")
        .pixel(IVec2::new(16, 16))
}

/// FI-13 [NORMATIVE] (ENGINEERING-SPEC :5956-5962): an Adjustment node's own
/// `opacity` folds into the §12.1 lerp weight —
/// `w = node.opacity · Π coverage(node.masks) · opᵢ.opacity · mᵢ` — rather
/// than into a source alpha.
///
/// Backdrop is white (1.0) and `adjustment.invert` takes it to 0.0, so the
/// served channel value IS `1 − w` and reads the weight directly.
///
/// PERTURBATION EVIDENCE: three distinct opacities give three distinct
/// predicted values. A fold that ignored `node.opacity` would return 0.0 at
/// all three; one that applied it twice would return `1 − w²`
/// (0.9375/0.75/0.4375). Both alternatives are excluded by the same table,
/// so this cannot pass vacuously.
#[test]
fn fi13_node_opacity_folds_into_the_lerp_weight() {
    for (i, (op, want)) in [(0.25_f32, 0.75_f32), (0.5, 0.5), (0.75, 0.25)]
        .into_iter()
        .enumerate()
    {
        let (mut e, adj) = inverted_white_doc();
        apply(
            &mut e,
            &env(
                3 + i as u32,
                "layer.set_props",
                Some(json!({"layerId": adj})),
                json!({"opacity": op}),
            ),
        );
        let px = center(&e);
        for c in 0..3 {
            assert!(
                (px[c] - want).abs() < 1e-6,
                "FI-13 w = node.opacity: at opacity {op} channel {c} is {}, want {want}",
                px[c]
            );
        }
        // §11.2 raises α above αb at every non-zero coverage (:5964), which
        // an in-place adjustment must never do. The fold cannot: it returns a
        // value BETWEEN the backdrop and a result computed from it. Stated
        // precisely, because the general claim "alpha never moves" is NOT
        // what holds — an op may change alpha itself. Here `invert` preserves
        // it, so αb == αserved == 1.0 and BOTH endpoints are 1.0; any
        // departure from 1.0 could then only come from the weight, i.e. from
        // the §11.2 route the letter forbids.
        assert!(
            (px[3] - 1.0).abs() < 1e-6,
            "FI-13: the weight must not move alpha when both endpoints carry \
             αb == 1.0; got {} at opacity {op}",
            px[3]
        );
        // Both doors, never a serve↔refuse split.
        export_png_bytes(&e).expect("reference export door serves the same document");
    }
}

/// FI-13 with a full-coverage (`reveal_all`, E = 1) LAYER mask must be the
/// EXACT identity against no layer mask at all — :5964's worked example is
/// precisely this case: "layer-mask coverage E = 1 (i.e. **unmasked**, where
/// the mask must have no effect at all)", and it is where the §11.2 route
/// fails as an IDENTITY failure rather than a rounding one (it would yield
/// 0.3000 at α 0.7500 where in-place requires 0.2000 at α 0.5000).
///
/// PERTURBATION EVIDENCE: the same document at node opacity 0.5 moves the
/// value to 0.5 through the very same weight, so the fold is demonstrably
/// live on this path — the E = 1 identity is not just a dead code path
/// returning its input.
#[test]
fn fi13_full_coverage_layer_mask_is_the_exact_identity() {
    let (unmasked, _) = inverted_white_doc();
    let want = center(&unmasked);

    let (mut e, adj) = inverted_white_doc();
    apply(
        &mut e,
        &env(
            3,
            "mask.add",
            Some(json!({"layerId": adj})),
            json!({"kind": "raster", "from": "reveal_all",
                   "newMaskId": "0e140000-0000-4000-8000-0000000000b3"}),
        ),
    );
    let masked = center(&e);
    assert_eq!(
        masked, want,
        "FI-13/:5964: a reveal_all (E = 1) layer mask must have NO effect at all"
    );
    // The alpha half of the same requirement, stated separately because
    // :5964 calls out alpha specifically.
    assert!(
        (masked[3] - 1.0).abs() < 1e-6,
        "E = 1 must not raise alpha above αb; got {}",
        masked[3]
    );
    export_png_bytes(&e).expect("reference export door serves a masked adjustment");

    // The fold IS live on this path: node.opacity moves the same pixel.
    apply(
        &mut e,
        &env(
            4,
            "layer.set_props",
            Some(json!({"layerId": adj})),
            json!({"opacity": 0.5}),
        ),
    );
    let faded = center(&e);
    assert!(
        (faded[0] - 0.5).abs() < 1e-6,
        "the E = 1 identity is not a dead path: opacity 0.5 gives {}, want 0.5",
        faded[0]
    );
}

/// FI-13 at the OTHER endpoint. :5964: "the two routes agree **only** at
/// `E = 0` — where the adjustment does nothing regardless". A `hide_all`
/// layer mask is `E = 0`, so `w = 0` and the composite must be BIT-identical
/// to the document with no adjustment layer at all.
///
/// PERTURBATION EVIDENCE: the same document without the mask inverts white
/// to black, so the two states are maximally far apart — this assertion
/// cannot pass by both sides being trivially equal.
#[test]
fn fi13_zero_coverage_layer_mask_is_the_backdrop() {
    // Baseline: white document, no adjustment at all.
    let mut base = assembled_engine();
    apply(
        &mut base,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 32, "height": 32, "background": "white"}),
        ),
    );
    let want = center(&base);
    assert!((want[0] - 1.0).abs() < 1e-6, "baseline is white: {want:?}");

    let (mut e, adj) = inverted_white_doc();
    // Without a mask the adjustment inverts white to black — the two states
    // this test distinguishes are as far apart as the range allows.
    assert!(center(&e)[0] < 1e-6, "unmasked adjustment inverts to black");

    apply(
        &mut e,
        &env(
            3,
            "mask.add",
            Some(json!({"layerId": adj})),
            json!({"kind": "raster", "from": "hide_all",
                   "newMaskId": "0e140000-0000-4000-8000-0000000000b4"}),
        ),
    );
    assert_eq!(
        center(&e),
        want,
        "FI-13/:5964: at E = 0 the adjustment does nothing — exactly the backdrop"
    );
    export_png_bytes(&e).expect("reference export door agrees");
}

/// A FEATHERED layer mask on an adjustment refuses typed on BOTH doors —
/// §11.2 Sub-slice B, the same wall every other leaf kind takes, reached
/// through the same shared message. FI-13 lifts the UNFEATHERED layer-mask
/// refusal and must not have widened the serve past Sub-slice A.
#[test]
fn fi13_feathered_layer_mask_serves_as_a_spatial_gradient_on_both_doors() {
    // RENAMED from `..._still_refuses_on_both_doors`. §11.2 Sub-slice B
    // converts this construct from refuse to SERVE, and a test whose NAME
    // asserts a refusal it no longer checks is the stale-rationale defect —
    // an explanation outliving its subject, which no assertion can catch.
    const MASK: &str = "0e140000-0000-4000-8000-0000000000b5";
    const ADJ: &str = "0e140000-0000-4000-8000-0000000000b1";
    // A 128x128 canvas, NOT the shared 32x32 `inverted_white_doc`. MEASURED:
    // at 32x32 a feather of 18 reaches the centre from all four edges and the
    // interior reads 0.0115 rather than 0.0 — the falloff is real but the
    // canvas is too small to separate "feathered at the edge" from "scaled
    // everywhere", which is the discriminator this test exists to provide.
    // THE GEOMETRY IS PART OF THE FIXTURE, not an incidental size.
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 128, "height": 128, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": ADJ,
                "newOpId": "0e140000-0000-4000-8000-0000000000b2",
                "params": {},
            }),
        ),
    );
    let adj = ADJ;
    apply(
        &mut e,
        &env(
            3,
            "mask.add",
            Some(json!({"layerId": adj})),
            json!({"kind": "raster", "from": "reveal_all", "newMaskId": MASK}),
        ),
    );
    // Unfeathered baseline: E = 1 everywhere, so the white backdrop is fully
    // inverted to 0.0 at EVERY sample, corners included.
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let flat = composite_document_with(doc, e.assembly(), canvas).expect("unfeathered serves");
    assert!(
        flat.pixel(IVec2::new(0, 0))[0].abs() < 1e-6,
        "baseline: an unfeathered reveal_all mask inverts the CORNER too"
    );

    apply(
        &mut e,
        &env(
            4,
            "mask.set_props",
            Some(json!({"layerId": adj, "maskId": MASK})),
            json!({"feather": 18.0}),
        ),
    );
    let doc = e.document().unwrap();
    let soft = composite_document_with(doc, e.assembly(), canvas)
        .expect("Sub-slice B: a feathered mask on an adjustment now SERVES");

    // The served channel IS `1 − E` (white backdrop, invert → 0.0), so the
    // pixel reads the feathered coverage directly.
    //
    // A `reveal_all` mask covers the whole canvas, so feathering it blurs
    // INWARD FROM THE CANVAS EDGE: coverage stays 1 in the interior and falls
    // off at the corners. THAT ASYMMETRY IS THE WITNESS — it distinguishes a
    // real spatial feather from the two ways this could pass while broken:
    //   · feather silently DROPPED  → corner still 0.0, identical to `flat`
    //   · feather applied as a GLOBAL scale → centre would move too.
    let centre = soft.pixel(IVec2::new(64, 64))[0];
    let corner = soft.pixel(IVec2::new(0, 0))[0];
    assert!(
        centre.abs() < 1e-6,
        "feather must not move the INTERIOR of a reveal_all mask (a global \
         scale would): centre {centre}"
    );
    assert!(
        corner > 1e-3,
        "feather must reduce coverage at the CORNER — corner {corner} equal to \
         the unfeathered 0.0 would mean feather was silently dropped (QA-F8)"
    );
    assert!(
        corner < 1.0,
        "the corner is FEATHERED, not fully masked out: {corner}"
    );

    // Both doors, never a serve↔refuse split.
    export_png_bytes(&e).expect("the reference export door serves the feathered mask too");
}
// ---------------------------------------------------------------------------
// §22 FILL LAYERS — the FillContent→pixels leg, proved at BOTH doors
// ---------------------------------------------------------------------------
//
// METHOD NOTE (why these live here and not only in a core unit test):
// this lane already shipped a defect where the command door SERVED a fill
// layer while the composite door refused it typed, and an earlier one
// where export SUCCEEDED with the fill layer silently missing. Neither is
// visible from a unit test of the ramp math — both are seam defects. So
// every fill assertion below runs through `crosspath_pin`, which walks
// core's export door AND render's compositor and requires them to AGREE,
// and every one of them checks actual PIXELS rather than just `is_ok()`.

/// A doc.new white canvas with one §22 fill layer on top.
fn fill_doc(w: u32, h: u32, arm: Value) -> Engine {
    let mut e = seamed_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": w, "height": h, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "layer.add",
            None,
            json!({
                "kind": "fill",
                "newLayerId": "0e140000-0000-4000-8000-0000000000fa",
                "kindParams": arm,
            }),
        ),
    );
    e
}

/// A two-stop ramp, black → white, as the `$defs/gradient` wire object.
fn ramp_wire(kind: &str) -> Value {
    json!({
        "kind": kind,
        "stops": [
            {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
            {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}},
        ],
    })
}

/// SOLID fill content composites, at both doors, to the colour asked for.
///
/// The colour assertion is the point. `is_ok()` would have passed while
/// the layer was silently omitted (the QA-F8 defect this lane already
/// hit), and it would also pass for a fill that composited BLACK. Pure
/// red at full alpha over a WHITE background can only be the fill: the
/// backdrop is white, so a missing layer reads (255,255,255) and a
/// transparent one reads white too.
#[test]
fn solid_fill_layer_serves_and_pins_to_core() {
    let e = fill_doc(
        32,
        24,
        json!({"type": "solid", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render composite SERVES a solid fill layer");

    // Every pixel is the fill — a fill layer covers the whole canvas.
    for (x, y) in [(0, 0), (31, 23), (16, 12), (0, 23), (31, 0)] {
        let px = comp.pixel(IVec2::new(x, y));
        assert_eq!(px[3], 1.0, "solid fill is opaque at ({x},{y})");
        assert!(
            px[0] > 0.99 && px[1] < 0.01 && px[2] < 0.01,
            "solid fill must composite RED at ({x},{y}), got {px:?} \
             (white = the layer was omitted)"
        );
    }
    crosspath_pin(&e);
}

/// `none` fill content is a SERVED arm, not a refusal: it composites to
/// source-over identity, so the document looks exactly as if the layer
/// were absent — and both doors agree on that.
#[test]
fn none_fill_layer_serves_as_identity_at_both_doors() {
    let e = fill_doc(16, 16, json!({"type": "none"}));
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render composite SERVES a `none` fill layer");
    // The white doc.new background shows through untouched.
    let px = comp.pixel(IVec2::new(8, 8));
    assert!(
        px[0] > 0.99 && px[1] > 0.99 && px[2] > 0.99 && px[3] == 1.0,
        "`none` must be source-over identity, got {px:?}"
    );
    crosspath_pin(&e);
}

/// A GRADIENT fill layer composites an actual RAMP, oriented per the
/// §22.4 layer-arm geometry, and both doors agree pixel-for-pixel.
///
/// Three separate things are asserted because each fails differently:
///   1. it VARIES (a constant page would mean the ramp was never sampled);
///   2. it varies along Y and NOT along X (`angleDeg` defaults to 90 =
///      "upward", so iso-lines are horizontal) — this catches a
///      transposed axis, which assertion 1 cannot see;
///   3. it is BRIGHTER AT THE TOP (raster y grows downward, and the
///      default 90° axis runs from the bottom of the box to the top, so
///      the LAST stop — white — lands at the top). This catches a
///      sign-flipped axis, which assertions 1 and 2 cannot see.
#[test]
fn gradient_fill_layer_ramps_and_pins_to_core() {
    let e = fill_doc(
        32,
        32,
        json!({"type": "gradient", "gradient": ramp_wire("linear")}),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render composite SERVES a gradient fill layer");

    let lum = |x: i32, y: i32| comp.pixel(IVec2::new(x, y))[0];

    // 1. It varies.
    let (top, bottom) = (lum(16, 0), lum(16, 31));
    assert!(
        (top - bottom).abs() > 0.5,
        "a linear ramp must vary down the canvas: top {top} vs bottom {bottom}"
    );

    // 2. Constant along X (horizontal iso-lines at angleDeg = 90).
    for y in [0, 7, 16, 31] {
        let a = lum(0, y);
        for x in [1, 15, 31] {
            assert!(
                (lum(x, y) - a).abs() < 1e-6,
                "iso-lines must be horizontal at y={y}: x=0 {a} vs x={x} {}",
                lum(x, y)
            );
        }
    }

    // 3. Brighter at the TOP — the 90° axis runs bottom → top.
    assert!(
        top > bottom,
        "angleDeg 90 must put the LAST stop (white) at the top: \
         top {top} vs bottom {bottom}"
    );

    // 4. Monotone down the column (§34.4 property 10, end to end).
    let mut prev = lum(16, 0);
    for y in 1..32 {
        let v = lum(16, y);
        assert!(
            v <= prev + 1e-6,
            "the ramp must be monotone down the column at y={y}: {prev} -> {v}"
        );
        prev = v;
    }

    crosspath_pin(&e);
}

/// §19.1 [the derivation `resolve_parametric_page`'s Fill arm states]: a
/// NON-DESTRUCTIVE crop HIDES pixels, it never changes them. So the
/// gradient's box must be anchored on the CANVAS, not on the crop window
/// — otherwise applying a crop would re-flow the ramp and change the
/// colours inside the region the user KEPT, which is destructive in
/// effect and exactly what §19.1 forbids.
///
/// This test fails if the Fill arm reads `image_rect()` instead of
/// `canvas_rect()`: the retained sub-region's pixels would be re-derived
/// over the smaller box and no longer match.
#[test]
fn fill_gradient_is_invariant_under_a_non_destructive_crop() {
    let arm = json!({"type": "gradient", "gradient": ramp_wire("linear")});
    let crop = json!({"x": 8, "y": 8, "w": 16, "h": 16});

    // Uncropped: read the ramp over the region the crop will retain.
    let before = {
        let e = fill_doc(32, 32, arm.clone());
        let doc = e.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        let comp = composite_document_with(doc, e.assembly(), canvas).expect("serves");
        (0..16)
            .map(|k| comp.pixel(IVec2::new(8 + k % 16, 8 + k)))
            .collect::<Vec<_>>()
    };

    // Cropped non-destructively to that same region.
    let mut e = fill_doc(32, 32, arm);
    apply(
        &mut e,
        &env(
            3,
            "doc.crop",
            None,
            json!({"rect": crop, "deletePixels": false}),
        ),
    );
    let doc = e.document().unwrap();
    let frame = doc.geometry().image_rect();
    assert_eq!(
        (frame.width(), frame.height()),
        (16, 16),
        "the crop is active (otherwise this test proves nothing)"
    );
    let comp = composite_document_with(doc, e.assembly(), frame)
        .expect("render composite SERVES a cropped fill layer");
    let after: Vec<_> = (0..16)
        .map(|k| comp.pixel(IVec2::new(8 + k % 16, 8 + k)))
        .collect();

    assert_eq!(
        before, after,
        "a non-destructive crop must HIDE pixels, not re-flow the ramp — \
         the fill box is the CANVAS, not the crop window (§19.1)"
    );
}

/// §22.5 PATTERN content refuses TYPED at BOTH doors, and neither door
/// silently omits the layer.
///
/// The pairing is the whole point (QA-F8): a refusal on one door plus a
/// success on the other is the serve↔refuse split this lane already
/// shipped once. Both doors route through the same
/// `render_fill_page`, so this pins that they still do.
#[test]
fn pattern_fill_refuses_typed_at_both_doors() {
    let e = fill_doc(
        16,
        16,
        json!({"type": "pattern", "patternRef": "assets/paper"}),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();

    // The layer IS parametric content, so the raster fast path cannot run
    // and silently drop it.
    assert!(
        has_visible_parametric_content(doc),
        "a visible pattern fill layer must route to the parametric arm"
    );

    // Door 1: render's compositor.
    let render_err = composite_document_with(doc, e.assembly(), canvas)
        .expect_err("render must REFUSE pattern content, not invent a tile");

    // Door 2: core's export.
    let core_err = export_png_bytes(&e).expect_err("export must REFUSE pattern content");
    assert_eq!(
        core_err.code,
        appname_core::engine::error::ErrorCode::NotImplemented,
        "the export refusal must be TYPED, got {core_err:?}"
    );

    // And both name the same cause, so a user gets one story.
    let render_msg = format!("{render_err}");
    for needle in ["pattern", "22.5"] {
        assert!(
            render_msg.contains(needle),
            "render refusal must name {needle:?}: {render_msg}"
        );
        assert!(
            core_err.message.contains(needle),
            "core refusal must name {needle:?}: {}",
            core_err.message
        );
    }
}

/// A `noise` gradient is in the frozen §22.4 enum but carries no
/// parameters to synthesize from — it refuses TYPED at both doors rather
/// than being aliased onto another kind (which would render a plausible
/// wrong picture, the worst outcome).
#[test]
fn noise_gradient_fill_refuses_typed_at_both_doors() {
    let e = fill_doc(
        16,
        16,
        json!({"type": "gradient", "gradient": ramp_wire("noise")}),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    assert!(
        composite_document_with(doc, e.assembly(), canvas).is_err(),
        "render must REFUSE a noise gradient, not alias it to linear"
    );
    let core_err = export_png_bytes(&e).expect_err("export must REFUSE a noise gradient");
    assert_eq!(
        core_err.code,
        appname_core::engine::error::ErrorCode::NotImplemented,
        "typed refusal, got {core_err:?}"
    );
    assert!(
        core_err.message.contains("noise"),
        "the refusal must name the kind: {}",
        core_err.message
    );
}

/// D-48: two composites of the same fill document are BYTE-IDENTICAL.
/// The §22.4 ramp is pure in (params, box), and the perceptual space runs
/// `cbrt`/`powf` per pixel — a path where a non-deterministic fast-math
/// or a cached-state bug would show up as drift between runs.
#[test]
fn fill_gradient_replay_is_byte_identical() {
    for space in ["linearRgb", "perceptual"] {
        let mut g = ramp_wire("radial");
        g["space"] = json!(space);
        let arm = json!({"type": "gradient", "gradient": g, "angleDeg": 33.0, "scale": 1.5});
        let one = export_png_bytes(&fill_doc(24, 20, arm.clone())).expect("serves");
        let two = export_png_bytes(&fill_doc(24, 20, arm)).expect("serves");
        assert_eq!(
            one, two,
            "{space}: fill export must be byte-identical (D-48)"
        );
    }
}

/// PERCEPTUAL interpolation leaves the sRGB gamut, and the two doors must
/// still agree there.
///
/// This is not hypothetical: Oklab-interpolating saturated blue → saturated
/// yellow produces NEGATIVE linear components (measured: about −0.0067 at
/// the worst sample). That matters here because the two doors clamp at
/// different places — core's export clamps inside its sRGB encode, while
/// render blends premultiplied linear through its own kernels and clamps
/// later. An out-of-gamut page is exactly the input that would expose a
/// divergence between them, and no in-gamut fixture can.
///
/// `render_fill_page` deliberately does NOT clamp: the engine's working
/// space is unbounded linear f32 everywhere else, and gamut mapping is a
/// §5/§6 colour decision, not something the §22.4 ramp may quietly make.
/// This test pins that the choice is safe at both doors rather than
/// merely assumed to be.
#[test]
fn perceptual_out_of_gamut_gradient_agrees_across_both_doors() {
    let e = fill_doc(
        24,
        24,
        json!({"type": "gradient", "gradient": {
            "kind": "linear",
            "space": "perceptual",
            "stops": [
                {"pos": 0, "color": {"model": "rgb", "comps": [0, 0, 1, 0]}},
                {"pos": 1, "color": {"model": "rgb", "comps": [1, 1, 0, 0]}},
            ],
        }}),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render composite SERVES a perceptual gradient");

    // It really is a blue→yellow ramp and not a constant.
    let top = comp.pixel(IVec2::new(12, 0));
    let bottom = comp.pixel(IVec2::new(12, 23));
    assert!(
        top[0] > bottom[0] + 0.3,
        "the ramp must run blue (bottom) -> yellow (top): {bottom:?} -> {top:?}"
    );

    // THE POINT: both doors agree bit-strict after the shared quantization.
    crosspath_pin(&e);
}

/// ROUND-2 SEAM: a document whose ONLY visible content is a fill layer.
///
/// Every other fixture here sits a fill layer on top of `doc.new`'s
/// opaque background raster, so the accumulator always had a raster
/// source to anchor on and an opaque backdrop to hide arithmetic against.
/// On a TRANSPARENT background the fill page is the sole contributor —
/// which is the case that would expose an ROI derived from raster sources
/// only, or a page that composites correctly solely because something
/// opaque was already underneath it.
///
/// Half-alpha is used deliberately: over transparent, the result must be
/// the premultiplied half-alpha fill itself, so a page that was stored
/// STRAIGHT (or double-premultiplied) reads wrong here even though it
/// would look right over an opaque white backdrop.
#[test]
fn a_fill_layer_alone_over_transparent_composites_at_both_doors() {
    let mut e = seamed_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 16, "height": 16, "background": "transparent"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "layer.add",
            None,
            json!({
                "kind": "fill",
                "newLayerId": "0e140000-0000-4000-8000-0000000000fb",
                "kindParams": {
                    "type": "solid",
                    "color": {"model": "rgba", "comps": [1, 0, 0, 0.5]},
                },
            }),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("render composites a lone fill layer over transparent");

    let px = comp.pixel(IVec2::new(8, 8));
    assert!(
        (px[3] - 0.5).abs() < 1e-6,
        "alpha must be the fill's own 0.5 over transparent, got {px:?}"
    );
    assert!(
        (px[0] - 0.5).abs() < 1e-6 && px[1].abs() < 1e-6 && px[2].abs() < 1e-6,
        "premultiplied half-alpha red is (0.5, 0, 0, 0.5); straight storage \
         would read 1.0 in red: {px:?}"
    );
    crosspath_pin(&e);
}

// ---------------------------------------------------------------------
// RULING 2 (2026-08-28) — node.blend on an ADJUSTMENT node, served as the
// two-stage engine-binding interpretation:
//   stage 1   the §12.1 chain, verbatim, producing `net`
//   stage 2a  blend(in0, net, node.blend), COLOUR-ONLY, alpha = alpha0
//   stage 2b  the FI-13 fold `w = node.opacity · Π coverage · op.opacity · m`
// These two tests are the goldens the lane owed. Recorded as an
// INTERPRETATION, not a spec edit — see INTEGRATION-log 211.
// ---------------------------------------------------------------------

/// A red fill under an `adjustment.invert`, so the adjustment's own output
/// (`net` = cyan) differs from its backdrop (`in0` = red) in every channel.
/// Both doors see the same document.
fn inverted_red_fill_doc() -> (Engine, &'static str) {
    const ADJ: &str = "0e140000-0000-4000-8000-0000000000c1";
    let mut e = fill_doc(
        32,
        24,
        json!({"type": "solid", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
    );
    apply(
        &mut e,
        &env(
            3,
            "adjustment.add",
            None,
            json!({
                "opKind": "adjustment.invert",
                "newLayerId": ADJ,
                "newOpId": "0e140000-0000-4000-8000-0000000000c2",
                "params": {},
            }),
        ),
    );
    (e, ADJ)
}

/// RULING 2, STAGE 2a AT `Normal`: THE COLLAPSE IS STRUCTURAL, NOT NUMERIC.
///
/// `blend_node_unit` returns its input unchanged when `adj.blend ==
/// BlendMode::Normal` — an early return taken BEFORE any arithmetic, at both
/// doors. So a document that sets `"blend": "normal"` explicitly must be
/// BYTE-IDENTICAL to one that never mentions blend at all: not "equal within
/// 1e-6", identical.
///
/// This is the golden that replaces a float. The lane previously leaned on a
/// measured `4.441e-16` max deviation over the unit square — a number from an
/// UNSEEDED sample, recorded without its seed, which could not be reproduced
/// and which stopped carrying the claim the moment ruling 2 made the collapse
/// structural. An exact byte comparison asserts the property instead of
/// sampling it (INTEGRATION-log 209.1, G-501's sibling lesson).
///
/// PERTURBATION EVIDENCE — the third arm is what stops this passing vacuously.
/// If `blend_node_unit` were never reached at all (mis-threaded routing, a
/// dropped `blend` field on the wire), arms 1 and 2 would BOTH still be
/// byte-identical and the test would pass while serving nothing. Arm 3 sets
/// `multiply` and requires the bytes to MOVE, so the seam is proven live on
/// the very path the identity is claimed for.
#[test]
fn ruling2_normal_blend_is_bit_identical_to_no_blend_at_both_doors() {
    // Arm 1: blend never mentioned.
    let (bare, _) = inverted_red_fill_doc();
    let doc = bare.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let bare_comp = composite_document_with(doc, bare.assembly(), canvas).expect("serves");
    let bare_png = export_png_bytes(&bare).expect("export door serves");

    // Arm 2: blend set EXPLICITLY to normal.
    let (mut named, adj) = inverted_red_fill_doc();
    apply(
        &mut named,
        &env(
            4,
            "layer.set_props",
            Some(json!({"layerId": adj})),
            json!({"blend": "normal"}),
        ),
    );
    let doc = named.document().unwrap();
    let named_comp = composite_document_with(doc, named.assembly(), canvas).expect("serves");
    let named_png = export_png_bytes(&named).expect("export door serves");

    for (k, t) in named_comp.tiles() {
        let (_, bare_t) = bare_comp.tiles().find(|(pk, _)| *pk == k).unwrap();
        assert!(
            t == bare_t,
            "RULING 2: `blend: normal` must be the EXACT identity at the render \
             door, tile {k:?} — the Normal arm is an early return, so any \
             difference at all means arithmetic ran that should not have"
        );
    }
    assert_eq!(
        named_png, bare_png,
        "RULING 2: `blend: normal` must be BYTE-IDENTICAL at the export door too"
    );

    // Arm 3 (PERTURBATION): a real blend must MOVE the bytes, or arms 1-2
    // proved only that a dead seam is consistently dead.
    let (mut mult, adj) = inverted_red_fill_doc();
    apply(
        &mut mult,
        &env(
            4,
            "layer.set_props",
            Some(json!({"layerId": adj})),
            json!({"blend": "multiply"}),
        ),
    );
    let doc = mult.document().unwrap();
    let mult_comp = composite_document_with(doc, mult.assembly(), canvas).expect("serves");
    let moved_at_render = mult_comp.tiles().any(|(k, t)| {
        let (_, bare_t) = bare_comp.tiles().find(|(pk, _)| *pk == k).unwrap();
        t != bare_t
    });
    assert!(
        moved_at_render,
        "RULING 2: `multiply` must change the served bytes at the RENDER door. \
         Checking only the export door leaves render's stage 2a unwitnessed by \
         this test — measured: with render's Normal check forced true, this test \
         still passed while the Luminosity witness failed alone."
    );
    let mult_png = export_png_bytes(&mult).expect("export door serves a multiply-blended node");
    assert_ne!(
        mult_png, bare_png,
        "RULING 2: `multiply` must change the served bytes — if it does not, \
         stage 2a is not on this path and the Normal identity above is vacuous"
    );
}

/// RULING 2, STAGE 2a AT A NON-SEPARABLE KERNEL: the Luminosity witness.
///
/// `Luminosity = SetLum(Cb, Lum(Cs))` (blend.rs A-row 26) routes through the
/// TRIPLE kernel arm, not the separable one, so it witnesses the branch a
/// separable-only implementation would miss entirely.
///
/// Backdrop `in0` is opaque red (1,0,0); `adjustment.invert` makes `net` cyan
/// (0,1,1). With `lum = 0.3R + 0.59G + 0.11B`:
///   Lum(cyan) = 0.70; SetLum(red, 0.70) shifts to (1.4, 0.4, 0.4), which
///   clips (x = 1.4 > 1) to (1.0, 4/7, 4/7).
/// Predicted served colour: (1.0, 0.571429, 0.571429).
///
/// PERTURBATION EVIDENCE: that triple is distinct from BOTH degenerate
/// answers in every channel — the backdrop (1,0,0) if stage 2a were skipped,
/// and the un-blended chain output (0,1,1) if `node.blend` were ignored. A
/// separable misroute cannot land here either. THE VALUE IDENTIFIES THE PATH,
/// which is what a witness is for.
///
/// And it pins the COLOUR-ONLY half of stage 2a: alpha must remain alpha0 =
/// 1.0. A blend that composited alpha as well would not leave it exactly 1.0
/// here, because Luminosity is applied to unpremultiplied colour and the
/// re-premultiply uses alpha0 by construction.
#[test]
fn ruling2_luminosity_witness_blends_colour_only_and_preserves_backdrop_alpha() {
    let (mut e, adj) = inverted_red_fill_doc();
    apply(
        &mut e,
        &env(
            4,
            "layer.set_props",
            Some(json!({"layerId": adj})),
            json!({"blend": "luminosity"}),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("RULING 2: a Luminosity-blended adjustment SERVES at the render door");

    const WANT: [f32; 3] = [1.0, 4.0 / 7.0, 4.0 / 7.0];
    for (x, y) in [(0, 0), (31, 23), (16, 12)] {
        let px = comp.pixel(IVec2::new(x, y));
        for c in 0..3 {
            assert!(
                (px[c] - WANT[c]).abs() < 1e-5,
                "RULING 2 stage 2a Luminosity at ({x},{y}) channel {c}: got {}, \
                 want {} — (1,0,0) would mean stage 2a was skipped, (0,1,1) would \
                 mean node.blend was ignored",
                px[c],
                WANT[c]
            );
        }
        assert!(
            (px[3] - 1.0).abs() < 1e-6,
            "RULING 2 stage 2a is COLOUR-ONLY: alpha must stay alpha0 = 1.0 at \
             ({x},{y}), got {}",
            px[3]
        );
    }

    // Never a serve↔refuse split: the export door serves the same document —
    // and `crosspath_pin` holds core to the SAME VALUES, pixel for pixel, not
    // merely to serving. Without it this test kills only the render door: the
    // core arm could return its input unblended and the export-door call would
    // still succeed. (Measured — INTEGRATION-log 211.2.)
    crosspath_pin(&e);
}

// ---------------------------------------------------------------------
// LANE (c) pro/shape-stroke, merged 2026-08-28 AT FUNCTION GRANULARITY.
//
// The textual merge produced SIX conflict hunks and THEY ARE NOT FUNCTION-ALIGNED:
// git interleaved main's `fill_doc(w, h, arm)` with the lane's `stroked_shape_doc`
// because the two helpers are adjacent and textually similar. A keep-both applied at
// HUNK level would have SPLICED TWO DISTINCT FUNCTION BODIES TOGETHER.
//
// The function sets ARE disjoint -- 21 HEAD-side names vs 8 lane-side, intersection 0,
// measured before resolving -- so keep-both is correct AT THE FUNCTION LEVEL and wrong
// at the hunk level. NAME-DISJOINTNESS IS NECESSARY AND NOT SUFFICIENT; the granularity
// of the CHECK must match the granularity of the EDIT.
// ---------------------------------------------------------------------

/// THE FLIP: a visible stroke composites, on the render door and the
/// export door, and render still pins bit-strict to core.
#[test]
fn a_visible_shape_stroke_serves_on_both_doors_and_pins_to_core() {
    let e = stroked_shape_doc(red_stroke(6.0, "center"), blue());
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas)
        .expect("the render door SERVES a stroked shape (was the typed wall)");
    assert!(
        export_png_bytes(&e).is_ok(),
        "the export door serves it too — no readback-vs-export split"
    );
    // Rect (16,12)-(48,36); a 6 px CENTERED stroke straddles the top edge
    // y = 12, so it owns pixel ROWS 9..=14 (the band is y ∈ [9, 15)).
    // Rows 9..=11 are band-over-GROUND, rows 12..=14 are band-over-FILL,
    // row 15 is the first fill-only row — four distinct answers, so the
    // leg cannot pass on a stroke that painted nothing, painted
    // everything, or landed on the wrong side of the edge.
    let px = |x, y| comp.pixel(IVec2::new(x, y));
    assert_eq!(
        px(32, 10)[0..3],
        [1.0, 0.0, 0.0],
        "band outside the fill edge"
    );
    assert_eq!(
        px(32, 13)[0..3],
        [1.0, 0.0, 0.0],
        "band COVERS the fill it overlaps (§20.2 paint order: stroke over fill)"
    );
    assert_eq!(
        px(32, 15)[0..3],
        [0.0, 0.0, 1.0],
        "first row past the band is fill"
    );
    assert_eq!(px(32, 24)[0..3], [0.0, 0.0, 1.0], "shape interior is blue");
    assert_eq!(px(32, 5)[0..3], [1.0, 1.0, 1.0], "outside is the ground");
    assert_eq!(
        px(32, 8)[0..3],
        [1.0, 1.0, 1.0],
        "the row just above the band is ground"
    );
    crosspath_pin(&e);
}

fn blue() -> Value {
    json!({"type": "solid", "color": {"model": "rgba", "comps": [0.0, 0.0, 1.0, 1.0]}})
}

/// THE REGRESSION PIN THAT MATTERS: every spelling of "no visible stroke"
/// exports the SAME BYTES — the fill-only page the door produced before
/// this leg existed. The contract's own `shape_add` validMinimal carries
/// `stroke: {widthPx: 0}`, so this is the shape every shipped fixture,
/// golden and tour frame actually holds.
#[test]
fn no_visible_stroke_leaves_the_fill_only_bytes_untouched() {
    let baseline = export_png_bytes(&stroked_shape_doc(json!({"widthPx": 0}), blue()))
        .expect("fill-only serves");
    for stroke in [
        json!({}),
        json!({"widthPx": 0.0}),
        json!({"widthPx": 12, "fill": {"type": "none"}}),
        // A dash/profile payload on a ZERO-width stroke is still no
        // stroke — it must not wake the typed wall either.
        json!({"widthPx": 0, "dashes": [4, 2], "profile": [[0, 1], [1, 0.5]]}),
    ] {
        let got = export_png_bytes(&stroked_shape_doc(stroke.clone(), blue()))
            .expect("no-visible-stroke serves");
        assert_eq!(got, baseline, "byte drift on {stroke}");
    }
    // And a VISIBLE stroke is genuinely different bytes — the pin above
    // is not passing because everything is equal.
    let stroked =
        export_png_bytes(&stroked_shape_doc(red_stroke(6.0, "center"), blue())).expect("serves");
    assert_ne!(stroked, baseline, "a visible stroke must move pixels");
}

fn red_stroke(width: f64, align: &str) -> Value {
    json!({"widthPx": width,
           "color": {"model": "rgba", "comps": [1.0, 0.0, 0.0, 1.0]},
           "align": align})
}

/// A white doc + one shape layer carrying `stroke` verbatim.
fn stroked_shape_doc(stroke: Value, fill: Value) -> Engine {
    let mut e = seamed_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 64, "height": 48, "background": "white"}),
        ),
    );
    apply(
        &mut e,
        &env(
            2,
            "vector.shape_add",
            None,
            json!({
                "path": rect_path(16.0, 12.0, 32.0, 24.0),
                "fill": fill,
                "stroke": stroke,
                "newLayerId": "0e140000-0000-4000-8000-0000000005b1",
            }),
        ),
    );
    e
}

/// The stroke is part of the layer's OWN page, so layer opacity/blend fade
/// the stroked shape as ONE source (a stroke is not a second layer).
///
/// PROBED WHERE IT DISCRIMINATES — row 13, where the band OVERLAPS the
/// fill. One page: red wins the page outright, then the whole page fades,
/// so the answer is `0.5·red + 0.5·white = (1, 0.5, 0.5)`. Two sibling
/// leaves at 0.5 each would composite the band edge TWICE —
/// `0.5·red + 0.5·(0.5·blue + 0.5·white) = (0.75, 0.25, 0.5)` — a
/// different pixel. Probing the band OUTSIDE the fill would not tell the
/// two apart.
#[test]
fn the_stroke_rides_the_layer_page_not_a_second_leaf() {
    let mut e = stroked_shape_doc(red_stroke(6.0, "center"), blue());
    apply(
        &mut e,
        &env(
            3,
            "layer.set_props",
            Some(json!({"layerId": "0e140000-0000-4000-8000-0000000005b1"})),
            json!({"opacity": 0.5}),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document_with(doc, e.assembly(), canvas).expect("serves");
    let p = comp.pixel(IVec2::new(32, 13));
    assert!(
        (p[0] - 1.0).abs() < 1e-5 && (p[1] - 0.5).abs() < 1e-5 && (p[2] - 0.5).abs() < 1e-5,
        "ONE page: red@50% over white = (1, 0.5, 0.5) in linear — a second \
         leaf would give (0.75, 0.25, 0.5); got {p:?}"
    );
    // NO `crosspath_pin` here, deliberately: a non-default `opacity` is a
    // compositing prop the reference EXPORT door refuses upstream by
    // standing design (§4.4-only; the §4.5/§4.7 kernels are the render
    // row), so the two doors are not comparable on this document. The
    // cross-door pin rides the default-props sibling test above.
    assert!(
        export_png_bytes(&e).is_err(),
        "the export door still refuses non-default props (pin unchanged by this leg)"
    );
}

/// §20.2 rule 5: the three alignments put the SAME width band in three
/// different places relative to the fill edge. Probed at the pixel that
/// discriminates them — 4 px outside the top edge at y = 10, which only
/// an `outside` band reaches.
#[test]
fn the_three_stroke_alignments_land_the_band_in_three_places() {
    let plane = |align: &str| {
        let e = stroked_shape_doc(red_stroke(8.0, align), blue());
        let doc = e.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        let comp = composite_document_with(doc, e.assembly(), canvas).expect("serves");
        [
            comp.pixel(IVec2::new(32, 10))[0..3].to_vec(), // 2 px OUTSIDE
            comp.pixel(IVec2::new(32, 14))[0..3].to_vec(), // 2 px INSIDE
            comp.pixel(IVec2::new(32, 24))[0..3].to_vec(), // deep interior
        ]
    };
    let red = vec![1.0, 0.0, 0.0];
    let blue_v = vec![0.0, 0.0, 1.0];
    let white = vec![1.0, 1.0, 1.0];
    // inside: band = [12, 20) ⇒ outside probe is ground, inside probe red.
    assert_eq!(
        plane("inside"),
        [white.clone(), red.clone(), blue_v.clone()]
    );
    // center: band = [8, 16) ⇒ BOTH probes red.
    assert_eq!(plane("center"), [red.clone(), red.clone(), blue_v.clone()]);
    // outside: band = [4, 12) ⇒ outside probe red, inside probe fill.
    assert_eq!(plane("outside"), [red, blue_v.clone(), blue_v]);
}

/// QA-F8 on the arms this slice does NOT compute: refuse typed on BOTH
/// doors with the SAME code, naming the §20.2 rule that is missing —
/// never a silent omission, never a serve↔refuse split.
#[test]
fn the_unserved_stroke_arms_refuse_typed_on_both_doors() {
    for (label, stroke) in [
        ("dashes", json!({"widthPx": 4, "dashes": [6, 3]})),
        (
            "profile",
            json!({"widthPx": 4, "profile": [[0, 1], [1, 0.4]]}),
        ),
        (
            "gradient paint",
            json!({"widthPx": 4, "fill": {"type": "gradient",
                   "gradient": {"kind": "linear", "stops": [
                       {"pos": 0, "color": {"model": "rgba", "comps": [1, 0, 0, 1]}},
                       {"pos": 1, "color": {"model": "rgba", "comps": [0, 0, 1, 1]}}]}}}),
        ),
    ] {
        let e = stroked_shape_doc(stroke, blue());
        let doc = e.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        let render = composite_document_with(doc, e.assembly(), canvas)
            .expect_err(&format!("{label}: render refuses"));
        assert!(
            matches!(render, CompositeError::ParametricNotImplemented { .. }),
            "{label}: typed refusal, got {render:?}"
        );
        let export = export_png_bytes(&e).expect_err(&format!("{label}: export refuses"));
        assert_eq!(
            export.code.as_str(),
            "error.command.not_implemented",
            "{label}: the export door refuses with the same code"
        );
    }
}
