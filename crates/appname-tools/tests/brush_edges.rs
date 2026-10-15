// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Brush engine v1 edge cases, end-to-end over the public API only (the
//! exact surface O-CORE wires at CP-0.3 wave 2): sample blob → decode →
//! plan → raster → present, both command kinds.
//!
//! Spec: ES §15.3 (single-dab stroke), §15.5 (flow vs opacity, erase
//! dst-out), §40.5 family rules (selection empty-intersection = empty
//! DeltaSet), §9.6 rule 2 (samples-by-asset round trip).

use appname_core::{BlendMode, IRect, IVec2, TileCoord, TILE, TILE_I32};
use appname_tools::brush::{
    commit_hash, decode_samples, encode_samples, render_brush_stroke, render_eraser_stroke,
    BrushPreset, BrushStrokeSpec, EraserStrokeSpec, InputSample, StrokeCommit, StrokeContext,
    UniformBackdrop,
};

fn sample(x: f64, y: f64, pressure: f32, t: f64) -> InputSample {
    InputSample {
        x,
        y,
        pressure,
        tilt_x: 0.0,
        tilt_y: 0.0,
        rotation: 0.0,
        velocity: 0.0,
        t,
    }
}

fn pixel_of(commit: &StrokeCommit, p: IVec2) -> Option<[f32; 4]> {
    let (tx, ty) = (p.x.div_euclid(TILE_I32), p.y.div_euclid(TILE_I32));
    let patch = commit
        .patches
        .iter()
        .find(|q| q.coord.x == tx && q.coord.y == ty)?;
    let (px, py) = (
        p.x.rem_euclid(TILE_I32) as usize,
        p.y.rem_euclid(TILE_I32) as usize,
    );
    Some(patch.pixels[py * TILE + px])
}

fn ctx(clip: IRect) -> StrokeContext<'static> {
    StrokeContext {
        clip,
        selection: None,
        kernels: None,
        clamp_to_backdrop_alpha: false,
    }
}

const CANVAS: IRect = IRect::new(0, 0, 512, 512);

/// The full §9.6 rule-2 path: encode the sample stream as the
/// `appname.samples.v1` asset bytes, decode it back (as the command
/// implementation will), and paint from the decoded stream — byte-identical
/// to painting from the original.
#[test]
fn blob_round_trip_paints_identically() {
    let original = vec![
        sample(20.0, 20.0, 0.4, 0.0),
        sample(60.0, 40.0, 0.8, 0.010),
        sample(120.0, 30.0, 1.0, 0.020),
    ];
    let blob = encode_samples(&original).unwrap();
    let decoded = decode_samples(&blob).unwrap();
    let preset = BrushPreset::soft_round_pressure();
    let mk = |samples: &[InputSample]| {
        let spec = BrushStrokeSpec {
            samples,
            preset: &preset,
            size_px: Some(24.0),
            hardness: None,
            opacity: 0.9,
            flow: 0.8,
            color: [0.1, 0.5, 0.9],
            blend: BlendMode::Normal,
            seed: 7,
        };
        let out = render_brush_stroke(&spec, &UniformBackdrop([0.0; 4]), &ctx(CANVAS)).unwrap();
        commit_hash(&out.commit).to_hex()
    };
    assert_eq!(mk(&original), mk(&decoded));
}

/// §15.3: a zero-length stroke (single sample / single click) is exactly
/// one dab at the pen-down position; the contract yamls add nothing beyond
/// the §15.3 rule.
#[test]
fn zero_length_stroke_paints_one_dab() {
    let samples = vec![sample(100.0, 100.0, 1.0, 0.0)];
    let preset = BrushPreset::hard_round(16.0);
    let spec = BrushStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: None,
        hardness: None,
        opacity: 1.0,
        flow: 1.0,
        color: [1.0, 0.0, 0.0],
        blend: BlendMode::Normal,
        seed: 0,
    };
    let out = render_brush_stroke(&spec, &UniformBackdrop([0.0; 4]), &ctx(CANVAS)).unwrap();
    assert_eq!(out.plan.dabs.len(), 1);
    assert_eq!(out.plan.dabs[0].center, [100.0, 100.0]);
    // The dab actually landed: full alpha under the center.
    let px = pixel_of(&out.commit, IVec2::new(100, 100)).unwrap();
    assert!(px[3] > 0.99, "painted center, got {px:?}");
}

/// Out-of-bounds clipping: fully outside → zero patches (empty DeltaSet);
/// straddling → pixels beyond the clip stay untouched.
#[test]
fn out_of_bounds_strokes_clip() {
    let preset = BrushPreset::hard_round(30.0);
    fn mk_spec<'a>(samples: &'a [InputSample], preset: &'a BrushPreset) -> BrushStrokeSpec<'a> {
        BrushStrokeSpec {
            samples,
            preset,
            size_px: None,
            hardness: None,
            opacity: 1.0,
            flow: 1.0,
            color: [0.0, 1.0, 0.0],
            blend: BlendMode::Normal,
            seed: 0,
        }
    }
    // Entirely outside the canvas.
    let outside = vec![
        sample(-500.0, -500.0, 1.0, 0.0),
        sample(-400.0, -520.0, 1.0, 0.01),
    ];
    let out = render_brush_stroke(
        &mk_spec(&outside, &preset),
        &UniformBackdrop([0.0; 4]),
        &ctx(CANVAS),
    )
    .unwrap();
    assert!(
        out.commit.patches.is_empty(),
        "fully clipped stroke = empty DeltaSet"
    );
    assert!(
        !out.plan.dabs.is_empty(),
        "the plan itself still exists (clip is raster-stage)"
    );

    // Straddling the left edge: x < 0 must stay untouched even inside a
    // patch tile... which cannot exist: patches are canvas tiles only.
    let straddle = vec![
        sample(-10.0, 100.0, 1.0, 0.0),
        sample(30.0, 100.0, 1.0, 0.01),
    ];
    let out = render_brush_stroke(
        &mk_spec(&straddle, &preset),
        &UniformBackdrop([0.0; 4]),
        &ctx(CANVAS),
    )
    .unwrap();
    assert!(!out.commit.patches.is_empty());
    for patch in &out.commit.patches {
        assert!(
            patch.coord.x >= 0 && patch.coord.y >= 0,
            "no patches outside the canvas grid"
        );
    }
    // In-canvas pixel near the edge painted; the commit bounds honor clip.
    assert!(pixel_of(&out.commit, IVec2::new(5, 100)).unwrap()[3] > 0.0);
    assert!(out.commit.bounds.min.x >= 0);
}

/// §15.5 flow-vs-opacity semantics, end-to-end: a self-crossing stroke at
/// flow 0.4 / opacity 0.6 never exceeds alpha 0.6 anywhere, while a
/// single-pass region sits near flow-per-dab coverage.
#[test]
fn opacity_ceiling_holds_on_self_crossing_strokes() {
    // A tight figure-eight-ish path crossing itself at (100, 100).
    let samples = vec![
        sample(60.0, 100.0, 1.0, 0.00),
        sample(100.0, 100.0, 1.0, 0.01),
        sample(140.0, 100.0, 1.0, 0.02),
        sample(140.0, 140.0, 1.0, 0.03),
        sample(100.0, 100.0, 1.0, 0.04),
        sample(60.0, 60.0, 1.0, 0.05),
    ];
    let preset = BrushPreset::hard_round(20.0);
    let spec = BrushStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: None,
        hardness: None,
        opacity: 0.6,
        flow: 0.4,
        color: [1.0, 1.0, 1.0],
        blend: BlendMode::Normal,
        seed: 0,
    };
    let out = render_brush_stroke(&spec, &UniformBackdrop([0.0; 4]), &ctx(CANVAS)).unwrap();
    let mut max_alpha = 0.0f32;
    for patch in &out.commit.patches {
        for px in &patch.pixels {
            max_alpha = max_alpha.max(px[3]);
        }
    }
    assert!(
        max_alpha <= 0.6 + 1e-6,
        "opacity is the per-stroke ceiling, got {max_alpha}"
    );
    assert!(
        max_alpha > 0.55,
        "self-overlap builds toward the ceiling, got {max_alpha}"
    );
}

/// Eraser end-to-end: same dab pipeline, dst-out presentation; erasing the
/// painted region of a uniform backdrop scales all four channels by
/// (1 − src_a) and a full-strength pass clears to zero.
#[test]
fn eraser_is_dst_out_over_the_same_pipeline() {
    let backdrop = UniformBackdrop([0.4, 0.3, 0.2, 0.8]);
    let samples = vec![sample(50.0, 50.0, 1.0, 0.0), sample(150.0, 50.0, 1.0, 0.02)];
    let preset = BrushPreset::hard_round(24.0);
    let spec = EraserStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: None,
        opacity: 1.0,
        flow: 1.0,
        seed: 3,
    };
    let out = render_eraser_stroke(&spec, &backdrop, &ctx(CANVAS)).unwrap();
    // Center of the stroke: cov 1 → src_a 1 → fully erased.
    let px = pixel_of(&out.commit, IVec2::new(100, 50)).unwrap();
    assert_eq!(px, [0.0, 0.0, 0.0, 0.0]);
    // Partial-opacity erase: src_a = 0.5 → all channels halved.
    let spec_half = EraserStrokeSpec {
        opacity: 0.5,
        ..spec
    };
    let out = render_eraser_stroke(&spec_half, &backdrop, &ctx(CANVAS)).unwrap();
    let px = pixel_of(&out.commit, IVec2::new(100, 50)).unwrap();
    for (got, want) in px.iter().zip([0.2, 0.15, 0.1, 0.4]) {
        assert!((got - want).abs() < 1e-6, "dst-out at 0.5: got {px:?}");
    }
    // Untouched pixels keep the backdrop exactly.
    let far = pixel_of(&out.commit, IVec2::new(100, 80));
    if let Some(far) = far {
        assert_eq!(far, [0.4, 0.3, 0.2, 0.8]);
    }
}

/// Brush and eraser share the dab pipeline bit-for-bit: same params + seed
/// produce the identical dab sequence (only presentation differs, §15.5).
#[test]
fn brush_and_eraser_share_the_dab_pipeline() {
    let samples = vec![
        sample(20.0, 20.0, 0.3, 0.0),
        sample(80.0, 60.0, 0.7, 0.01),
        sample(160.0, 40.0, 1.0, 0.02),
    ];
    let preset = BrushPreset::soft_round_pressure();
    let b = BrushStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: Some(40.0),
        hardness: None,
        opacity: 0.8,
        flow: 0.6,
        color: [0.0, 0.0, 0.0],
        blend: BlendMode::Normal,
        seed: 99,
    };
    let e = EraserStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: Some(40.0),
        opacity: 0.8,
        flow: 0.6,
        seed: 99,
    };
    let ob = render_brush_stroke(&b, &UniformBackdrop([0.0; 4]), &ctx(CANVAS)).unwrap();
    let oe =
        render_eraser_stroke(&e, &UniformBackdrop([0.2, 0.2, 0.2, 0.9]), &ctx(CANVAS)).unwrap();
    assert_eq!(
        ob.plan.dabs, oe.plan.dabs,
        "one engine, two presentations (§15.5/§15.7)"
    );
}

/// Patches are canvas-grid tiles carrying the FULL post-stroke tile plane:
/// pixels the stroke missed hold backdrop values verbatim (the DeltaSet
/// write O-CORE performs is a whole-tile install).
#[test]
fn patches_carry_backdrop_in_unpainted_pixels() {
    let backdrop = UniformBackdrop([0.25, 0.5, 0.75, 1.0]);
    let samples = vec![sample(10.0, 10.0, 1.0, 0.0)];
    let preset = BrushPreset::hard_round(8.0);
    let spec = BrushStrokeSpec {
        samples: &samples,
        preset: &preset,
        size_px: None,
        hardness: None,
        opacity: 1.0,
        flow: 1.0,
        color: [1.0, 0.0, 0.0],
        blend: BlendMode::Normal,
        seed: 0,
    };
    let out = render_brush_stroke(&spec, &backdrop, &ctx(CANVAS)).unwrap();
    assert_eq!(out.commit.patches.len(), 1);
    assert_eq!(out.commit.patches[0].coord, TileCoord::new(0, 0, 0));
    let far = pixel_of(&out.commit, IVec2::new(200, 200)).unwrap();
    assert_eq!(
        far,
        [0.25, 0.5, 0.75, 1.0],
        "unpainted pixels = backdrop verbatim"
    );
}
