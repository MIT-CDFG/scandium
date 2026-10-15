// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `image.auto_tone.yaml` / `image.auto_contrast.yaml` /
//! `image.auto_color.yaml`; ES §17.1/§17.2 (analyses + histogram
//! domain), §16.2 (selection masking), §40.1 (stage order, empty push),
//! §40.2 (family rules), §10.4/§34.4 (undo byte-identity, replay).
//!
//! End-to-end battery for the three `image.auto_*` bake kinds through a
//! FULLY ASSEMBLED engine — THE production constructor (core door slice
//! 2 MAJOR-1 ruled fix, TEST-FILE-ONLY cross-fence companion flagged
//! for O-TOOLS ratification: the workspace-walk guard flushed this rig
//! out as a FOURTH stale copy — it claimed "the same composition the
//! cli/cabi assembly points get" while missing the filters AND vector
//! wraps). The pixel-level pins here are the kinds' deterministic
//! corpus companions (in-crate per the fence; flagged to O-QA for
//! golden ratification in the lane log — goldens/ itself is not
//! touched).

use appname_core::{Engine, EngineAssembly, ErrorCode, IVec2};
use serde_json::{json, Value};

fn assembly() -> EngineAssembly {
    appname_assembly::production_assembly()
}

fn env(n: u32, kind: &str, params: Value) -> String {
    json!({
        "schema": "appname.command.v1",
        "id": format!("a0704d00-0000-4000-8000-0000000{n:05x}"),
        "kind": kind,
        "params": params,
    })
    .to_string()
}

fn doc_new(e: &mut Engine, n: u32) -> appname_core::LayerId {
    doc_new_sized(e, n, 64, 64)
}

fn doc_new_sized(e: &mut Engine, n: u32, w: i32, h: i32) -> appname_core::LayerId {
    e.apply_json(&env(
        n,
        "doc.new",
        json!({"width": w, "height": h, "background": "white"}),
    ))
    .unwrap();
    e.document().unwrap().z_order_walk()[0]
}

/// Select `rect` hard-edged (replace).
fn select_rect(e: &mut Engine, n: u32, x: i32, y: i32, w: i32, h: i32) {
    e.apply_json(&env(
        n,
        "selection.set",
        json!({"source": "rect", "rect": {"x": x, "y": y, "w": w, "h": h}, "antialias": false}),
    ))
    .unwrap();
}

/// Fill the current selection (or all) with an §22.1 display-encoded rgb.
fn fill_rgb(e: &mut Engine, n: u32, layer: appname_core::LayerId, rgb: [f64; 3]) {
    let text = json!({
        "schema": "appname.command.v1",
        "id": format!("a0704d00-0000-4000-8000-0000000{n:05x}"),
        "kind": "fill",
        "target": {"layerId": layer.to_string()},
        "params": {"content": "color",
                    "color": {"model": "rgb", "comps": [rgb[0], rgb[1], rgb[2], 1.0]}},
    })
    .to_string();
    e.apply_json(&text).unwrap();
}

fn deselect(e: &mut Engine, n: u32) {
    e.apply_json(&env(n, "selection.none", json!({}))).unwrap();
}

/// Straight (un-premultiplied) display-encoded pixel readback.
fn encoded_at(e: &Engine, layer: appname_core::LayerId, x: i32, y: i32) -> [f32; 3] {
    let p = e
        .document()
        .unwrap()
        .layer(layer)
        .unwrap()
        .pixels()
        .unwrap()
        .read_pixel_f32(IVec2::new(x, y));
    let s = appname_core::io::srgb::unpremul(p);
    [
        appname_core::io::srgb::srgb_encode(s[0]),
        appname_core::io::srgb::srgb_encode(s[1]),
        appname_core::io::srgb::srgb_encode(s[2]),
    ]
}

/// Two-tone doc: left half encoded-51 gray, right half white.
fn two_tone(e: &mut Engine) -> appname_core::LayerId {
    let layer = doc_new(e, 1);
    select_rect(e, 2, 0, 0, 32, 64);
    fill_rgb(e, 3, layer, [0.2, 0.2, 0.2]);
    deselect(e, 4);
    layer
}

#[test]
fn auto_tone_stretches_bakes_undoes_and_replays() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    let before = e.state_hash();
    let receipt = e.apply_json(&env(5, "image.auto_tone", json!({}))).unwrap();
    assert!(!receipt.dirty.is_empty(), "pixels changed");
    let after = e.state_hash();
    assert_ne!(before, after);
    // §17.2 per-channel stretch: lo = bin 51, hi = 255 → the gray floor
    // maps to black, white stays white (corpus pin; LUT tolerance).
    let left = encoded_at(&e, layer, 8, 32);
    let right = encoded_at(&e, layer, 48, 32);
    for c in 0..3 {
        assert!(left[c] < 0.01, "left → black, got {left:?}");
        assert!(right[c] > 0.99, "right stays white, got {right:?}");
    }
    // §10.4 tile-delta inverse: byte identity both ways.
    e.undo().unwrap();
    assert_eq!(e.state_hash(), before);
    e.redo().unwrap();
    assert_eq!(e.state_hash(), after);
    // D-48: replay through the same assembly reproduces the bytes (the
    // histogram is recomputed from the same journaled state).
    let replayed = e.replay_assembled(e.journal()).unwrap();
    assert_eq!(replayed.state_hash(), after, "replay determinism");
}

#[test]
fn selection_gates_both_statistics_and_the_bake() {
    let mut e = assembly().build();
    let layer = two_tone(&mut e);
    // Re-select the LEFT half: stats see only the gray field, and the
    // bake must not touch the right half (§16.2 lerp, §40.2 row).
    select_rect(&mut e, 5, 0, 0, 32, 64);
    e.apply_json(&env(6, "image.auto_tone", json!({}))).unwrap();
    let left = encoded_at(&e, layer, 8, 32);
    let right = encoded_at(&e, layer, 48, 32);
    assert!(
        (right[0] - 1.0).abs() < 1e-6 && (right[1] - 1.0).abs() < 1e-6,
        "outside the selection support nothing changes, got {right:?}"
    );
    // Degenerate single-value support: the §17.2 gap guard remaps the
    // field through its own 1/255 window — a strong darkening (the exact
    // landing value is [NORMATIVE]-LUT-quantized: the window is narrower
    // than the 1024-entry step here, so the pin is directional).
    assert!(
        left[0] < 0.15 && (left[0] - 0.2).abs() > 0.02,
        "selected field remapped, got {left:?}"
    );
}

#[test]
fn auto_contrast_writes_one_luma_tuple_to_all_channels() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1);
    fill_rgb(&mut e, 2, layer, [0.6, 0.4, 0.4]); // uniform red-cast field
    e.apply_json(&env(3, "image.auto_contrast", json!({})))
        .unwrap();
    let px = encoded_at(&e, layer, 32, 32);
    // ONE tuple on all three channels (cast preserved): the identical
    // remap keeps channel order and keeps G == B exactly.
    assert!(px[0] > px[1], "cast order preserved, got {px:?}");
    assert!((px[1] - px[2]).abs() < 1e-6, "G == B through the one tuple");
}

#[test]
fn auto_color_neutralizes_the_midtone_cast_to_half_gray() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1);
    // Cast midtone field (enc 150/120/120, luma bin 129 ∈ [110, 145])
    // with black + white anchor corners so the per-channel stretch is
    // identity and the gamma step is isolated (§17.2).
    fill_rgb(
        &mut e,
        2,
        layer,
        [150.0 / 255.0, 120.0 / 255.0, 120.0 / 255.0],
    );
    select_rect(&mut e, 3, 0, 0, 4, 4);
    fill_rgb(&mut e, 4, layer, [0.0, 0.0, 0.0]);
    select_rect(&mut e, 5, 60, 0, 4, 4);
    fill_rgb(&mut e, 6, layer, [1.0, 1.0, 1.0]);
    deselect(&mut e, 7);
    e.apply_json(&env(8, "image.auto_color", json!({})))
        .unwrap();
    // The defining property (§17.2 gamma ruling FINAL): each channel's
    // midtone mean maps to 0.5 — the cast field lands neutral.
    let px = encoded_at(&e, layer, 32, 32);
    for c in 0..3 {
        assert!(
            (px[c] - 0.5).abs() < 5e-3,
            "midtone neutralized to 0.5, got {px:?}"
        );
    }
}

#[test]
fn auto_color_without_neutralization_on_full_range_is_the_pinned_no_op() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1);
    fill_rgb(
        &mut e,
        2,
        layer,
        [150.0 / 255.0, 120.0 / 255.0, 120.0 / 255.0],
    );
    select_rect(&mut e, 3, 0, 0, 4, 4);
    fill_rgb(&mut e, 4, layer, [0.0, 0.0, 0.0]);
    select_rect(&mut e, 5, 60, 0, 4, 4);
    fill_rgb(&mut e, 6, layer, [1.0, 1.0, 1.0]);
    deselect(&mut e, 7);
    let before = e.state_hash();
    let receipt = e
        .apply_json(&env(
            8,
            "image.auto_color",
            json!({"neutralizeMidtones": false}),
        ))
        .unwrap();
    // Identity analysis (full-range histogram, gamma 1) → §40.1 empty
    // push (pinned: the LUT's re-quantization never dirties a no-op).
    assert!(receipt.dirty.is_empty(), "empty-delta push");
    assert_eq!(e.state_hash(), before, "no bytes moved");
}

/// Boundary: a FULLY SATURATED document (every channel in bin 255 — the
/// `doc.new` white background is the common real-world case). The §17.2
/// analysis cannot stretch a channel with no headroom, so the window the
/// gap rule opens (inBlack = 254/255, inWhite = 1) must map white back to
/// white THROUGH THE 1024-ENTRY BAKE LUT — a distinct risk from the pure
/// analysis, since the window here is ~4× narrower than one LUT step.
/// Asserted property is §17.2-derived, not fitted: white in, white out.
#[test]
fn saturated_document_survives_the_bake_unchanged() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1); // background: white ⇒ all channels bin 255
    let before = e.state_hash();
    e.apply_json(&env(2, "image.auto_tone", json!({}))).unwrap();
    let px = encoded_at(&e, layer, 32, 32);
    for c in 0..3 {
        assert!(
            (px[c] - 1.0).abs() < 1e-6,
            "saturated field has no headroom to stretch into, got {px:?}"
        );
    }
    // Byte identity: no headroom means no pixel may move at all.
    assert_eq!(e.state_hash(), before, "saturated bake moves no bytes");
    // Same on the luma path (Auto Contrast) and the gamma path (Auto
    // Color): a saturated field yields no midtone pixels, so the
    // neutralization step must be the None path, not a divide-by-zero.
    e.apply_json(&env(3, "image.auto_contrast", json!({})))
        .unwrap();
    e.apply_json(&env(4, "image.auto_color", json!({})))
        .unwrap();
    let px = encoded_at(&e, layer, 32, 32);
    for c in 0..3 {
        assert!((px[c] - 1.0).abs() < 1e-6, "still saturated, got {px:?}");
    }
    assert!(px.iter().all(|v| v.is_finite()), "no NaN from the rail");
}

/// HSV hue in degrees; `None` for a neutral (zero-chroma) colour, whose
/// hue is undefined. Standard hexcone formula — no engine code involved,
/// so the property below is checked against an INDEPENDENT computation.
fn hue_of(c: [f32; 3]) -> Option<f32> {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    if d < 1e-6 {
        return None;
    }
    let h = if max == c[0] {
        60.0 * (((c[1] - c[2]) / d) % 6.0)
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    Some((h + 360.0) % 360.0)
}

/// A tinted-anchor fixture: two NON-neutral anchors drive each channel's
/// own extremes apart (R spans enc 40..230, G 70..200, B 80..190) so the
/// per-channel and luma analyses cannot coincide, while three WIDE-chroma
/// patches span enc 80..190 — inside the luma window (lo = 63, hi = 206 by
/// hand from the anchors' lumas), so Auto Contrast maps them affinely with
/// no clamping and the comparison isolates the analysis, not the rails.
/// The no-clamping precondition is ASSERTED, not assumed, below.
///
/// The chroma is deliberately wide because hue resolution is set by the
/// u8 storage step: `∂H/∂channel` scales as `60/(max − min)`, so a narrow
/// patch would drown the invariant in ±0.5/255 quantization (see the
/// derived envelope on the test below).
fn tinted_anchor_fixture(e: &mut Engine) -> appname_core::LayerId {
    let layer = doc_new(e, 1);
    let mut fill = |n: u32, rect: Option<(i32, i32, i32, i32)>, rgb: [f64; 3]| {
        match rect {
            Some((x, y, w, h)) => select_rect(e, n, x, y, w, h),
            None => deselect(e, n),
        }
        fill_rgb(
            e,
            n + 1,
            layer,
            [rgb[0] / 255.0, rgb[1] / 255.0, rgb[2] / 255.0],
        );
    };
    fill(2, None, [120.0, 120.0, 120.0]); // neutral field
    fill(4, Some((0, 0, 8, 8)), [40.0, 70.0, 90.0]); // dark anchor, tinted
    fill(6, Some((56, 0, 8, 8)), [230.0, 200.0, 175.0]); // bright anchor
    fill(8, Some((0, 16, 16, 16)), [80.0, 130.0, 190.0]); // bluish
    fill(10, Some((16, 16, 16, 16)), [190.0, 130.0, 80.0]); // orangeish
    fill(12, Some((32, 16, 16, 16)), [80.0, 190.0, 130.0]); // greenish
    deselect(e, 14);
    layer
}

/// The contract's named unit `unit_cmd_image_auto_contrast_cast_preserved
/// (hue histogram unchanged)` — `contracts/commands/image.auto_contrast.yaml`
/// `tests.unit`, ES:5355 "colour cast preserved", ES:7724 "written
/// identically to all three channels".
///
/// The property is DERIVED, not fitted: one `(lo, hi)` written to all
/// three channels at gamma 1 is the affine map `v ↦ (v − lo)/(hi − lo)`
/// applied identically to R, G and B. HSV hue depends only on which
/// channel is max/min and on `(mid − min)/(max − min)`, both of which an
/// identical affine map leaves untouched — so hue is preserved EXACTLY
/// wherever no channel clamps. The existing
/// `auto_contrast_writes_one_luma_tuple_to_all_channels` only checks
/// channel ORDER on a uniform field, which per-channel Auto Tone would
/// also pass; this is the strictly stronger pin the contract names.
///
/// TOLERANCE, derived — never fitted. The document is u8 (the `doc.new`
/// default), so each channel carries a ±0.5/255 storage step. For the
/// bluish patch `H = 240 + 60·(R−G)/(B−R)`, and summing the hexcone
/// partials gives `Σ|∂H/∂c| = 278.2 °/unit` before the bake and
/// `156.0 °/unit` after the 1.78× stretch; at ε = 0.5/255 that is
/// 0.545° + 0.306° = **0.85°** of pure quantization, so 1.0° is the
/// envelope and anything larger is a genuine hue shift. (A narrower
/// patch would put the floor at 2.34° — hence the wide-chroma fixture.)
///
/// The same fixture doubles as this test's own NEGATIVE CONTROL: Auto
/// Tone, whose whole point per ES:7723 is that it "may shift colour
/// balance", must move these hues by far more than that envelope.
#[test]
fn auto_contrast_leaves_the_hue_histogram_unchanged() {
    let probes = [(8, 24), (24, 24), (40, 24)]; // one per chroma patch

    let mut e = assembly().build();
    let layer = tinted_anchor_fixture(&mut e);
    let before: Vec<Option<f32>> = probes
        .iter()
        .map(|&(x, y)| hue_of(encoded_at(&e, layer, x, y)))
        .collect();
    assert!(
        before.iter().all(Option::is_some),
        "fixture must carry real chroma, got {before:?}"
    );

    e.apply_json(&env(20, "image.auto_contrast", json!({})))
        .unwrap();

    // PRECONDITION, asserted rather than assumed: hue is affine-invariant
    // only where NO channel hits a rail. If a future window change started
    // clamping these patches, the tolerance below would fail for a reason
    // that has nothing to do with the cast-preservation property — so
    // catch that here, where the message says what actually went wrong.
    for &(x, y) in &probes {
        let px = encoded_at(&e, layer, x, y);
        assert!(
            px.iter().all(|v| *v > 1e-3 && *v < 1.0 - 1e-3),
            "fixture precondition: patch at ({x},{y}) clamped onto a rail \
             ({px:?}) — the affine-hue invariant does not apply there"
        );
    }

    let after: Vec<Option<f32>> = probes
        .iter()
        .map(|&(x, y)| hue_of(encoded_at(&e, layer, x, y)))
        .collect();
    for (i, (b, a)) in before.iter().zip(&after).enumerate() {
        let (b, a) = (b.unwrap(), a.unwrap());
        assert!(
            (a - b).abs() < 1.0,
            "patch {i}: one luma tuple on all channels is affine, so hue \
             is invariant within the 0.85° u8 envelope — {b}° became {a}°"
        );
    }

    // NEGATIVE CONTROL on the identical fixture: the per-channel analysis
    // must move the same hues (ES:7723 "may shift colour balance").
    let mut e2 = assembly().build();
    let layer2 = tinted_anchor_fixture(&mut e2);
    e2.apply_json(&env(20, "image.auto_tone", json!({})))
        .unwrap();
    let toned: Vec<f32> = probes
        .iter()
        .map(|&(x, y)| hue_of(encoded_at(&e2, layer2, x, y)).unwrap())
        .collect();
    let moved = before
        .iter()
        .zip(&toned)
        .filter(|(b, t)| (**t - b.unwrap()).abs() > 5.0)
        .count();
    assert!(
        moved >= 2,
        "Auto Tone must shift these hues or the invariant above is vacuous \
         — before {before:?}, toned {toned:?}"
    );
}

/// D-48 byte-identical replay + §10.4 tile-delta undo for the two kinds
/// `auto_tone_stretches_bakes_undoes_and_replays` does NOT cover. Both
/// carry state the tone path does not: Auto Contrast reads the LUMA
/// plane, and Auto Color runs a second full-plane scan (the midtone
/// means) whose result must recompute identically on replay.
#[test]
fn contrast_and_color_replay_and_undo_byte_identically() {
    for kind in ["image.auto_contrast", "image.auto_color"] {
        let mut e = assembly().build();
        let _ = tinted_anchor_fixture(&mut e);
        let before = e.state_hash();
        let receipt = e.apply_json(&env(20, kind, json!({}))).unwrap();
        assert!(!receipt.dirty.is_empty(), "{kind}: pixels changed");
        let after = e.state_hash();
        assert_ne!(before, after, "{kind}: the bake moved bytes");

        // §10.4: the tile-delta inverse restores the exact bytes.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "{kind}: undo byte identity");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after, "{kind}: redo byte identity");

        // D-48: replaying the journal through the same assembly re-derives
        // the histogram (and, for auto_color, the midtone means) from the
        // same state and must land on the same bytes.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "{kind}: D-48 replay");
    }
}

/// The smallest possible document — ONE pixel. Every §17.1 statistic is a
/// single sample, so `total = 1`, `lo = hi`, and the §17.2 gap rule opens
/// the window; the midtone scan sees either one qualifying pixel or none.
/// Nothing here may panic, divide by zero, or leak a non-finite value,
/// and the D-48/undo contract holds at this size too.
#[test]
fn one_pixel_document_survives_every_auto_kind() {
    for kind in ["image.auto_tone", "image.auto_contrast", "image.auto_color"] {
        let mut e = assembly().build();
        let layer = doc_new_sized(&mut e, 1, 1, 1);
        // Mid-gray: a degenerate single-value histogram that is NOT on a
        // rail, so the gap window straddles the value (the rails already
        // have their own pin above).
        fill_rgb(&mut e, 2, layer, [120.0 / 255.0; 3]);
        let before = e.state_hash();
        e.apply_json(&env(3, kind, json!({}))).unwrap();

        let px = encoded_at(&e, layer, 0, 0);
        assert!(
            px.iter().all(|v| v.is_finite()),
            "{kind}: finite, got {px:?}"
        );
        assert!(
            px.iter().all(|v| (0.0..=1.0).contains(v)),
            "{kind}: in range, got {px:?}"
        );
        // TRANSPARENCY unaffected (ES:5354 lock note): alpha stays opaque.
        let raw = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(0, 0));
        assert!((raw[3] - 1.0).abs() < 1e-6, "{kind}: alpha untouched");

        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "{kind}: one-pixel undo identity");
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "{kind}: one-pixel D-48 replay"
        );
    }
}

#[test]
fn adversarial_rejects_match_the_contract_taxonomy() {
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1);

    // Cross-field: clipLow + clipHigh = 1.0 is not < 1.0 (§40.2).
    let err = e
        .apply_json(&env(
            2,
            "image.auto_tone",
            json!({"clipLow": 0.5, "clipHigh": 0.5}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange, "cross-field corner");

    // Per-field bounds (stage 5 or the defensive stage-6 echo).
    let err = e
        .apply_json(&env(3, "image.auto_tone", json!({"clipLow": 0.6})))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    let err = e
        .apply_json(&env(4, "image.auto_color", json!({"clipLow": -0.1})))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);

    // targetLayer that does not resolve (auto_tone's explicit param).
    let err = e
        .apply_json(&env(
            5,
            "image.auto_tone",
            json!({"targetLayer": "dead0000-0000-4000-8000-000000000001"}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadTarget);

    // targetLayer resolving to a NON-Raster layer (adjustment layer via
    // adjustment.add — live end-to-end through the §12.1 op bridge).
    let adj_layer = "ad700000-0000-4000-8000-000000000001";
    e.apply_json(
        &json!({
            "schema": "appname.command.v1",
            "id": "a0704d00-0000-4000-8000-00000000eee1",
            "kind": "adjustment.add",
            "params": {"opKind": "adjustment.invert", "params": {},
                        "newLayerId": adj_layer,
                        "newOpId": "ad700000-0000-4000-8000-000000000002"},
        })
        .to_string(),
    )
    .unwrap();
    let err = e
        .apply_json(&env(
            7,
            "image.auto_tone",
            json!({"targetLayer": adj_layer}),
        ))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::TargetKindMismatch);

    // §11.1/§40.2 lock table: a PIXELS lock on the default target blocks.
    e.apply_json(
        &json!({
            "schema": "appname.command.v1",
            "id": "a0704d00-0000-4000-8000-00000000eee2",
            "kind": "layer.set_props",
            "target": {"layerId": layer.to_string()},
            "params": {"locks": {"pixels": true}},
        })
        .to_string(),
    )
    .unwrap();
    let err = e
        .apply_json(&env(9, "image.auto_contrast", json!({})))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
}

#[test]
fn ops_registry_is_live_through_the_full_assembly() {
    // The §12.1 seam end-to-end: adjustment.add resolves a real op,
    // op.set_params re-validates through it, unknown kinds answer the
    // §40.6 contract id.
    let mut e = assembly().build();
    doc_new(&mut e, 1);
    let err = e
        .apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "a0704d00-0000-4000-8000-00000000eee3",
                "kind": "adjustment.add",
                "params": {"opKind": "adjustment.not_a_thing", "params": {},
                            "newLayerId": "ad700000-0000-4000-8000-000000000011",
                            "newOpId": "ad700000-0000-4000-8000-000000000012"},
            })
            .to_string(),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownOpKind);
    e.apply_json(
        &json!({
            "schema": "appname.command.v1",
            "id": "a0704d00-0000-4000-8000-00000000eee4",
            "kind": "adjustment.add",
            "params": {"opKind": "adjustment.levels",
                        "params": {"r": {"inBlack": 0.1}},
                        "newLayerId": "ad700000-0000-4000-8000-000000000021",
                        "newOpId": "ad700000-0000-4000-8000-000000000022"},
        })
        .to_string(),
    )
    .unwrap();
    // Nested re-validation through the registered op (§16.1 route).
    let err = e
        .apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "a0704d00-0000-4000-8000-00000000eee5",
                "kind": "op.set_params",
                "params": {"opId": "ad700000-0000-4000-8000-000000000022",
                            "params": {"r": {"inBlack": 0.9, "inWhite": 0.9}}},
            })
            .to_string(),
        )
        .unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::OutOfRange,
        "§17.2 gap rule via the bridge"
    );
}

#[test]
fn auto_under_pixels_locked_group_refuses_locked_transitively() {
    // §11.1 transitive letter through `Document::effective_locks` (the
    // O-CORE door-slice consolidation, adopted this round): a PIXELS
    // lock on an ancestor GROUP blocks the auto bake on the child —
    // own-node locks alone under-enforced this until the adoption.
    let mut e = assembly().build();
    let layer = doc_new(&mut e, 1);
    e.apply_json(
        &json!({
            "schema": "appname.command.v1",
            "id": "a0704d00-0000-4000-8000-00000000f001",
            "kind": "layer.group",
            "params": {"layerIds": [layer.to_string()],
                        "newGroupId": "ad700000-0000-4000-8000-0000000000f1"},
        })
        .to_string(),
    )
    .unwrap();
    e.apply_json(
        &json!({
            "schema": "appname.command.v1",
            "id": "a0704d00-0000-4000-8000-00000000f002",
            "kind": "layer.set_props",
            "target": {"layerId": "ad700000-0000-4000-8000-0000000000f1"},
            "params": {"locks": {"pixels": true}},
        })
        .to_string(),
    )
    .unwrap();
    let before = e.state_hash();
    let err = e
        .apply_json(&env(3, "image.auto_contrast", json!({})))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked, "group PIXELS binds the child");
    assert_eq!(before, e.state_hash(), "refusal mutates nothing (§10.2)");
}

// ---------------------------------------------------------------------------
// Strictness ruling 2026-08-17: adjustment params through the §12.1 seam.
// ---------------------------------------------------------------------------

#[test]
fn unknown_adjustment_params_refuse_typed_on_the_wire() {
    let mut e = assembly().build();
    doc_new(&mut e, 1);
    let before = e.state_hash();
    // Top-level unknown key on a registered op.
    let err = e
        .apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "a0704d00-0000-4000-8000-00000000f001",
                "kind": "adjustment.add",
                "params": {"opKind": "adjustment.invert", "params": {"bogus": 1},
                            "newLayerId": "ad700000-0000-4000-8000-00000000f002",
                            "newOpId": "ad700000-0000-4000-8000-00000000f003"},
            })
            .to_string(),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    assert!(err.message.contains("bogus"), "{}", err.message);
    // Nested unknown key inside a levels tuple (schema letter at depth).
    let err = e
        .apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "a0704d00-0000-4000-8000-00000000f004",
                "kind": "adjustment.add",
                "params": {"opKind": "adjustment.levels",
                            "params": {"composite": {"inBlack": 0.1, "typo": 1}},
                            "newLayerId": "ad700000-0000-4000-8000-00000000f005",
                            "newOpId": "ad700000-0000-4000-8000-00000000f006"},
            })
            .to_string(),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::OutOfRange);
    assert!(err.message.contains("typo"), "{}", err.message);
    assert_eq!(e.state_hash(), before, "refusals mutation-free");
}
