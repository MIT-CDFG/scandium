// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//! FI-13 [NORMATIVE] — the Adjustment node's own weight, end to end.
//!
//! ENGINEERING-SPEC :5956-5962, frozen at `v5.6-freeze`:
//!
//! ```text
//! w = node.opacity · Π coverage(node.masks) · opᵢ.opacity · mᵢ
//! ```
//!
//! `eval_chain` applies `opᵢ.opacity · mᵢ`; the fold applies the two NODE
//! factors as a second lerp against the same backdrop. This suite drives the
//! whole path through the command surface and the reference export door, so
//! it exercises the COMPOSITION of the two stages rather than either alone.
//!
//! # THIS FILE IS NOT ALL OF FI-13's COVERAGE — READ THE OTHER ONE TOO
//!
//! This suite drives the COMMAND SURFACE and asserts CHANNEL 0 ONLY. It says
//! nothing about alpha. A reader who opens the file named for FI-13 and stops
//! here will conclude the coverage is thinner than it is.
//!
//! The complement is `crates/appname-render/tests/parametric_composite.rs`,
//! whose `fi13_*` tests iterate node opacity 0.25 / 0.5 / 0.75, assert RGB
//! per channel AND `px[3]` at every partial weight, and pin the E = 0 and
//! feathered-mask arms. ALPHA THROUGH THE FOLD AT PARTIAL WEIGHT IS COVERED
//! THERE, NOT HERE.
//!
//! The split is deliberate — conformance drives the command surface, the
//! render suite drives the compositor — but the signposting was not, and a
//! reader arriving cold at the FI-13-named file deserves the pointer.
//! (O-WATCH's discoverability note, reviewer pass at b07c8da5.)
//!
//! # Why four legs, and what each one is FOR
//!
//! The legs do TWO DIFFERENT JOBS and they fail differently. A reader who
//! treats this as four detectors will misread a failure, so the job is
//! labelled per leg.
//!
//! | leg | w     | job             | fold removed | min for product | fold ignores fade |
//! |-----|-------|-----------------|--------------|-----------------|-------------------|
//! |  1  | 1.000 | INTERPRETABILITY| survives     | survives        | survives          |
//! |  2  | 0.500 | detection       | DIES         | survives        | survives          |
//! |  3  | 0.125 | detection       | DIES         | DIES            | survives          |
//! |  4  | 0.125 | detection       | DIES         | survives        | DIES              |
//!
//! (Every leg dies if the adjustment does not apply at all.)
//!
//! **Leg 1 is not a detector.** No perturbation kills it that does not also
//! kill a detector leg. It is the PRECONDITION THAT MAKES LEGS 2-4
//! INTERPRETABLE: if leg 1 fails, legs 2-4's numbers mean nothing, because
//! the serve is not known to have happened at all. Keep it; do not read it
//! as coverage.
//!
//! # Why the constants are what they are
//!
//! A PRODUCT TESTED ONLY AT ENDPOINTS PINS THE SHAPE AND NEVER PINS THE
//! COMBINATOR: at {0,1} the product agrees with `min`, `max`, `average` and
//! with either factor alone. Partial values are required — and they must
//! DIFFER FROM EACH OTHER, because at `o == c` the product also agrees with
//! `o²` and `c²`, so the leg cannot detect ONE FACTOR USED TWICE AND THE
//! OTHER DROPPED.
//!
//! Leg 3 (outer stage): node opacity 0.5, coverage 0.25 → w = 0.125 against
//! c² 0.0625, o² 0.25, min 0.25, coverage-alone 0.25, average 0.375, max 0.5,
//! opacity-alone 0.5. Eight distinct values; the product is separated from
//! all seven rivals.
//!
//! Leg 4 (both stages): per-op opacity 0.5, node opacity 0.25 → the same
//! separation one level up.
//!
//! # Why `hide_all` and not `reveal_all` for partial coverage
//!
//! §11.2 [NORMATIVE] is `m' = 1 − density·(1 − c)` — density lerps the mask
//! TOWARD 1. On `reveal_all` (c = 1) it is a NO-OP: E = 1 for every density.
//! A partial-coverage leg built the obvious way would silently run at
//! coverage 1.0 and become a second copy of leg 2 — still passing under
//! `min`, and looking exactly like the fix. `hide_all` (c = 0) gives
//! `m' = 1 − density`, so density 0.75 is the coverage 0.25 this suite needs.
//!
//! # Assertion form
//!
//! Ordering comparisons, never absolute values with a tolerance constant:
//! `|served − expected| < |served − rival|`. That needs no epsilon, survives
//! the 8-bit sRGB round trip through the PNG, and rejects a merely-plausible
//! intermediate (0.9 or 0.1 pass "differs from both endpoints"; they do not
//! pass this).

use appname_conformance::assembly_point;
use appname_core::{decode_png_to_buffer, export_png_bytes, Engine, IVec2, ProfileId};
use serde_json::{json, Value};

const L: &str = "f1130000-0000-4000-8000-0000000000c1";
const ADJ: &str = "f1130000-0000-4000-8000-0000000000a1";
const OP: &str = "f1130000-0000-4000-8000-0000000000a2";
const MASK: &str = "f1130000-0000-4000-8000-0000000000d1";

fn cmd(e: &mut Engine, n: u32, kind: &str, target: Option<Value>, params: Value) {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("f1130000-0000-4000-8000-{n:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    e.apply_json(&v.to_string())
        .unwrap_or_else(|err| panic!("{kind} -> {:?} {}", err.code, err.message));
}

/// 64×64 white doc with a full-canvas BLUE raster layer.
fn blue_doc() -> Engine {
    let mut e = assembly_point::assembled_engine();
    cmd(
        &mut e,
        1,
        "doc.new",
        None,
        json!({"width": 64, "height": 64, "background": "white"}),
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

/// `blue_doc` plus an `adjustment.invert` layer above it.
fn inverted_doc() -> Engine {
    let mut e = blue_doc();
    cmd(
        &mut e,
        4,
        "adjustment.add",
        None,
        json!({"opKind": "adjustment.invert", "newLayerId": ADJ, "newOpId": OP, "params": {}}),
    );
    e
}

/// Centre pixel of the reference export, decoded back to linear f32.
fn px(e: &Engine) -> [f32; 4] {
    let png = export_png_bytes(e).expect("FI-13: the reference export door serves");
    let (buf, _w, _h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    buf.read_pixel_f32(IVec2::new(32, 32))
}

/// The value channel 0 would take at weight `w` between the two endpoints.
fn at(blue: [f32; 4], inv: [f32; 4], w: f32) -> f32 {
    blue[0] + (inv[0] - blue[0]) * w
}

/// Ordering assertion: `served` must sit nearer the weight `want` than any
/// `rival` weight. No tolerance constant — see the module doc.
fn nearer_to(served: [f32; 4], blue: [f32; 4], inv: [f32; 4], want: f32, rivals: &[(&str, f32)]) {
    let target = at(blue, inv, want);
    for (name, r) in rivals {
        let rv = at(blue, inv, *r);
        assert!(
            (served[0] - target).abs() < (served[0] - rv).abs(),
            "FI-13: served {:.5} is not nearer w={want} ({:.5}) than {name} w={r} ({:.5})",
            served[0],
            target,
            rv
        );
    }
}

/// Endpoints: the un-adjusted backdrop and the adjustment at full strength.
fn endpoints() -> ([f32; 4], [f32; 4]) {
    (px(&blue_doc()), px(&inverted_doc()))
}

/// LEG 1 — INTERPRETABILITY, not detection. A `reveal_all` (E = 1) layer mask
/// on an adjustment must be the EXACT identity against no layer mask at all
/// (:5964: "E = 1 (i.e. **unmasked**, where the mask must have no effect at
/// all)"), and the serve must be real rather than merely non-refusing.
#[test]
fn fi13_leg1_full_coverage_serves_and_is_the_exact_identity() {
    let (blue, inv) = endpoints();
    assert!(
        (inv[0] - blue[0]).abs() > 0.5,
        "precondition: the adjustment must move channel 0 materially ({blue:?} -> {inv:?})"
    );
    let mut e = inverted_doc();
    cmd(
        &mut e,
        5,
        "mask.add",
        Some(json!({"layerId": ADJ})),
        json!({"kind": "raster", "from": "reveal_all", "newMaskId": MASK}),
    );
    assert_eq!(
        px(&e),
        inv,
        "FI-13/:5964: a reveal_all (E = 1) layer mask must have NO effect at all"
    );
}

/// LEG 2 — DETECTION (the fold runs at all). Node opacity 0.5 with E = 1 is
/// w = 0.5: strictly intermediate. Removing the fold collapses this onto the
/// full-strength endpoint.
#[test]
fn fi13_leg2_node_opacity_folds_into_the_weight() {
    let (blue, inv) = endpoints();
    let mut e = inverted_doc();
    cmd(
        &mut e,
        5,
        "mask.add",
        Some(json!({"layerId": ADJ})),
        json!({"kind": "raster", "from": "reveal_all", "newMaskId": MASK}),
    );
    cmd(
        &mut e,
        6,
        "layer.set_props",
        Some(json!({"layerId": ADJ})),
        json!({"opacity": 0.5}),
    );
    nearer_to(
        px(&e),
        blue,
        inv,
        0.5,
        &[("no fold (full strength)", 1.0), ("no adjustment", 0.0)],
    );
}

/// LEG 3 — DETECTION (the COMBINATOR is a product, at the outer stage).
/// Node opacity 0.5 with coverage 0.25 is w = 0.125, which no rival
/// combinator reproduces. `hide_all` at density 0.75 is the coverage; see the
/// module doc for why `reveal_all` + density cannot be.
#[test]
fn fi13_leg3_outer_stage_is_a_product_not_min_or_max() {
    let (blue, inv) = endpoints();
    let mut e = inverted_doc();
    cmd(
        &mut e,
        5,
        "mask.add",
        Some(json!({"layerId": ADJ})),
        json!({"kind": "raster", "from": "hide_all", "newMaskId": MASK}),
    );
    cmd(
        &mut e,
        6,
        "mask.set_props",
        Some(json!({"layerId": ADJ, "maskId": MASK})),
        json!({"density": 0.75}),
    );
    cmd(
        &mut e,
        7,
        "layer.set_props",
        Some(json!({"layerId": ADJ})),
        json!({"opacity": 0.5}),
    );
    nearer_to(
        px(&e),
        blue,
        inv,
        0.125,
        &[
            ("coverage squared", 0.0625),
            ("opacity squared", 0.25),
            ("min(o,c)", 0.25),
            ("coverage alone", 0.25),
            ("average", 0.375),
            ("max(o,c)", 0.5),
            ("opacity alone", 0.5),
        ],
    );
}

/// LEG 4 — DETECTION (the two STAGES compose). Per-op opacity 0.5 (inner,
/// `eval_chain`) against node opacity 0.25 (outer, the fold) is w = 0.125.
/// Every other leg runs at inner weight 1, where the served plane IS the
/// kernel output and the nested form collapses to the outer lerp alone — so
/// without this leg the composition is never exercised.
///
/// ★ IGNORED: THE INNER WEIGHT IS NOT REACHABLE FROM THE COMMAND SURFACE.
/// Measured, not assumed — this test was written, run, and REFUSED:
///
/// ```text
/// op.set_params -> OutOfRange unknown parameter `opacity`
///                  (additionalProperties: false)
/// ```
///
/// `contracts/commands/op.set_params.yaml:24-26` declares
/// `required: [opId, params]` with `additionalProperties: false`, so neither
/// `opacity` nor `blend` may be sent — while `kinds_ops.rs:290-298` READS
/// BOTH under the comment "Full-state block: params + optional fade fields
/// (§40.6)". THE IMPLEMENTATION READS TWO FIELDS ITS OWN CONTRACT FORBIDS;
/// both branches are dead from the command surface. Filed for the operator,
/// independent of FI-13 — and note the gate's `contract-sync` step PASSES,
/// so it does not check this direction.
///
/// The other inner-weight route is also closed: `op.set_mask{maskId}` is
/// explicitly not implemented (`kinds_ops.rs:337-339`, "arrives with the §14
/// mask/channel library"), leaving only `from: "selection"`, which needs an
/// active selection and yields binary coverage before density.
///
/// SO THE COMPOSITION GAP O-WATCH IDENTIFIED IS REAL AND REMAINS OPEN. It is
/// not closable end-to-end today. Closing it needs either a selection-derived
/// op mask with density, or a unit-level test against the fold directly.
/// Kept as an ignored test rather than deleted, because the gap is the record.
#[test]
#[ignore = "inner weight unreachable: op.set_params contract forbids `opacity` \
            (additionalProperties: false) while kinds_ops.rs:294 reads it; \
            op.set_mask{maskId} unimplemented. Composition gap OPEN."]
fn fi13_leg4_the_two_stages_compose() {
    let (blue, inv) = endpoints();
    let mut e = inverted_doc();
    cmd(
        &mut e,
        5,
        "mask.add",
        Some(json!({"layerId": ADJ})),
        json!({"kind": "raster", "from": "reveal_all", "newMaskId": MASK}),
    );
    cmd(
        &mut e,
        6,
        "op.set_params",
        None,
        json!({"opId": OP, "params": {}, "opacity": 0.5}),
    );
    cmd(
        &mut e,
        7,
        "layer.set_props",
        Some(json!({"layerId": ADJ})),
        json!({"opacity": 0.25}),
    );
    nearer_to(
        px(&e),
        blue,
        inv,
        0.125,
        &[
            ("node-opacity squared", 0.0625),
            ("per-op squared", 0.25),
            ("min", 0.25),
            ("node opacity alone", 0.25),
            ("average", 0.375),
            ("max", 0.5),
            ("per-op alone", 0.5),
        ],
    );
}
