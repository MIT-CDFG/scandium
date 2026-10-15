// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! CP-0.3 compositor integration suite: real documents through core's
//! PUBLIC doors only (fixture factory in [`support`]).
//!
//! Spec: ES §4.4 (composite math), §4.6 (walk order/visibility), §4.7
//! (pass-through groups — row 6 flatten-through is v1's whole group
//! story), §3.7 (dirty recomposition), §6.4 (alpha edges); plan §2.2 rows
//! "cross-check tests computing both paths via public APIs", "z-order/
//! visibility/opacity property tests", "dirty-region correctness".
//!
//! **Cross-check contract** (plan §2.2): core carries a minimal §4.4
//! source-over export composite from CP-0.1 (`export_png_bytes`); on the
//! shared reachable semantics — Normal blend, opacity 1, visibility —
//! the two paths must agree. Fixture values are dyadic rationals so both
//! evaluation orders (core composites premultiplied directly; the render
//! kernel un-premultiplies for `B` per §4.4) are exact in f32 and the
//! comparison is bit-strict after the shared sRGB quantization.

mod support;

use appname_core::io::srgb::{srgb_decode, srgb_encode, unpremul};
use appname_core::{
    decode_png_to_buffer, export_png_bytes, Document, Engine, IRect, IVec2, LayerId, ProfileId,
    TileCoord,
};
use appname_render::{
    composite_document, flatten_document, recomposite_dirty, CompositeError, DirtyMap, DirtyRecord,
    RenderItem,
};
use support::{engine_doc_new, load_fixture, LayerSpec, PkgSpec, TileSpec};

/// Resolve a fixture layer id by its (unique) name through the public
/// read surface.
fn layer_by_name(doc: &Document, name: &str) -> LayerId {
    doc.z_order_walk()
        .into_iter()
        .find(|id| doc.layer(*id).map(|n| n.name() == name).unwrap_or(false))
        .unwrap_or_else(|| panic!("fixture layer {name} not found"))
}

/// The cross-check: composite with the render path, quantize with core's
/// own public sRGB helpers exactly as `export_png_bytes` does, then
/// compare — bit-strict — against core's actual PNG export decoded back
/// through core's own decoder. Byte-identical exports ⇒ bit-identical
/// decoded pixels; one quantization bucket of drift would show.
fn crosscheck_vs_core_export(engine: &Engine) {
    let doc = engine.document().expect("initialized");
    let canvas = doc.geometry().canvas_rect();

    // Path A (core, CP-0.1): §4.4 source-over export composite → PNG.
    let png = export_png_bytes(engine).expect("core export");
    let (core_buf, w, h) = decode_png_to_buffer(&png, ProfileId::nil()).expect("decode");
    assert_eq!((w as i32, h as i32), (canvas.width(), canvas.height()));

    // Path B (render): CPU reference composite.
    let comp = composite_document(doc, canvas).expect("render composite");

    for y in 0..canvas.height() {
        for x in 0..canvas.width() {
            let p = IVec2::new(x, y);
            // Quantize the render result with the same public helpers the
            // core exporter uses (§8.2 straight-alpha 8-bit sRGB).
            let s = unpremul(comp.pixel(p));
            let bytes = [
                (srgb_encode(s[0].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (srgb_encode(s[1].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (srgb_encode(s[2].clamp(0.0, 1.0)) * 255.0).round() as u8,
                (s[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ];
            // What core's decoder produces from exactly those bytes
            // (§6.4 straight→working: decode TRC → linear, × a).
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
                "cross-check mismatch at ({x},{y}): core-export pixel {got:?} vs render-path \
                 prediction {predicted:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------
// Cross-check vs core's CP-0.1 export composite (Normal/opacity-1 set).
// ---------------------------------------------------------------------

#[test]
fn crosscheck_doc_new_white_u8() {
    // Single opaque white background, sub-tile canvas (dense u8 tile).
    let engine = engine_doc_new(64, 48, "u8", "white");
    crosscheck_vs_core_export(&engine);
}

#[test]
fn crosscheck_doc_new_black_multitile_u8() {
    // 300×300: tile (0,0) fully covered (Uniform storage), edge tiles
    // dense — exercises both u8 conversion arms across a 2×2 tile grid.
    let engine = engine_doc_new(300, 300, "u8", "black");
    crosscheck_vs_core_export(&engine);
}

#[test]
fn crosscheck_doc_new_transparent_f32() {
    // Fully transparent document: both paths must export alpha 0.
    let engine = engine_doc_new(40, 40, "f32", "transparent");
    crosscheck_vs_core_export(&engine);
}

#[test]
fn crosscheck_crafted_f32_stack_fractional_alpha() {
    // Three overlapping fractional-alpha layers + one hidden layer, all
    // Normal/opacity-1 (the shared-semantics set; visibility IS shared —
    // core's export honors it). Dyadic premultiplied values keep both
    // evaluation orders exact (file doc). Layer C extends beyond the
    // canvas: out-of-canvas content must not disturb the export region.
    let spec = PkgSpec {
        width: 300,
        height: 260,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "a",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-00000000000a",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.5, 0.25, 0.125, 0.5],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.25, 0.25, 0.0, 0.25],
                        },
                    ],
                )
            },
            LayerSpec {
                name: "b",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-00000000000b",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.25, 0.0, 0.25, 0.25],
                    }],
                )
            },
            LayerSpec {
                name: "hidden",
                visible: false,
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-00000000000c",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [1.0, 1.0, 1.0, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "c",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-00000000000d",
                    vec![
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.0, 0.5, 0.0, 0.5],
                        },
                        TileSpec {
                            x: 1,
                            y: 1,
                            value: [0.5, 0.0, 0.0, 0.5],
                        }, // beyond canvas
                    ],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-00000000000a",
            "00000000-0000-4000-8000-00000000000b",
            "00000000-0000-4000-8000-00000000000c",
            "00000000-0000-4000-8000-00000000000d",
        ],
    };
    let engine = load_fixture(&spec);
    crosscheck_vs_core_export(&engine);

    // Independent §4.4 hand-check of one pixel (derivation): at (0,0)
    // layers a then b apply; c is off-tile, hidden is skipped.
    //   a over transparent: acc = a's premult value = (0.5, 0.25, 0.125, 0.5)
    //   b (αs = 0.25, Cs = (1, 0, 1)) over acc (αb = 0.5, Cb = (1, 0.5, 0.25)):
    //     Co = 0.25·0.5·Cs + 0.25·0.5·Cs + 0.75·0.5·Cb
    //       (B = Cs for Normal; the first two §4.4 terms share Cs)
    //        = 0.25·Cs + 0.375·Cb = (0.625, 0.1875, 0.34375)
    //     αo = 0.25 + 0.5·0.75 = 0.625
    let doc = engine.document().unwrap();
    let comp = composite_document(doc, doc.geometry().canvas_rect()).unwrap();
    assert_eq!(
        comp.pixel(IVec2::new(0, 0)),
        [0.625, 0.1875, 0.34375, 0.625]
    );
}

// ---------------------------------------------------------------------
// Blend set v1 on real documents (loader-carried modes).
// ---------------------------------------------------------------------

#[test]
fn blend_modes_v1_on_real_documents_match_oracle() {
    // The §4.4 worked-example pixel pair as a real two-layer document per
    // mode: backdrop Cb = 0.5 @ αb = 0.8 (premult 0.4/0.8), source
    // Cs = 0.6 @ αs = 0.5 (premult 0.3/0.5). Expected premultiplied
    // channel = 0.26 + 0.4·B (derivation in the blend oracle table,
    // src/blend.rs); Normal and Multiply are the §4.4 worked example's own
    // numbers.
    let cases: [(&str, f32); 8] = [
        ("normal", 0.50),
        ("darken", 0.46),
        ("multiply", 0.38),
        ("lighten", 0.50),
        ("screen", 0.58),
        ("linearDodge", 0.66),
        ("overlay", 0.50),
        ("difference", 0.30),
    ];
    for (blend, want) in cases {
        let spec = PkgSpec {
            width: 32,
            height: 32,
            depth: "f32",
            premultiplied: true,
            layers: vec![
                LayerSpec {
                    name: "backdrop",
                    ..LayerSpec::raster(
                        "00000000-0000-4000-8000-000000000010",
                        vec![TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.4, 0.4, 0.4, 0.8],
                        }],
                    )
                },
                LayerSpec {
                    name: "source",
                    blend,
                    ..LayerSpec::raster(
                        "00000000-0000-4000-8000-000000000011",
                        vec![TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.3, 0.3, 0.3, 0.5],
                        }],
                    )
                },
            ],
            root_children: vec![
                "00000000-0000-4000-8000-000000000010",
                "00000000-0000-4000-8000-000000000011",
            ],
        };
        let engine = load_fixture(&spec);
        let doc = engine.document().unwrap();
        let comp = composite_document(doc, doc.geometry().canvas_rect()).unwrap();
        let got = comp.pixel(IVec2::new(16, 16));
        for (c, &g) in got.iter().enumerate().take(3) {
            assert!(
                (g - want).abs() <= 1e-6,
                "{blend}: channel {c} got {g} want {want}"
            );
        }
        assert!((got[3] - 0.90).abs() <= 1e-6, "{blend}: alpha {}", got[3]);
    }
}

// ---------------------------------------------------------------------
// Opacity, visibility, groups (z-order semantics on real documents).
// ---------------------------------------------------------------------

#[test]
fn layer_opacity_and_fill_opacity_scale_source_alpha() {
    // White opaque layer at opacity 0.5 × fillOpacity 0.5 over opaque
    // black: αs_eff = 0.25 → Co = 0.25·1 + 0.75·0 = 0.25 (§4.4; both
    // opacities reduce to the αs scale with no effects at CP-0.3).
    let spec = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000020",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "white",
                opacity: 0.5,
                fill_opacity: 0.5,
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000021",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [1.0, 1.0, 1.0, 1.0],
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-000000000020",
            "00000000-0000-4000-8000-000000000021",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let comp = composite_document(doc, doc.geometry().canvas_rect()).unwrap();
    assert_eq!(comp.pixel(IVec2::new(8, 8)), [0.25, 0.25, 0.25, 1.0]);
}

#[test]
fn opacity_zero_layer_is_skipped_exactly() {
    let mk = |with_zero: bool| {
        let mut layers = vec![LayerSpec {
            name: "bg",
            ..LayerSpec::raster(
                "00000000-0000-4000-8000-000000000030",
                vec![TileSpec {
                    x: 0,
                    y: 0,
                    value: [0.25, 0.5, 0.25, 1.0],
                }],
            )
        }];
        let mut children = vec!["00000000-0000-4000-8000-000000000030"];
        if with_zero {
            layers.push(LayerSpec {
                name: "ghost",
                opacity: 0.0,
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000031",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [1.0, 0.0, 0.0, 1.0],
                    }],
                )
            });
            children.push("00000000-0000-4000-8000-000000000031");
        }
        PkgSpec {
            width: 16,
            height: 16,
            depth: "f32",
            premultiplied: true,
            layers,
            root_children: children,
        }
    };
    let with = load_fixture(&mk(true));
    let without = load_fixture(&mk(false));
    let (dw, dwo) = (with.document().unwrap(), without.document().unwrap());
    // αs = 0 ⇒ identity for every mode (§4.4): flatten drops the layer …
    assert_eq!(flatten_document(dw).unwrap().len(), 1);
    // … and the composites agree pixel-for-pixel.
    let canvas = dw.geometry().canvas_rect();
    let a = composite_document(dw, canvas)
        .unwrap()
        .region_rgba_f32(canvas);
    let b = composite_document(dwo, canvas)
        .unwrap()
        .region_rgba_f32(canvas);
    assert_eq!(a, b);
}

#[test]
fn hidden_group_hides_its_whole_subtree() {
    // §4.6 step 1: a hidden GROUP skips its children even when the
    // children themselves are marked visible.
    let spec = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000040",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.125, 0.125, 0.125, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "g",
                visible: false,
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-000000000041",
                    vec!["00000000-0000-4000-8000-000000000042"],
                )
            },
            LayerSpec {
                name: "inner",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000042",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [1.0, 0.0, 0.0, 1.0],
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-000000000040",
            "00000000-0000-4000-8000-000000000041",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let flat = flatten_document(doc).unwrap();
    assert_eq!(
        flat.len(),
        1,
        "only the background survives the hidden group"
    );
    let comp = composite_document(doc, doc.geometry().canvas_rect()).unwrap();
    assert_eq!(comp.pixel(IVec2::new(4, 4)), [0.125, 0.125, 0.125, 1.0]);
    // Render-vs-reference agreement (the hidden-group ruling's whole point):
    // core's CPU-reference export must MATCH render here — i.e. it must also
    // exclude the own-visible `inner` leaf inside the hidden group `g`.
    // Pre-fix the reference raster arm filtered by own visibility only and
    // exported `inner` (pure red) while render showed only the bg → this
    // cross-check failed. It now agrees bit-strict.
    crosscheck_vs_core_export(&engine);
}

#[test]
fn pass_through_group_flattens_identically() {
    // ES §4.7 row 6 (the v1 group story, module doc of the compositor):
    // an undecorated group's children blend against the accumulated
    // backdrop exactly as if the group were not there.
    let rasters = || {
        vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000050",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.0, 0.0, 0.5, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "m1",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000052",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.25, 0.0, 0.0, 0.25],
                    }],
                )
            },
            LayerSpec {
                name: "m2",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000053",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.0, 0.125, 0.0, 0.125],
                    }],
                )
            },
        ]
    };
    let mut grouped_layers = rasters();
    grouped_layers.push(LayerSpec::group(
        "00000000-0000-4000-8000-000000000051",
        vec![
            "00000000-0000-4000-8000-000000000052",
            "00000000-0000-4000-8000-000000000053",
        ],
    ));
    let grouped = PkgSpec {
        width: 32,
        height: 32,
        depth: "f32",
        premultiplied: true,
        layers: grouped_layers,
        root_children: vec![
            "00000000-0000-4000-8000-000000000050",
            "00000000-0000-4000-8000-000000000051",
        ],
    };
    let flat_doc = PkgSpec {
        width: 32,
        height: 32,
        depth: "f32",
        premultiplied: true,
        layers: rasters(),
        root_children: vec![
            "00000000-0000-4000-8000-000000000050",
            "00000000-0000-4000-8000-000000000052",
            "00000000-0000-4000-8000-000000000053",
        ],
    };
    let (ea, eb) = (load_fixture(&grouped), load_fixture(&flat_doc));
    let (da, db) = (ea.document().unwrap(), eb.document().unwrap());
    let canvas = da.geometry().canvas_rect();
    let a = composite_document(da, canvas)
        .unwrap()
        .region_rgba_f32(canvas);
    let b = composite_document(db, canvas)
        .unwrap()
        .region_rgba_f32(canvas);
    assert_eq!(
        a, b,
        "pass-through group must compose identically to its flattening"
    );
}

#[test]
fn flatten_order_matches_z_order_walk() {
    // On a document with no hidden GROUPS, the flatten order is exactly
    // core's canonical z_order_walk filtered to visible rasters.
    let spec = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec::raster("00000000-0000-4000-8000-000000000060", vec![]),
            LayerSpec::group(
                "00000000-0000-4000-8000-000000000061",
                vec![
                    "00000000-0000-4000-8000-000000000062",
                    "00000000-0000-4000-8000-000000000063",
                ],
            ),
            LayerSpec::raster("00000000-0000-4000-8000-000000000062", vec![]),
            LayerSpec {
                name: "hidden-raster",
                visible: false,
                ..LayerSpec::raster("00000000-0000-4000-8000-000000000063", vec![])
            },
            LayerSpec::raster("00000000-0000-4000-8000-000000000064", vec![]),
        ],
        root_children: vec![
            "00000000-0000-4000-8000-000000000060",
            "00000000-0000-4000-8000-000000000061",
            "00000000-0000-4000-8000-000000000064",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let flat: Vec<LayerId> = flatten_document(doc)
        .unwrap()
        .iter()
        .map(|l| l.layer_id())
        .collect();
    let expected: Vec<LayerId> = doc
        .z_order_walk()
        .into_iter()
        .filter(|id| {
            let n = doc.layer(*id).unwrap();
            n.visible() && n.pixels().is_some()
        })
        .collect();
    assert_eq!(flat, expected);
}

// ---------------------------------------------------------------------
// Typed refusals (never silent wrong math).
// ---------------------------------------------------------------------

fn one_layer_spec(mutator: impl FnOnce(&mut LayerSpec)) -> PkgSpec {
    let mut layer = LayerSpec::raster(
        "00000000-0000-4000-8000-000000000070",
        vec![TileSpec {
            x: 0,
            y: 0,
            value: [0.25, 0.25, 0.25, 0.5],
        }],
    );
    mutator(&mut layer);
    PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![layer],
        root_children: vec!["00000000-0000-4000-8000-000000000070"],
    }
}

fn group_over_layer_spec(mutator: impl FnOnce(&mut LayerSpec)) -> PkgSpec {
    let mut group = LayerSpec::group(
        "00000000-0000-4000-8000-000000000071",
        vec!["00000000-0000-4000-8000-000000000070"],
    );
    mutator(&mut group);
    PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            group,
            LayerSpec::raster(
                "00000000-0000-4000-8000-000000000070",
                vec![TileSpec {
                    x: 0,
                    y: 0,
                    value: [0.25, 0.25, 0.25, 0.5],
                }],
            ),
        ],
        root_children: vec!["00000000-0000-4000-8000-000000000071"],
    }
}

#[test]
fn cp05_constructs_are_typed_refusals() {
    // CP-0.5 slice A: the full 27-mode catalog flattens — the CP-0.3
    // 19-mode refusal is retired (a colorBurn raster now composites; the
    // conformance-side pin retires with O-QA's wave-2 extension).
    let engine = load_fixture(&one_layer_spec(|l| l.blend = "colorBurn"));
    let doc = engine.document().unwrap();
    let flat = flatten_document(doc).expect("full catalog flattens");
    assert_eq!(flat.len(), 1);
    assert_eq!(
        flat[0].as_layer().expect("leaf").mode,
        appname_core::BlendMode::ColorBurn
    );

    // clip_to_below → ClippingNotImplemented (§4.4 clipping groups).
    let engine = load_fixture(&one_layer_spec(|l| l.clip_to_below = true));
    assert!(matches!(
        flatten_document(engine.document().unwrap()),
        Err(CompositeError::ClippingNotImplemented { .. })
    ));

    // Non-identity transform → TransformNotImplemented (§18).
    let engine = load_fixture(&one_layer_spec(|l| {
        l.transform = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 0.0, 1.0])
    }));
    assert!(matches!(
        flatten_document(engine.document().unwrap()),
        Err(CompositeError::TransformNotImplemented { .. })
    ));

    // Group isolation (§4.7 decision table) — CP-0.5 slice B: rows 2/3
    // now BUILD isolated subtrees instead of refusing (semantics pinned
    // by the group-compositing suite below); fill_opacity no longer
    // isolates or refuses (§4.6 step 4b — ignored for groups). Row 1
    // (explicit isolated=true) is compiled in but unreachable through
    // public doors (loader pins isolated=false — lane-log note).
    let engine = load_fixture(&group_over_layer_spec(|g| g.blend = "multiply"));
    let flat = flatten_document(engine.document().unwrap()).unwrap();
    assert!(
        matches!(&flat[..], [RenderItem::Isolated(g)] if g.children().len() == 1),
        "row 2 (blend) must isolate: {flat:?}"
    );
    let engine = load_fixture(&group_over_layer_spec(|g| g.opacity = 0.5));
    let flat = flatten_document(engine.document().unwrap()).unwrap();
    assert!(
        matches!(&flat[..], [RenderItem::Isolated(g)] if g.opacity == 0.5),
        "row 3 (opacity) must isolate: {flat:?}"
    );
    let engine = load_fixture(&group_over_layer_spec(|g| g.fill_opacity = 0.5));
    let flat = flatten_document(engine.document().unwrap()).unwrap();
    assert!(
        matches!(&flat[..], [RenderItem::Layer(_)]),
        "group fill_opacity neither isolates nor refuses (§4.6 step 4b): {flat:?}"
    );

    // A hidden decorated group is still skipped wholesale — visibility
    // wins before isolation is even considered (§4.6 step 1).
    let engine = load_fixture(&group_over_layer_spec(|g| {
        g.blend = "multiply";
        g.visible = false;
    }));
    assert!(flatten_document(engine.document().unwrap())
        .unwrap()
        .is_empty());
}

// ---------------------------------------------------------------------
// CP-0.5 slice B: pass-through vs isolated groups (§4.7 / §4.6 step 4b).
// ---------------------------------------------------------------------

/// bg (opaque) at root, then a group containing one paint layer.
fn bg_group_paint_spec(
    bg_value: [f32; 4],
    paint_value: [f32; 4],
    g: impl FnOnce(&mut LayerSpec),
) -> PkgSpec {
    let mut group = LayerSpec::group(
        "00000000-0000-4000-8000-0000000000a1",
        vec!["00000000-0000-4000-8000-0000000000a2"],
    );
    g(&mut group);
    PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000a0",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: bg_value,
                    }],
                )
            },
            group,
            LayerSpec {
                name: "paint",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000a2",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: paint_value,
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000a0",
            "00000000-0000-4000-8000-0000000000a1",
        ],
    }
}

fn probe(comp: &appname_render::Composite, x: i32, y: i32) -> [f32; 4] {
    comp.pixel(appname_core::IVec2::new(x, y))
}

/// The row-2 discriminator, hand-computed both ways (§4.4/§4.6 step 4b):
/// group{blend=multiply}[gray 0.6 @ α .5, Normal] over opaque gray 0.5.
///
///   Isolated (correct): T = paint over transparency = (0.3, ·, ·, 0.5).
///   T over bg with Multiply: Cs = 0.6, αs = 0.5, Cb = 0.5, αb = 1:
///   B = 0.30; Co = 0.5·0·0.6 + 0.5·1·0.30 + 0.5·1·0.5 = 0.40; αo = 1.
///
///   Flatten-through (wrong here): the child's own Normal against the
///   backdrop: Co = 0.5·0.6 + 0.5·0.5 = 0.55 — visibly different.
#[test]
fn isolated_group_blend_discriminates_from_pass_through() {
    let spec = bg_group_paint_spec([0.5, 0.5, 0.5, 1.0], [0.30, 0.30, 0.30, 0.50], |g| {
        g.blend = "multiply"
    });
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    let got = probe(&comp, 7, 7);
    for (c, &g) in got.iter().enumerate().take(3) {
        assert!((g - 0.40).abs() <= 1e-6, "isolated multiply: ch{c} = {g}");
        assert!(
            (g - 0.55).abs() > 0.1,
            "must NOT be the flatten-through value"
        );
    }
    assert_eq!(got[3], 1.0);
    assert_equals_naive(doc, canvas, &comp);
}

/// The row-3 discriminator (the classic group-fade case): group{opacity
/// 0.5}[green opaque, red opaque] over opaque white.
///
///   Isolated (correct): T = red (the top child covers green in T).
///   T over white at group opacity 0.5: Co = 0.5·Cs + 0.5·Cb →
///   (1.0, 0.5, 0.5), αo = 1.
///
///   Per-layer fade (wrong): red@.5 over (green@.5 over white) =
///   (0.75, 0.5, 0.25) — the halo the intermediate exists to prevent.
#[test]
fn group_opacity_fades_the_composite_not_the_layers() {
    let spec = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000b0",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [1.0, 1.0, 1.0, 1.0],
                    }],
                )
            },
            LayerSpec {
                opacity: 0.5,
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000b1",
                    vec![
                        "00000000-0000-4000-8000-0000000000b2",
                        "00000000-0000-4000-8000-0000000000b3",
                    ],
                )
            },
            LayerSpec {
                name: "green",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000b2",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.0, 1.0, 0.0, 1.0],
                    }],
                )
            },
            LayerSpec {
                name: "red",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000b3",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [1.0, 0.0, 0.0, 1.0],
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000b0",
            "00000000-0000-4000-8000-0000000000b1",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    let got = probe(&comp, 3, 12);
    let want = [1.0f32, 0.5, 0.5, 1.0];
    let wrong = [0.75f32, 0.5, 0.25];
    for c in 0..4 {
        assert!(
            (got[c] - want[c]).abs() <= 1e-6,
            "ch{c}: {} want {}",
            got[c],
            want[c]
        );
    }
    assert!((got[0] - wrong[0]).abs() > 0.1 && (got[2] - wrong[2]).abs() > 0.1);
    assert_equals_naive(doc, canvas, &comp);
}

/// Nesting: isolated-inside-pass-through-inside-isolated, verified
/// against the recursive naive reference; two loads are bit-identical
/// (determinism).
#[test]
fn nested_isolation_equals_naive_and_is_deterministic() {
    let spec = PkgSpec {
        width: 300,
        height: 260,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000c0",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.125, 0.125, 0.125, 1.0],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.25, 0.25, 0.25, 1.0],
                        },
                    ],
                )
            },
            // outer ISOLATED (row 3: opacity .5)
            LayerSpec {
                name: "outer",
                opacity: 0.5,
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000c1",
                    vec![
                        "00000000-0000-4000-8000-0000000000c2",
                        "00000000-0000-4000-8000-0000000000c3",
                    ],
                )
            },
            LayerSpec {
                name: "a",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000c2",
                    vec![TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.5, 0.25, 0.125, 0.5],
                    }],
                )
            },
            // middle PASS-THROUGH (row 6)
            LayerSpec {
                name: "mid",
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000c3",
                    vec!["00000000-0000-4000-8000-0000000000c4"],
                )
            },
            // inner ISOLATED (row 2: screen)
            LayerSpec {
                name: "inner",
                blend: "screen",
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000c4",
                    vec!["00000000-0000-4000-8000-0000000000c5"],
                )
            },
            LayerSpec {
                name: "b",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000c5",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.25, 0.0, 0.25, 0.25],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.0, 0.5, 0.0, 0.5],
                        },
                    ],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000c0",
            "00000000-0000-4000-8000-0000000000c1",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    assert_equals_naive(doc, canvas, &comp);
    // Structure: root = [bg leaf, isolated(outer)]; outer children =
    // [leaf a, isolated(inner)] — "mid" flattened through.
    let tree = flatten_document(doc).unwrap();
    assert_eq!(tree.len(), 2);
    let outer = tree[1].as_isolated().expect("outer isolates (row 3)");
    assert_eq!(outer.children().len(), 2);
    assert!(
        outer.children()[1].as_isolated().is_some(),
        "inner isolates (row 2), mid flattens"
    );
    // Determinism across loads.
    let engine2 = load_fixture(&spec);
    let doc2 = engine2.document().unwrap();
    let comp2 = composite_document(doc2, canvas).unwrap();
    assert_eq!(comp.region_rgba_f32(canvas), comp2.region_rgba_f32(canvas));
}

/// Group opacity 0 skips the whole subtree — bitwise identical to the
/// document without the group (§4.4 αs = 0 identity, exact).
#[test]
fn group_opacity_zero_skips_subtree_bitwise() {
    let with_group = bg_group_paint_spec([0.5, 0.5, 0.5, 1.0], [0.30, 0.30, 0.30, 0.50], |g| {
        g.opacity = 0.0
    });
    let bg_only = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: true,
        layers: vec![LayerSpec {
            name: "bg",
            ..LayerSpec::raster(
                "00000000-0000-4000-8000-0000000000a0",
                vec![TileSpec {
                    x: 0,
                    y: 0,
                    value: [0.5, 0.5, 0.5, 1.0],
                }],
            )
        }],
        root_children: vec!["00000000-0000-4000-8000-0000000000a0"],
    };
    let (ea, eb) = (load_fixture(&with_group), load_fixture(&bg_only));
    let (da, db) = (ea.document().unwrap(), eb.document().unwrap());
    let canvas = da.geometry().canvas_rect();
    let (ca, cb) = (
        composite_document(da, canvas).unwrap(),
        composite_document(db, canvas).unwrap(),
    );
    assert_eq!(ca.region_rgba_f32(canvas), cb.region_rgba_f32(canvas));
}

/// Group fill_opacity is ignored for group compositing (§4.6 step 4b
/// passes only blend + opacity for groups; module-doc note): composites
/// with fill 0.5 and fill 1.0 are bitwise identical.
#[test]
fn group_fill_opacity_is_ignored_per_4_6_step_4b() {
    let half = bg_group_paint_spec([0.5, 0.5, 0.5, 1.0], [0.30, 0.30, 0.30, 0.50], |g| {
        g.blend = "multiply";
        g.fill_opacity = 0.5;
    });
    let full = bg_group_paint_spec([0.5, 0.5, 0.5, 1.0], [0.30, 0.30, 0.30, 0.50], |g| {
        g.blend = "multiply"
    });
    let (ea, eb) = (load_fixture(&half), load_fixture(&full));
    let (da, db) = (ea.document().unwrap(), eb.document().unwrap());
    let canvas = da.geometry().canvas_rect();
    assert_eq!(
        composite_document(da, canvas)
            .unwrap()
            .region_rgba_f32(canvas),
        composite_document(db, canvas)
            .unwrap()
            .region_rgba_f32(canvas),
    );
}

/// A group carrying Dissolve seeds from the GROUP's uuid (§4.5 seed
/// rule applied to the compositing node): the pass/fail pattern follows
/// bn_offsets(doc, GROUP) exactly — checked pixel-by-pixel against the
/// public threshold formula — and the whole composite equals the naive
/// reference (which re-derives the context independently).
#[test]
fn group_dissolve_seeds_from_group_uuid() {
    let spec = bg_group_paint_spec(
        [0.5, 0.5, 0.5, 1.0],
        [0.30, 0.30, 0.30, 0.50], // T alpha = 0.5 → mixed coverage
        |g| g.blend = "dissolve",
    );
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    assert_equals_naive(doc, canvas, &comp);
    let tree = flatten_document(doc).unwrap();
    let group_id = tree[1]
        .as_isolated()
        .expect("dissolve group isolates (row 2)")
        .layer;
    let off = appname_render::bn_offsets(doc.id(), group_id);
    for y in 0..16 {
        for x in 0..16 {
            let n = appname_render::bn_threshold(x, y, off);
            let got = probe(&comp, x, y);
            if 0.5 > n {
                // Passing: T replaces (Cs = 0.6 gray at α 1 over bg).
                assert!(
                    (got[0] - 0.6).abs() <= 1e-6,
                    "({x},{y}) expected pass: {got:?}"
                );
            } else {
                // Failing: the opaque backdrop, bitwise (αb = 1 exact).
                assert_eq!(got, [0.5, 0.5, 0.5, 1.0], "({x},{y}) expected fail");
            }
        }
    }
}

/// Slice-B review MINOR-1: the group's FINAL composite through the
/// whole-pixel and non-separable routings (§4.6 step 4b holds for all
/// 27 modes). The naive reference derives its dispatch independently
/// (composite_tile_ctx by MODE), so a production mis-map of
/// WholePixel/NonSeparable in the group arm diverges here.
#[test]
fn isolated_group_composites_through_triple_routings() {
    for blend in [
        "hue",
        "saturation",
        "color",
        "luminosity",
        "darkerColor",
        "lighterColor",
    ] {
        let spec = bg_group_paint_spec(
            [0.8, 0.1, 0.1, 1.0],    // saturated red backdrop
            [0.05, 0.15, 0.45, 0.5], // blue-ish translucent paint
            |g| g.blend = blend,
        );
        let engine = load_fixture(&spec);
        let doc = engine.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        let comp = composite_document(doc, canvas).unwrap();
        assert_equals_naive(doc, canvas, &comp);
        // The group really took the triple path: result differs from the
        // same fixture with a Normal-blend group (guards a silent
        // fall-through to B = Cs).
        let normal = bg_group_paint_spec(
            [0.8, 0.1, 0.1, 1.0],
            [0.05, 0.15, 0.45, 0.5],
            |g| g.opacity = 0.999, // isolate via row 3, keep Normal blend
        );
        let en = load_fixture(&normal);
        let dn = en.document().unwrap();
        let cn = composite_document(dn, canvas).unwrap();
        assert_ne!(
            comp.region_rgba_f32(canvas),
            cn.region_rgba_f32(canvas),
            "{blend}: triple-routed group must differ from a Normal group"
        );
    }
}

/// Slice-B review MINOR-2: group Dissolve across MULTIPLE tiles — the
/// group EvalFrame's DissolveCtx.tile_origin is exercised away from
/// (0,0), where an origin swap/sign bug becomes visible (§4.5 document
/// space; tile seams invisible). Verified against the raw threshold
/// formula at tile (1,0) coordinates AND the naive reference.
#[test]
fn group_dissolve_document_space_across_tiles() {
    let spec = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000e5",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.5, 0.5, 0.5, 1.0],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.5, 0.5, 0.5, 1.0],
                        },
                    ],
                )
            },
            LayerSpec {
                name: "g",
                blend: "dissolve",
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000e6",
                    vec!["00000000-0000-4000-8000-0000000000e7"],
                )
            },
            LayerSpec {
                name: "paint",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000e7",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.30, 0.30, 0.30, 0.50],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.30, 0.30, 0.30, 0.50],
                        },
                    ],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000e5",
            "00000000-0000-4000-8000-0000000000e6",
        ],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    assert_equals_naive(doc, canvas, &comp);
    let tree = flatten_document(doc).unwrap();
    let off = appname_render::bn_offsets(doc.id(), tree[1].as_isolated().unwrap().layer);
    // Probe rows across BOTH tiles including the seam column x = 256.
    for y in [0i32, 100, 255] {
        for x in [0i32, 200, 255, 256, 257, 400, 511] {
            let n = appname_render::bn_threshold(x, y, off);
            let got = comp.pixel(appname_core::IVec2::new(x, y));
            if 0.5 > n {
                assert!(
                    (got[0] - 0.6).abs() <= 1e-6,
                    "({x},{y}) expected pass: {got:?}"
                );
            } else {
                assert_eq!(got, [0.5, 0.5, 0.5, 1.0], "({x},{y}) expected fail");
            }
        }
    }
}

/// An isolated group none of whose children have a present tile at a
/// coordinate is skipped there BITWISE — matching the absent-tile leaf
/// convention (module-doc rule).
#[test]
fn untouched_group_scope_skips_bitwise() {
    // Canvas 512×256: bg covers tiles (0,0)+(1,0); the group's paint has
    // a tile ONLY at (1,0).
    let with_group = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000d0",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.1, 0.7, 0.3, 0.9],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.2, 0.9, 0.05, 0.95],
                        },
                    ],
                )
            },
            LayerSpec {
                blend: "multiply",
                ..LayerSpec::group(
                    "00000000-0000-4000-8000-0000000000d1",
                    vec!["00000000-0000-4000-8000-0000000000d2"],
                )
            },
            LayerSpec {
                name: "paint",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000d2",
                    vec![TileSpec {
                        x: 1,
                        y: 0,
                        value: [0.25, 0.0, 0.0, 0.25],
                    }],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000d0",
            "00000000-0000-4000-8000-0000000000d1",
        ],
    };
    let bg_only = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![LayerSpec {
            name: "bg",
            ..LayerSpec::raster(
                "00000000-0000-4000-8000-0000000000d0",
                vec![
                    TileSpec {
                        x: 0,
                        y: 0,
                        value: [0.1, 0.7, 0.3, 0.9],
                    },
                    TileSpec {
                        x: 1,
                        y: 0,
                        value: [0.2, 0.9, 0.05, 0.95],
                    },
                ],
            )
        }],
        root_children: vec!["00000000-0000-4000-8000-0000000000d0"],
    };
    let (ea, eb) = (load_fixture(&with_group), load_fixture(&bg_only));
    let (da, db) = (ea.document().unwrap(), eb.document().unwrap());
    let canvas = da.geometry().canvas_rect();
    let ca = composite_document(da, canvas).unwrap();
    let cb = composite_document(db, canvas).unwrap();
    // Tile (0,0): the group contributed nothing — bitwise the bg tile.
    assert!(
        ca.tile(0, 0).unwrap() == cb.tile(0, 0).unwrap(),
        "untouched scope must be a bitwise no-op"
    );
    // Tile (1,0): the group applied — differs from bg-only, equals naive.
    assert!(ca.tile(1, 0).unwrap() != cb.tile(1, 0).unwrap());
    assert_equals_naive(da, canvas, &ca);
}

// ---------------------------------------------------------------------
// Straight-alpha storage (§6.4 straight→working) — render-path semantics.
// ---------------------------------------------------------------------

#[test]
fn straight_alpha_f32_storage_premultiplies_on_working_conversion() {
    // premultiplied=false f32 storage: working conversion multiplies by
    // alpha (§6.4 straight→working row). NOTE (lane-log divergence flag):
    // core's CP-0.1 export treats every f32 buffer as premultiplied —
    // straight-f32 is unreachable from commands/imports today, so the
    // shared set is untouched; the wave-2 delegation should route through
    // this conversion.
    let spec = PkgSpec {
        width: 16,
        height: 16,
        depth: "f32",
        premultiplied: false,
        layers: vec![LayerSpec::raster(
            "00000000-0000-4000-8000-000000000080",
            vec![TileSpec {
                x: 0,
                y: 0,
                value: [1.0, 0.5, 0.25, 0.5],
            }],
        )],
        root_children: vec!["00000000-0000-4000-8000-000000000080"],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let comp = composite_document(doc, doc.geometry().canvas_rect()).unwrap();
    // Working = (1.0·0.5, 0.5·0.5, 0.25·0.5, 0.5) over transparency.
    assert_eq!(comp.pixel(IVec2::new(0, 0)), [0.5, 0.25, 0.125, 0.5]);
}

// ---------------------------------------------------------------------
// Dirty-region correctness (§3.7): dirty subset recomposite == full.
// ---------------------------------------------------------------------

fn dirty_fixture(edited: bool) -> PkgSpec {
    // Canvas 512×256 → tiles (0,0) and (1,0). The "edit" changes layer
    // "paint"'s tile (1,0) only.
    PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: true,
        layers: vec![
            LayerSpec {
                name: "bg",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000090",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.125, 0.125, 0.125, 1.0],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [0.125, 0.125, 0.125, 1.0],
                        },
                    ],
                )
            },
            LayerSpec {
                name: "paint",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-000000000091",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.25, 0.0, 0.0, 0.25],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: if edited {
                                [0.0, 0.5, 0.0, 0.5]
                            } else {
                                [0.25, 0.0, 0.0, 0.25]
                            },
                        },
                    ],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-000000000090",
            "00000000-0000-4000-8000-000000000091",
        ],
    }
}

#[test]
fn dirty_subset_recomposite_equals_full_recomposite() {
    let before = load_fixture(&dirty_fixture(false));
    let after = load_fixture(&dirty_fixture(true));
    let (db, da) = (before.document().unwrap(), after.document().unwrap());
    let canvas = db.geometry().canvas_rect();

    // Full composite of the BEFORE state, then a receipt-shaped dirty feed
    // for the edited tile, recomposited against the AFTER state.
    let mut comp = composite_document(db, canvas).unwrap();
    let edited_rect = TileCoord::new(0, 1, 0).pixel_rect();
    let dirty = DirtyMap::from_records(&[DirtyRecord {
        layer: layer_by_name(da, "paint"),
        rect: edited_rect,
    }]);
    let stats = recomposite_dirty(&mut comp, da, &dirty).unwrap();
    assert_eq!(
        stats.tiles_recomposited, 1,
        "exactly the dirty tile recomposites"
    );
    assert_eq!(stats.tiles_in_composite, 2);

    // Dirty-subset result == full recomposite of the AFTER state.
    let full = composite_document(da, canvas).unwrap();
    assert_eq!(comp.region_rgba_f32(canvas), full.region_rgba_f32(canvas));

    // Negative control: an empty dirty map recomposites nothing and the
    // stale composite still matches the BEFORE state.
    let mut stale = composite_document(db, canvas).unwrap();
    let none = DirtyMap::from_records(&[]);
    let stats = recomposite_dirty(&mut stale, da, &none).unwrap();
    assert_eq!(stats.tiles_recomposited, 0);
    let before_full = composite_document(db, canvas).unwrap();
    assert_eq!(
        stale.region_rgba_f32(canvas),
        before_full.region_rgba_f32(canvas)
    );
}

#[test]
fn dirty_rect_granularity_is_tiles() {
    // A 1-px dirty rect inside tile (0,0) recomposites that whole tile and
    // nothing else (tile-granular contract, §3.7 consumers note).
    let before = load_fixture(&dirty_fixture(false));
    let after = load_fixture(&dirty_fixture(true));
    let (db, da) = (before.document().unwrap(), after.document().unwrap());
    let canvas = db.geometry().canvas_rect();
    let mut comp = composite_document(db, canvas).unwrap();
    let dirty = DirtyMap::from_records(&[DirtyRecord {
        layer: layer_by_name(da, "paint"),
        rect: IRect::new(10, 10, 11, 11), // inside tile (0,0) — the UNEDITED tile
    }]);
    let stats = recomposite_dirty(&mut comp, da, &dirty).unwrap();
    assert_eq!(stats.tiles_recomposited, 1);
    // Tile (0,0) content is identical in both states, so the composite
    // still equals the BEFORE full composite (the edit at (1,0) was not
    // marked, hence — correctly — not recomposited).
    let before_full = composite_document(db, canvas).unwrap();
    assert_eq!(
        comp.region_rgba_f32(canvas),
        before_full.region_rgba_f32(canvas)
    );
}

// ---------------------------------------------------------------------
// MAJOR-3b/MINOR-5: optimized paths vs a naive public-API reference.
// ---------------------------------------------------------------------

/// The reference composite, reconstructed from PUBLIC API only: flatten
/// (same walk/refusals), then per tile a fresh accumulator with every
/// present source tile fully materialized via `from_storage` and
/// composited via the mode-dispatched `composite_tile` — no cache, no
/// uniform fast path, no in-place reuse. The byte-equality reference for
/// the cached/uniform/in-place paths.
///
/// One NORMATIVE exception mirrors the O-INT ruling (perf-round review
/// minor): a `Uniform` source whose converted working alpha is 0 is
/// SKIPPED — §4.4 with αs = 0 is the identity, and the materialized
/// kernel's un-premultiply→re-premultiply round trip is 1-ulp non-exact
/// on ~11% of backdrop pairs, so the ruling forbids handing it
/// zero-alpha uniform sources. (Discovered REACHABLE through the public
/// loader by the tripwire below — not CP-0.5-latent as first assumed.)
/// One scope of the naive reference, RECURSIVE (test-only clarity —
/// fixtures are shallow; production uses the iterative evaluator):
/// §4.6 step 4b re-derived independently — isolated children composite
/// onto transparency, then ONE composite_tile_ctx against the backdrop
/// with the group's mode and opacity; scopes with no contribution are
/// skipped, mirroring the production rule. Returns "touched".
fn naive_scope_into(
    doc: &Document,
    items: &[RenderItem<'_>],
    acc: &mut appname_render::TileRgba,
    tx: i32,
    ty: i32,
) -> bool {
    let c = appname_core::TileCoord::new(0, tx, ty);
    let tile_origin = (tx * appname_core::TILE_I32, ty * appname_core::TILE_I32);
    let mut touched = false;
    for item in items {
        match item {
            RenderItem::Layer(l) => {
                let buf = doc.layer(l.layer).unwrap().pixels().unwrap();
                let Some(storage) = buf.get_tile(c) else {
                    continue;
                };
                let src = appname_render::TileRgba::from_storage(
                    storage,
                    buf.kind(),
                    buf.premultiplied(),
                );
                if storage.is_uniform() && src.get(0, 0)[3] == 0.0 {
                    continue; // NORMATIVE zero-alpha-uniform skip (fn doc)
                }
                // Independent Dissolve context (§4.5): re-derived from
                // the public seed rule.
                let ctx = appname_render::DissolveCtx {
                    bn_off: appname_render::bn_offsets(doc.id(), l.layer),
                    tile_origin,
                };
                appname_render::composite_tile_ctx(&mut *acc, &src, l.mode, l.opacity, ctx);
                touched = true;
            }
            RenderItem::Isolated(g) => {
                let mut inner = appname_render::TileRgba::transparent();
                if !naive_scope_into(doc, g.children(), &mut inner, tx, ty) {
                    continue; // untouched-scope skip (production rule)
                }
                let ctx = appname_render::DissolveCtx {
                    bn_off: appname_render::bn_offsets(doc.id(), g.layer),
                    tile_origin,
                };
                appname_render::composite_tile_ctx(&mut *acc, &inner, g.mode, g.opacity, ctx);
                touched = true;
            }
            RenderItem::Smart(_) | RenderItem::Adjustment(_) | RenderItem::Page(_) => {
                unreachable!("this naive reference's fixtures are raster/group only")
            }
        }
    }
    touched
}

fn naive_reference_tiles(
    doc: &Document,
    roi: IRect,
) -> Vec<((i32, i32), appname_render::TileRgba)> {
    let tree = flatten_document(doc).expect("reference flatten");
    let mut out = Vec::new();
    for (tx, ty) in roi.tiles(appname_core::TILE_I32) {
        let mut acc = appname_render::TileRgba::transparent();
        naive_scope_into(doc, &tree, &mut acc, tx, ty);
        out.push(((tx, ty), acc));
    }
    out
}

fn assert_equals_naive(doc: &Document, roi: IRect, comp: &appname_render::Composite) {
    let naive = naive_reference_tiles(doc, roi);
    let mut n = 0;
    for ((tx, ty), want) in &naive {
        let got = comp.tile(*tx, *ty).expect("tile present");
        assert!(
            got == want,
            "tile ({tx},{ty}) diverges from the naive reference"
        );
        n += 1;
    }
    assert_eq!(n, comp.tiles().count(), "same tile cover");
}

#[test]
fn optimized_composite_equals_naive_reference() {
    // (a) doc.new black 300×300 u8: Uniform full tile + dense edge tiles
    //     — exercises the uniform fast path AND the u8 LUT dense path
    //     against full materialization.
    let engine = engine_doc_new(300, 300, "u8", "black");
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    assert_equals_naive(doc, canvas, &comp);

    // (b) doc.new white 300×300 f32: F32 Uniform + dense edges.
    let engine = engine_doc_new(300, 300, "f32", "white");
    let doc = engine.document().unwrap();
    let comp = composite_document(doc, canvas).unwrap();
    assert_equals_naive(doc, canvas, &comp);

    // (c) crafted multi-layer fractional-alpha f32 stack (dense tiles,
    //     cache-served on repeat visits) — the wave-1 cross-check fixture.
    let spec = dirty_fixture(false);
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    assert_equals_naive(doc, canvas, &comp);

    // (d) CP-0.5 slice A: every routing family through the document walk
    //     — one fixture per representative mode over the same stack,
    //     production ≡ naive (the naive path re-derives the Dissolve
    //     context independently, so this pins the §4.5 seed rule
    //     end-to-end through the public loader).
    for mode in [
        "colorBurn",
        "vividLight",
        "darkerColor",
        "hue",
        "luminosity",
        "dissolve",
    ] {
        let mut spec = dirty_fixture(false);
        spec.layers[1].blend = mode; // "paint" (α 0.25) over opaque "bg"
        let engine = load_fixture(&spec);
        let doc = engine.document().unwrap();
        let canvas = doc.geometry().canvas_rect();
        let comp = composite_document(doc, canvas).unwrap();
        assert_equals_naive(doc, canvas, &comp);
    }
}

/// Dissolve through the document walk is deterministic (same document →
/// bit-identical composites), document-stable (two loads of the same
/// package agree — the seed reads persisted uuids, never process state),
/// and actually thresholds (a fractional-alpha layer yields a mix of
/// passed/failed pixels).
#[test]
fn document_dissolve_is_deterministic_and_seeded_from_uuids() {
    let mut spec = dirty_fixture(false);
    spec.layers[1].blend = "dissolve"; // fractional α_eff = 0.25 → mixed
    let engine_a = load_fixture(&spec);
    let engine_b = load_fixture(&spec);
    let (da, db) = (engine_a.document().unwrap(), engine_b.document().unwrap());
    let canvas = da.geometry().canvas_rect();
    let ca = composite_document(da, canvas).unwrap();
    let cb = composite_document(db, canvas).unwrap();
    assert_eq!(
        ca.region_rgba_f32(canvas),
        cb.region_rgba_f32(canvas),
        "same package bytes → same uuids → same dissolve pattern"
    );
    // The pattern is threshold-driven: both outcomes occur somewhere
    // (the opaque bg pins every alpha at 1, so the mix shows in color —
    // passing pixels take the paint's un-premultiplied red, failing
    // pixels keep the gray bg).
    let px = ca.region_rgba_f32(canvas);
    let uniq: std::collections::BTreeSet<[u32; 3]> = px
        .chunks_exact(4)
        .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
        .collect();
    assert!(
        uniq.len() >= 2,
        "expected mixed pass/fail coverage, got one uniform color"
    );
}

#[test]
fn recomposite_after_edit_equals_naive_of_new_state() {
    // Cached recomposite (working cache warm from the BEFORE composite,
    // in-place tile reuse) must equal the naive reference of the AFTER
    // document — cache/in-place/uniform paths never leak stale bytes.
    let before = load_fixture(&dirty_fixture(false));
    let after = load_fixture(&dirty_fixture(true));
    let (db, da) = (before.document().unwrap(), after.document().unwrap());
    let canvas = db.geometry().canvas_rect();
    let mut comp = composite_document(db, canvas).unwrap();
    let dirty = DirtyMap::from_records(&[DirtyRecord {
        layer: layer_by_name(da, "paint"),
        rect: TileCoord::new(0, 1, 0).pixel_rect(),
    }]);
    recomposite_dirty(&mut comp, da, &dirty).unwrap();
    assert_equals_naive(da, canvas, &comp);
}

#[test]
fn repeated_recomposite_is_byte_stable_under_cache() {
    // Recompositing the SAME state repeatedly (all conversions served
    // from the memo after the first pass) must reproduce the identical
    // bytes every time — memoization is invisible in output.
    let engine = load_fixture(&dirty_fixture(false));
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let baseline = composite_document(doc, canvas).unwrap();
    let mut comp = composite_document(doc, canvas).unwrap();
    let everything = DirtyMap::from_records(&[DirtyRecord {
        layer: layer_by_name(doc, "paint"),
        rect: canvas,
    }]);
    for round in 0..3 {
        let stats = recomposite_dirty(&mut comp, doc, &everything).unwrap();
        assert_eq!(
            stats.tiles_recomposited, 2,
            "round {round}: both tiles recomposited"
        );
        assert_eq!(
            comp.region_rgba_f32(canvas),
            baseline.region_rgba_f32(canvas),
            "round {round}: cache-served recomposite must be byte-stable"
        );
    }
}

// ---------------------------------------------------------------------
// Zero-alpha-uniform tripwire (O-INT ruling, perf-round review minor).
// ---------------------------------------------------------------------

/// Sweep every raster layer of `doc`: no persistent tile may be a
/// `Uniform` whose converted working alpha is 0 (the state whose
/// compositor semantics are the NORMATIVE skip, pinned in-crate by
/// `compositor::tests::equality_suite_zero_alpha_uniform_pins_skip_semantics`).
fn assert_no_persistent_zero_alpha_uniform(doc: &Document) {
    for id in doc.z_order_walk() {
        let Some(buf) = doc.layer(id).and_then(|n| n.pixels()) else {
            continue;
        };
        for (c, tile) in buf.tiles().iter() {
            if let Some(v) = tile.uniform_value() {
                assert!(
                    v[3] != 0.0,
                    "layer {id} tile ({},{}) is a persistent zero-alpha Uniform {v:?} — \
                     a public door now yields the state the normative skip governs; \
                     extend the document-level equality suites with reachable fixtures \
                     (see the tripwire test's CP-0.5 `fill` note)",
                    c.x,
                    c.y
                );
            }
        }
    }
}

/// TRIPWIRE (O-INT ruling item 3) — with a FINDING the tripwire itself
/// produced on first run, recorded here and in the lane log:
///
/// The review premise "no public door yields a persistent zero-alpha
/// `Uniform` today" is FALSE. The loader parses chunks dense-only, but
/// core's post-load maintenance (`recompute_bounds_pixel_exact` →
/// `prune_at` step 2) COLLAPSES any uniform-valued dense tile to
/// `TileStorage::Uniform` — so a zero-alpha, non-bitwise-zero chunk
/// (straight-f32 hidden RGB per §6.4, or u8 hidden color) loads into a
/// PERSISTENT zero-alpha Uniform through `load_package_bytes` TODAY.
/// The normative skip is therefore live now, not CP-0.5-latent, and the
/// document-level equality suite carries a reachable fixture pinned to
/// it (`zero_alpha_uniform_reachable_via_loader_pins_skip_semantics`).
///
/// What this test asserts is the exact DOOR INVENTORY:
/// * `doc.new` backgrounds: never (transparent stores no tiles at all;
///   white/black store alpha-1 uniforms);
/// * loader, bitwise-zero chunk: pruned to absent — never persists;
/// * loader, zero-alpha uniform-valued chunk: PERSISTS as a zero-alpha
///   Uniform (the reachable door — asserted present, so a future core
///   change to this behavior is flagged here first);
/// * core's `make_mut`: materializes Uniform before mutable access —
///   transient by construction, core-internal, not reachable from this
///   crate (lane-log note);
/// * **CP-0.5 `fill` flip point**: the `fill` kind is still typed
///   `error.command.not_implemented`. WHOEVER LANDS `fill` meets this
///   failing assertion first: fill adds a COMMAND door to the same
///   state (e.g. zero-alpha color fills) — extend the equality suites
///   with fill-produced fixtures against the same pinned skip semantics
///   (loader-door fixtures already exist as the template), then update
///   this probe.
#[test]
fn tripwire_zero_alpha_uniform_door_inventory() {
    // Door 1 — doc.new backgrounds: absence.
    for (depth, background) in [
        ("u8", "transparent"),
        ("f32", "transparent"),
        ("u8", "white"),
        ("f32", "black"),
    ] {
        let engine = engine_doc_new(300, 300, depth, background);
        let doc = engine.document().unwrap();
        assert_no_persistent_zero_alpha_uniform(doc);
        if background == "transparent" {
            let bg = doc
                .z_order_walk()
                .into_iter()
                .find_map(|id| doc.layer(id).and_then(|n| n.pixels()))
                .expect("background raster");
            assert!(
                bg.tiles().is_empty(),
                "transparent background stores no tiles at all"
            );
        }
    }

    // Door 2a — loader, bitwise-zero chunk: pruned to absent.
    let spec = PkgSpec {
        width: 300,
        height: 300,
        depth: "f32",
        premultiplied: true,
        layers: vec![LayerSpec::raster(
            "00000000-0000-4000-8000-0000000000a0",
            vec![
                TileSpec {
                    x: 0,
                    y: 0,
                    value: [0.0; 4],
                },
                TileSpec {
                    x: 1,
                    y: 0,
                    value: [0.25, 0.25, 0.25, 1.0],
                }, // control
            ],
        )],
        root_children: vec!["00000000-0000-4000-8000-0000000000a0"],
    };
    let engine = load_fixture(&spec);
    let doc = engine.document().unwrap();
    assert_no_persistent_zero_alpha_uniform(doc);
    let buf = doc
        .layer(layer_by_name(doc, "layer"))
        .and_then(|n| n.pixels())
        .unwrap();
    assert!(
        buf.get_tile(TileCoord::new(0, 0, 0)).is_none(),
        "bitwise-zero chunk must prune to absent on load"
    );
    assert!(
        buf.get_tile(TileCoord::new(0, 1, 0)).is_some(),
        "control tile survives"
    );

    // Door 2b — loader, zero-alpha uniform-valued chunks, updated to the
    // CP-0.5 §6.4 reader-sanitize truth [O-CORE probe update per this
    // test's own flip instructions — flagged for O-RENDER ratification]:
    // * STRAIGHT storage keeps hidden RGB (§6.4 rule 2) and still
    //   persists as a zero-alpha Uniform, warning-free (unchanged pin);
    // * PREMULTIPLIED hidden color under a = 0 is a violation the loader
    //   now SANITIZES (a=0 ⇒ rgb=0, one warning) — the tile becomes
    //   bitwise zero and prunes to ABSENT, closing that door.
    {
        let spec = PkgSpec {
            width: 300,
            height: 300,
            depth: "f32",
            premultiplied: false,
            layers: vec![LayerSpec::raster(
                "00000000-0000-4000-8000-0000000000a0",
                vec![TileSpec {
                    x: 0,
                    y: 0,
                    value: [0.5f32, 0.25, 0.125, 0.0],
                }],
            )],
            root_children: vec!["00000000-0000-4000-8000-0000000000a0"],
        };
        let engine = load_fixture(&spec); // warning-free by construction
        let doc = engine.document().unwrap();
        let buf = doc
            .layer(layer_by_name(doc, "layer"))
            .and_then(|n| n.pixels())
            .unwrap();
        let tile = buf
            .get_tile(TileCoord::new(0, 0, 0))
            .expect("straight zero-alpha hidden RGB persists through the loader");
        assert!(
            tile.is_uniform() && tile.uniform_value().map(|v| v[3]) == Some(0.0),
            "straight storage: expected the §2.4 collapse to a persistent zero-alpha Uniform"
        );
    }
    {
        let spec = PkgSpec {
            width: 300,
            height: 300,
            depth: "u8",
            premultiplied: true,
            layers: vec![LayerSpec::raster(
                "00000000-0000-4000-8000-0000000000a0",
                vec![TileSpec {
                    x: 0,
                    y: 0,
                    value: [128.0 / 255.0, 0.0, 0.0, 0.0],
                }],
            )],
            root_children: vec!["00000000-0000-4000-8000-0000000000a0"],
        };
        let bytes = crate::support::build_package(&spec);
        let loaded = appname_core::load_package_bytes(&bytes).expect("package loads");
        assert_eq!(
            loaded.warnings.len(),
            1,
            "§6.4 sanitize records one warning: {:?}",
            loaded.warnings
        );
        assert!(loaded.warnings[0].detail.contains("sanitized"));
        let doc = loaded.engine.document().unwrap();
        let buf = doc
            .layer(layer_by_name(doc, "layer"))
            .and_then(|n| n.pixels())
            .unwrap();
        assert!(
            buf.get_tile(TileCoord::new(0, 0, 0)).is_none(),
            "premult hidden color sanitizes to bitwise zero and prunes to absent"
        );
    }

    // Door 3 — the CP-0.5 `fill` flip point (command route).
    let mut engine = engine_doc_new(32, 32, "f32", "white");
    let target = engine.document().unwrap().z_order_walk()[0];
    let env_id = appname_core::DocumentId::new().as_uuid();
    let env = format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"{env_id}\",\"kind\":\"fill\",\
         \"target\":{{\"layerId\":\"{}\"}},\"params\":{{\"content\":\"foreground\"}}}}",
        target.as_uuid()
    );
    let err = engine.apply_json(&env).expect_err("fill is a CP-0.5 kind");
    assert_eq!(
        err.code,
        appname_core::ErrorCode::NotImplemented,
        "fill has been implemented — CP-0.5 flip point reached: extend the equality \
         suites with fill-produced zero-alpha fixtures pinned to the normative skip \
         semantics, then update this tripwire (see doc comment)"
    );
}

/// Item-4 pin, DOCUMENT level, on the reachable state the tripwire
/// found: a zero-alpha Uniform layer (loaded through the public loader)
/// over a real backdrop composites to EXACTLY the backdrop — the
/// normative skip semantics as expected bytes. Asserted two ways:
/// with-vs-without the zero-alpha layer (bitwise composite equality),
/// and against the naive reference (which implements the ruling's skip).
#[test]
fn zero_alpha_uniform_reachable_via_loader_pins_skip_semantics() {
    let backdrop_layer = || LayerSpec {
        name: "backdrop",
        ..LayerSpec::raster(
            "00000000-0000-4000-8000-0000000000b0",
            vec![
                TileSpec {
                    x: 0,
                    y: 0,
                    value: [0.1, 0.3, 0.7, 0.3],
                },
                TileSpec {
                    x: 1,
                    y: 0,
                    value: [0.2, 0.9, 0.05, 0.95],
                },
            ],
        )
    };
    let with = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: false, // straight storage: hidden RGB under a = 0 is §6.4-legit
        layers: vec![
            backdrop_layer(),
            LayerSpec {
                name: "zero-alpha",
                ..LayerSpec::raster(
                    "00000000-0000-4000-8000-0000000000b1",
                    vec![
                        TileSpec {
                            x: 0,
                            y: 0,
                            value: [0.5, 0.25, 0.125, 0.0],
                        },
                        TileSpec {
                            x: 1,
                            y: 0,
                            value: [1.0, 1.0, 1.0, 0.0],
                        },
                    ],
                )
            },
        ],
        root_children: vec![
            "00000000-0000-4000-8000-0000000000b0",
            "00000000-0000-4000-8000-0000000000b1",
        ],
    };
    let without = PkgSpec {
        width: 512,
        height: 256,
        depth: "f32",
        premultiplied: false,
        layers: vec![backdrop_layer()],
        root_children: vec!["00000000-0000-4000-8000-0000000000b0"],
    };

    let (ew, ewo) = (load_fixture(&with), load_fixture(&without));
    let (dw, dwo) = (ew.document().unwrap(), ewo.document().unwrap());
    let canvas = dw.geometry().canvas_rect();

    // Precondition: the zero-alpha layer really is persistent Uniform
    // (the reachable state under the ruling — if core stops collapsing,
    // the tripwire inventory fails first and this test follows).
    let zbuf = dw
        .layer(layer_by_name(dw, "zero-alpha"))
        .and_then(|n| n.pixels())
        .unwrap();
    for (_, t) in zbuf.tiles().iter() {
        assert!(t.is_uniform() && t.uniform_value().map(|v| v[3]) == Some(0.0));
    }

    // Pinned expectation: bitwise identity with the backdrop-only doc.
    let cw = composite_document(dw, canvas).unwrap();
    let cwo = composite_document(dwo, canvas).unwrap();
    assert_eq!(
        cw.region_rgba_f32(canvas),
        cwo.region_rgba_f32(canvas),
        "zero-alpha uniform layer must be the exact §4.4 identity (normative skip)"
    );

    // And the naive reference (with the ruling's skip) agrees.
    assert_equals_naive(dw, canvas, &cw);
}

// ---------------------------------------------------------------------
// Readback surface.
// ---------------------------------------------------------------------

#[test]
fn region_readback_matches_pixel_reads() {
    let engine = engine_doc_new(70, 40, "u8", "white");
    let doc = engine.document().unwrap();
    let canvas = doc.geometry().canvas_rect();
    let comp = composite_document(doc, canvas).unwrap();
    let flat = comp.region_rgba_f32(canvas);
    assert_eq!(flat.len(), 70 * 40 * 4);
    for &(x, y) in &[(0, 0), (69, 0), (0, 39), (69, 39), (35, 20)] {
        let i = ((y * 70 + x) * 4) as usize;
        assert_eq!(
            &flat[i..i + 4],
            &comp.pixel(IVec2::new(x, y)),
            "readback vs pixel at ({x},{y})"
        );
    }
}
