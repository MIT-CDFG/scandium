// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! SUBTREE-COMPOSITE DOOR integration suite (O-SCRIPT ABI-1.4 handoff:
//! the GROUP arm of `appname_doc_read_layer` — the render-side door).
//!
//! Spec: ES §4.4 (composite math), §4.6 step 4b (children onto
//! transparency), §4.7 (isolation rows apply WITHIN the subtree), §12.1/
//! §12.2 (parametric serving + inherited refusal matrix), §11.4/§17.1
//! (adjustment reach — refused in subtree scopes), QA-F8
//! (refuse-never-omit); plan §3.4 + the O-SCRIPT log's filed handoff
//! ("content, not contribution" extended to groups).
//!
//! Contract legs proven here:
//!
//! 1. **Identity cross-check**: subtree-composite with the ROOT as target
//!    is byte-equal to `composite_document_with` — on plain, grouped, AND
//!    parametric (adjustment-at-root, segment-pipeline) documents.
//! 2. **Nested-group probe**: a group target containing a nested isolated
//!    group matches a hand-composed §4.4 oracle exactly (dyadic values).
//! 3. **Content, not contribution**: the target's own blend / opacity /
//!    visibility never change the served bytes (while they DO change the
//!    document composite).
//! 4. **Refusal-matrix inheritance**: target- and child-level constructs
//!    the full compositor refuses typed refuse identically through the
//!    subtree door (transform, clipToBelow, masks, enabled-op-under-bare-
//!    registry), plus the door's own documented strictness (adjustments
//!    in a group-target subtree; non-group leaf targets).
//!
//! All documents are built through PUBLIC doors only (package fixtures
//! via [`support`]; envelopes through the shipped assembly composition).

mod support;

use appname_core::EngineAssembly;
use appname_core::{Document, Engine, IVec2, LayerId};
use appname_render::{
    composite_document, composite_document_with, composite_subtree, composite_subtree_with,
    Composite, CompositeError,
};
use serde_json::{json, Value};
use support::{load_fixture, LayerSpec, PkgSpec, TileSpec};

// ---------------------------------------------------------------------
// Fixture: bg + pass-through group A [ paint P, isolated group B [ Q ] ].
// ---------------------------------------------------------------------

const BG: &str = "00000000-0000-4000-8000-0000000000d0";
const GROUP_A: &str = "00000000-0000-4000-8000-0000000000d1";
const PAINT_P: &str = "00000000-0000-4000-8000-0000000000d2";
const GROUP_B: &str = "00000000-0000-4000-8000-0000000000d3";
const PAINT_Q: &str = "00000000-0000-4000-8000-0000000000d4";

/// The nested fixture (all values dyadic — exact in f32):
///
/// * root: `bg` opaque gray 0.5, then group `A` (Normal, opacity 1 —
///   pass-through in the document walk);
/// * `A`: paint `P` = premultiplied `[0.25, 0.25, 0.25, 0.5]` (gray 0.5 @
///   α 0.5), then group `B` (blend multiply — §4.7 row 2 isolates);
/// * `B`: paint `Q` = opaque gray 0.5.
///
/// `mutate` post-edits the spec (target-prop variants, refusal variants).
fn nested_spec(mutate: impl FnOnce(&mut PkgSpec)) -> PkgSpec {
    let mut spec = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    BG,
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.5, 0.5, 0.5, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "A",
                ..LayerSpec::group(GROUP_A, vec![PAINT_P, GROUP_B])
            },
            LayerSpec {
                name: "P",
                ..LayerSpec::raster(
                    PAINT_P,
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.25, 0.25, 0.25, 0.5],
                    }],
                )
            },
            LayerSpec {
                name: "B",
                blend: "multiply",
                ..LayerSpec::group(GROUP_B, vec![PAINT_Q])
            },
            LayerSpec {
                name: "Q",
                ..LayerSpec::raster(
                    PAINT_Q,
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.5, 0.5, 0.5, 1.0],
                    }],
                )
            },
        ],
        root_children: vec![BG, GROUP_A],
    };
    mutate(&mut spec);
    spec
}

fn layer_by_name(doc: &Document, name: &str) -> LayerId {
    doc.z_order_walk()
        .into_iter()
        .find(|id| doc.layer(*id).map(|n| n.name() == name).unwrap_or(false))
        .unwrap_or_else(|| panic!("fixture layer {name} not found"))
}

fn spec_layer<'s>(spec: &'s mut PkgSpec, id: &str) -> &'s mut LayerSpec {
    spec.layers
        .iter_mut()
        .find(|l| l.id == id)
        .expect("fixture layer id")
}

/// Bitwise composite equality: same ROI, same tile grid, same bytes
/// (TileRgba equality is exact f32 payload equality).
fn assert_bitwise_equal(a: &Composite, b: &Composite, ctx: &str) {
    assert_eq!(a.roi(), b.roi(), "{ctx}: ROI");
    let ka: Vec<_> = a.tiles().map(|(k, _)| k).collect();
    let kb: Vec<_> = b.tiles().map(|(k, _)| k).collect();
    assert_eq!(ka, kb, "{ctx}: tile grids");
    for ((k, ta), (_, tb)) in a.tiles().zip(b.tiles()) {
        assert!(ta == tb, "{ctx}: tile {k:?} bytes differ");
    }
}

// ---------------------------------------------------------------------
// Leg 1 — identity cross-check: root-as-target ≡ composite_document_with.
// ---------------------------------------------------------------------

/// Plain + grouped document: subtree(root) is the document composite,
/// bitwise (pass-through AND isolated groups in the tree).
#[test]
fn root_subtree_is_document_composite_bitwise() {
    let engine = load_fixture(&nested_spec(|_| {}));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let whole = composite_document(doc, canvas).expect("document door serves");
    let sub = composite_subtree(doc, doc.root(), canvas).expect("subtree door serves the root");
    assert_bitwise_equal(&whole, &sub, "root-as-subtree identity");
}

/// Parametric document (adjustment at root — the SEGMENT pipeline):
/// subtree(root) rides the same serve, byte-equal, through the `_with`
/// registry door.
#[test]
fn root_subtree_identity_holds_on_parametric_documents() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 48, "height": 32, "background": "white"}),
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
                "newLayerId": "0e150000-0000-4000-8000-0000000000a1",
                "newOpId": "0e150000-0000-4000-8000-0000000000a2",
                "params": {},
            }),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let ops = e.assembly();
    let whole = composite_document_with(doc, ops, canvas).expect("document door serves");
    let sub = composite_subtree_with(doc, ops, doc.root(), canvas)
        .expect("subtree door serves the root, adjustments included");
    assert_bitwise_equal(&whole, &sub, "parametric root-as-subtree identity");
    // The adjustment actually bit (white inverted): not a vacuous pin.
    assert!(sub.pixel(IVec2::new(4, 4))[0] < 0.5);
}

// ---------------------------------------------------------------------
// Leg 2 — nested-group probe vs a hand-composed §4.4 oracle.
// ---------------------------------------------------------------------

/// subtree(A) with A = [P, B{multiply}[Q]], hand-composed (§4.4 Porter-
/// Duff over transparency; all dyadic, exact in f32):
///
/// * scope starts TRANSPARENT; P composites Normal:
///   acc = P = premul `[0.25, 0.25, 0.25, 0.5]`.
/// * B isolates (§4.7 row 2): T_B = Q over transparency = Q (opaque
///   gray 0.5). T_B blends MULTIPLY against acc: αs = 1, Cs = 0.5,
///   αb = 0.5, Cb = 0.25/0.5 = 0.5, B(Cb,Cs) = 0.25;
///   Co = αs(1−αb)Cs + αs·αb·B + (1−αs)αbCb = 0.5·0.5 + 0.5·0.25 + 0
///   = 0.375; αo = 1.
///
/// Content = uniform premultiplied `[0.375, 0.375, 0.375, 1.0]` — and it
/// is NEITHER the document pixel over bg (0.25, computed in-test) NOR the
/// bg value (0.5): the backdrop demonstrably did not leak in.
#[test]
fn group_subtree_matches_hand_composed_oracle() {
    let engine = load_fixture(&nested_spec(|_| {}));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let a = layer_by_name(doc, "A");
    let sub = composite_subtree(doc, a, canvas).expect("group subtree serves");
    for p in [(0, 0), (7, 7), (15, 15)] {
        let got = sub.pixel(IVec2::new(p.0, p.1));
        assert_eq!(
            got.map(f32::to_bits),
            [0.375f32, 0.375, 0.375, 1.0].map(f32::to_bits),
            "hand-composed oracle at {p:?}: got {got:?}"
        );
    }
    // The document pixel is different math (P and B composite over bg):
    // P over bg = 0.5 opaque; multiply(T_B) over that = 0.25 opaque.
    let whole = composite_document(doc, canvas).unwrap();
    let dp = whole.pixel(IVec2::new(7, 7));
    assert_eq!(
        dp.map(f32::to_bits),
        [0.25f32, 0.25, 0.25, 1.0].map(f32::to_bits),
        "document oracle: got {dp:?}"
    );
}

// ---------------------------------------------------------------------
// Leg 3 — content, not contribution.
// ---------------------------------------------------------------------

/// The target's own blend / opacity / visibility change the DOCUMENT
/// composite but never the served subtree bytes.
#[test]
fn subtree_content_ignores_target_contribution_props() {
    let base = load_fixture(&nested_spec(|_| {}));
    let base_doc = base.document().unwrap();
    let canvas = base_doc.geometry().canvas_rect();
    let base_sub = composite_subtree(base_doc, layer_by_name(base_doc, "A"), canvas).unwrap();

    // Decorated + hidden target: screen blend, opacity 0.25, invisible.
    let variant = load_fixture(&nested_spec(|s| {
        let a = spec_layer(s, GROUP_A);
        a.blend = "screen";
        a.opacity = 0.25;
        a.visible = false;
    }));
    let vdoc = variant.document().unwrap();
    let vsub = composite_subtree(vdoc, layer_by_name(vdoc, "A"), canvas)
        .expect("hidden/faded/blended target still serves its content");
    assert_bitwise_equal(&base_sub, &vsub, "content-not-contribution");

    // …while the document composite DID change (hidden A ⇒ bg only).
    let vwhole = composite_document(vdoc, canvas).unwrap();
    assert_eq!(
        vwhole.pixel(IVec2::new(7, 7)).map(f32::to_bits),
        [0.5f32, 0.5, 0.5, 1.0].map(f32::to_bits),
        "hidden A leaves only bg in the document composite"
    );
}

// ---------------------------------------------------------------------
// Leg 4 — refusal-matrix inheritance + the door's own typed refusals.
// ---------------------------------------------------------------------

/// Target-level refusals: unresolvable id, non-group leaf, transform,
/// clipToBelow — each typed, mirroring the flatten arms.
#[test]
fn subtree_target_refusals_are_typed() {
    let engine = load_fixture(&nested_spec(|_| {}));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();

    // Unresolvable target.
    let ghost =
        LayerId::from_uuid(uuid::Uuid::parse_str("00000000-0000-4000-8000-00000000dead").unwrap());
    assert!(
        matches!(
            composite_subtree(doc, ghost, canvas),
            Err(CompositeError::UnsupportedSource { layer, .. }) if layer == ghost
        ),
        "unresolvable target refuses typed"
    );

    // Non-group leaf target (raster P): the door serves root/groups only.
    let p = layer_by_name(doc, "P");
    match composite_subtree(doc, p, canvas) {
        Err(CompositeError::UnsupportedSource { layer, detail }) => {
            assert_eq!(layer, p);
            assert!(
                detail.contains("GROUP"),
                "leaf-target refusal names the door contract: {detail}"
            );
        }
        other => panic!("leaf target must refuse typed, got {other:?}"),
    }

    // Target transform → the §18 refusal, same as the full compositor.
    let engine = load_fixture(&nested_spec(|s| {
        spec_layer(s, GROUP_A).transform = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 0.0, 1.0]);
    }));
    let doc = engine.document().unwrap();
    let a = layer_by_name(doc, "A");
    assert!(matches!(
        composite_subtree(doc, a, canvas),
        Err(CompositeError::TransformNotImplemented { layer }) if layer == a
    ));
    // Inheritance evidence: the full compositor refuses this document too.
    assert!(matches!(
        composite_document(doc, canvas),
        Err(CompositeError::TransformNotImplemented { .. })
    ));

    // Target clipToBelow → the §4.4 clipping refusal, same as the full
    // compositor.
    let engine = load_fixture(&nested_spec(|s| {
        spec_layer(s, GROUP_A).clip_to_below = true;
    }));
    let doc = engine.document().unwrap();
    let a = layer_by_name(doc, "A");
    assert!(matches!(
        composite_subtree(doc, a, canvas),
        Err(CompositeError::ClippingNotImplemented { layer }) if layer == a
    ));
    assert!(matches!(
        composite_document(doc, canvas),
        Err(CompositeError::ClippingNotImplemented { .. })
    ));
}

/// Child-level inheritance: a construct INSIDE the subtree the full
/// compositor refuses typed (clipToBelow on Q, inside nested group B)
/// refuses identically through the subtree door — same variant, same
/// layer named.
#[test]
fn subtree_children_inherit_the_refusal_matrix() {
    let engine = load_fixture(&nested_spec(|s| {
        spec_layer(s, PAINT_Q).clip_to_below = true;
    }));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let q = layer_by_name(doc, "Q");
    let via_document = composite_document(doc, canvas).unwrap_err();
    let via_subtree = composite_subtree(doc, layer_by_name(doc, "A"), canvas).unwrap_err();
    assert!(
        matches!(&via_document, CompositeError::ClippingNotImplemented { layer } if *layer == q)
    );
    assert_eq!(
        via_document, via_subtree,
        "child-level refusals are IDENTICAL through both doors (shared walk)"
    );
}

// ---------------------------------------------------------------------
// Envelope-built legs: adjustments, masks, bare-registry parametric.
// ---------------------------------------------------------------------

/// THE shipped assembly, via the single-source constructor (core door
/// slice 2: no hand-rolled chains outside `appname-assembly`). This suite
/// needs no partiality — every kind it applies ships in production, and
/// its pins are composite bytes + refusal types (the one bare-registry
/// leg builds `EngineAssembly::new()` explicitly, orthogonal to this).
fn assembled_engine() -> Engine {
    appname_assembly::production_assembly().build()
}

fn apply(e: &mut Engine, v: &Value) -> appname_core::ApplyReceipt {
    e.apply_json(&v.to_string())
        .unwrap_or_else(|err| panic!("{v} -> {:?} {}", err.code, err.message))
}

fn env(id: u32, kind: &str, target: Option<Value>, params: Value) -> Value {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("0e150000-0000-4000-8000-{id:012x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(t) = target {
        v["target"] = t;
    }
    v
}

fn sole_group(doc: &Document) -> LayerId {
    doc.z_order_walk()
        .into_iter()
        .find(|id| doc.layer(*id).map(|n| n.is_group()).unwrap_or(false))
        .expect("fixture group")
}

/// The door's documented strictness: an adjustment inside a group-target
/// subtree refuses typed (its §17.1 result over the door's transparency
/// backdrop is not its document content) — while the DOCUMENT door serves
/// the very same document (pass-through group splices the adjustment to
/// root reach). One-directional inheritance, by design.
#[test]
fn adjustment_in_group_subtree_refuses_while_document_serves() {
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
                "newLayerId": "0e150000-0000-4000-8000-0000000000b1",
                "newOpId": "0e150000-0000-4000-8000-0000000000b2",
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
                "layerIds": ["0e150000-0000-4000-8000-0000000000b1"],
                "newGroupId": "0e150000-0000-4000-8000-0000000000b3",
            }),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let ops = e.assembly();
    // Document door: the default-props group is pass-through — serves.
    composite_document_with(doc, ops, canvas)
        .expect("pass-through group splices the adjustment to root reach");
    // Subtree door: refused typed, scope named.
    let g = sole_group(doc);
    match composite_subtree_with(doc, ops, g, canvas) {
        Err(CompositeError::ParametricNotImplemented { detail, .. }) => {
            assert!(
                detail.contains("subtree-composite"),
                "the subtree-door scope is named: {detail}"
            );
        }
        other => panic!("expected ParametricNotImplemented, got {other:?}"),
    }
}

/// Bare-registry inheritance (§12.1 matrix) + masks + a second identity
/// leg: a SOLE pass-through group's subtree equals the document composite
/// (transparent backdrop on both sides — genuinely different walks).
#[test]
fn group_subtree_parametric_and_mask_refusals_inherit() {
    let mut e = assembled_engine();
    apply(
        &mut e,
        &env(
            1,
            "doc.new",
            None,
            json!({"width": 48, "height": 32, "background": "white"}),
        ),
    );
    let bg = e.document().unwrap().z_order_walk()[0];
    apply(
        &mut e,
        &env(
            2,
            "smartobject.convert",
            None,
            json!({
                "layerIds": [bg.as_uuid().to_string()],
                "newLayerId": "0e150000-0000-4000-8000-0000000000c1",
            }),
        ),
    );
    apply(
        &mut e,
        &env(
            3,
            "filter.smart_add",
            Some(json!({"layerId": "0e150000-0000-4000-8000-0000000000c1"})),
            json!({
                "filterId": "filter.gaussian_blur",
                "newOpId": "0e150000-0000-4000-8000-0000000000c2",
                "params": {"radius": 2.0},
            }),
        ),
    );
    apply(
        &mut e,
        &env(
            4,
            "layer.group",
            None,
            json!({
                "layerIds": ["0e150000-0000-4000-8000-0000000000c1"],
                "newGroupId": "0e150000-0000-4000-8000-0000000000c3",
            }),
        ),
    );
    let doc = e.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let g = sole_group(doc);

    // Inherited §12.1 matrix: enabled op under a BARE registry refuses
    // typed through the subtree door, exactly like the full compositor.
    let bare = composite_subtree_with(doc, &EngineAssembly::new(), g, canvas).unwrap_err();
    assert!(
        matches!(bare, CompositeError::ParametricNotImplemented { .. }),
        "enabled op under a bare registry refuses typed: {bare}"
    );
    assert!(matches!(
        composite_document(doc, canvas),
        Err(CompositeError::ParametricNotImplemented { .. })
    ));

    // Assembled registry: serves — and the sole pass-through group's
    // content over transparency IS the document composite (both walks end
    // on a transparent backdrop; the chain evaluates in both).
    let ops = e.assembly();
    let sub = composite_subtree_with(doc, ops, g, canvas).expect("assembled registry serves");
    let whole = composite_document_with(doc, ops, canvas).unwrap();
    assert_bitwise_equal(&whole, &sub, "sole pass-through group identity");

    // Masks: a masked GROUP target refuses typed through BOTH doors (the
    // §11.2 application has no composite path yet — mirrored arms).
    apply(
        &mut e,
        &env(
            5,
            "mask.add",
            Some(json!({"layerId": "0e150000-0000-4000-8000-0000000000c3"})),
            json!({"kind": "raster", "newMaskId": "0e150000-0000-4000-8000-0000000000c4"}),
        ),
    );
    let doc = e.document().unwrap();
    let ops = e.assembly();
    assert!(matches!(
        composite_subtree_with(doc, ops, g, canvas),
        Err(CompositeError::MasksNotImplemented { layer }) if layer == g
    ));
    assert!(matches!(
        composite_document_with(doc, ops, canvas),
        Err(CompositeError::MasksNotImplemented { layer }) if layer == g
    ));
}
