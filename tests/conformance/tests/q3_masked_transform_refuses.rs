// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! q-scenario/Q-3 regression — a `layer.transform` on a MASKED layer must
//! REFUSE typed rather than bake the layer's pixels while leaving its mask in
//! canvas space (which composites wrong pixels silently, ok:true, on both
//! export paths — the exact §11.2 Sub-slice C outcome the refusal exists to
//! prevent).
//!
//! Driven through the SHIPPED assembly (`assembly_point::assembled_engine`)
//! because `layer.transform` lives in appname-tools while `mask.add` lives in
//! appname-core — only the production composition carries both. The two
//! remaining §11.2 deferrals (masked-group / masked-adjustment) are asserted
//! to STAY refusing; the third former sibling, FEATHER, is now SERVED
//! (Sub-slice B) and is asserted to composite rather than wall. The unmasked
//! transform is confirmed to serve and bake exactly as before (byte-stability
//! pin).
//!
//! FULL CLASS-KILL: the same guard is enforced on EVERY masked-layer transform
//! bake door — the scriptable `layer.transform` AND the interactive
//! `transform.free` / `transform.perspective` / `transform.warp` doors (all of
//! which bake via `resolve_transform_target`). The interactive battery below
//! mirrors the scriptable one: masked → refuse, unmasked → serve, identity /
//! disabled-mask → exempt.

use appname_conformance::assembly_point;
use appname_core::{decode_png_to_buffer, export_png_bytes, Engine, IVec2, ProfileId};
use serde_json::{json, Value};

const L: &str = "22b30000-0000-4000-8000-0000000000c1";
const M: &str = "22b30000-0000-4000-8000-0000000000d1";

fn cmd(e: &mut Engine, n: u32, kind: &str, target: Option<Value>, params: Value) {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("22b30000-0000-4000-8000-{n:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    e.apply_json(&v.to_string())
        .unwrap_or_else(|err| panic!("{kind} -> {:?} {}", err.code, err.message));
}

/// `layer.transform` on layer `L`, returning the raw command result.
fn try_transform(e: &mut Engine, n: u32, matrix: [f64; 9]) -> Result<(), (String, String)> {
    let v = json!({
        "schema": "appname.command.v1",
        "id": format!("22b30000-0000-4000-8000-{n:012x}"),
        "kind": "layer.transform",
        "target": {"layerId": L},
        "params": {"matrix": matrix.to_vec()},
    });
    e.apply_json(&v.to_string())
        .map(|_| ())
        .map_err(|err| (err.code.as_str().to_string(), err.message))
}

/// A 200×120 white doc with a full-canvas BLUE raster layer `L`, masked to the
/// rect x∈[20,80) y∈[20,80) (authored `from: selection`, the normal path).
fn masked_blue_doc() -> Engine {
    let mut e = assembly_point::assembled_engine();
    cmd(
        &mut e,
        1,
        "doc.new",
        None,
        json!({"width": 200, "height": 120, "background": "white"}),
    );
    cmd(
        &mut e,
        2,
        "layer.add",
        None,
        json!({"kind": "raster", "name": "art", "newLayerId": L}),
    );
    cmd(
        &mut e,
        3,
        "fill",
        Some(json!({"layerId": L})),
        json!({"content": "color", "color": {"model": "rgba", "comps": [0, 0, 1, 1]}}),
    );
    cmd(
        &mut e,
        4,
        "selection.set",
        None,
        json!({"source": "rect", "rect": {"x": 20, "y": 20, "w": 60, "h": 60}}),
    );
    cmd(
        &mut e,
        5,
        "mask.add",
        Some(json!({"layerId": L})),
        json!({"kind": "raster", "newMaskId": M, "from": "selection"}),
    );
    cmd(&mut e, 6, "selection.none", None, json!({}));
    e
}

/// Same doc WITHOUT the mask (full-canvas blue over white).
fn unmasked_blue_doc() -> Engine {
    let mut e = assembly_point::assembled_engine();
    cmd(
        &mut e,
        1,
        "doc.new",
        None,
        json!({"width": 200, "height": 120, "background": "white"}),
    );
    cmd(
        &mut e,
        2,
        "layer.add",
        None,
        json!({"kind": "raster", "name": "art", "newLayerId": L}),
    );
    cmd(
        &mut e,
        3,
        "fill",
        Some(json!({"layerId": L})),
        json!({"content": "color", "color": {"model": "rgba", "comps": [0, 0, 1, 1]}}),
    );
    e
}

/// The half-open blue run `[start, end)` on scanline `y` of the exported PNG.
fn blue_window(e: &Engine, y: i32) -> (i32, i32) {
    let png = export_png_bytes(e).expect("export serves");
    let (buf, w, _h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    let is_blue = |p: [f32; 4]| p[2] > 0.9 && p[0] < 0.1 && p[3] > 0.9;
    let mut start = None;
    let mut end = 0;
    for x in 0..w as i32 {
        if is_blue(buf.read_pixel_f32(IVec2::new(x, y))) {
            if start.is_none() {
                start = Some(x);
            }
            end = x + 1;
        }
    }
    (start.unwrap_or(0), end)
}

/// The §18.3 arm battery: the exact integer-translate arm plus general
/// resample arms (scale up / down, shear, 90° rotate — all exact-valued so the
/// battery stays platform-portable). EVERY arm is a non-identity transform and
/// must refuse on a masked layer.
fn arms() -> Vec<(&'static str, [f64; 9])> {
    vec![
        // column-major [m00,m10,m20, m01,m11,m21, tx,ty,1]
        (
            "translate(60,0) — §18.3 exact",
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 60.0, 0.0, 1.0],
        ),
        (
            "scale(2,2) — §18.3 general",
            [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0],
        ),
        (
            "scale(0.5,0.5) — general minify",
            [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 1.0],
        ),
        (
            "shear(0.3) — general",
            [1.0, 0.0, 0.0, 0.3, 1.0, 0.0, 0.0, 0.0, 1.0],
        ),
        (
            "rotate(90°) — general",
            [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        ),
    ]
}

/// Q-3 headline: a non-identity transform on a masked layer refuses typed on
/// EVERY §18.3 arm — parameterized over the arm battery.
#[test]
fn q3_masked_layer_transform_refuses_on_every_arm() {
    for (i, (label, matrix)) in arms().into_iter().enumerate() {
        let mut e = masked_blue_doc();
        let err = try_transform(&mut e, 100 + i as u32, matrix)
            .expect_err(&format!("{label}: masked transform must refuse"));
        assert_eq!(
            err.0, "error.command.not_implemented",
            "{label}: typed refusal (got {err:?})"
        );
        assert!(
            err.1.contains("mask"),
            "{label}: the refusal names the mask (got {})",
            err.1
        );
        // The refusal is honest: the mask is still there, unmoved.
        assert_eq!(
            e.document().unwrap().mask_count(),
            1,
            "{label}: the mask survives the refusal"
        );
        // And the composite still serves the ORIGINAL masked window — the
        // wrong-pixel serve never happened.
        assert_eq!(
            blue_window(&e, 50),
            (20, 80),
            "{label}: composite unchanged after refusal"
        );
    }
}

/// Q-3 exemption: an IDENTITY `layer.transform` is a no-op, not a refusal (no
/// pixel change ⇒ no mask desync) — the guard must not over-fire.
#[test]
fn q3_identity_transform_on_masked_layer_is_a_noop() {
    let mut e = masked_blue_doc();
    try_transform(&mut e, 200, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
        .expect("identity transform on a masked layer is a no-op, never a refusal");
    assert_eq!(
        blue_window(&e, 50),
        (20, 80),
        "identity leaves the composite intact"
    );
}

/// The UNMASKED transform is unaffected — it still serves and bakes exactly:
/// a `translate(60,0)` moves the full-canvas blue so the vacated columns clear
/// to the white background. Byte-stability pin for the correct case (my guard
/// is gated on an enabled mask, so this path is untouched by construction).
#[test]
fn q3_unmasked_transform_still_serves_and_bakes() {
    let mut e = unmasked_blue_doc();
    // Baseline: full-canvas blue.
    assert_eq!(blue_window(&e, 50), (0, 200), "baseline is full-width blue");
    try_transform(&mut e, 300, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 60.0, 0.0, 1.0])
        .expect("unmasked translate still serves (ok:true)");
    // Baked: content shifted +60, vacated [0,60) cleared to white bg.
    assert_eq!(
        blue_window(&e, 50),
        (60, 200),
        "unmasked translate bakes exactly as before the fix"
    );
}

/// Attach a `raster` mask to `target_layer`, then confirm the masked construct
/// is REFUSED typed — at whichever door walls it: either `mask.add` itself
/// refuses, or (if the mask attaches) the composite/export refuses. Sub-slice C
/// siblings are "refused typed on BOTH paths", so a served export is the bug.
fn assert_masked_construct_refused(e: &mut Engine, n: u32, target_layer: &str, what: &str) {
    let add = e.apply_json(
        &json!({
            "schema": "appname.command.v1",
            "id": format!("22b30000-0000-4000-8000-{n:012x}"),
            "kind": "mask.add",
            "target": {"layerId": target_layer},
            "params": {"kind": "raster", "newMaskId": M},
        })
        .to_string(),
    );
    match add {
        // The command door walls it — a typed refusal at mask.add.
        Err(err) => assert_eq!(
            err.code.as_str(),
            "error.command.not_implemented",
            "{what}: mask.add refuses typed"
        ),
        // The mask attaches — then the composite/export door MUST wall it.
        Ok(_) => {
            export_png_bytes(e).expect_err(&format!(
                "{what}: the masked construct must wall the export"
            ));
        }
    }
}

/// The §11.2 sibling constructs, RECONCILED across two lanes that each
/// converted one of them from refuse to serve:
///
/// * masked GROUP — STILL REFUSES (§4.7 row 4; neither lane touched it),
/// * masked ADJUSTMENT — SERVES, per FI-13 [NORMATIVE] (:5956-5962, frozen
///   at `v5.6-freeze`): the node's own `masks` and `opacity` fold into
///   §12.1's lerp weight, `w = node.opacity · Π coverage(node.masks) ·
///   opᵢ.opacity · mᵢ`,
/// * FEATHERED mask — SERVES, per §11.2 Sub-slice B.
///
/// THE TEST NAME IS NOW WIDER THAN ITS SUBJECT and is kept deliberately: it
/// is cited by name in the conformance register, and ruling 7 says
/// identifiers never renumber. Only ONE of the three arms still refuses.
///
/// NEITHER LANE WEAKENED THIS GUARD. Each converted arm asserts its serve
/// POSITIVELY — the adjustment arm on both doors, the feather arm with an
/// `assert_ne!` against the unfeathered bytes, because a serve that returned
/// the unfeathered export would be the silent drop QA-F8 forbids and would
/// pass a bare `is_ok()`.
///
/// This is the regression the workspace gate caught that six targeted suites
/// did not — it lives in the conformance crate, outside `crates/*/tests/`.
#[test]
fn q3_sibling_deferrals_stay_refusing() {
    // masked group: group the blue layer, then mask the GROUP node.
    {
        let mut e = unmasked_blue_doc();
        let g = "22b30000-0000-4000-8000-0000000000e1";
        cmd(
            &mut e,
            10,
            "layer.group",
            None,
            json!({"layerIds": [L], "newGroupId": g}),
        );
        assert_masked_construct_refused(&mut e, 0x11, g, "masked-group");
    }
    // masked adjustment: add an adjustment layer, then mask it.
    {
        let mut e = unmasked_blue_doc();
        let al = "22b30000-0000-4000-8000-0000000000f1";
        let ao = "22b30000-0000-4000-8000-0000000000f2";
        cmd(
            &mut e,
            20,
            "adjustment.add",
            Some(json!({"layerId": L})),
            json!({"opKind": "adjustment.invert", "newLayerId": al, "newOpId": ao, "params": {}}),
        );
        // FI-13: the mask must ATTACH and both doors must SERVE.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "22b30000-0000-4000-8000-000000000021",
                "kind": "mask.add",
                "target": {"layerId": al},
                "params": {"kind": "raster", "newMaskId": M},
            })
            .to_string(),
        )
        .expect("FI-13: mask.add attaches to an adjustment layer");
        export_png_bytes(&e).expect("FI-13: a masked adjustment SERVES the reference export door");
    }
    // Feathered mask: Sub-slice B SERVES it. The reference export must
    // succeed AND differ from the unfeathered export — a serve that
    // returned the unfeathered bytes would be the silent-drop QA-F8
    // forbids, and would pass a mere `is_ok()` check.
    {
        let unfeathered =
            export_png_bytes(&masked_blue_doc()).expect("the unfeathered baseline exports");
        let mut e = masked_blue_doc();
        cmd(
            &mut e,
            30,
            "mask.set_props",
            Some(json!({"layerId": L, "maskId": M})),
            json!({"feather": 18.0}),
        );
        let feathered = export_png_bytes(&e).expect("a feathered mask now SERVES the export");
        assert_ne!(
            feathered, unfeathered,
            "the feathered export must differ from the unfeathered one — equal \
             bytes would mean feather was silently dropped"
        );
    }
}

// ===========================================================================
// Interactive transform doors — the class-kill extension.
// ===========================================================================

/// Apply an arbitrary interactive transform `kind` on layer `L`, returning the
/// raw (code, message) on refusal.
fn try_kind(e: &mut Engine, n: u32, kind: &str, params: Value) -> Result<(), (String, String)> {
    let v = json!({
        "schema": "appname.command.v1",
        "id": format!("22b30000-0000-4000-8000-{n:012x}"),
        "kind": kind,
        "target": {"layerId": L},
        "params": params,
    });
    e.apply_json(&v.to_string())
        .map(|_| ())
        .map_err(|err| (err.code.as_str().to_string(), err.message))
}

/// `transform.free` scale(2,2) — a non-identity affine.
fn free_scale2() -> Value {
    json!({"matrix": [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0], "reference": [100.0, 60.0]})
}
/// `transform.free` translate(60,0).
fn free_translate60() -> Value {
    json!({"matrix": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 60.0, 0.0, 1.0], "reference": [0.0, 0.0]})
}
/// `transform.free` identity.
fn free_identity() -> Value {
    json!({"matrix": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], "reference": [0.0, 0.0]})
}
/// `transform.perspective` — a valid convex TL/TR/BR/BL quad (no identity arm).
fn perspective_quad() -> Value {
    json!({"quad": [[10.0, 10.0], [190.0, 15.0], [185.0, 110.0], [15.0, 105.0]]})
}
/// `transform.warp` — cols=1,rows=1 ⇒ a 4×4 control lattice over the 200×120
/// canvas; `dx` perturbs one interior control point (dx == 0 ⇒ identity mesh,
/// matching `WarpMesh::base_lattice` bit-for-bit).
fn warp_params(dx: f64) -> Value {
    let (w, h) = (200.0f64, 120.0f64);
    let mut pts: Vec<Value> = Vec::with_capacity(16);
    for j in 0..4 {
        for i in 0..4 {
            let mut x = w * i as f64 / 3.0;
            let y = h * j as f64 / 3.0;
            if i == 1 && j == 1 {
                x += dx;
            }
            pts.push(json!([x, y]));
        }
    }
    json!({"mesh": {"cols": 1, "rows": 1, "points": pts}})
}

/// Class-kill headline: a non-identity transform on a masked layer refuses
/// typed on every INTERACTIVE bake door — free / perspective / warp.
#[test]
fn q3_interactive_masked_transform_refuses_on_every_door() {
    let doors: Vec<(&str, Value)> = vec![
        ("transform.free", free_scale2()),
        ("transform.perspective", perspective_quad()),
        ("transform.warp", warp_params(8.0)),
    ];
    for (i, (kind, params)) in doors.into_iter().enumerate() {
        let mut e = masked_blue_doc();
        let err = try_kind(&mut e, 400 + i as u32, kind, params)
            .expect_err(&format!("{kind}: masked interactive transform must refuse"));
        assert_eq!(
            err.0, "error.command.not_implemented",
            "{kind}: typed refusal (got {err:?})"
        );
        assert!(
            err.1.contains("mask"),
            "{kind}: the refusal names the mask (got {})",
            err.1
        );
        assert_eq!(
            e.document().unwrap().mask_count(),
            1,
            "{kind}: the mask survives the refusal"
        );
        assert_eq!(
            blue_window(&e, 50),
            (20, 80),
            "{kind}: composite unchanged after refusal"
        );
    }
}

/// The UNMASKED interactive transforms are unaffected — each still serves and
/// bakes (byte-stability pin for the correct case; the guard is gated on an
/// enabled mask, so these paths are untouched by construction).
#[test]
fn q3_interactive_unmasked_transform_still_serves_and_bakes() {
    // free translate(60,0): exact pixel pin — content shifts +60, vacated
    // columns clear to the white background.
    {
        let mut e = unmasked_blue_doc();
        try_kind(&mut e, 500, "transform.free", free_translate60())
            .expect("unmasked free still serves (ok:true)");
        assert_eq!(
            blue_window(&e, 50),
            (60, 200),
            "unmasked free bakes exactly as before the fix"
        );
    }
    // perspective + warp: serve, export, and leave real baked blue content
    // (the resample ran — unaffected by the guard).
    for (n, kind, params) in [
        (501u32, "transform.perspective", perspective_quad()),
        (502u32, "transform.warp", warp_params(8.0)),
    ] {
        let mut e = unmasked_blue_doc();
        try_kind(&mut e, n, kind, params)
            .unwrap_or_else(|e| panic!("unmasked {kind} still serves: {e:?}"));
        assert!(export_png_bytes(&e).is_ok(), "{kind}: export serves");
        let (s, en) = blue_window(&e, 50);
        assert!(en > s, "{kind}: baked visible content ({s}..{en})");
    }
}

/// The guard must NOT over-fire: an identity interactive transform, an identity
/// mesh, and a DISABLED mask are all exempt (no pixel change / no live mask ⇒
/// no desync).
#[test]
fn q3_interactive_identity_and_disabled_mask_are_exempt() {
    // Identity free on a masked layer: the is_identity no-op arm returns before
    // the guard.
    {
        let mut e = masked_blue_doc();
        try_kind(&mut e, 600, "transform.free", free_identity())
            .expect("identity free on a masked layer is a no-op, not a refusal");
        assert_eq!(
            blue_window(&e, 50),
            (20, 80),
            "identity free leaves the composite intact"
        );
    }
    // Identity MESH warp on a masked layer: the identity-mesh no-op arm returns
    // before the guard.
    {
        let mut e = masked_blue_doc();
        try_kind(&mut e, 601, "transform.warp", warp_params(0.0))
            .expect("identity-mesh warp on a masked layer is a no-op, not a refusal");
        assert_eq!(
            blue_window(&e, 50),
            (20, 80),
            "identity mesh leaves the composite intact"
        );
    }
    // A DISABLED mask is a §11.2 no-op ⇒ the interactive bake serves.
    {
        let mut e = masked_blue_doc();
        cmd(
            &mut e,
            610,
            "mask.set_props",
            Some(json!({"layerId": L, "maskId": M})),
            json!({"enabled": false}),
        );
        try_kind(&mut e, 611, "transform.free", free_scale2())
            .expect("a disabled mask does not block the interactive transform (§11.2 no-op)");
    }
}
