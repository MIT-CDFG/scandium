// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Doors 6+10 service battery (`crate::door::TextLayoutService` against
//! the procedural spike face — exact oracles at em 100 over UPM 1000,
//! scale 0.1: advances/rects are exact decimal fractions).
//!
//! Covers: schema/structure gates, the add-params build, THE FIXPOINT
//! (emission == the validator's byte-exact re-emission — the same
//! invariant class DEFECT-1 pinned on the vector side), §7.7 unknown-
//! field preservation, §23.1 merge/refuse, §10.3 edit inheritance,
//! set_style split/re-merge, paragraph patch, scalar counting, compose
//! mass + fill arms, Door 9 outline round-trip through the strict
//! `appname.vector.v1` writer, caret/hit round trips + the bidi split,
//! and the Door 10 font bridge (idempotence, stamp, fsType verdict).

mod common;

use serde_json::{json, Map, Value};
use uuid::Uuid;

use appname_core::engine::error::ErrorCode;
use appname_core::engine::text_ops::{TextLayout, TEXT_SIDECAR_SCHEMA};
use appname_core::geom::IRect;
use appname_text::door::TextLayoutService;

const EM: f64 = 100.0; // pt == px at the PPI-72 pin; UPM 1000 → scale 0.1

fn svc() -> TextLayoutService {
    let s = TextLayoutService::default();
    s.register_embedded_face(Uuid::from_u128(1), &common::build())
        .expect("spike face registers");
    s
}

fn params(v: Value) -> Map<String, Value> {
    v.as_object().expect("params object").clone()
}

/// A point-layout sidecar for `text` at size `EM`, origin (0, 100).
fn add(s: &TextLayoutService, text: &str) -> Vec<u8> {
    s.sidecar_from_add_params(&params(json!({
        "layout": "point",
        "origin": [0, 100],
        "text": text,
        "style": {"size": EM},
    })))
    .expect("add builds")
    .sidecar
}

fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("sidecar is JSON")
}

// ---------------------------------------------------------------------

#[test]
fn schema_and_structure_gates() {
    let s = svc();
    // Wrong stamp → wrong_schema.
    let e = s
        .validate_sidecar(br#"{"schema": "appname.text.v2"}"#)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::WrongSchema);
    // Not JSON → out_of_range.
    let e = s.validate_sidecar(b"not json").unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
    // Zero-length run → out_of_range (§23.1).
    let bad = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 0]},
        "runs": [{"text": ""}],
    });
    let e = s.validate_sidecar(bad.to_string().as_bytes()).unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
    // Unknown layout kind / missing payload → out_of_range with pointers.
    for layout in [
        json!({"kind": "spiral"}),
        json!({"kind": "point"}),                  // no origin
        json!({"kind": "paragraph"}),              // no rect
        json!({"kind": "on_path", "pathId": "x"}), // not a UUID
    ] {
        let v = json!({"schema": TEXT_SIDECAR_SCHEMA, "layout": layout, "runs": []});
        let e = s.validate_sidecar(v.to_string().as_bytes()).unwrap_err();
        assert_eq!(e.code, ErrorCode::OutOfRange, "layout {layout}");
    }
}

#[test]
fn add_params_build_the_canonical_sidecar_with_exact_bounds() {
    let s = svc();
    let r = s
        .sidecar_from_add_params(&params(json!({
            "layout": "point",
            "origin": [0, 100],
            "text": "A",
            "style": {"size": EM},
        })))
        .expect("add builds");
    let v = parse(&r.sidecar);
    assert_eq!(v["layout"]["kind"], "point");
    assert_eq!(v["layout"]["origin"], json!([0, 100]));
    assert_eq!(v["runs"][0]["text"], "A", "flat styledRun");
    assert_eq!(v["runs"][0]["size"], json!(100.0));
    // Exact oracle: 'A' rect (100..500, 0..700) at scale 0.1, baseline
    // y=100 → ink hull x 10..50, y 30..100.
    assert_eq!(r.bounds, IRect::new(10, 30, 50, 100));
}

#[test]
fn emission_is_the_validators_byte_exact_fixpoint() {
    let s = svc();
    // Style-bearing, float-bearing, unknown-field-bearing sidecar.
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0.1, 100.75]},
        "runs": [
            {"text": "AB", "size": 100.0, "tracking": 12.5,
             "xFutureField": {"nested": [1.25e-3, "keep"]}},
            {"text": "C", "size": 50.25},
        ],
        "paragraph": {"alignment": "center"},
        "xReserved": "preserved",
    });
    let first = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    let second = s.validate_sidecar(&first.sidecar).unwrap();
    assert_eq!(
        first.sidecar, second.sidecar,
        "emission must be the validator's fixpoint (D-48/DD ㉒)"
    );
    assert_eq!(first.bounds, second.bounds);
    // §7.7: unknown fields survived, top level and per run.
    let v = parse(&first.sidecar);
    assert_eq!(v["xReserved"], "preserved");
    assert_eq!(v["runs"][0]["xFutureField"]["nested"][1], "keep");
}

#[test]
fn adjacent_equal_runs_merge_to_minimal_form() {
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [
            {"text": "A", "size": EM},
            {"text": "B", "size": EM},          // identical identity → merges
            {"text": "C", "size": EM / 2.0},    // differs → stays
        ],
    });
    let r = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    let v = parse(&r.sidecar);
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "minimal runs (§23.1)");
    assert_eq!(runs[0]["text"], "AB");
    assert_eq!(runs[1]["text"], "C");
}

#[test]
fn empty_layers_are_legal_and_inert() {
    let s = svc();
    let r = s
        .sidecar_from_add_params(&params(json!({
            "layout": "point", "origin": [5, 50],
        })))
        .expect("create-then-type: empty add is legal");
    assert_eq!(r.bounds, IRect::EMPTY);
    assert_eq!(s.content_len(&r.sidecar).unwrap(), 0);
    let (page, metrics) = s.layout_and_compose(&r.sidecar).unwrap();
    assert_eq!(metrics.line_count, 0);
    assert!(!metrics.overflowed);
    assert!(page.pixels.is_empty());
    // The empty-layer caret sits at the anchor.
    let carets = s.caret_geometry(&r.sidecar, 0).unwrap();
    assert_eq!(carets.len(), 1);
    assert_eq!(carets[0].x, 5.0);
    assert!(carets[0].primary);
    assert_eq!(s.hit_test(&r.sidecar, [123.0, 456.0]).unwrap(), 0);
}

#[test]
fn edit_is_full_state_and_inherits_the_first_run_style() {
    let s = svc();
    let cur = add(&s, "A");
    // New runs omit `size` → inherited from the current first run.
    let r = s
        .apply_edit(&cur, &params(json!({"runs": [{"text": "ABAB"}]})))
        .unwrap();
    let v = parse(&r.sidecar);
    assert_eq!(v["runs"][0]["text"], "ABAB");
    assert_eq!(v["runs"][0]["size"], json!(100.0), "inherited (§10.3 note)");
    // Full-state layout replacement rides the same call.
    let r = s
        .apply_edit(
            &r.sidecar,
            &params(json!({
                "runs": [{"text": "AB"}],
                "layout": {"kind": "paragraph", "rect": {"x": 0, "y": 0, "w": 400, "h": 200}},
            })),
        )
        .unwrap();
    let v = parse(&r.sidecar);
    assert_eq!(v["layout"]["kind"], "paragraph");
    // Stage-6 live check: zero-length run refuses.
    let e = s
        .apply_edit(&r.sidecar, &params(json!({"runs": [{"text": ""}]})))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
    // Missing runs → missing_field (full-state payload).
    let e = s.apply_edit(&r.sidecar, &params(json!({}))).unwrap_err();
    assert_eq!(e.code, ErrorCode::MissingField);
}

#[test]
fn style_range_splits_patches_and_remerges() {
    let s = svc();
    let cur = add(&s, "AB");
    // Patch [0,1): the run splits.
    let r = s
        .apply_style_range(&cur, (0, 1), &params(json!({"size": 50.0})))
        .unwrap();
    let v = parse(&r.sidecar);
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "split at the patch boundary");
    assert_eq!(
        (&runs[0]["text"], &runs[0]["size"]),
        (&json!("A"), &json!(50.0))
    );
    assert_eq!(
        (&runs[1]["text"], &runs[1]["size"]),
        (&json!("B"), &json!(100.0))
    );
    // Patch [1,2) to the same value: §23.1 minimality re-merges.
    let r = s
        .apply_style_range(&r.sidecar, (1, 2), &params(json!({"size": 50.0})))
        .unwrap();
    let v = parse(&r.sidecar);
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "adjacent equal styles re-merge");
    assert_eq!(runs[0]["text"], "AB");
    // Range gates (family rule, Unicode scalars).
    for bad in [(2, 1), (0, 3)] {
        let e = s
            .apply_style_range(&r.sidecar, bad, &params(json!({"size": 12.0})))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::OutOfRange, "range {bad:?}");
    }
    // A live §23.4 range violation is typed, not a panic.
    let e = s
        .apply_style_range(&r.sidecar, (0, 1), &params(json!({"size": 9999.0})))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

#[test]
fn paragraph_patch_applies_to_the_one_layer_style() {
    let s = svc();
    let cur = add(&s, "AB");
    let r = s
        .apply_paragraph_range(&cur, (0, 1), &params(json!({"alignment": "center"})))
        .unwrap();
    let v = parse(&r.sidecar);
    assert_eq!(v["paragraph"]["alignment"], "center");
    // Second patch merges fields (each present field carries its value).
    let r = s
        .apply_paragraph_range(&r.sidecar, (0, 0), &params(json!({"direction": "rtl"})))
        .unwrap();
    let v = parse(&r.sidecar);
    assert_eq!(v["paragraph"]["alignment"], "center", "prior field kept");
    assert_eq!(v["paragraph"]["direction"], "rtl");
    let e = s
        .apply_paragraph_range(&r.sidecar, (0, 99), &params(json!({})))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

#[test]
fn content_len_counts_unicode_scalars() {
    let s = svc();
    // 永 (in-face) + é (tofu) + A: 3 scalars, 7 bytes.
    let cur = add(&s, "永éA");
    assert_eq!(s.content_len(&cur).unwrap(), 3);
}

#[test]
fn compose_carries_the_exact_raster_mass_through_the_fill_arms() {
    let s = svc();
    // Default fill = opaque black: Σalpha == the coverage mass (the
    // compose_spike oracle: 'A' at em 100 = 40×70 full coverage).
    let cur = add(&s, "A");
    let (page, metrics) = s.layout_and_compose(&cur).unwrap();
    let mass: f64 = page.pixels.iter().map(|p| f64::from(p[3])).sum();
    assert_eq!(mass, 2800.0);
    assert_eq!(metrics.bounds, IRect::new(10, 30, 50, 100));
    assert_eq!(metrics.line_count, 1);
    // fill none → fully transparent page.
    let r = s
        .apply_style_range(&cur, (0, 1), &params(json!({"fill": {"type": "none"}})))
        .unwrap();
    let (page, _) = s.layout_and_compose(&r.sidecar).unwrap();
    assert!(page.pixels.iter().all(|p| *p == [0.0; 4]));
    // Solid mid-gray rgba: premultiplied linear channels == alpha·linear(0.5).
    let r = s
        .apply_style_range(
            &cur,
            (0, 1),
            &params(json!({"fill": {"type": "solid",
                "color": {"model": "rgba", "comps": [0.5, 0.5, 0.5, 0.5]}}})),
        )
        .unwrap();
    let (page, _) = s.layout_and_compose(&r.sidecar).unwrap();
    let inked = page.pixels.iter().find(|p| p[3] > 0.0).expect("ink exists");
    let lin = ((0.5f64 + 0.055) / 1.055).powf(2.4) as f32;
    assert_eq!(inked[3], 0.5);
    assert!((inked[0] - lin * 0.5).abs() < 1e-6, "premultiplied linear");
}

#[test]
fn outline_geometry_round_trips_through_the_strict_vector_wire() {
    let s = svc();
    let cur = add(&s, "AB");
    let bytes = s.outline_geometry(&cur).unwrap();
    // The strict Door-1 wire parses it back with no notes: the emission
    // already satisfies §20.1 writer invariants.
    let (path, notes) = appname_vector::path_from_sidecar_json(
        appname_core::PathId::from_uuid(Uuid::nil()),
        &bytes,
    )
    .expect("vector wire parses");
    assert!(notes.is_empty(), "no reader fixups needed: {notes:?}");
    // Two rectangle glyphs → two closed 4-segment contours.
    assert_eq!(path.subpaths.len(), 2);
    for sub in &path.subpaths {
        assert!(sub.closed);
        assert_eq!(sub.segments.len(), 4, "rect contour");
    }
    // 'A' contour spans x 10..50, y 30..100 (doc space, y-down).
    let b = appname_vector::control_bounds(&path).expect("nonempty path");
    assert_eq!((b.min.x, b.min.y), (10.0, 30.0));
    assert_eq!(b.max.y, 100.0);
    // Fully-tofu content has no outlines: typed refusal, not a panic.
    let tofu = add(&s, "é");
    let e = s.outline_geometry(&tofu).unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

#[test]
fn caret_and_hit_round_trip_on_exact_advances() {
    let s = svc();
    let cur = add(&s, "AB");
    // Advances at scale 0.1: A=60, B=62.
    let expect_x = [0.0, 60.0, 122.0];
    for (i, &x) in expect_x.iter().enumerate() {
        let carets = s.caret_geometry(&cur, i as u64).unwrap();
        assert_eq!(carets.len(), 1, "plain LTR: one caret at {i}");
        assert_eq!(carets[0].x, x, "caret {i}");
        assert!(carets[0].primary);
        // Full-height: face hhea 800/−200 at scale 0.1 around y=100.
        assert_eq!((carets[0].top, carets[0].bottom), (20.0, 120.0));
        assert_eq!(s.hit_test(&cur, [x, 100.0]).unwrap(), i as u64, "hit {i}");
    }
    // Out-of-range caret index refuses typed.
    let e = s.caret_geometry(&cur, 3).unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

#[test]
fn bidi_boundary_emits_the_secondary_caret() {
    let s = svc();
    // A + א (both real face rows): LTR item then RTL item.
    let cur = add(&s, "A\u{05D0}");
    let carets = s.caret_geometry(&cur, 1).unwrap();
    assert_eq!(carets.len(), 2, "level parity changes at the boundary");
    assert!(carets[0].primary);
    assert!(!carets[1].primary);
    assert!(
        carets[1].bottom - carets[1].top < carets[0].bottom - carets[0].top,
        "secondary is half-height"
    );
}

#[test]
fn on_path_answers_the_typed_ni() {
    let s = svc();
    let e = s
        .sidecar_from_add_params(&params(json!({
            "layout": "on_path",
            "pathId": Uuid::from_u128(7).to_string(),
            "text": "A",
        })))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotImplemented);
}

#[test]
fn font_bridge_registers_idempotently_and_stamps() {
    let s = TextLayoutService::default();
    let before = s.fonts_stamp();
    assert!(s.enumerate_faces().is_empty(), "no faces vendored (§3.10)");
    let asset = Uuid::from_u128(42);
    s.register_embedded_face(asset, &common::build()).unwrap();
    let stamped = s.fonts_stamp();
    assert_ne!(stamped, before, "face-set change bumps the stamp");
    // Idempotent per (asset, bytes): no error, no second bump.
    s.register_embedded_face(asset, &common::build()).unwrap();
    assert_eq!(s.fonts_stamp(), stamped);
    // Same asset, different bytes: typed refusal.
    let e = s
        .register_embedded_face(asset, &common::build_subset(&['A']))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
    // Unusable bytes: typed refusal.
    let e = s
        .register_embedded_face(Uuid::from_u128(43), b"not a font")
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
    // Enumeration: deterministic metadata + the fsType verdict (the
    // spike face carries no OS/2 → installable).
    let faces = s.enumerate_faces();
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].postscript_name, format!("embedded-{asset}"));
    assert!(faces[0].embeddable);
    // Layout is a pure function of (sidecar, fonts): the SAME sidecar
    // lays out differently once the face set changes. 'A' resolves to
    // the face → exact bounds; on a bare service it would be tofu.
    let cur = add(&s, "A");
    let bare = TextLayoutService::default();
    let with_face = s.validate_sidecar(&cur).unwrap();
    let tofu = bare.validate_sidecar(&cur).unwrap();
    assert_eq!(with_face.bounds, IRect::new(10, 30, 50, 100));
    assert_ne!(with_face.bounds, tofu.bounds, "fonts are layout INPUT");
}

#[test]
fn the_service_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TextLayoutService>();
}

// ---------------------------------------------------------------------
// Fix-round pins (§5 review MAJOR-1/2/3 + MINOR-4/5/6 probe cases)
// ---------------------------------------------------------------------

#[test]
fn newlines_render_nothing_anywhere() {
    // Review MAJOR-1: unmapped break chars must not become tofu boxes.
    let s = svc();
    let cur = add(&s, "A\nA");
    let (page, metrics) = s.layout_and_compose(&cur).unwrap();
    assert_eq!(metrics.line_count, 2);
    // No hex-box ink: both lines span exactly 'A''s ink columns 10..50.
    assert_eq!((metrics.bounds.min.x, metrics.bounds.max.x), (10, 50));
    // Exactly two 'A' glyphs' worth of coverage mass, nothing more.
    let mass: f64 = page.pixels.iter().map(|p| f64::from(p[3])).sum();
    assert_eq!(mass, 2.0 * 2800.0);
    // The caret after 'A' on line 1 sits at the advance WITHOUT any
    // newline-glyph width.
    let carets = s.caret_geometry(&cur, 1).unwrap();
    assert_eq!(carets[0].x, 60.0);
    // Door 9 agrees with compose: two outline contours, no third box.
    let bytes = s.outline_geometry(&cur).unwrap();
    let (path, _) = appname_vector::path_from_sidecar_json(
        appname_core::PathId::from_uuid(Uuid::nil()),
        &bytes,
    )
    .unwrap();
    assert_eq!(path.subpaths.len(), 2, "render == bake glyph census");
}

#[test]
fn trailing_newline_caret_starts_the_next_line() {
    // Review MINOR-4 (create-then-type): Enter at the end must yield a
    // caret on the not-yet-populated next line.
    let s = svc();
    let cur = add(&s, "A\n");
    let c1 = s.caret_geometry(&cur, 1).unwrap();
    assert_eq!(c1[0].x, 60.0, "line-1 end, no break-glyph advance");
    let c2 = s.caret_geometry(&cur, 2).unwrap();
    assert_eq!(c2.len(), 1);
    assert_eq!(c2[0].x, 0.0, "next line starts at the leading edge");
    // One §23.3 auto leading below baseline 100: max(1.2·100, asc+desc
    // +gap at em 100) = 120 → baseline 220, hhea 80/20 box.
    assert_eq!((c2[0].top, c2[0].bottom), (140.0, 240.0));
}

#[test]
fn per_run_fills_compose_independently() {
    // Review MAJOR-2 (probe F1): run 1 fill:none must not erase run 2.
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [
            {"text": "A", "size": EM, "fill": {"type": "none"}},
            {"text": "B", "size": EM, "fill": {"type": "solid",
                "color": {"model": "rgba", "comps": [1.0, 0.0, 0.0, 1.0]}}},
        ],
    });
    let r = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    let (page, _) = s.layout_and_compose(&r.sidecar).unwrap();
    let mass: f64 = page.pixels.iter().map(|p| f64::from(p[3])).sum();
    // Only 'B''s ink is visible: 42×70 full coverage.
    assert_eq!(mass, 2940.0, "the none-filled run hides ONLY itself");
    // Pixel probes: inside 'A''s ink (x 10..50) transparent; inside
    // 'B''s ink (x 70..112) opaque linear red (premultiplied).
    let px = |x: i32, y: i32| {
        let (w, l, t) = (page.rect.width(), page.rect.min.x, page.rect.min.y);
        page.pixels[((y - t) * w + (x - l)) as usize]
    };
    assert_eq!(px(30, 65), [0.0; 4], "run-1 ink is clear");
    assert_eq!(px(80, 65), [1.0, 0.0, 0.0, 1.0], "run-2 ink is red");
}

#[test]
fn bad_enum_and_type_values_refuse_typed() {
    // Review MAJOR-3: known fields with bad values refuse out_of_range.
    let s = svc();
    let base = |runs: Value, paragraph: Value| {
        let mut v = json!({
            "schema": TEXT_SIDECAR_SCHEMA,
            "layout": {"kind": "point", "origin": [0, 100]},
            "runs": runs,
        });
        if !paragraph.is_null() {
            v["paragraph"] = paragraph;
        }
        v.to_string()
    };
    let cases = [
        base(json!([{"text": "A", "size": "big"}]), Value::Null),
        base(json!([{"text": "A", "tracking": "loose"}]), Value::Null),
        base(
            json!([{"text": "A", "features": {"liga": "yes"}}]),
            Value::Null,
        ),
        base(
            json!([{"text": "A", "variations": {"wght": "heavy"}}]),
            Value::Null,
        ),
        base(json!([{"text": "A", "language": 7}]), Value::Null),
        base(json!([{"text": "A", "kerning": "auto"}]), Value::Null),
        base(
            json!([{"text": "A", "fill": {"type": "plaid"}}]),
            Value::Null,
        ),
        base(
            json!([{"text": "A", "fill": {"type": "solid",
                "color": {"model": "hsv", "comps": [0, 0, 0, 0]}}}]),
            Value::Null,
        ),
        base(json!([{"text": "A"}]), json!({"alignment": "diagonal"})),
        base(json!([{"text": "A"}]), json!({"direction": "up"})),
        base(json!([{"text": "A"}]), json!({"justifyLastLine": "middle"})),
        base(json!([{"text": "A"}]), json!({"autoLeadingPct": 700})),
        base(json!([{"text": "A"}]), json!({"hyphenation": "yes"})),
    ];
    for (i, c) in cases.iter().enumerate() {
        let e = s.validate_sidecar(c.as_bytes()).unwrap_err();
        assert_eq!(e.code, ErrorCode::OutOfRange, "case {i}: {c}");
    }
    // Unknown FIELDS (not values) still ride §7.7 untouched.
    let ok = base(
        json!([{"text": "A", "xVendor": {"anything": [1, "e"]}}]),
        json!({"xFuture": true}),
    );
    let r = s.validate_sidecar(ok.as_bytes()).unwrap();
    let v = parse(&r.sidecar);
    assert_eq!(v["runs"][0]["xVendor"]["anything"][1], "e");
    assert_eq!(v["paragraph"]["xFuture"], true);
}

#[test]
fn numeric_spelling_does_not_block_the_merge() {
    // Review MINOR-5: 100 and 100.0 are the same size (§23.1
    // minimality is not JSON-spelling-sensitive).
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [
            {"text": "A", "size": 100},
            {"text": "B", "size": 100.0},
        ],
    });
    let first = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    let v = parse(&first.sidecar);
    assert_eq!(v["runs"].as_array().unwrap().len(), 1);
    assert_eq!(v["runs"][0]["text"], "AB");
    // The merged form stays the validator's byte-exact fixpoint.
    let second = s.validate_sidecar(&first.sidecar).unwrap();
    assert_eq!(first.sidecar, second.sidecar);
}

#[test]
fn hit_test_refuses_non_finite_points() {
    // Review MINOR-6: the multi-line comparator path must not panic.
    let s = svc();
    let cur = add(&s, "A\nA");
    for bad in [[f64::NAN, 50.0], [0.0, f64::NAN], [f64::INFINITY, 0.0]] {
        let e = s.hit_test(&cur, bad).unwrap_err();
        assert_eq!(e.code, ErrorCode::OutOfRange);
    }
}

#[test]
fn paragraph_rect_layout_breaks_and_reports_overflow() {
    // The §23.3 paragraph arm through the service (review NOTE-13).
    let s = svc();
    let sidecar = |h: i64| {
        json!({
            "schema": TEXT_SIDECAR_SCHEMA,
            "layout": {"kind": "paragraph", "rect": {"x": 0, "y": 0, "w": 130, "h": h}},
            "runs": [{"text": "AA AA AA", "size": EM}],
        })
        .to_string()
    };
    // 'A' = 60 px, space = 50 px: "AA" (120) fits the 130 measure,
    // "AA A" does not → three lines at auto leading 120.
    let (_, m) = s.layout_and_compose(sidecar(500).as_bytes()).unwrap();
    assert_eq!(m.line_count, 3);
    assert!(!m.overflowed);
    // h = 100: line 1 (baseline 80, descent 20) exactly fits; line 2
    // (baseline 200) clips → §23.3 "after the last fully fitting line".
    let (_, m) = s.layout_and_compose(sidecar(100).as_bytes()).unwrap();
    assert_eq!(m.line_count, 1);
    assert!(m.overflowed);
}

#[test]
fn unknown_run_fields_survive_style_range_splits() {
    // §7.7 through the split machinery (review NOTE-13).
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [{"text": "AB", "size": EM, "xMeta": {"k": 1}}],
    });
    let cur = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    let r = s
        .apply_style_range(&cur.sidecar, (0, 1), &params(json!({"size": 50.0})))
        .unwrap();
    let v = parse(&r.sidecar);
    let runs = v["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    for run in runs {
        assert_eq!(run["xMeta"]["k"], 1, "unknown field survives the split");
    }
    let again = s.validate_sidecar(&r.sidecar).unwrap();
    assert_eq!(r.sidecar, again.sidecar, "fixpoint after split");
}

// ---------------------------------------------------------------------
// Re-review carry-forward pins (NEW-1/2/3/5)
// ---------------------------------------------------------------------

#[test]
fn huge_coordinates_refuse_typed_instead_of_panicking() {
    // Re-review NEW-1: finite-but-huge anchors overflowed compose's
    // i32 page arithmetic (debug panic, release wrap).
    let s = svc();
    let cases = [
        json!({"kind": "point", "origin": [3.0e9, 100]}),
        json!({"kind": "point", "origin": [0, -3.0e9]}),
        json!({"kind": "paragraph", "rect": {"x": 1.0e9, "y": 0, "w": 10, "h": 10}}),
        json!({"kind": "paragraph", "rect": {"x": 0, "y": 0, "w": -5, "h": 10}}),
    ];
    for layout in cases {
        let v = json!({
            "schema": TEXT_SIDECAR_SCHEMA,
            "layout": layout,
            "runs": [{"text": "A", "size": EM}],
        });
        let e = s.validate_sidecar(v.to_string().as_bytes()).unwrap_err();
        assert_eq!(e.code, ErrorCode::OutOfRange, "layout {}", v["layout"]);
    }
}

#[test]
fn rtl_fresh_line_carets_use_the_visual_right_edge() {
    // Re-review NEW-2: the synthesized and glyphless caret paths must
    // follow the base direction. אב at em 100: א=55, ב=56 → the line's
    // RTL leading (visual right, advance) edge is 111.
    let s = svc();
    let cur = add(&s, "\u{05D0}\u{05D1}\n");
    let c = s.caret_geometry(&cur, 3).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].x, 111.0, "fresh RTL line starts at the visual right");
    assert!(c[0].top > 100.0, "one leading below the last baseline");
}

#[test]
fn glyphless_line_caret_honors_the_reference_indent() {
    // Re-review NEW-3: the between-breaks caret follows the nearest
    // glyph-bearing line's leading edge — indent included.
    let s = svc();
    let v = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "paragraph", "rect": {"x": 0, "y": 0, "w": 400, "h": 500}},
        "runs": [{"text": "A\n\nA", "size": EM}],
        "paragraph": {"indentLeft": 30},
    });
    let r = s.validate_sidecar(v.to_string().as_bytes()).unwrap();
    let c = s.caret_geometry(&r.sidecar, 2).unwrap();
    assert_eq!(c[0].x, 30.0, "empty line carets at the indented edge");
}

#[test]
fn variations_beyond_f32_refuse_typed() {
    // Re-review NEW-5: no silent ∞ axis coordinates.
    let s = svc();
    let v = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [{"text": "A", "variations": {"wght": 1.0e300}}],
    });
    let e = s.validate_sidecar(v.to_string().as_bytes()).unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

// ---------------------------------------------------------------------
// C-6 slice 1: Manual kerning consumption
// ---------------------------------------------------------------------

#[test]
fn manual_kerning_tightens_the_pair_gap_exactly() {
    // Kerning::Manual(k) = k ‰-em on the LEFT glyph's advance at each
    // intra-item pair boundary (module-header pin; the MAJOR-1 shape —
    // never the gap machinery). −100 ‰ at em 100 = −10 px
    // at the A|B boundary: line width 60 + 62 − 10 = 112.
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [{"text": "AB", "size": EM, "kerning": -100}],
    });
    let r = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    let carets = s.caret_geometry(&r.sidecar, 2).unwrap();
    assert_eq!(carets[0].x, 112.0, "line end reflects the manual kern");
    // 'B''s ink shifts left by exactly 10 px: hull 10..(112−10).
    assert_eq!(
        (r.bounds.min.x, r.bounds.max.x),
        (10, 102),
        "ink hull tightened by the pair kern"
    );
    // "metric" (and default) keep the plain advances.
    let plain = add(&s, "AB");
    let carets = s.caret_geometry(&plain, 2).unwrap();
    assert_eq!(carets[0].x, 122.0);
    // The kern value re-emits verbatim (fixpoint holds with the field).
    let again = s.validate_sidecar(&r.sidecar).unwrap();
    assert_eq!(r.sidecar, again.sidecar);
}

// ---------------------------------------------------------------------
// Request-10 engine half: host-provisioned faces (the §44.1 reading)
// ---------------------------------------------------------------------

#[test]
fn provided_faces_resolve_through_the_system_arm() {
    use appname_text::fontdb::FaceSource;
    // A bare service + ONE host-provisioned System face: text resolves
    // through the §44.2 step-5 arm with exact bounds — no tofu, no
    // engine-side discovery, layout a pure function of provided fonts.
    let s = TextLayoutService::default();
    let before = s.fonts_stamp();
    s.register_provided_face(
        Uuid::from_u128(9),
        &common::build(),
        FaceSource::System,
        "Spike Sans",
        "Regular",
        "SpikeSans-Regular",
    )
    .unwrap();
    assert_ne!(s.fonts_stamp(), before, "provision bumps the stamp");
    let cur = add(&s, "A");
    let r = s.validate_sidecar(&cur).unwrap();
    assert_eq!(r.bounds, IRect::new(10, 30, 50, 100), "resolved, not tofu");
    // Enumeration carries the host-derived metadata verbatim.
    let faces = s.enumerate_faces();
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].family, "Spike Sans");
    assert_eq!(faces[0].postscript_name, "SpikeSans-Regular");
    assert!(faces[0].embeddable);
    // Idempotent per (asset, bytes); DocEmbedded refused on this door.
    s.register_provided_face(
        Uuid::from_u128(9),
        &common::build(),
        FaceSource::System,
        "Spike Sans",
        "Regular",
        "SpikeSans-Regular",
    )
    .unwrap();
    assert_eq!(s.enumerate_faces().len(), 1);
    let e = s
        .register_provided_face(
            Uuid::from_u128(10),
            &common::build_subset(&['A']),
            FaceSource::DocEmbedded,
            "X",
            "X",
            "X",
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange, "DocEmbedded is the loader's");
}

#[test]
fn embedded_faces_keep_step_2_priority_over_provided() {
    use appname_text::fontdb::FaceSource;
    // §44.2 order: the document's own face (step 2) resolves before a
    // provided System face (step 5). The embedded SUBSET covers only
    // 'A'; the provided FULL face picks up 'B' — both render real ink.
    let s = TextLayoutService::default();
    s.register_embedded_face(Uuid::from_u128(1), &common::build_subset(&['A']))
        .unwrap();
    s.register_provided_face(
        Uuid::from_u128(2),
        &common::build(),
        FaceSource::System,
        "Spike Sans",
        "Regular",
        "SpikeSans-Regular",
    )
    .unwrap();
    let cur = add(&s, "AB");
    let r = s.validate_sidecar(&cur).unwrap();
    // Full real-ink hull: A 10..50, B at pen 60 → 70..112, both 0..700.
    assert_eq!(r.bounds, IRect::new(10, 30, 112, 100), "no tofu anywhere");
}

// ---------------------------------------------------------------------
// The exported fill surface (the composite-leg one-math-library ruling)
// ---------------------------------------------------------------------

#[test]
fn exported_fill_conversion_pins_the_shared_constants() {
    use appname_text::{fill_to_linear_rgba, premultiply_coverage};
    // Absent = opaque black; none = clear.
    assert_eq!(fill_to_linear_rgba(None), [0.0, 0.0, 0.0, 1.0]);
    let none = json!({"type": "none"});
    assert_eq!(fill_to_linear_rgba(Some(&none)), [0.0, 0.0, 0.0, 0.0]);
    // Solid mid-gray rgba: the exact sRGB EOTF value both legs share.
    let f = json!({"type": "solid",
        "color": {"model": "rgba", "comps": [0.5, 0.5, 0.5, 0.5]}});
    let lin = (((0.5f64 + 0.055) / 1.055).powf(2.4)) as f32;
    let rgba = fill_to_linear_rgba(Some(&f));
    assert_eq!(rgba, [lin, lin, lin, 0.5]);
    // gray replicates; cmyk flags to opaque black (the pinned interim).
    let g = json!({"type": "solid", "color": {"model": "gray", "comps": [1, 0, 0, 0]}});
    assert_eq!(fill_to_linear_rgba(Some(&g)), [1.0, 1.0, 1.0, 1.0]);
    let c = json!({"type": "solid", "color": {"model": "cmyk", "comps": [0, 0, 0, 1]}});
    assert_eq!(fill_to_linear_rgba(Some(&c)), [0.0, 0.0, 0.0, 1.0]);
    // Premultiply: straight × coverage, alpha folds in.
    assert_eq!(
        premultiply_coverage([1.0, 0.5, 0.0, 0.5], 0.5),
        [0.25, 0.125, 0.0, 0.25]
    );
    assert_eq!(premultiply_coverage([1.0, 1.0, 1.0, 1.0], 0.0), [0.0; 4]);
}

// ---------------------------------------------------------------------
// C-6 review fix-round pins (MAJOR-1/2, MINOR-3/4, NOTE-5) + Optical
// ---------------------------------------------------------------------

#[test]
fn manual_kern_pair_range_kerns_exactly_one_gap() {
    // Review MAJOR-1 (the reviewer's probe): "ABC" with the pair range
    // [0,2) kerned −100 ‰ must kern ONLY the A|B boundary — B|C keeps
    // its plain gap. Advances 60/62/64 at em 100, kern −10 on A.
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [
            {"text": "AB", "size": EM, "kerning": -100},
            {"text": "C", "size": EM},
        ],
    });
    let r = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    assert_eq!(
        s.caret_geometry(&r.sidecar, 2).unwrap()[0].x,
        112.0,
        "B's trailing edge: 50 + 62 — the run edge is NOT kerned"
    );
    assert_eq!(
        s.caret_geometry(&r.sidecar, 3).unwrap()[0].x,
        176.0,
        "line end: 50 + 62 + 64"
    );
    // C's ink ends at pen 112 + x1 54 = 166.
    assert_eq!(r.bounds.max.x, 166, "hull reflects exactly one kern");
}

#[test]
fn cross_door_same_bytes_membership_is_order_independent() {
    // Review MAJOR-2 (the reviewer's probe class): the same
    // (asset, bytes) arriving through BOTH doors must resolve and
    // enumerate identically in either arrival order.
    use appname_text::fontdb::FaceSource;
    let asset = Uuid::from_u128(77);
    let build = || common::build();
    let doc_first = TextLayoutService::default();
    doc_first.register_embedded_face(asset, &build()).unwrap();
    doc_first
        .register_provided_face(
            asset,
            &build(),
            FaceSource::System,
            "Spike Sans",
            "Regular",
            "SpikeSans-Regular",
        )
        .unwrap();
    let host_first = TextLayoutService::default();
    host_first
        .register_provided_face(
            asset,
            &build(),
            FaceSource::System,
            "Spike Sans",
            "Regular",
            "SpikeSans-Regular",
        )
        .unwrap();
    host_first.register_embedded_face(asset, &build()).unwrap();

    let cur = add(&doc_first, "A");
    let a = doc_first.validate_sidecar(&cur).unwrap();
    let b = host_first.validate_sidecar(&cur).unwrap();
    assert_eq!(a.bounds, b.bounds, "resolution ignores arrival order");
    assert_eq!(a.sidecar, b.sidecar);
    let (ea, eb) = (doc_first.enumerate_faces(), host_first.enumerate_faces());
    assert_eq!(ea.len(), 1);
    assert_eq!(ea[0].postscript_name, eb[0].postscript_name);
    assert_eq!(ea[0].family, "Spike Sans", "host metadata wins enumeration");
    // Membership growth bumped the stamp in both orders; identical
    // re-registration bumps NOTHING (reviewer: untested before).
    let stamp = doc_first.fonts_stamp();
    doc_first.register_embedded_face(asset, &build()).unwrap();
    doc_first
        .register_provided_face(
            asset,
            &build(),
            FaceSource::System,
            "Spike Sans",
            "Regular",
            "SpikeSans-Regular",
        )
        .unwrap();
    assert_eq!(doc_first.fonts_stamp(), stamp, "idempotent = no bump");
}

#[test]
fn provided_psname_collision_refuses_typed() {
    // Review MINOR-3: silent §23.5 shadowing could hand tofu to text
    // while enumeration lists the covering face — surface the host bug.
    use appname_text::fontdb::FaceSource;
    let s = TextLayoutService::default();
    s.register_provided_face(
        Uuid::from_u128(1),
        &common::build(),
        FaceSource::System,
        "Spike Sans",
        "Regular",
        "Dup-Regular",
    )
    .unwrap();
    let e = s
        .register_provided_face(
            Uuid::from_u128(2),
            &common::build_subset(&['A']),
            FaceSource::System,
            "Other Sans",
            "Regular",
            "Dup-Regular",
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

#[test]
fn fill_export_is_a_documented_total_function() {
    // Review MINOR-4 + NOTE-5: garbage falls to the opaque-black arm
    // (never a refusal — validation is the sidecar gate's job), and
    // premultiply clamps coverage to the plane domain.
    use appname_text::{fill_to_linear_rgba, premultiply_coverage};
    for garbage in [
        json!({"type": "soild"}),
        json!("just a string"),
        json!({"type": "solid", "color": {"model": "rgb", "comps": ["x", "y", "z", 1]}}),
        json!(42),
    ] {
        assert_eq!(
            fill_to_linear_rgba(Some(&garbage)),
            [0.0, 0.0, 0.0, 1.0],
            "total function: {garbage}"
        );
    }
    assert_eq!(premultiply_coverage([1.0, 1.0, 1.0, 1.0], 2.0), [1.0; 4]);
    assert_eq!(premultiply_coverage([1.0, 1.0, 1.0, 1.0], -1.0), [0.0; 4]);
}

#[test]
fn optical_kerning_normalizes_the_pair_gap_exactly() {
    // C-6 slice 2 (the pinned algorithm): 'A' rsb 10 + 'B' lsb 10 =
    // gap 20 px at em 100 (0.2 em); target 0.16 em → kern −4 px on A.
    let s = svc();
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [{"text": "AB", "size": EM, "kerning": "optical"}],
    });
    let r = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    assert_eq!(
        s.caret_geometry(&r.sidecar, 1).unwrap()[0].x,
        56.0,
        "A's advance took the −4 optical kern"
    );
    assert_eq!(s.caret_geometry(&r.sidecar, 2).unwrap()[0].x, 118.0);
    assert_eq!(r.bounds.max.x, 108, "B's ink shifted left by 4");
    // Spaces never kern (inkless side): "A B" keeps plain advances.
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [{"text": "A B", "size": EM, "kerning": "optical"}],
    });
    let r = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    assert_eq!(
        s.caret_geometry(&r.sidecar, 3).unwrap()[0].x,
        172.0,
        "60 + 50 + 62: no optical kern across the space"
    );
}

// ---------------------------------------------------------------------
// Re-review round-2 pins (NEW-1/2/3)
// ---------------------------------------------------------------------

#[test]
fn step_2_order_ignores_host_provisioning_time() {
    // Re-review NEW-1 (the reviewer's probe): the §44.2 step-2 probe
    // order is the EMBED door's sequence on every host — a host that
    // pre-provisioned one of the document's faces must not hoist it.
    use appname_text::fontdb::FaceSource;
    let plain = common::build();
    let kerned = common::build_with_kern();
    let (e_id, f_id) = (Uuid::from_u128(1), Uuid::from_u128(2));

    let clean_host = TextLayoutService::default();
    clean_host.register_embedded_face(e_id, &plain).unwrap();
    clean_host.register_embedded_face(f_id, &kerned).unwrap();

    let provisioned_host = TextLayoutService::default();
    provisioned_host
        .register_provided_face(f_id, &kerned, FaceSource::System, "K", "R", "K-R")
        .unwrap();
    provisioned_host
        .register_embedded_face(e_id, &plain)
        .unwrap();
    provisioned_host
        .register_embedded_face(f_id, &kerned)
        .unwrap();

    let cur = add(&clean_host, "AB");
    let a = clean_host.validate_sidecar(&cur).unwrap();
    let b = provisioned_host.validate_sidecar(&cur).unwrap();
    assert_eq!(a.bounds, b.bounds, "same sidecar, same ink on every host");
    assert_eq!(
        clean_host.caret_geometry(&cur, 2).unwrap()[0].x,
        provisioned_host.caret_geometry(&cur, 2).unwrap()[0].x,
        "step-2 winner is the embed-order face (plain, no GPOS kern)"
    );
    assert_eq!(clean_host.caret_geometry(&cur, 2).unwrap()[0].x, 122.0);
}

#[test]
fn pair_kerns_never_bridge_a_dropped_break_glyph() {
    // Re-review NEW-2: break glyphs are dropped after shaping, so
    // buffer-adjacent glyphs can straddle a line break — they must not
    // kern. Reachable via tofu (one item spans the break).
    let s = svc();
    let plain = add(&s, "\u{e9}\n\u{e9}");
    let src = json!({
        "schema": TEXT_SIDECAR_SCHEMA,
        "layout": {"kind": "point", "origin": [0, 100]},
        "runs": [{"text": "\u{e9}\n\u{e9}", "size": EM, "kerning": -100}],
    });
    let kerned = s.validate_sidecar(src.to_string().as_bytes()).unwrap();
    assert_eq!(
        s.caret_geometry(&kerned.sidecar, 1).unwrap()[0].x,
        s.caret_geometry(&plain, 1).unwrap()[0].x,
        "the é|é 'pair' across the dropped newline glyph did not kern"
    );
}
