// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Brush engine v1 — pure, deterministic dab stamping (CP-0.3, plan §2.2
//! row "Brush engine v1").
//!
//! Spec: ES Part 15 — §15.1 (input samples), §15.3 (stroke geometry:
//! centripetal Catmull-Rom + arc-length march), §15.4 (dynamics + response
//! curves, PCG32 jitter), §15.5 (dab/stroke compositing model, tip coverage,
//! CPU mirror discipline), §15.7 (presets); §10.3 stroke commons; §10.5
//! merge/session rules (engine-external); §40.5 stroke family rules;
//! contracts `brush.stroke.yaml` + `eraser.stroke.yaml`. DD ⑮.
//! Dynamics semantics informed by `appname-proto/research/memos/
//! MEMO-libmypaint-brush-dynamics.md` (concept reference only — libmypaint
//! is ISC-licensed per the census row; **no code is ported here**, and the
//! memo itself pins that Appname's multiplicative monotone-cubic model and
//! smoothstep tip are the SPEC's own design, not libmypaint's additive
//! model or two-segment falloff).
//!
//! # What this module is (and is not)
//!
//! The engine is **pure**: stroke params + samples + a pre-stroke backdrop
//! view in → a deterministic dab sequence and composited result tiles out.
//! It performs **no document mutation** (D-48: `appname-core`'s command
//! engine is the only mutation door; every write door there is
//! `pub(crate)`). O-CORE's CP-0.3 wave-2 command implementations for
//! `brush.stroke` / `eraser.stroke` call this crate 1:1 with the contract
//! params and install the returned tile patches through core's own doors.
//! Until that wiring lands, both kinds keep returning
//! `error.command.not_implemented`.
//!
//! Pipeline (all stages public, so tests and O-CORE can hold any seam):
//!
//! ```text
//! decode_samples(blob)                 §15.1/§9.6  sample.rs
//!   → plan_stroke(samples, preset, params)  §15.3/§15.4  path.rs + plan.rs
//!       = StrokePlan { dabs, bounds }       (the deterministic dab sequence)
//!   → rasterize_plan(&plan, clip)      §15.5 steps 1–3  raster.rs
//!       = StrokeScratch (per-tile coverage, screen accumulation)
//!   → present(&scratch, backdrop, op, opts)  §15.5 step 4  present.rs
//!       = StrokeCommit { patches, bounds }   (composited f32 tiles keyed by
//!                                             appname-core TileCoord)
//! ```
//!
//! # v1 determinism pins (frozen wire behavior — changing any of these
//! changes replay output and requires a spec-owner decision)
//!
//! RNG schedule (§15.4: PCG32, seeded with the command-recorded `seed`,
//! sub-streamed so evaluation order is irrelevant):
//!
//! * Stream ids: `DynOutput` ordinal 0–5 (Size, Opacity, Flow, Angle,
//!   Scatter, Roundness); stream 6 = the shared `DynInput::Random` value.
//! * Per march position, in march order: stream 6 draws exactly **one**
//!   uniform (the dab's Random input value), unconditionally.
//! * Per output: jitter draws one `U(−1,1)` per attached curve **with
//!   `jitter > 0`**, in curve-array order, from that output's stream.
//! * Scatter offsets (stream 4, after the scatter-output jitter draws):
//!   one `U(0,1)` count draw iff `countJitter > 0`; then per spawned dab,
//!   one `U(−1,1)` normal draw iff `amount ≠ 0`, plus one `U(−1,1)` tangent
//!   draw iff `bothAxes`.
//!
//! Math pins:
//!
//! * Coincident consecutive samples (bitwise-equal positions) are dropped
//!   before knot computation (§15.3); the dropped samples' dynamics fields
//!   drop with them.
//! * Centripetal Catmull-Rom is evaluated with the pyramidal (Barry–
//!   Goldman) formulation; a zero knot interval (the §15.3 duplicated
//!   endpoints) resolves each degenerate lerp to its first operand — exact,
//!   since coincident knots come from coincident points.
//! * Dynamics multiplier curves attenuate (`[0,1]` products, §15.4); the
//!   angle output adds `eval·2π` radians per curve (the curve's `[0,1]`
//!   range spans one turn — the §15.4 jitter `range` for angle).
//! * `diameter = base · size_mult`, floored at `min(1 px, base)` — the
//!   §15.4 "floor 1 px" guard, kept from overriding sub-pixel bases, which
//!   the §10.3 `sizePx` range explicitly allows down to 0.1.
//! * Per-dab evaluated `opacity` (`base · opacity_mult`) is recorded on the
//!   `Dab` but v1 presentation applies the **stroke** `opacity` param as the
//!   §15.5 step-4 ceiling. The default `.bbrush` opacity table is empty, so
//!   v1 defaults are exact; the composition of per-dab opacity with the
//!   single presentation ceiling is an open spec question (lane log).
//! * Stroke heading where undefined (single-point stroke) is +x.
//! * Rasterization mirrors the §15.5 `cs_stamp` WGSL op-for-op in f32, in
//!   march order (the shader's per-pixel dab loop is the same order), so the
//!   CPU result is the §34.3 parity oracle for the later GPU path.
//! * `angle_rad == 0` short-circuits to `(cos, sin) = (1, 0)` — the default
//!   stroke matrix stays free of platform-`libm` `sin`/`cos` variance.
//!   Non-zero angles and `Direction`-input curves use `sin`/`cos`/`atan2`
//!   and are bit-stable per build, not across libm implementations
//!   (documented caveat; cross-platform golden policy is O-QA's).
//!
//! Flow vs opacity (§15.5 [NORMATIVE], the two-buffer model): per-dab
//! `a = flow · t(p) · (1 − cov)` accumulates coverage ("screen"), so
//! repeated passes within one stroke approach 1 at a rate set by `flow`;
//! presentation applies `src_a = opacity · cov · sel(p)` once against the
//! **pre-stroke** backdrop, so `opacity` is the per-stroke ceiling and
//! self-overlap never exceeds it. The eraser is the same pipeline with
//! dst-out presentation (`layer' = layer · (1 − src_a)`).
//!
//! Deferred beyond v1 (fields intentionally absent from the v1 preset;
//! plan §2 rows): textured/dual tips, wet edges, build-up/airbrush timer
//! dabs, pencil mode, smudge, `.bbrush` JSON + resource index (CP-0.5),
//! `.myb` import (CP-0.7), stabilization (§15.6 — a UI-side path transform;
//! recorded samples are pre-stabilization by spec).

pub mod curve;
pub(crate) mod path;
pub mod plan;
pub mod present;
pub mod preset;
pub mod raster;
pub mod rng;
pub mod sample;

pub use curve::{CurveError, MonotoneCurve};
pub use plan::{plan_stroke, Dab, StrokeError, StrokeParams, StrokePlan};
pub use present::{
    commit_hash, present, present_tile, Backdrop, BlendKernels, CoverageRead, PaintOp,
    PresentOptions, StrokeCommit, TilePatch, UniformBackdrop,
};
pub use preset::{
    BrushDynamics, BrushPreset, DynInput, DynOutput, DynamicCurve, PresetError, ScatterParams,
};
pub use raster::{dab_coverage_at, rasterize_plan, StrokeScratch};
pub use sample::{
    decode_samples, encode_samples, validate_samples, InputSample, SampleBlobError, SampleField,
    SAMPLE_BLOB_HEADER_BYTES, SAMPLE_BLOB_MAGIC, SAMPLE_BLOB_MAX_COUNT, SAMPLE_BLOB_VERSION,
    SAMPLE_RECORD_BYTES,
};

use appname_core::{BlendMode, IRect};

/// `brush.stroke` 1:1 with the contract params (`brush.stroke.yaml`
/// paramsSchema): O-CORE resolves `brushPreset` (resource index §28.1) and
/// the `samples` asset (via `sample::decode_samples`), converts the §22.1
/// `color` object to straight working-space linear RGB, maps
/// `sizePx`/`hardness`/`opacity`/`flow`/`blend`/`seed` verbatim, and calls
/// [`render_brush_stroke`].
#[derive(Clone, Copy, Debug)]
pub struct BrushStrokeSpec<'a> {
    /// Decoded `samples` asset (§9.6 rule 2).
    pub samples: &'a [sample::InputSample],
    /// The resolved brush preset (§15.7).
    pub preset: &'a preset::BrushPreset,
    /// Contract `sizePx` (0.1..=10 000; absent = preset size).
    pub size_px: Option<f32>,
    /// Contract `hardness` (0..=1; absent = preset hardness). Symmetric with
    /// `size_px`: a per-command override of the §15.7 `Stamp.hardness`.
    pub hardness: Option<f32>,
    /// Contract `opacity` (0..=1, default 1).
    pub opacity: f32,
    /// Contract `flow` (0..=1, default 1).
    pub flow: f32,
    /// Contract `color`, converted to straight working-space linear RGB.
    pub color: [f32; 3],
    /// Contract `blend` (§4.5 wire mode, default normal).
    pub blend: BlendMode,
    /// Contract `seed` (§15.4 per-stroke dynamics seed).
    pub seed: u64,
}

/// `eraser.stroke` 1:1 with the contract params (`eraser.stroke.yaml`).
/// `eraseToHistory` is a stage-level semantic gate (`true` →
/// `error.unsupported.erase_to_history`, §40.5) enforced upstream by
/// O-CORE — it never reaches the engine. The eraser's TRANSPARENCY-lock
/// hard block is likewise stage 4.
#[derive(Clone, Copy, Debug)]
pub struct EraserStrokeSpec<'a> {
    pub samples: &'a [sample::InputSample],
    pub preset: &'a preset::BrushPreset,
    pub size_px: Option<f32>,
    pub opacity: f32,
    pub flow: f32,
    pub seed: u64,
}

/// Everything surrounding the stroke that the command implementation owns:
/// the surface clip rect, the active selection, the §4.5 kernel table, and
/// the TRANSPARENCY-lock constrain flag.
#[derive(Clone, Copy)]
pub struct StrokeContext<'a> {
    /// Rasterization clip (the canvas/surface rect); dab area outside is
    /// dropped (§15.5 tile-touch enumeration "intersected with the canvas").
    pub clip: IRect,
    /// `sel(p)` (§13.1); `None` = fully selected.
    pub selection: Option<&'a dyn present::CoverageRead>,
    /// §4.5 kernels for non-`normal` blends (O-RENDER's shared math
    /// library, passed through by O-CORE at wiring).
    pub kernels: Option<&'a dyn present::BlendKernels>,
    /// §40.5 TRANSPARENCY lock constrain (paint: coverage clamps to
    /// existing alpha).
    pub clamp_to_backdrop_alpha: bool,
}

/// The full deterministic result: the dab sequence (exposed for tests and
/// goldens) and the composited tiles.
pub struct StrokeOutcome {
    pub plan: plan::StrokePlan,
    pub commit: present::StrokeCommit,
}

/// Plan → rasterize → present one `brush.stroke` against an RGBA surface.
pub fn render_brush_stroke<B: present::Backdrop + ?Sized>(
    spec: &BrushStrokeSpec<'_>,
    backdrop: &B,
    ctx: &StrokeContext<'_>,
) -> Result<StrokeOutcome, plan::StrokeError> {
    let params = plan::StrokeParams {
        size_px: spec.size_px,
        hardness: spec.hardness,
        opacity: spec.opacity,
        flow: spec.flow,
        seed: spec.seed,
    };
    let stroke_plan = plan::plan_stroke(spec.samples, spec.preset, &params)?;
    let scratch = raster::rasterize_plan(&stroke_plan, ctx.clip);
    let op = present::PaintOp::Paint {
        color: spec.color,
        blend: spec.blend,
        kernels: ctx.kernels,
    };
    let opts = present::PresentOptions {
        opacity: spec.opacity,
        selection: ctx.selection,
        clamp_to_backdrop_alpha: ctx.clamp_to_backdrop_alpha,
    };
    let commit = present::present(&scratch, backdrop, &op, &opts)?;
    Ok(StrokeOutcome {
        plan: stroke_plan,
        commit,
    })
}

/// Plan → rasterize → present one `eraser.stroke` (dst-out) against an
/// RGBA surface. Mask/channel targets use the same phases with
/// `PaintOp::PaintSingle`/`EraseSingle` (§14.1 single-component forms).
pub fn render_eraser_stroke<B: present::Backdrop + ?Sized>(
    spec: &EraserStrokeSpec<'_>,
    backdrop: &B,
    ctx: &StrokeContext<'_>,
) -> Result<StrokeOutcome, plan::StrokeError> {
    let params = plan::StrokeParams {
        size_px: spec.size_px,
        hardness: None, // eraser.stroke exposes no hardness override in v1
        opacity: spec.opacity,
        flow: spec.flow,
        seed: spec.seed,
    };
    let stroke_plan = plan::plan_stroke(spec.samples, spec.preset, &params)?;
    let scratch = raster::rasterize_plan(&stroke_plan, ctx.clip);
    let opts = present::PresentOptions {
        opacity: spec.opacity,
        selection: ctx.selection,
        clamp_to_backdrop_alpha: false, // eraser: TRANSPARENCY blocks at stage 4 instead
    };
    let commit = present::present(&scratch, backdrop, &present::PaintOp::Erase, &opts)?;
    Ok(StrokeOutcome {
        plan: stroke_plan,
        commit,
    })
}
