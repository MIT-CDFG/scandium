// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `vector.shape_add` §40.7 DIRTY-RECT battery: D must cover the PAINTED
//! result, not the bare primitive.
//!
//! THE DEFECT this battery pins (re-derived first-hand before the fix):
//! `vector.shape_add` emitted `D = geom.bounds` verbatim at EVERY
//! `widthPx` — a FLAT line across 0 / 4 / 20 / 40 — while the composite
//! door happily painted a §20.2 stroke band reaching `width/2` (Center)
//! or `width` (Outside) OUTSIDE those bounds. Every one of those pixels
//! was ink the shell was never told to repaint, i.e. user-visible stale
//! pixels. Before the §20.2 SHAPE-LAYER stroke slice (`8bd5ea1`) the
//! exposure was unreachable — the compositor REFUSED `widthPx > 0` — so
//! serving strokes is exactly what makes it reachable.
//!
//! WHY BOTH DOORS. The command door (`applyCommand` → `ApplyReceipt::
//! dirty`) is where D is announced; the COMPOSITE door
//! (`composite_plane_f32`) is where the user's pixels actually land.
//! A command-door-only assertion would prove nothing about staleness, so
//! every width here measures the real ink bounding box off the composited
//! plane and pins `D ⊇ ink`.
//!
//! NOT IN SCOPE: FI-25, the SPEC half — whether the frozen
//! `vector.shape_add.yaml:163` `dirtyRect: "D = shape bounds"` means the
//! primitive or the painted result is a fold-in wording question. These
//! tests stand on the user-visible staleness either way.

use appname_conformance::assembly_point;
use appname_core::{composite_plane_f32, Engine, ErrorCode, IRect};
use serde_json::{json, Value};

/// Canvas big enough that the widest sweep point's ink is never clipped
/// by the frame (width 40 Outside reaches `width` = 40 px per side off a
/// shape inset 80 px).
const CANVAS: i64 = 256;

/// The swept primitive: an axis-aligned rect, bounds 80..176 × 80..144.
const SHAPE: (f64, f64, f64, f64) = (80.0, 80.0, 96.0, 64.0);

fn env(n: u32, kind: &str, layer: Option<&str>, params: Value) -> String {
    let mut v = json!({
        "schema": "appname.command.v1",
        "id": format!("d127a000-0000-4000-8000-0004{n:08x}"),
        "kind": kind,
        "params": params,
    });
    if let Some(l) = layer {
        v["target"] = json!({"layerId": l});
    }
    v.to_string()
}

/// A TRANSPARENT-background document: composite alpha is then ink alpha
/// and nothing else, so the ink bounding box below needs no background
/// subtraction.
fn ready() -> Engine {
    let mut e = assembly_point::assembled_engine();
    e.apply_json(&env(
        1,
        "doc.new",
        None,
        json!({"width": CANVAS, "height": CANVAS, "background": "transparent"}),
    ))
    .expect("doc.new");
    e
}

fn blue_fill() -> Value {
    json!({"type": "solid", "color": {"model": "rgba", "comps": [0.0, 0.0, 1.0, 1.0]}})
}

/// A SOLID red stroke of the given width — opaque so its ink is
/// unambiguous in the alpha scan, and solid so the §22 gradient/pattern
/// wall never fires.
fn red_stroke(width: f64) -> Value {
    json!({"widthPx": width, "color": {"model": "rgba", "comps": [1.0, 0.0, 0.0, 1.0]}})
}

/// Apply one `vector.shape_add` and return `(D, primitive bounds)`.
///
/// The primitive bounds are read back off the created PATH node rather
/// than recomputed here, so "D grew past the bounds" is measured against
/// the engine's own hull and not against a number this file invented.
fn add_shape(e: &mut Engine, stroke: Value) -> (IRect, IRect) {
    let (x, y, w, h) = SHAPE;
    let receipt = e
        .apply_json(&env(
            2,
            "vector.shape_add",
            None,
            json!({
                "primitive": {"kind": "rect", "geometry": {"x": x, "y": y, "w": w, "h": h}},
                "fill": blue_fill(),
                "stroke": stroke,
                "newLayerId": "d127a000-0000-4000-8000-00000000beef",
            }),
        ))
        .expect("shape_add serves");
    assert_eq!(receipt.dirty.len(), 1, "shape_add announces exactly one D");
    let d = receipt.dirty[0].rect;
    let doc = e.document().expect("document");
    let pid = doc
        .layer(receipt.dirty[0].layer)
        .expect("the dirtied layer resolves")
        .shape_data()
        .expect("shape layer")
        .path();
    let bounds = doc.path(pid).expect("base path resolves").bounds();
    (d, bounds)
}

/// The bounding box of every pixel the COMPOSITE door actually inked
/// (alpha > 0), in document coordinates. `IRect::EMPTY` when nothing
/// painted.
///
/// This is the engine-true answer D is measured against: it is read off
/// `composite_plane_f32`, the same §19.1 output-frame door the reference
/// export and the CLI/cabi render doors frame on.
fn ink_bbox(e: &Engine) -> IRect {
    let (w, h, plane) = composite_plane_f32(e).expect("composite serves");
    let (mut lx, mut ly, mut hx, mut hy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let a = plane[((y as usize * w as usize) + x as usize) * 4 + 3];
            if a > 0.0 {
                lx = lx.min(x);
                ly = ly.min(y);
                hx = hx.max(x);
                hy = hy.max(y);
            }
        }
    }
    if lx == i32::MAX {
        return IRect::EMPTY;
    }
    IRect::new(lx, ly, hx + 1, hy + 1)
}

/// SELF-CHECK on the oracle (test doctrine: to trust a check, exhibit an
/// input on which it says NO). An empty document must scan EMPTY, and a
/// painted one must not — otherwise every `D ⊇ ink` assertion below is
/// the "assertion summing over an empty region" false green.
#[test]
fn the_ink_oracle_can_say_no() {
    let e = ready();
    assert_eq!(
        ink_bbox(&e),
        IRect::EMPTY,
        "a transparent empty canvas inks nothing — if this reds, the alpha \
         scan is measuring the background and every containment assertion \
         in this file is vacuous"
    );
    let mut e = ready();
    add_shape(&mut e, red_stroke(20.0));
    let ink = ink_bbox(&e);
    assert!(!ink.is_empty(), "a stroked shape inks something");
    assert!(
        !IRect::new(0, 0, 1, 1).contains_rect(&ink),
        "and the scan reports a REGION, not a degenerate point"
    );
}

/// THE SWEEP — not a point. `widthPx` 0 / 4 / 20 / 40 is a 10× range; a
/// flat D across it is the defect, and a single width could not tell the
/// two apart.
#[test]
fn dirty_rect_tracks_stroke_width_across_the_sweep() {
    // COLLECT the whole sweep before asserting anything: a flat line is
    // only legible as a TABLE, and a loop that dies on its first width
    // reports one point when the finding is the shape of four.
    let mut seen: Vec<(f64, IRect, IRect, IRect)> = Vec::new();
    for width in [0.0f64, 4.0, 20.0, 40.0] {
        let mut e = ready();
        let (d, bounds) = add_shape(&mut e, red_stroke(width));
        let ink = ink_bbox(&e);
        seen.push((width, d, bounds, ink));
    }
    for (width, d, bounds, ink) in &seen {
        eprintln!(
            "widthPx {width:>5}  D inflate {:>3}  ink reach {:>3}   D {:?}  bounds {:?}  ink {:?}",
            bounds.min.x - d.min.x,
            bounds.min.x - ink.min.x,
            d,
            bounds,
            ink,
        );
    }
    // THE INVARIANT: the announced repaint region covers every pixel the
    // composite door actually inked. Stale pixels are exactly the pixels
    // this misses.
    for (width, d, bounds, ink) in &seen {
        // `contains_rect` is VACUOUSLY TRUE on an empty rect, so the
        // containment assertion below is worthless unless the ink is
        // known to exist — the "assertion summing over an empty region"
        // false green, guarded here per width rather than once globally.
        assert!(
            !ink.is_empty(),
            "widthPx {width} inked NOTHING — the containment check that \
             follows would pass vacuously"
        );
        assert!(
            d.contains_rect(ink),
            "widthPx {width}: D {d:?} does NOT cover the painted ink {ink:?} \
             — {} px of engine-true ink land outside the repainted region",
            (bounds.min.x - ink.min.x).max(ink.max.x - bounds.max.x),
        );
    }
    // widthPx 0 is NO VISIBLE STROKE (§20.3) — D stays the bare bounds,
    // so the fix cannot have blanket-inflated every shape.
    assert_eq!(
        seen[0].1, seen[0].2,
        "widthPx 0: D == shape bounds, exactly"
    );
    // ... and D is STRICTLY MONOTONIC in the width over the rest. This is
    // the assertion the flat line fails: pre-fix all four D were equal.
    for pair in seen.windows(2) {
        let (wa, da, ..) = pair[0];
        let (wb, db, ..) = pair[1];
        assert!(
            db.min.x < da.min.x && db.max.x > da.max.x,
            "D must GROW from widthPx {wa} ({da:?}) to widthPx {wb} ({db:?}) — \
             a D that is flat across a 10× width range is the defect, not an \
             off-by-one"
        );
    }
}

/// The §20.2 align arms reach different distances, and D must follow each
/// one: `Inside` is gated by the fill (no outward ink at all), `Center`
/// is the ±width/2 band, `Outside` runs the full width out.
#[test]
fn dirty_rect_follows_the_align_arm() {
    let mut measured = Vec::new();
    for align in ["inside", "center", "outside"] {
        let mut e = ready();
        let mut stroke = red_stroke(20.0);
        stroke["align"] = json!(align);
        let (d, bounds) = add_shape(&mut e, stroke);
        let ink = ink_bbox(&e);
        assert!(!ink.is_empty(), "align {align} inked nothing (vacuous)");
        assert!(
            d.contains_rect(&ink),
            "align {align}: D {d:?} misses painted ink {ink:?}"
        );
        measured.push((align, d, bounds, ink));
    }
    let (_, inside_d, inside_bounds, inside_ink) = measured[0];
    assert_eq!(
        inside_d, inside_bounds,
        "an inside-aligned stroke paints only where the FILL covers, so it \
         owes no inflation at all"
    );
    assert!(
        inside_bounds.contains_rect(&inside_ink),
        "and the composite door agrees: inside ink stays within bounds"
    );
    let outside_d = measured[2].1;
    let center_d = measured[1].1;
    assert!(
        outside_d.min.x < center_d.min.x,
        "outside reaches further than center ({outside_d:?} vs {center_d:?})"
    );
}

/// UNDO AND REDO INHERIT THE INFLATION.
///
/// Stale pixels after an undo are the same defect as stale pixels after
/// the apply — arguably worse, since the ink being removed is the ink
/// left behind. `HistoryRow::dirty` is documented to echo the step's
/// receipt rects identically on apply / undo / redo (arbitration 2), so
/// this fix reaches both directions BY CONSTRUCTION rather than by a
/// second call site. That is exactly the kind of "by construction" claim
/// worth a check: if the echo is ever broken, the inflation silently
/// stops covering undo.
#[test]
fn undo_and_redo_echo_the_inflated_rect() {
    let mut e = ready();
    let (applied, bounds) = add_shape(&mut e, red_stroke(20.0));
    assert!(
        applied != bounds,
        "precondition: the applied D is inflated off the bare hull"
    );
    let undo = e.undo().expect("undo");
    assert_eq!(
        undo.dirty.len(),
        1,
        "the undo row carries the step's dirty rects"
    );
    assert_eq!(
        undo.dirty[0].rect, applied,
        "undo repaints the SAME inflated region the apply announced — a \
         bare-hull undo would leave the stroke band's ink on screen"
    );
    let redo = e.redo().expect("redo");
    assert_eq!(redo.dirty[0].rect, applied, "and so does redo");
}

/// THE INPUT ON WHICH A HALF-WIDTH BOUND SAYS NO.
///
/// The obvious fix here is "inflate by half the width", and on the
/// axis-aligned rect the sweep uses it would look perfect: the measured
/// ink reach there is exactly `width/2`. It is still WRONG. A §20.2
/// MITER join runs out to its apex, which the coverage admits all the way
/// to `miterLimit · h` — so a shape with a sharp vertex inks well past
/// half the width, and this test exhibits one rather than arguing it.
///
/// A star's tips are that vertex. If this ever reds because the measured
/// reach fell back to `h`, the miter term in
/// `ShapeStrokeStyle::outward_reach` has stopped being load-bearing and
/// the bound can be tightened — that is a finding, not a nuisance.
#[test]
fn a_sharp_miter_vertex_inks_past_half_the_width() {
    let mut e = ready();
    let receipt = e
        .apply_json(&env(
            2,
            "vector.shape_add",
            None,
            json!({
                // A 4-point star: its tips lie ON the axes, so the miter
                // apex pushes the axis-aligned bounding box the dirty
                // rect IS. inset 0.6 puts the tip angle near 43°, whose
                // apex sits at ~2.7·h — comfortably past half the width
                // and still inside the default miterLimit of 4, so the
                // coverage MITERS rather than falling back to a bevel.
                // (Sharpen it much further and the limit rejects the
                // apex, the join bevels, and the reach drops again.)
                "primitive": {"kind": "polygon", "sides": 4,
                              "geometry": {"x": 60, "y": 60, "w": 136, "h": 136,
                                           "inset": 0.6}},
                "fill": blue_fill(),
                "stroke": red_stroke(20.0),
                "newLayerId": "d127a000-0000-4000-8000-00000000cafe",
            }),
        ))
        .expect("star shape_add serves");
    let d = receipt.dirty[0].rect;
    let doc = e.document().expect("document");
    let pid = doc
        .layer(receipt.dirty[0].layer)
        .expect("layer")
        .shape_data()
        .expect("shape")
        .path();
    let bounds = doc.path(pid).expect("path").bounds();
    let ink = ink_bbox(&e);
    let reach = (bounds.min.x - ink.min.x)
        .max(bounds.min.y - ink.min.y)
        .max(ink.max.x - bounds.max.x)
        .max(ink.max.y - bounds.max.y);
    eprintln!("star  D {d:?}  bounds {bounds:?}  ink {ink:?}  reach {reach}");
    assert!(
        reach > 10,
        "a 5-point star at widthPx 20 must ink PAST half the width (10 px) at \
         its miter tips — measured reach {reach}. If this is now ≤ 10, the \
         miter term is no longer load-bearing and outward_reach can tighten."
    );
    assert!(
        d.contains_rect(&ink),
        "and D still covers it: D {d:?} vs ink {ink:?}"
    );
}

/// The GUARD on this fix's one silent arm. `shape_stroke_dirty_inflate`
/// returns 0 for a payload `ShapeStrokeStyle::from_wire` REFUSES, and the
/// rationale is that such a payload paints nothing at all: the composite
/// door refuses the whole render (QA-F8 — refuse, never omit), so no
/// pixel moves and there is nothing to repaint.
///
/// That rationale is CHECKED here rather than asserted in a comment. The
/// day a later slice serves `dashes` or `profile`, this test reds and
/// sends whoever served it back to the inflate.
#[test]
fn the_refused_stroke_arms_still_paint_nothing() {
    // `from_wire`'s refusal set ENTIRE — dashes (§20.2 rule 4), profile
    // (rule 6), and non-solid stroke paint (the §22 binding). Guarding
    // two of three would leave the third free to start serving behind
    // this bound's back.
    for stroke in [
        json!({"widthPx": 20, "color": {"model": "rgba", "comps": [1, 0, 0, 1]},
               "dashes": [8.0, 4.0]}),
        json!({"widthPx": 20, "color": {"model": "rgba", "comps": [1, 0, 0, 1]},
               "profile": [[0.0, 1.0], [1.0, 0.5]]}),
        json!({"widthPx": 20,
               "fill": {"type": "gradient",
                        "gradient": {"stops": [
                            {"t": 0.0, "color": {"model": "rgba", "comps": [1, 0, 0, 1]}},
                            {"t": 1.0, "color": {"model": "rgba", "comps": [0, 0, 1, 1]}}]}}}),
    ] {
        let mut e = ready();
        // The COMMAND door accepts it — the payload is contract-shaped.
        let (d, bounds) = add_shape(&mut e, stroke.clone());
        assert_eq!(
            d, bounds,
            "a composite-refused stroke owes no inflation: {stroke}"
        );
        // The COMPOSITE door is where it stops. This is a live case of
        // "the command door ACCEPTS and the composite door REFUSES".
        let err = composite_plane_f32(&e).expect_err(
            "the §20.2 rule-4 / rule-6 arms are NOT served by this slice — if \
             this now serves, `shape_stroke_dirty_inflate`'s zero arm is stale \
             and must bound the served reach",
        );
        assert_eq!(
            err.code,
            ErrorCode::NotImplemented,
            "typed refusal: {stroke}"
        );
    }
}
