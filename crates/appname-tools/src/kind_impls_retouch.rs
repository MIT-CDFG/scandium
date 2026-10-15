// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `clone.stroke.yaml` / `heal.stroke.yaml` /
//! `dodge_burn.stroke.yaml` / `smudge.stroke.yaml` /
//! `blur_sharpen.stroke.yaml` (params + lock tables + cross-fields), ES
//! §40.5 (stroke family rules), §21.1/§21.2/§21.5, Part 45 §45.1/§45.2
//! via `crate::retouch` (the pure engines), §45.3 (spot content-aware
//! fill mode → the exemplar engine, CAF-consumer precedent), §11.1
//! (locks), §45 intro (retouch stochastic seeds derive from the envelope
//! mergeKey, blake3).
//!
//! The five CP-1.0 retouch-row [`KindImpl`]s — the checkpoint's last real
//! NI kinds. Same assembly seam as every stroke kind (integration-log
//! wave-2 arbitration 1: `kind_impls::assembly`; bare core never sees
//! them). Stage map (§40.5 over the engine's pinned order): stage 5 =
//! contract-generated schema (enums/ranges/required; the contracts'
//! `allOf` cross-fields are stage-6 here, kind-impl precedent); stage 6 =
//! target resolution (layer-only, the standing brush-family posture —
//! mask/channel stroke surfaces stay a typed refusal until the family
//! grows them together, lane log) → §11.1 locks (per-kind table) →
//! samples/preset resolve → the kind's pinned cross-field micro-order →
//! source/pattern resolution → render → install through the
//! recorder-coupled doors.
//!
//! Pinned micro-orders (documented per contract, tested below):
//!
//! * `clone.stroke`: source shape parse → §45.2 tri-state
//!   (`composite`+`sampleAllLayers:false` = the invalid combination →
//!   `error.schema.out_of_range`, contract note 1; the clone validMinimal
//!   conflicts with that note — recorded + escalated, note implemented) →
//!   the sample-scope arms. ALL THREE VALID ARMS NOW SERVE:
//!   `{layerId}`+false = the armed layer; `{layerId}`+true =
//!   `current_and_below`; `"composite"`+true = the full document
//!   composite — the last two through core's lazy sampling composite
//!   ([`appname_core::resolve_sampling_composite`]), which shares ONE
//!   per-pixel source-over with `image.flatten` so "sample all layers"
//!   and "flatten, then sample" cannot drift. §45.2 `ignoreAdjustments`
//!   is honored on the `"composite"` arm. §45.2's per-DAB liveness is a
//!   RECORDED DIVERGENCE — sampling resolves once per coalesced stroke;
//!   see `resolve_sampling_surface`'s doc for the reason and the scope
//!   it is escalated to.
//! * `heal.stroke`: (1) spot forbids source/offset → `out_of_range`;
//!   (2) sampled+pattern together → `error.schema.exclusive_params`
//!   (§45.1); (3) healing requires (source AND offset) OR pattern →
//!   `missing_field`; pattern then resolves — no pattern arena exists in
//!   v1, so every pattern id answers the contract's typed
//!   `error.retouch.pattern_missing` (deleted and never-registered are
//!   indistinguishable; posture recorded).
//! * `blur_sharpen.stroke`: RAW-params presence of `protectDetail` with
//!   `mode:"blur"` → `out_of_range` (§40.5 cross-field; the stage-5
//!   default fill makes the EFFECTIVE map carry it always, so the check
//!   reads the envelope's raw params).
//! * `smudge.stroke`: `fingerPainting:true` needs the §26.1 record-time
//!   effective color, and the schema carries NO color field — contract
//!   gap, typed `not_implemented` + escalation (lane log).
//!
//! Determinism: `seed` command-recorded (§15.4, omitted → 0, the
//! recorded family divergence); the spot content-aware seed derives
//! blake3(mergeKey) per the §45 intro (absent mergeKey → blake3 of the
//! envelope id string — replay-stable either way; pin recorded).

use std::sync::Arc;

use crate::kind_impls::{
    canvas_clip, check_locks, has_lock, map_stroke_error, param_f32, param_f32_opt, param_seed,
    param_uuid, resolve_preset, resolve_samples, resolve_stroke_target,
};
use crate::retouch::{
    footprint_mask, render_blur_sharpen_stroke, render_clone_stroke, render_dodge_burn_stroke,
    render_heal_stroke, render_smudge_stroke, render_spot_heal_stroke, CloneSpec, FocusMode,
    FocusSpec, HealSampledSpec, RetouchCommit, RetouchError, RetouchIo, SmudgeSpec, SpotArm,
    ToneMode, ToneRange, ToneSpec,
};
use appname_core::{Buffer, EngineError, ErrorCode, KindContext, KindImpl, KindOutcome, LayerId};
use serde_json::Value;
use uuid::Uuid;

pub struct CloneStrokeKind;
pub struct HealStrokeKind;
pub struct DodgeBurnStrokeKind;
pub struct SmudgeStrokeKind;
pub struct BlurSharpenStrokeKind;

// ---------------------------------------------------------------------------
// Shared plumbing.
// ---------------------------------------------------------------------------

/// Decoded stroke commons + the resolved destination, one door for all
/// five kinds (family rules, §40.5).
struct Commons {
    layer: LayerId,
    samples: Vec<crate::brush::InputSample>,
    preset: crate::brush::BrushPreset,
    size_px: Option<f32>,
    opacity: f32,
    flow: f32,
    seed: u64,
    clamp: bool,
}

fn stroke_commons(cx: &KindContext<'_>, transparency_clamps: bool) -> Result<Commons, EngineError> {
    let layer = resolve_stroke_target(cx)?;
    check_locks(cx, layer, false)?;
    let clamp = transparency_clamps && has_lock(cx, layer, "transparency");
    let samples_bytes = resolve_samples(cx)?;
    let samples = crate::brush::decode_samples(&samples_bytes)
        .map_err(|e| EngineError::bad_target(format!("samples asset failed to parse: {e}")))?;
    let preset = resolve_preset(param_uuid(cx, "brushPreset")?)?;
    Ok(Commons {
        layer,
        samples,
        preset,
        size_px: param_f32_opt(cx, "sizePx"),
        opacity: param_f32(cx, "opacity", 1.0),
        flow: param_f32(cx, "flow", 1.0),
        seed: param_seed(cx),
        clamp,
    })
}

fn io_of<'a>(cx: &'a KindContext<'_>, c: &'a Commons, backdrop: &'a Buffer) -> RetouchIo<'a> {
    RetouchIo {
        samples: &c.samples,
        preset: &c.preset,
        size_px: c.size_px,
        opacity: c.opacity,
        flow: c.flow,
        seed: c.seed,
        backdrop,
        premultiplied: backdrop.premultiplied(),
        clip: canvas_clip(cx),
        selection: Some(cx.document().selection()),
        clamp_to_backdrop_alpha: c.clamp,
    }
}

/// Install the commit through the recorder-coupled doors. `can_erase`:
/// clone/smudge can reduce alpha (tile-granular bounds re-tighten, the
/// eraser rule); the alpha-preserving kinds grow eagerly (§2.4).
fn install_commit(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    commit: RetouchCommit,
    can_erase: bool,
) -> Result<KindOutcome, EngineError> {
    for patch in &commit.patches {
        cx.install_tile_plane(layer, patch.coord, &patch.pixels)?;
    }
    if commit.patches.is_empty() {
        return Ok(KindOutcome::default()); // §40.1 empty DeltaSet push
    }
    // Q-1 determinism class: finish through the shared stroke-bounds door so
    // heal/blur_sharpen (grow) AND smudge/clone (erase) land on the loader's
    // pixel-exact extent, not the pre-quantization coverage rect (grow) or the
    // tile-granular shrink (erase) that both forked `state_hash` across save→
    // reopen and killed replay-as-recovery undo.
    crate::kind_impls::finish_write_bounds(cx, layer, commit.bounds, can_erase)?;
    Ok(KindOutcome::default())
}

fn map_retouch_error(e: RetouchError) -> EngineError {
    match e {
        RetouchError::Stroke(e) => map_stroke_error(e),
        RetouchError::NoSourceFound => EngineError::new(
            ErrorCode::RetouchNoSourceFound,
            "spot healing found no valid auto-source after the widened pass (§45.1 rule 3); \
             the stroke aborts atomically",
        ),
    }
}

/// The parsed `source` union (§40.5: a resolvable layer or the literal
/// "composite"; any other shape is a stage-6 structured-union violation).
enum SourceParam {
    Layer(Uuid),
    Composite,
}

fn parse_source(cx: &KindContext<'_>) -> Result<Option<SourceParam>, EngineError> {
    match cx.params().get("source") {
        None => Ok(None),
        Some(Value::String(s)) if s == "composite" => Ok(Some(SourceParam::Composite)),
        Some(Value::Object(o)) => {
            let id = o
                .get("layerId")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| {
                    EngineError::range(
                        "source.layerId",
                        "`source` object form requires a uuid `layerId` (§40.5)",
                    )
                })?;
            if o.len() != 1 {
                return Err(EngineError::range(
                    "source",
                    "`source` object form carries exactly `layerId` \
                     (additionalProperties: false)",
                ));
            }
            Ok(Some(SourceParam::Layer(id)))
        }
        Some(_) => Err(EngineError::range(
            "source",
            "`source` must be {layerId: uuid} or the literal \"composite\" (§40.5)",
        )),
    }
}

fn parse_offset(cx: &KindContext<'_>) -> Result<Option<[f64; 2]>, EngineError> {
    match cx.params().get("offset") {
        None => Ok(None),
        Some(Value::Array(a)) if a.len() == 2 => {
            let dx = a[0]
                .as_f64()
                .ok_or_else(|| EngineError::range("offset", "`offset` items must be numbers"))?;
            let dy = a[1]
                .as_f64()
                .ok_or_else(|| EngineError::range("offset", "`offset` items must be numbers"))?;
            Ok(Some([dx, dy]))
        }
        Some(_) => Err(EngineError::range(
            "offset",
            "`offset` must be [dx, dy] (2 numbers, §40.5)",
        )),
    }
}

/// The resolved §45.2 sampling surface: either ONE layer's buffer (the
/// `current` arm) or the lazy multi-layer composite (`current_and_below` /
/// `all`). Both answer the stroke engines' [`crate::brush::present::
/// Backdrop`] seam, so `render_clone_stroke`/`render_heal_stroke` never
/// learn which arm they are on.
enum SamplingSurface<'a> {
    Layer(&'a Buffer),
    Composite(CompositeBackdrop<'a>),
}

impl SamplingSurface<'_> {
    fn backdrop(&self) -> &dyn crate::brush::present::Backdrop {
        match self {
            SamplingSurface::Layer(b) => *b,
            SamplingSurface::Composite(c) => c,
        }
    }

    fn premultiplied(&self) -> bool {
        match self {
            SamplingSurface::Layer(b) => b.premultiplied(),
            SamplingSurface::Composite(c) => c.0.premultiplied(),
        }
    }
}

/// [`Backdrop`](crate::brush::present::Backdrop) over core's lazy
/// §45.2/§13.3 sampling composite. Reads composite bottom-to-top on
/// demand — no canvas-sized plane is ever materialized (core module doc).
pub(crate) struct CompositeBackdrop<'a>(pub(crate) appname_core::SamplingComposite<'a>);

impl crate::brush::present::Backdrop for CompositeBackdrop<'_> {
    fn read_pixel(&self, p: appname_core::IVec2) -> [f32; 4] {
        self.0.read_pixel_storage(p)
    }
}

/// §45.2 sample-scope tri-state, shared by clone and heal sampled
/// guidance:
///
/// | `source` | `sampleAllLayers` | surface |
/// |---|---|---|
/// | `{layerId}` | `false` | that layer alone (`current`) |
/// | `{layerId}` | `true` | the layer + everything below (`current_and_below`) |
/// | `"composite"` | `true` | the full document composite (`all`) |
/// | `"composite"` | `false` | INVALID — `error.schema.out_of_range` |
///
/// `target` is the destination layer, needed only for §45.2's
/// `ignoreAdjustments` (adjustments strictly ABOVE it are disabled, and
/// only ever with `source:"composite"`).
///
/// # ★ RECORDED DIVERGENCE — per-dab liveness (§45.2), not silently taken
///
/// §45.2 [NORMATIVE] ends its adjustment paragraph with: "The sampling
/// composite is snapshotted PER DAB from the current document state —
/// cloning is live, not frozen at stroke start", and the ES §40.5 clone
/// entry repeats it ("Sampling composites snapshot per dab from the
/// current document state").
///
/// **This resolution is per STROKE, not per dab.** The surface is
/// resolved once here, holding `&Buffer` borrows into the PRE-command
/// document, and every dab of the coalesced stroke reads that same
/// surface; the deposit accumulates in the retouch window and installs at
/// commit. So within ONE `clone.stroke` / `heal.stroke` command, dab *n*
/// does NOT see dab *n−1*'s pixels.
///
/// Observable exactly when the sampled scope contains the destination
/// surface and the source region overlaps the painted region — the
/// small-offset self-referential clone, where a per-dab-live sampler
/// smears and this one repeats. It is NOT a regression introduced by the
/// composite arms: the pre-existing `current` arm (`source:{layerId}`)
/// froze the same way, and the composite arms simply inherit that
/// posture rather than inventing a second one.
///
/// Left as-is DELIBERATELY and escalated rather than patched here:
/// per-dab liveness means re-resolving the composite inside the dab loop
/// against a document that the in-flight stroke is mutating, which is a
/// change to the stroke-family commit model (`RetouchIo` / `Window` /
/// the §9.6 rule-3 coalescing), not to this function — and it would move
/// committed stroke goldens. Scoped to whoever owns the stroke-commit
/// row. Recorded so no later reader mistakes silence for compliance.
fn resolve_sampling_surface<'a>(
    cx: &'a KindContext<'_>,
    source: &SourceParam,
    sample_all_layers: bool,
    target: LayerId,
    ignore_adjustments: bool,
) -> Result<SamplingSurface<'a>, EngineError> {
    use appname_core::{resolve_sampling_composite, SampleRequest, SampleScope};
    match source {
        SourceParam::Composite => {
            if !sample_all_layers {
                // §45.2 sample-scope tri-state: composite+false is the
                // invalid combination (clone.stroke note 1; §40.5
                // family adoption). NOT a refusal-for-want-of-an-engine —
                // the combination has no meaning, so it stays typed even
                // now that the composite serves.
                return Err(EngineError::range(
                    "source",
                    "source:\"composite\" with sampleAllLayers:false is the §45.2 \
                     tri-state's invalid combination (§40.5)",
                ));
            }
            let req = SampleRequest {
                scope: SampleScope::Document,
                // §45.2: the toggle is honored ONLY with source
                // "composite" — this is that arm.
                ignore_adjustments_above: ignore_adjustments.then_some(target),
            };
            Ok(SamplingSurface::Composite(CompositeBackdrop(
                resolve_sampling_composite(cx.document(), &req)?,
            )))
        }
        SourceParam::Layer(id) => {
            let lid = LayerId::from_uuid(*id);
            let node = cx.document().layer(lid).ok_or_else(|| {
                EngineError::bad_target(format!(
                    "params.source.layerId {id} does not resolve (§40.5: source layerId \
                     resolves or is \"composite\")"
                ))
            })?;
            if sample_all_layers {
                // §45.2 `current_and_below`. The armed layer need not be
                // raster here — the scope walk refuses any kind it cannot
                // composite, naming the row.
                let req = SampleRequest {
                    scope: SampleScope::AtOrBelow(lid),
                    // ignoreAdjustments is composite-only (§45.2).
                    ignore_adjustments_above: None,
                };
                return Ok(SamplingSurface::Composite(CompositeBackdrop(
                    resolve_sampling_composite(cx.document(), &req)?,
                )));
            }
            node.pixels().map(SamplingSurface::Layer).ok_or_else(|| {
                EngineError::bad_target(format!(
                    "params.source.layerId {id} is not a raster layer (§40.5)"
                ))
            })
        }
    }
}

fn param_bool(cx: &KindContext<'_>, name: &str, default: bool) -> bool {
    cx.params()
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

/// RAW-params presence (the stage-5 default fill makes `cx.params()`
/// carry defaulted keys; cross-fields on optional-with-default params
/// must read the envelope's own map).
fn raw_has(cx: &KindContext<'_>, name: &str) -> bool {
    cx.envelope()
        .params_map()
        .map(|m| m.contains_key(name))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// clone.stroke — §21.1/§45.2.
// ---------------------------------------------------------------------------

impl KindImpl for CloneStrokeKind {
    fn kind(&self) -> &'static str {
        "clone.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // TRANSPARENCY constrains clone (it writes alpha, §45.2).
        let c = stroke_commons(cx, true)?;
        let source = parse_source(cx)?.ok_or_else(|| EngineError::missing("source"))?;
        let offset = parse_offset(cx)?.ok_or_else(|| EngineError::missing("offset"))?;
        let sample_all = param_bool(cx, "sampleAllLayers", false);
        // ignoreAdjustments: legal only with source "composite" (§45.2);
        // otherwise ignored. The contract's "otherwise ignored with a
        // warn log" needs a minted MessageId for `ApplyReceipt.warnings`
        // (i18n registry) — NOT minted in this lane; the ignore itself is
        // served exactly as specified (`resolve_sampling_surface` takes
        // the flag only on the composite arm).
        let ignore_adjustments = param_bool(cx, "ignoreAdjustments", false);
        let rotation_deg = cx
            .params()
            .get("rotationDeg")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let scale = cx
            .params()
            .get("scale")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        let outcome = {
            let src =
                resolve_sampling_surface(cx, &source, sample_all, c.layer, ignore_adjustments)?;
            let backdrop = cx
                .document()
                .layer(c.layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let io = io_of(cx, &c, backdrop);
            let spec = CloneSpec {
                source: src.backdrop(),
                source_premultiplied: src.premultiplied(),
                offset,
                rotation_deg,
                scale,
            };
            render_clone_stroke(&io, &spec).map_err(map_retouch_error)?
        };
        // Clone copies source coverage — alpha can decrease (§45.2).
        install_commit(cx, c.layer, outcome, true)
    }
}

// ---------------------------------------------------------------------------
// heal.stroke — §21.2/§45.1.
// ---------------------------------------------------------------------------

impl KindImpl for HealStrokeKind {
    fn kind(&self) -> &'static str {
        "heal.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // TRANSPARENCY is a no-op constraint (alpha never modified,
        // §45.1) — pass false so the engine never clamps.
        let c = stroke_commons(cx, false)?;
        let mode = cx
            .params()
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("healing")
            .to_string();
        let source = parse_source(cx)?;
        let offset = parse_offset(cx)?;
        let pattern = cx.params().get("pattern").and_then(Value::as_str);

        // Pinned cross-field micro-order (module docs):
        // (1) spot FORBIDS source/offset (§40.5).
        if mode == "spot" && (source.is_some() || offset.is_some()) {
            return Err(EngineError::range(
                "source|offset",
                "mode:\"spot\" forbids source/offset (§40.5 cross-field)",
            ));
        }
        // (2) sampled and pattern guidance are mutually exclusive (§45.1).
        if source.is_some() && pattern.is_some() {
            return Err(EngineError::with_field(
                ErrorCode::ExclusiveParams,
                "source|pattern",
                "sampled (`source`) and `pattern` guidance are mutually exclusive (§45.1)",
            ));
        }
        // (3) healing REQUIRES (source AND offset) OR pattern (§40.5 +
        // the §45.8 pattern alternative).
        if mode == "healing" && pattern.is_none() {
            if source.is_none() {
                return Err(EngineError::missing("source"));
            }
            if offset.is_none() {
                return Err(EngineError::missing("offset"));
            }
        }
        // Pattern guidance: §45.1 live resolution. No pattern arena
        // exists in v1 — every id answers the contract's typed code
        // (deleted and never-registered are indistinguishable; recorded).
        if let Some(p) = pattern {
            return Err(EngineError::with_field(
                ErrorCode::RetouchPatternMissing,
                "pattern",
                format!(
                    "pattern {p} does not resolve — no §22.5 pattern registry exists in \
                     this build, so pattern guidance has no live patterns (§45.1)"
                ),
            ));
        }

        let outcome = if mode == "spot" {
            let spot_fill = cx
                .params()
                .get("spotFill")
                .and_then(Value::as_str)
                .unwrap_or("content_aware")
                .to_string();
            let backdrop = cx
                .document()
                .layer(c.layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let io = io_of(cx, &c, backdrop);
            if spot_fill == "proximity" {
                render_spot_heal_stroke(&io, SpotArm::Proximity).map_err(map_retouch_error)?
            } else {
                spot_content_aware(cx, &io)?
            }
        } else {
            let sample_all = param_bool(cx, "sampleAllLayers", false);
            // §45.2 ignoreAdjustments — composite arm only (see clone).
            let ignore_adjustments = param_bool(cx, "ignoreAdjustments", false);
            let src = resolve_sampling_surface(
                cx,
                source.as_ref().expect("checked by cross-field 3"),
                sample_all,
                c.layer,
                ignore_adjustments,
            )?;
            let backdrop = cx
                .document()
                .layer(c.layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let io = io_of(cx, &c, backdrop);
            let spec = HealSampledSpec {
                source: src.backdrop(),
                source_premultiplied: src.premultiplied(),
                offset: offset.expect("checked by cross-field 3"),
            };
            render_heal_stroke(&io, &spec).map_err(map_retouch_error)?
        };
        // heal.stroke never modifies alpha (§45.1) — bounds grow only.
        install_commit(cx, c.layer, outcome, false)
    }
}

/// The §45.1 spot content-aware arm: fill H = Ω per §45.3 with
/// `sampleArea = auto`, quality `default`, and the derived stroke seed
/// (blake3 of the envelope mergeKey, §45 intro). Blend back color-only
/// by `m·opacity` (§45.1 alpha rule), CAF-consumer plumbing.
fn spot_content_aware(
    cx: &KindContext<'_>,
    io: &RetouchIo<'_>,
) -> Result<RetouchCommit, EngineError> {
    use appname_algo::exemplar::{exemplar_fill, ExemplarError, ExemplarParams};
    use appname_algo::plane::Plane;
    use appname_core::{IRect, IVec2, TileCoord, TILE, TILE_I32};

    let (plan, bounds, m) = footprint_mask(io).map_err(map_stroke_error)?;
    if bounds.is_empty() || !m.iter().any(|&v| v > 0.5) {
        return Ok(RetouchCommit {
            patches: Vec::new(),
            bounds: IRect::EMPTY,
            plan,
        });
    }
    // §45 intro: retouch stochastic choices derive their seed from the
    // envelope mergeKey (blake3); absent mergeKey → the envelope id
    // string (pin, module docs). First 4 LE bytes → u32.
    let seed_src = cx
        .envelope()
        .merge_key
        .clone()
        .unwrap_or_else(|| cx.envelope().id.to_string());
    let hash = appname_core::ContentHash::of_bytes(seed_src.as_bytes());
    let seed = u32::from_le_bytes(hash.as_bytes()[0..4].try_into().expect("4 bytes"));

    // Canvas-sized planes (CAF headless-adapter precedent; windowing is
    // the recorded perf note).
    let canvas = io.clip;
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let mut planes = vec![Plane::new(w, h); 4]; // premultiplied RGB + A
    for y in 0..h {
        for x in 0..w {
            let v = io
                .backdrop
                .read_pixel(IVec2::new(canvas.min.x + x as i32, canvas.min.y + y as i32));
            let (rgbv, a) = if io.premultiplied {
                ([v[0], v[1], v[2]], v[3])
            } else {
                ([v[0] * v[3], v[1] * v[3], v[2] * v[3]], v[3])
            };
            for k in 0..3 {
                *planes[k].at_mut(x, y) = rgbv[k];
            }
            *planes[3].at_mut(x, y) = a;
        }
    }
    let bw = bounds.width() as usize;
    let mut hole = Plane::new(w, h);
    for y in bounds.min.y..bounds.max.y {
        for x in bounds.min.x..bounds.max.x {
            let mi = ((y - bounds.min.y) as usize) * bw + (x - bounds.min.x) as usize;
            if m[mi] > 0.5 {
                *hole.at_mut((x - canvas.min.x) as usize, (y - canvas.min.y) as usize) = 1.0;
            }
        }
    }
    let params = ExemplarParams {
        seed,
        ..ExemplarParams::default() // quality default, sampleArea auto
    };
    let alpha_ref = planes[3].clone();
    let result =
        exemplar_fill(&planes, Some(&alpha_ref), &hole, None, &params).map_err(|e| match e {
            ExemplarError::SampleRegionEmpty => EngineError::new(
                ErrorCode::RetouchNoSourceFound,
                "spot content-aware fill has neither samples nor boundary over Ω — no \
                 valid source (§45.1/§45.3); the stroke aborts atomically",
            ),
        })?;

    // Blend back color-only by m·opacity; alpha untouched; write-back
    // masks to m > 0 (§45.1).
    let mut patches = Vec::new();
    let mut out_bounds = IRect::EMPTY;
    for (tx, ty) in bounds.tiles(TILE_I32) {
        let coord = TileCoord::new(0, tx, ty);
        let trect = coord.pixel_rect();
        let mut pixels = vec![[0.0f32; 4]; TILE * TILE];
        io.backdrop.read_tile_dense(coord, &mut pixels);
        let mut any = false;
        let mut ti = 0;
        for y in trect.min.y..trect.max.y {
            for x in trect.min.x..trect.max.x {
                let p = IVec2::new(x, y);
                if bounds.contains(p) && canvas.contains(p) {
                    let mi = ((y - bounds.min.y) as usize) * bw + (x - bounds.min.x) as usize;
                    let mv = m[mi] * io.opacity;
                    if m[mi] > 0.0 && mv > 0.0 {
                        let (ux, uy) = ((x - canvas.min.x) as usize, (y - canvas.min.y) as usize);
                        let dst = pixels[ti];
                        // Straight destination + straight synthesized
                        // color (result planes are premultiplied).
                        let dst_straight = if io.premultiplied && dst[3] > 0.0 {
                            [dst[0] / dst[3], dst[1] / dst[3], dst[2] / dst[3]]
                        } else if io.premultiplied {
                            [0.0, 0.0, 0.0]
                        } else {
                            [dst[0], dst[1], dst[2]]
                        };
                        let sa = result.planes[3].at(ux, uy);
                        let synth = if sa > 0.0 {
                            [
                                result.planes[0].at(ux, uy) / sa,
                                result.planes[1].at(ux, uy) / sa,
                                result.planes[2].at(ux, uy) / sa,
                            ]
                        } else {
                            dst_straight
                        };
                        let mut out_straight = [0.0f32; 3];
                        for kc in 0..3 {
                            out_straight[kc] =
                                dst_straight[kc] + (synth[kc] - dst_straight[kc]) * mv;
                        }
                        pixels[ti] = if io.premultiplied {
                            [
                                out_straight[0] * dst[3],
                                out_straight[1] * dst[3],
                                out_straight[2] * dst[3],
                                dst[3],
                            ]
                        } else {
                            [out_straight[0], out_straight[1], out_straight[2], dst[3]]
                        };
                        any = true;
                        out_bounds = out_bounds.union(&IRect::new(x, y, x + 1, y + 1));
                    }
                }
                ti += 1;
            }
        }
        if any {
            patches.push(crate::brush::TilePatch { coord, pixels });
        }
    }
    Ok(RetouchCommit {
        patches,
        bounds: out_bounds,
        plan,
    })
}

// ---------------------------------------------------------------------------
// dodge_burn.stroke — §21.5.
// ---------------------------------------------------------------------------

impl KindImpl for DodgeBurnStrokeKind {
    fn kind(&self) -> &'static str {
        "dodge_burn.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Tonal edits never touch alpha — TRANSPARENCY is a no-op
        // constraint (§40.5).
        let c = stroke_commons(cx, false)?;
        let mode = match cx.params().get("mode").and_then(Value::as_str) {
            Some("dodge") => ToneMode::Dodge,
            Some("burn") => ToneMode::Burn,
            Some("saturate") => ToneMode::Saturate,
            Some("desaturate") => ToneMode::Desaturate,
            _ => return Err(EngineError::missing("mode")), // stage-5 owns the enum
        };
        let range = match cx.params().get("range").and_then(Value::as_str) {
            Some("shadows") => ToneRange::Shadows,
            Some("highlights") => ToneRange::Highlights,
            _ => ToneRange::Midtones,
        };
        let spec = ToneSpec {
            mode,
            range,
            exposure: param_f32(cx, "exposure", 0.5),
            // Carried per the contract; no §21.5 math term (recorded
            // contract gap — behavior identical either way).
            protect_tones: param_bool(cx, "protectTones", true),
        };
        let outcome = {
            let backdrop = cx
                .document()
                .layer(c.layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let io = io_of(cx, &c, backdrop);
            render_dodge_burn_stroke(&io, &spec).map_err(map_retouch_error)?
        };
        install_commit(cx, c.layer, outcome, false)
    }
}

// ---------------------------------------------------------------------------
// smudge.stroke — §21.5/§15.7.
// ---------------------------------------------------------------------------

impl KindImpl for SmudgeStrokeKind {
    fn kind(&self) -> &'static str {
        "smudge.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // TRANSPARENCY constrains (coverage clamps to existing alpha).
        let c = stroke_commons(cx, true)?;
        if param_bool(cx, "fingerPainting", false) {
            // §21.5/§26.1: the stroke starts loaded with the FG color,
            // and the EFFECTIVE color is captured in params at record
            // time — but the schema carries no color field to capture it
            // in. Contract gap: typed refusal + escalation (lane log).
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "fingerPainting:true needs the §26.1 record-time effective color and the \
                 schema carries no color field — contract gap escalated (lane log); \
                 fingerPainting:false is live",
            ));
        }
        if param_bool(cx, "sampleAllLayers", false) {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "sampleAllLayers:true picks up from the composite below (§21.5 \
                 non-destructive mode); this row is not wired to the sampling-composite \
                 door yet. NOTE: the door itself now EXISTS \
                 (appname_core::resolve_sampling_composite, the §45.2/§13.3 sample-scope \
                 slice) and \"the composite below\" is its SampleScope::AtOrBelow arm — \
                 the old \"no composite is reachable headless\" reason no longer holds \
                 and is not restated here",
            ));
        }
        let spec = SmudgeSpec {
            strength: param_f32(cx, "strength", 0.5),
        };
        let outcome = {
            let backdrop = cx
                .document()
                .layer(c.layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let io = io_of(cx, &c, backdrop);
            render_smudge_stroke(&io, &spec).map_err(map_retouch_error)?
        };
        // Smudge drags coverage — alpha can decrease.
        install_commit(cx, c.layer, outcome, true)
    }
}

// ---------------------------------------------------------------------------
// blur_sharpen.stroke — §21.5.
// ---------------------------------------------------------------------------

impl KindImpl for BlurSharpenStrokeKind {
    fn kind(&self) -> &'static str {
        "blur_sharpen.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Alpha untouched — TRANSPARENCY is a no-op constraint (§40.5).
        let c = stroke_commons(cx, false)?;
        let mode = match cx.params().get("mode").and_then(Value::as_str) {
            Some("blur") => FocusMode::Blur,
            Some("sharpen") => FocusMode::Sharpen,
            _ => return Err(EngineError::missing("mode")),
        };
        // §40.5 cross-field on the RAW params (stage-5 fills the default).
        if mode == FocusMode::Blur && raw_has(cx, "protectDetail") {
            return Err(EngineError::range(
                "protectDetail",
                "protectDetail is sharpen-only (§10.3); present with mode:\"blur\" is a \
                 §40.5 cross-field violation",
            ));
        }
        let spec = FocusSpec {
            mode,
            strength: param_f32(cx, "strength", 0.5),
            protect_detail: param_bool(cx, "protectDetail", true),
        };
        let outcome = {
            let backdrop = cx
                .document()
                .layer(c.layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let io = io_of(cx, &c, backdrop);
            render_blur_sharpen_stroke(&io, &spec).map_err(map_retouch_error)?
        };
        install_commit(cx, c.layer, outcome, false)
    }
}

/// Registration helper mirror (assembly() in `kind_impls` is the one
/// production seam; this exists for focused in-crate tests).
pub fn retouch_kinds() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(CloneStrokeKind),
        Arc::new(HealStrokeKind),
        Arc::new(DodgeBurnStrokeKind),
        Arc::new(SmudgeStrokeKind),
        Arc::new(BlurSharpenStrokeKind),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{encode_samples, InputSample};
    use crate::kind_impls::{assembly, preset_hard_round_16_id};
    use appname_core::Engine;
    use serde_json::json;

    fn sample_line() -> Vec<u8> {
        let pts = [(20.0, 20.0, 0.0), (40.0, 24.0, 0.04), (60.0, 28.0, 0.08)];
        let samples: Vec<InputSample> = pts
            .iter()
            .map(|(x, y, t)| InputSample {
                x: *x,
                y: *y,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: *t,
            })
            .collect();
        encode_samples(&samples).unwrap()
    }

    fn doc_new(engine: &mut Engine, n: u32) -> appname_core::LayerId {
        let text = json!({
            "schema": "appname.command.v1",
            "id": format!("e0000000-0000-4000-8000-0000000{n:05x}"),
            "kind": "doc.new",
            "params": {"width": 128, "height": 128, "background": "white"},
        })
        .to_string();
        engine.apply_json(&text).unwrap();
        engine.document().unwrap().z_order_walk()[0]
    }

    fn env(
        n: u32,
        kind: &str,
        layer: appname_core::LayerId,
        samples: Uuid,
        extra: serde_json::Value,
    ) -> String {
        let mut params = json!({"brushPreset": preset_hard_round_16_id().to_string()});
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                params[k] = v.clone();
            }
        }
        json!({
            "schema": "appname.command.v1",
            "id": format!("e0000000-0000-4000-8000-0002{n:08x}"),
            "kind": kind,
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": params,
            "samples": samples.to_string(),
        })
        .to_string()
    }

    fn ready() -> (Engine, appname_core::LayerId, Uuid) {
        let mut e = assembly().build();
        let layer = doc_new(&mut e, 1);
        let asset = e.add_samples_asset(sample_line());
        (e, layer, asset)
    }

    fn err_code(e: &mut Engine, envelope: &str) -> String {
        let err = e.apply_json(envelope).unwrap_err();
        serde_json::to_value(&err).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_string()
    }

    // ---- clone.stroke ----------------------------------------------------

    #[test]
    fn clone_stroke_applies_undoes_and_replays() {
        let (mut e, layer, asset) = ready();
        // Non-uniform content first (a red brush stroke), so the clone
        // deposit is observable against the white ground.
        e.apply_json(&env(
            0,
            "brush.stroke",
            layer,
            asset,
            json!({"color": {"model": "rgb", "comps": [1, 0, 0, 0]}, "sizePx": 8.0}),
        ))
        .unwrap();
        let src_layer = layer; // self-clone with offset
        let envl = env(
            1,
            "clone.stroke",
            layer,
            asset,
            json!({
                "source": {"layerId": src_layer.as_uuid().to_string()},
                "offset": [0.0, -14.0],
                "seed": 3
            }),
        );
        let before = e.state_hash();
        e.apply_json(&envl).unwrap();
        let after = e.state_hash();
        assert_ne!(before, after, "clone deposited");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "COW-delta undo byte identity");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        // Replay determinism through the same assembly + assets.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "replay byte-identical");
    }

    #[test]
    fn clone_stroke_contract_rejects() {
        let (mut e, layer, asset) = ready();
        // missing source → stage-3 missing_field (contract invalid 1).
        let c = err_code(
            &mut e,
            &env(2, "clone.stroke", layer, asset, json!({"offset": [-10, 0]})),
        );
        assert_eq!(c, "error.schema.missing_field");
        // scale out of range → stage-3 (contract invalid 2).
        let c = err_code(
            &mut e,
            &env(
                3,
                "clone.stroke",
                layer,
                asset,
                json!({"source": "composite", "offset": [-10, 0], "scale": 20}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // unresolvable source layer → bad_target (contract invalid 3).
        let c = err_code(
            &mut e,
            &env(
                4,
                "clone.stroke",
                layer,
                asset,
                json!({
                    "source": {"layerId": "00000000-0003-4000-8000-0000000000ff"},
                    "offset": [-10, 0]
                }),
            ),
        );
        assert_eq!(c, "error.command.bad_target");
        // §45.2 tri-state: composite + sampleAllLayers:false (explicit)
        // is the invalid combination (contract note 1).
        let c = err_code(
            &mut e,
            &env(
                5,
                "clone.stroke",
                layer,
                asset,
                json!({"source": "composite", "offset": [0, 0], "sampleAllLayers": false}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // composite + true is the §45.2 `all` arm and now SERVES (it was
        // the typed composite door until the sample-scope arm landed);
        // the tri-state's INVALID combination above stays typed.
        e.apply_json(&env(
            6,
            "clone.stroke",
            layer,
            asset,
            json!({"source": "composite", "offset": [0, 0], "sampleAllLayers": true}),
        ))
        .expect("§45.2 `all` serves");
        // unknown param → stage-3 strictness.
        let c = err_code(
            &mut e,
            &env(
                7,
                "clone.stroke",
                layer,
                asset,
                json!({"source": "composite", "offset": [0, 0], "bogus": 1}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
    }

    // ---- heal.stroke -----------------------------------------------------

    #[test]
    fn heal_stroke_cross_field_micro_order() {
        let (mut e, layer, asset) = ready();
        // healing (defaulted) missing source+offset → missing_field
        // (contract invalid 1).
        let c = err_code(&mut e, &env(10, "heal.stroke", layer, asset, json!({})));
        assert_eq!(c, "error.schema.missing_field");
        // explicit healing, source without offset → missing_field.
        let c = err_code(
            &mut e,
            &env(
                11,
                "heal.stroke",
                layer,
                asset,
                json!({"mode": "healing", "source": "composite"}),
            ),
        );
        assert_eq!(c, "error.schema.missing_field");
        // spot carrying source/offset → out_of_range (contract invalid 2).
        let c = err_code(
            &mut e,
            &env(
                12,
                "heal.stroke",
                layer,
                asset,
                json!({"mode": "spot", "source": "composite", "offset": [10, 10]}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // sampled + pattern together → exclusive_params (contract
        // invalid 3).
        let c = err_code(
            &mut e,
            &env(
                13,
                "heal.stroke",
                layer,
                asset,
                json!({
                    "mode": "healing",
                    "source": "composite",
                    "offset": [10, 10],
                    "pattern": "00000000-0004-4000-8000-000000000009"
                }),
            ),
        );
        assert_eq!(c, "error.schema.exclusive_params");
        // pattern referencing any id → pattern_missing (contract
        // invalid 4; no pattern arena in v1, posture recorded).
        let c = err_code(
            &mut e,
            &env(
                14,
                "heal.stroke",
                layer,
                asset,
                json!({
                    "mode": "healing",
                    "pattern": "00000000-0004-4000-8000-0000000000ff"
                }),
            ),
        );
        assert_eq!(c, "error.retouch.pattern_missing");
    }

    #[test]
    fn heal_stroke_sampled_heals_and_preserves_alpha() {
        let (mut e, layer, asset) = ready();
        // Paint a dark blemish with the brush first, then heal it from a
        // clean offset region.
        e.apply_json(&env(
            20,
            "brush.stroke",
            layer,
            asset,
            json!({"color": {"model": "rgb", "comps": [0, 0, 0, 0]}, "sizePx": 8.0}),
        ))
        .unwrap();
        let before = e.state_hash();
        e.apply_json(&env(
            21,
            "heal.stroke",
            layer,
            asset,
            json!({
                "mode": "healing",
                "source": {"layerId": layer.as_uuid().to_string()},
                "offset": [0.0, 40.0],
                "sizePx": 16.0
            }),
        ))
        .unwrap();
        let after = e.state_hash();
        assert_ne!(before, after, "heal changed pixels");
        // Undo restores the pre-heal state exactly (delta inverse).
        e.undo().unwrap();
        assert_eq!(before, e.state_hash(), "undo restores pre-heal state");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        // Replay determinism through the same assembly + assets.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "heal replay byte-identical");
    }

    #[test]
    fn spot_heal_proximity_and_content_aware_run() {
        let (mut e, layer, asset) = ready();
        e.apply_json(&env(
            30,
            "heal.stroke",
            layer,
            asset,
            json!({"mode": "spot", "spotFill": "proximity", "sizePx": 10.0}),
        ))
        .unwrap();
        e.apply_json(&env(
            31,
            "heal.stroke",
            layer,
            asset,
            json!({"mode": "spot", "sizePx": 10.0}), // spotFill defaults content_aware
        ))
        .unwrap();
        // Both accepted; replay stable (incl. the blake3(mergeKey)-less
        // envelope-id seed derivation pin).
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "spot replay byte-identical");
    }

    // ---- dodge_burn.stroke ----------------------------------------------

    #[test]
    fn dodge_burn_contract_rejects_and_applies() {
        let (mut e, layer, asset) = ready();
        // missing mode → stage-3 missing_field (contract invalid 1).
        let c = err_code(
            &mut e,
            &env(40, "dodge_burn.stroke", layer, asset, json!({})),
        );
        assert_eq!(c, "error.schema.missing_field");
        // exposure out of range (contract invalid 2).
        let c = err_code(
            &mut e,
            &env(
                41,
                "dodge_burn.stroke",
                layer,
                asset,
                json!({"mode": "burn", "exposure": 1.5}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // enum violation → stage-3.
        let c = err_code(
            &mut e,
            &env(
                42,
                "dodge_burn.stroke",
                layer,
                asset,
                json!({"mode": "brighten"}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // live apply (all four modes) + undo/redo.
        for (n, mode) in [
            (43, "dodge"),
            (44, "burn"),
            (45, "saturate"),
            (46, "desaturate"),
        ] {
            e.apply_json(&env(
                n,
                "dodge_burn.stroke",
                layer,
                asset,
                json!({"mode": mode, "range": "midtones", "exposure": 1.0}),
            ))
            .unwrap();
        }
        e.undo().unwrap();
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "tone replay byte-identical");
    }

    // ---- smudge.stroke ---------------------------------------------------

    #[test]
    fn smudge_contract_rejects_and_applies() {
        let (mut e, layer, asset) = ready();
        // strength out of range (contract invalid 1).
        let c = err_code(
            &mut e,
            &env(50, "smudge.stroke", layer, asset, json!({"strength": -0.1})),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // missing brushPreset (contract invalid 2).
        let text = json!({
            "schema": "appname.command.v1",
            "id": "e0000000-0000-4000-8000-000200000033",
            "kind": "smudge.stroke",
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": {"strength": 0.6},
            "samples": asset.to_string(),
        })
        .to_string();
        let c = err_code(&mut e, &text);
        assert_eq!(c, "error.schema.missing_field");
        // samples asset missing (contract invalid 3).
        let ghost = Uuid::new_v4();
        let c = err_code(&mut e, &env(52, "smudge.stroke", layer, ghost, json!({})));
        assert_eq!(c, "error.command.bad_target");
        // fingerPainting → typed contract-gap refusal.
        let c = err_code(
            &mut e,
            &env(
                53,
                "smudge.stroke",
                layer,
                asset,
                json!({"fingerPainting": true}),
            ),
        );
        assert_eq!(c, "error.command.not_implemented");
        // live apply.
        e.apply_json(&env(
            54,
            "smudge.stroke",
            layer,
            asset,
            json!({"strength": 0.8}),
        ))
        .unwrap();
        e.undo().unwrap();
        e.redo().unwrap();
    }

    // ---- blur_sharpen.stroke ---------------------------------------------

    #[test]
    fn blur_sharpen_contract_rejects_and_applies() {
        let (mut e, layer, asset) = ready();
        // missing mode (contract invalid 1).
        let c = err_code(
            &mut e,
            &env(60, "blur_sharpen.stroke", layer, asset, json!({})),
        );
        assert_eq!(c, "error.schema.missing_field");
        // protectDetail with blur (contract invalid 2) — RAW presence.
        let c = err_code(
            &mut e,
            &env(
                61,
                "blur_sharpen.stroke",
                layer,
                asset,
                json!({"mode": "blur", "protectDetail": true}),
            ),
        );
        assert_eq!(c, "error.schema.out_of_range");
        // …but blur WITHOUT the raw key is live (the stage-5 default fill
        // must not trip the cross-field).
        e.apply_json(&env(
            62,
            "blur_sharpen.stroke",
            layer,
            asset,
            json!({"mode": "blur", "strength": 1.0}),
        ))
        .unwrap();
        // unresolvable brush preset (contract invalid 3).
        let c = err_code(
            &mut e,
            &json!({
                "schema": "appname.command.v1",
                "id": "e0000000-0000-4000-8000-000200000063",
                "kind": "blur_sharpen.stroke",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {"brushPreset": "00000000-0007-4000-8000-0000000000ff", "mode": "blur"},
                "samples": asset.to_string(),
            })
            .to_string(),
        );
        assert_eq!(c, "error.command.bad_target");
        // sharpen live.
        e.apply_json(&env(
            64,
            "blur_sharpen.stroke",
            layer,
            asset,
            json!({"mode": "sharpen", "strength": 1.0, "protectDetail": true}),
        ))
        .unwrap();
        e.undo().unwrap();
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "focus replay byte-identical");
    }

    // ---- family gates ----------------------------------------------------

    #[test]
    fn locks_and_targets_gate_all_five() {
        // Full composition: core's own kinds (layer.set_props) + the
        // tools assembly (the shipped five-crate shape).
        let mut e = appname_core::engine::kinds::register(assembly()).build();
        let layer = doc_new(&mut e, 1);
        let asset = e.add_samples_asset(sample_line());
        // PIXELS lock blocks every retouch kind.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "e0000000-0000-4000-8000-000200000070",
                "kind": "layer.set_props",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {"locks": {"pixels": true}},
            })
            .to_string(),
        )
        .unwrap();
        for (n, kind, extra) in [
            (
                71u32,
                "clone.stroke",
                json!({"source": "composite", "offset": [0, 0]}),
            ),
            (72, "heal.stroke", json!({"mode": "spot"})),
            (73, "dodge_burn.stroke", json!({"mode": "dodge"})),
            (74, "smudge.stroke", json!({})),
            (75, "blur_sharpen.stroke", json!({"mode": "blur"})),
        ] {
            let c = err_code(&mut e, &env(n, kind, layer, asset, extra));
            assert_eq!(c, "error.command.locked", "{kind}: PIXELS lock");
        }
        // Mask/channel stroke surfaces: the standing family posture.
        let text = json!({
            "schema": "appname.command.v1",
            "id": "e0000000-0000-4000-8000-000200000076",
            "kind": "dodge_burn.stroke",
            "target": {"channelId": "00000000-0005-4000-8000-000000000004"},
            "params": {"brushPreset": preset_hard_round_16_id().to_string(), "mode": "dodge"},
            "samples": asset.to_string(),
        })
        .to_string();
        let mut e2 = assembly().build();
        doc_new(&mut e2, 9);
        let c = err_code(&mut e2, &text);
        assert_eq!(c, "error.command.bad_target");
    }

    #[test]
    fn selection_gates_deposits() {
        let mut e = appname_core::engine::kinds::register(assembly()).build();
        let layer = doc_new(&mut e, 1);
        let asset = e.add_samples_asset(sample_line());
        // A far-away selection: strokes intersect nothing → §40.1 empty
        // DeltaSet accept.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "e0000000-0000-4000-8000-000200000080",
                "kind": "selection.set",
                "params": {"source": "rect", "rect": {"x": 100, "y": 100, "w": 20, "h": 20}, "op": "replace"},
            })
            .to_string(),
        )
        .unwrap();
        let before = e.state_hash();
        e.apply_json(&env(
            81,
            "dodge_burn.stroke",
            layer,
            asset,
            json!({"mode": "dodge", "exposure": 1.0}),
        ))
        .unwrap();
        assert_eq!(
            before,
            e.state_hash(),
            "empty stroke∩selection = §40.1 empty DeltaSet (state unchanged)"
        );
    }

    // ---- §45.2 SAMPLE SCOPE (the sample-all-layers arm) ------------------
    //
    // Fixture discipline: every probe layer is HARD-EDGED and fully opaque
    // over exact u8 storage values (a binary rect selection + a solid
    // fill), never an antialiased brush stroke. That is deliberate — it
    // makes "sample the composite" and "flatten, then sample" comparable
    // on RAW BITS instead of within a quantization tolerance, because the
    // flatten path writes its result back through u8 storage and an
    // antialiased edge would quantize there. Where a test needs the soft
    // case it says so.

    /// The full tools+core composition (`layer.add`, `layer.set_props`,
    /// `fill`, `selection.*` alongside the retouch kinds).
    fn full() -> Engine {
        appname_core::engine::kinds::register(crate::kind_impls::with_tools_fill(assembly()))
            .build()
    }

    fn apply(e: &mut Engine, n: u32, kind: &str, target: Option<&str>, params: serde_json::Value) {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("e0000000-0000-4000-8000-0003{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(t) = target {
            v["target"] = json!({"layerId": t});
        }
        e.apply_json(&v.to_string())
            .unwrap_or_else(|err| panic!("{kind} (#{n}) refused: {err:?}"));
    }

    /// Add a raster layer at the top of the stack and paint a hard-edged,
    /// fully opaque rect of `rgb` into it.
    fn add_block(
        e: &mut Engine,
        n: u32,
        id: &str,
        rect: (i32, i32, i32, i32),
        rgb: [f64; 3],
    ) -> appname_core::LayerId {
        apply(
            e,
            n,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": id}),
        );
        apply(
            e,
            n + 1,
            "selection.set",
            None,
            json!({
                "source": "rect",
                "rect": {"x": rect.0, "y": rect.1, "w": rect.2, "h": rect.3},
                "op": "replace",
                "antialias": false,
            }),
        );
        apply(
            e,
            n + 2,
            "fill",
            Some(id),
            json!({
                "content": "color",
                "color": {"model": "rgb", "comps": [rgb[0], rgb[1], rgb[2], 0.0]},
                "antialias": false,
            }),
        );
        apply(e, n + 3, "selection.none", None, json!({}));
        appname_core::LayerId::from_uuid(Uuid::parse_str(id).unwrap())
    }

    const MID_ID: &str = "0e450200-0000-4000-8000-000000000001";
    const HIGH_ID: &str = "0e450200-0000-4000-8000-000000000002";
    const TOP_ID: &str = "0e450200-0000-4000-8000-000000000003";

    /// L0 = doc.new's opaque white ground · L1 = an opaque GREEN block over
    /// the left half · L2 = an opaque BLUE block over the right half ·
    /// L3 = an empty raster layer on top, the retouch TARGET.
    ///
    /// The three scopes therefore see three DIFFERENT surfaces — which is
    /// what makes the scope assertions falsifiable rather than tautological.
    fn layered() -> (Engine, [appname_core::LayerId; 4], Uuid) {
        layered_in(full())
    }

    fn layered_in(mut e: Engine) -> (Engine, [appname_core::LayerId; 4], Uuid) {
        let bg = doc_new(&mut e, 1);
        let asset = e.add_samples_asset(sample_line());
        let mid = add_block(&mut e, 100, MID_ID, (0, 0, 64, 128), [0.0, 1.0, 0.0]);
        let high = add_block(&mut e, 110, HIGH_ID, (64, 0, 64, 128), [0.0, 0.0, 1.0]);
        apply(
            &mut e,
            120,
            "layer.add",
            None,
            json!({"kind": "raster", "newLayerId": TOP_ID}),
        );
        let top = appname_core::LayerId::from_uuid(Uuid::parse_str(TOP_ID).unwrap());
        assert_eq!(
            e.document().unwrap().z_order_walk(),
            vec![bg, mid, high, top],
            "fixture z-order is bottom-first bg → mid → high → top"
        );
        (e, [bg, mid, high, top], asset)
    }

    /// Every canvas pixel of a layer, in storage space.
    fn plane(e: &Engine, l: appname_core::LayerId) -> Vec<[f32; 4]> {
        let doc = e.document().unwrap();
        let buf = doc.layer(l).unwrap().pixels().unwrap();
        let r = doc.geometry().canvas_rect();
        let mut out = Vec::with_capacity((r.width() * r.height()) as usize);
        for y in r.min.y..r.max.y {
            for x in r.min.x..r.max.x {
                out.push(buf.read_pixel_f32(appname_core::IVec2::new(x, y)));
            }
        }
        out
    }

    fn bits(p: &[[f32; 4]]) -> Vec<[u32; 4]> {
        p.iter()
            .map(|v| {
                [
                    v[0].to_bits(),
                    v[1].to_bits(),
                    v[2].to_bits(),
                    v[3].to_bits(),
                ]
            })
            .collect()
    }

    /// A clone whose source crosses the fixture's green/blue seam, so the
    /// deposit differs pixel-by-pixel between scopes.
    fn clone_env(n: u32, target: appname_core::LayerId, asset: Uuid, source: Value) -> String {
        env(
            n,
            "clone.stroke",
            target,
            asset,
            json!({
                "source": source,
                "offset": [40.0, 0.0],
                "sampleAllLayers": true,
                "sizePx": 24.0,
                "seed": 7,
            }),
        )
    }

    /// **Invariant 1** (core `engine::sampling` module doc): on a ONE-layer
    /// document the composite IS that layer, so the `sampleAllLayers`
    /// tri-state must not fork. This is the invariant that rules out
    /// sampling through the LINEAR reference export — that arm round-trips
    /// u8 storage through `srgb_decode`/`srgb_encode` and would answer
    /// different pixels for the same user-visible request.
    #[test]
    fn sample_all_layers_does_not_fork_on_a_one_layer_document() {
        let run = |source: Value, all: bool| -> (Vec<[u32; 4]>, appname_core::ContentHash) {
            let mut e = full();
            let layer = doc_new(&mut e, 1);
            let asset = e.add_samples_asset(sample_line());
            // Soft, antialiased content on purpose: the no-fork claim must
            // hold on FRACTIONAL alpha too, not only on hard blocks.
            e.apply_json(&env(
                200,
                "brush.stroke",
                layer,
                asset,
                json!({"color": {"model": "rgb", "comps": [1, 0, 0, 0]}, "sizePx": 12.0}),
            ))
            .unwrap();
            let pre = e.state_hash();
            e.apply_json(&env(
                201,
                "clone.stroke",
                layer,
                asset,
                json!({
                    "source": source,
                    "offset": [0.0, -18.0],
                    "sampleAllLayers": all,
                    "sizePx": 24.0,
                    "seed": 7,
                }),
            ))
            .unwrap();
            assert_ne!(pre, e.state_hash(), "the clone must actually deposit");
            (bits(&plane(&e, layer)), e.state_hash())
        };
        let (single, h_single) = run(json!({"layerId": doc_new_layer_id()}), false);
        let (composite, h_composite) = run(json!("composite"), true);
        assert_eq!(
            single, composite,
            "§45.2 `current` and `all` must answer BIT-identical pixels when the \
             document has exactly one layer"
        );
        assert_eq!(h_single, h_composite, "…and identical state hashes");
    }

    /// The id `doc_new` hands back is deterministic (UUIDv5 over the
    /// creating envelope id, §9.6), so the one-layer probe can name it.
    fn doc_new_layer_id() -> String {
        let mut e = full();
        doc_new(&mut e, 1).as_uuid().to_string()
    }

    /// **§45.2 `current_and_below`**: `source:{layerId}` + `sampleAllLayers:
    /// true` samples the armed layer AND EVERYTHING BELOW IT — never a
    /// layer above. Pinned two ways so neither half can pass vacuously:
    /// it must DIFFER from the `all` scope (which sees the layer above),
    /// and it must EQUAL the `all` scope computed with that layer hidden.
    #[test]
    fn current_and_below_stops_at_the_armed_layer() {
        // (a) current_and_below(mid) — sees bg + mid, never `high`.
        let (mut a, [_, mid, _, top], asset) = layered();
        a.apply_json(&clone_env(210, top, asset, json!({"layerId": MID_ID})))
            .unwrap();
        let below = bits(&plane(&a, top));

        // (b) all — sees bg + mid + high.
        let (mut b, [_, _, _, top_b], asset_b) = layered();
        b.apply_json(&clone_env(210, top_b, asset_b, json!("composite")))
            .unwrap();
        let all = bits(&plane(&b, top_b));
        assert_ne!(
            below, all,
            "the fixture must actually distinguish the scopes — the clone source \
             crosses the seam where `high` lives, so `all` and `current_and_below` \
             cannot legitimately agree"
        );

        // (c) all, with `high` HIDDEN — the same node set as (a), so the
        // pixels must match BIT-for-bit.
        let (mut c, [_, _, high_c, top_c], asset_c) = layered();
        apply(
            &mut c,
            211,
            "layer.set_props",
            Some(&high_c.as_uuid().to_string()),
            json!({"visible": false}),
        );
        c.apply_json(&clone_env(210, top_c, asset_c, json!("composite")))
            .unwrap();
        assert_eq!(
            below,
            bits(&plane(&c, top_c)),
            "current_and_below(mid) must equal the full composite with everything \
             above `mid` hidden — that IS its definition (§45.2)"
        );
        assert_eq!(
            mid,
            c.document().unwrap().z_order_walk()[1],
            "fixture sanity: `mid` is still the second node bottom-up"
        );
    }

    /// **A GROUP armed as the `current_and_below` scope layer includes its
    /// OWN CONTENT.** `z_order_walk` is depth-first and lists a group
    /// immediately BEFORE its subtree, so a naive `..=position(id)` cut
    /// would stop at the group node — which carries no pixels — and the
    /// sample would silently lose everything inside it. Pinned three ways:
    /// arming the group must equal the full composite (nothing above it in
    /// this fixture), must DIFFER from arming the layer below the group,
    /// and the group's content must actually be reachable at the sampled
    /// offset (else the first leg passes vacuously).
    #[test]
    fn current_and_below_of_a_group_includes_its_subtree() {
        const GROUP_ID: &str = "0e450200-0000-4000-8000-00000000000c";
        // bg · mid(green, left half) · GROUP{ high(blue, right half) } · top
        let build = || {
            let mut e = full();
            let bg = doc_new(&mut e, 1);
            let asset = e.add_samples_asset(sample_line());
            let mid = add_block(&mut e, 100, MID_ID, (0, 0, 64, 128), [0.0, 1.0, 0.0]);
            apply(
                &mut e,
                130,
                "layer.add",
                None,
                json!({"kind": "group", "newLayerId": GROUP_ID}),
            );
            add_block_in(
                &mut e,
                110,
                HIGH_ID,
                Some(GROUP_ID),
                (64, 0, 64, 128),
                [0.0, 0.0, 1.0],
            );
            apply(
                &mut e,
                120,
                "layer.add",
                None,
                json!({"kind": "raster", "newLayerId": TOP_ID}),
            );
            let g = appname_core::LayerId::from_uuid(Uuid::parse_str(GROUP_ID).unwrap());
            let high = appname_core::LayerId::from_uuid(Uuid::parse_str(HIGH_ID).unwrap());
            let top = appname_core::LayerId::from_uuid(Uuid::parse_str(TOP_ID).unwrap());
            assert_eq!(
                e.document().unwrap().z_order_walk(),
                vec![bg, mid, g, high, top],
                "the group must be listed IMMEDIATELY BEFORE its child — the whole \
                 point of this test"
            );
            (e, mid, top, asset)
        };

        // (a) arming the GROUP == the full document composite here.
        let (mut a, _, top_a, asset_a) = build();
        a.apply_json(&clone_env(
            290,
            top_a,
            asset_a,
            json!({"layerId": GROUP_ID}),
        ))
        .expect("arming a pass-through group must serve");
        let armed_group = bits(&plane(&a, top_a));

        let (mut b, _, top_b, asset_b) = build();
        b.apply_json(&clone_env(290, top_b, asset_b, json!("composite")))
            .unwrap();
        assert_eq!(
            armed_group,
            bits(&plane(&b, top_b)),
            "current_and_below(group) must include the group's OWN subtree — if the \
             cut stopped at the group node the blue child would be missing"
        );

        // (b) …and it must DIFFER from arming the layer below the group,
        //     which is what proves the subtree is what makes the
        //     difference rather than the cut being a no-op.
        let (mut c, mid_c, top_c, asset_c) = build();
        c.apply_json(&clone_env(
            290,
            top_c,
            asset_c,
            json!({"layerId": mid_c.as_uuid().to_string()}),
        ))
        .unwrap();
        assert_ne!(
            armed_group,
            bits(&plane(&c, top_c)),
            "the fixture must actually distinguish the two cuts"
        );
    }

    /// `add_block` with an explicit parent (used by the group fixture).
    fn add_block_in(
        e: &mut Engine,
        n: u32,
        id: &str,
        parent: Option<&str>,
        rect: (i32, i32, i32, i32),
        rgb: [f64; 3],
    ) {
        let mut add = json!({"kind": "raster", "newLayerId": id});
        if let Some(p) = parent {
            add["parent"] = json!(p);
        }
        apply(e, n, "layer.add", None, add);
        apply(
            e,
            n + 1,
            "selection.set",
            None,
            json!({
                "source": "rect",
                "rect": {"x": rect.0, "y": rect.1, "w": rect.2, "h": rect.3},
                "op": "replace",
                "antialias": false,
            }),
        );
        apply(
            e,
            n + 2,
            "fill",
            Some(id),
            json!({
                "content": "color",
                "color": {"model": "rgb", "comps": [rgb[0], rgb[1], rgb[2], 0.0]},
                "antialias": false,
            }),
        );
        apply(e, n + 3, "selection.none", None, json!({}));
    }

    /// D-48 determinism spine: the composite arms journal, undo, redo and
    /// REPLAY byte-identically. Sampling from a multi-layer composite is
    /// exactly the class of change that can break replay (the sampled
    /// surface is derived from other layers' live state), so both retouch
    /// kinds are pinned here explicitly.
    #[test]
    fn composite_sampling_replays_byte_identically() {
        // `target_pick` indexes `layered()`'s [bg, mid, high, top].
        // clone WRITES alpha, so the empty `top` layer is a fine target.
        // heal NEVER writes alpha (§45.1) — "healing fully transparent
        // pixels is a defined no-op on visible output" — so its target
        // must be an OPAQUE layer or the command legitimately deposits
        // nothing; `mid` is the opaque green block the stroke runs across.
        for (n, kind, target_pick, extra) in [
            (220u32, "clone.stroke", 3usize, json!({})),
            (230, "heal.stroke", 1usize, json!({"mode": "healing"})),
        ] {
            let (mut e, ids, asset) = layered();
            let top = ids[target_pick];
            let before = e.state_hash();
            let mut params = json!({
                "source": "composite",
                "offset": [40.0, 0.0],
                "sampleAllLayers": true,
                "sizePx": 24.0,
                "seed": 7,
            });
            for (k, v) in extra.as_object().unwrap() {
                params[k] = v.clone();
            }
            e.apply_json(&env(n, kind, top, asset, params)).unwrap();
            let after = e.state_hash();
            assert_ne!(before, after, "{kind}: the composite clone must deposit");
            e.undo().unwrap();
            assert_eq!(
                before,
                e.state_hash(),
                "{kind}: COW-delta undo byte identity"
            );
            e.redo().unwrap();
            assert_eq!(after, e.state_hash(), "{kind}: redo byte identity");
            let replayed = e.replay_assembled(e.journal()).unwrap();
            assert_eq!(
                after,
                replayed.state_hash(),
                "{kind}: composite-sampled replay must be byte-identical (D-48)"
            );
            // Replay TWICE from the same journal — two independent engines
            // must agree, so the composite resolution carries no hidden
            // iteration-order or address-derived state.
            let again = e.replay_assembled(e.journal()).unwrap();
            assert_eq!(
                replayed.state_hash(),
                again.state_hash(),
                "{kind}: two replays of one journal must agree"
            );
        }
    }

    /// QA-F8 on the sampling door: a layer the storage-space composite
    /// cannot serve REFUSES typed, it is never silently omitted from the
    /// sample. Each arm is paired with the GREEN control that proves the
    /// refusal is caused by the perturbation and nothing else.
    #[test]
    fn sampling_composite_refuses_unservable_layers_typed() {
        // Control: the untouched fixture serves.
        let (mut e, [_, _, _, top], asset) = layered();
        e.apply_json(&clone_env(240, top, asset, json!("composite")))
            .expect("control: the plain raster fixture serves");

        for (n, layer_pick, props, needle) in [
            (241u32, 2usize, json!({"blend": "multiply"}), "blend"),
            (242, 2, json!({"clipToBelow": true}), "clips to below"),
        ] {
            let (mut e, ids, asset) = layered();
            apply(
                &mut e,
                n + 900,
                "layer.set_props",
                Some(&ids[layer_pick].as_uuid().to_string()),
                props.clone(),
            );
            let err = e
                .apply_json(&clone_env(n, ids[3], asset, json!("composite")))
                .expect_err("an unservable layer in scope must refuse");
            assert_eq!(
                err.code,
                ErrorCode::NotImplemented,
                "props {props} must refuse TYPED, never silently drop the layer"
            );
            assert!(
                err.message.contains(needle),
                "the refusal must name its row; got: {}",
                err.message
            );
        }
    }

    /// A layer that is HIDDEN, or outside the requested scope, must not
    /// force a refusal — the walk only judges what it actually composites.
    #[test]
    fn unservable_layers_outside_the_sampled_set_do_not_refuse() {
        // (a) hidden: `high` carries an unservable blend but is invisible.
        let (mut e, ids, asset) = layered();
        apply(
            &mut e,
            250,
            "layer.set_props",
            Some(&ids[2].as_uuid().to_string()),
            json!({"blend": "multiply", "visible": false}),
        );
        e.apply_json(&clone_env(251, ids[3], asset, json!("composite")))
            .expect("a HIDDEN unservable layer is not composited, so it must not refuse");

        // (b) out of scope: `high` is unservable and VISIBLE, but
        // current_and_below(mid) never reaches it.
        let (mut e, ids, asset) = layered();
        apply(
            &mut e,
            252,
            "layer.set_props",
            Some(&ids[2].as_uuid().to_string()),
            json!({"blend": "multiply"}),
        );
        e.apply_json(&clone_env(253, ids[3], asset, json!({"layerId": MID_ID})))
            .expect("current_and_below must not judge layers above the armed layer");
        // …and the very same document DOES refuse on the `all` scope, so
        // (b) is not passing because the blend stopped mattering.
        let err = e
            .apply_json(&clone_env(254, ids[3], asset, json!("composite")))
            .expect_err("the `all` scope reaches `high` and must refuse");
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    /// **The tri-state's INVALID combination stays typed.** §45.2's table
    /// has three rows only; `source:"composite"` with `sampleAllLayers`
    /// false is an INEXPRESSIBLE scope (ES §40.5 clone entry:
    /// "'composite'+false → error.schema.out_of_range (inexpressible
    /// scope)"), and serving the other two arms must not "fix" it into an
    /// acceptance.
    ///
    /// Both spellings of false are pinned — the EXPLICIT `false` and the
    /// DEFAULTED absence — because the schema default is what a real
    /// client sends, and only the explicit form was covered before. Both
    /// stroke kinds are pinned: `resolve_sampling_surface` is shared, and
    /// §40.5's family adoption of the §45.2 tri-state binds heal too.
    #[test]
    fn composite_without_sample_all_layers_is_the_invalid_tri_state_combination() {
        for (n, kind, extra) in [
            (260u32, "clone.stroke", json!({})),
            (262, "heal.stroke", json!({"mode": "healing"})),
        ] {
            for (k, spelling) in [(0u32, json!({})), (1, json!({"sampleAllLayers": false}))] {
                let (mut e, [_, _, _, top], asset) = layered();
                let mut params = json!({"source": "composite", "offset": [40.0, 0.0]});
                for (a, b) in extra.as_object().unwrap() {
                    params[a] = b.clone();
                }
                for (a, b) in spelling.as_object().unwrap() {
                    params[a] = b.clone();
                }
                let before = e.state_hash();
                let err = e
                    .apply_json(&env(n + k, kind, top, asset, params))
                    .expect_err("composite + sampleAllLayers:false has no §45.2 row");
                assert_eq!(
                    err.code,
                    ErrorCode::OutOfRange,
                    "{kind} / {spelling}: the invalid combination must be a TYPED \
                     refusal (error.schema.out_of_range), never an acceptance and \
                     never a not_implemented"
                );
                assert_eq!(
                    before,
                    e.state_hash(),
                    "{kind} / {spelling}: a rejected command mutates nothing (§10.2)"
                );
            }
            // GREEN control on the SAME fixture: flipping ONLY the flag
            // reaches the `all` row and serves — so the refusals above are
            // caused by the flag and not by anything else in the payload.
            let (mut e, [_, _, _, top], asset) = layered();
            let mut params = json!({
                "source": "composite", "offset": [40.0, 0.0], "sampleAllLayers": true,
            });
            for (a, b) in extra.as_object().unwrap() {
                params[a] = b.clone();
            }
            e.apply_json(&env(n + 2, kind, top, asset, params))
                .unwrap_or_else(|err| panic!("{kind}: the `all` row must serve: {err:?}"));
        }
    }

    // ---- §45.2 ignoreAdjustments ----------------------------------------

    /// A minimal registered op so the fixture can carry a real ADJUSTMENT
    /// layer. Identity on pixels — the sampling door refuses on the layer
    /// KIND, never on what the op computes, so the kernel is irrelevant
    /// and an identity keeps the fixture honest.
    struct ProbeInvertOp;

    impl appname_core::engine::op_impl::OpImpl for ProbeInvertOp {
        fn op_id(&self) -> &'static str {
            "adjustment.invert"
        }
        fn validate_params(&self, _p: &Value) -> Result<(), EngineError> {
            Ok(())
        }
        fn default_params(&self) -> Value {
            json!({})
        }
        fn roi_out_to_in(&self, _p: &Value, out: appname_core::IRect) -> appname_core::IRect {
            out
        }
        fn pointwise(&self) -> bool {
            true
        }
        fn preview(&self) -> appname_core::engine::op_impl::PreviewPolicy {
            appname_core::engine::op_impl::PreviewPolicy::LiveFull
        }
        fn apply_cpu(
            &self,
            _p: &Value,
            _rect: appname_core::IRect,
            input: &[[f32; 4]],
            output: &mut [[f32; 4]],
        ) -> Result<(), EngineError> {
            output.copy_from_slice(input);
            Ok(())
        }
    }

    const ADJ_LAYER_ID: &str = "0e450200-0000-4000-8000-00000000000a";
    const ADJ_OP_ID: &str = "0e450200-0000-4000-8000-00000000000b";

    fn layered_with_ops() -> (Engine, [appname_core::LayerId; 4], Uuid) {
        layered_in(
            appname_core::engine::kinds::register(crate::kind_impls::with_tools_fill(assembly()))
                .with_op(Arc::new(ProbeInvertOp))
                .build(),
        )
    }

    /// Insert an adjustment layer immediately ABOVE `anchor`.
    fn add_adjustment(e: &mut Engine, n: u32, anchor: appname_core::LayerId) {
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": format!("e0000000-0000-4000-8000-0003{n:08x}"),
                "kind": "adjustment.add",
                "target": {"layerId": anchor.as_uuid().to_string()},
                "params": {
                    "opKind": "adjustment.invert",
                    "newLayerId": ADJ_LAYER_ID,
                    "newOpId": ADJ_OP_ID,
                    "params": {},
                },
            })
            .to_string(),
        )
        .unwrap();
    }

    /// §45.2 [NORMATIVE]: "when `sample = all` and the toggle is on, the
    /// sampling composite is rendered with every adjustment layer ABOVE
    /// THE TARGET LAYER disabled". Three legs:
    ///
    ///   (a) toggle OFF — the adjustment is inside the sampled scope and
    ///       the storage-space composite cannot evaluate it, so the door
    ///       REFUSES typed rather than sampling a composite that quietly
    ///       lacks the adjustment (QA-F8);
    ///   (b) toggle ON — it serves;
    ///   (c) and what it serves is BIT-identical to the same composite
    ///       with that adjustment layer HIDDEN — i.e. the layer is
    ///       genuinely absent from the sample, not merely tolerated.
    #[test]
    fn ignore_adjustments_disables_adjustments_above_the_target() {
        let clone_with = |ignore: bool| {
            json!({
                "source": "composite",
                "offset": [40.0, 0.0],
                "sampleAllLayers": true,
                "ignoreAdjustments": ignore,
                "sizePx": 24.0,
                "seed": 7,
            })
        };

        // (a) an adjustment ABOVE the target, toggle off → typed refusal.
        let (mut e, ids, asset) = layered_with_ops();
        add_adjustment(&mut e, 270, ids[3]);
        let err = e
            .apply_json(&env(271, "clone.stroke", ids[3], asset, clone_with(false)))
            .expect_err("an adjustment in the sampled scope must refuse typed");
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("adjustment layer"),
            "the refusal must name its row; got: {}",
            err.message
        );

        // (b) the SAME document with the toggle on serves.
        let (mut e, ids, asset) = layered_with_ops();
        add_adjustment(&mut e, 270, ids[3]);
        e.apply_json(&env(271, "clone.stroke", ids[3], asset, clone_with(true)))
            .expect("§45.2: ignoreAdjustments disables adjustments above the target");
        let ignored = bits(&plane(&e, ids[3]));

        // (c) …and equals the composite with that layer HIDDEN.
        let (mut e, ids, asset) = layered_with_ops();
        add_adjustment(&mut e, 270, ids[3]);
        apply(
            &mut e,
            272,
            "layer.set_props",
            Some(ADJ_LAYER_ID),
            json!({"visible": false}),
        );
        e.apply_json(&env(271, "clone.stroke", ids[3], asset, clone_with(false)))
            .expect("a hidden adjustment is not composited");
        assert_eq!(
            ignored,
            bits(&plane(&e, ids[3])),
            "ignoreAdjustments must make the adjustment GENUINELY ABSENT from the \
             sample — bit-identical to hiding it"
        );

        // Falsifiability guard: without the adjustment layer at all, the
        // toggle changes nothing, so (b) vs (c) above is only meaningful
        // because the layer exists. Pin that the plain fixture is
        // unaffected by the flag.
        let (mut e, ids, asset) = layered_with_ops();
        e.apply_json(&env(273, "clone.stroke", ids[3], asset, clone_with(true)))
            .unwrap();
        let with_flag = bits(&plane(&e, ids[3]));
        let (mut e, ids, asset) = layered_with_ops();
        e.apply_json(&env(273, "clone.stroke", ids[3], asset, clone_with(false)))
            .unwrap();
        assert_eq!(
            with_flag,
            bits(&plane(&e, ids[3])),
            "with no adjustment layer present the toggle is a no-op"
        );
    }

    /// §45.2: `ignoreAdjustments` "is ignored (with a `warn` log) unless
    /// `source = \"composite\"`". Pinned as a REFUSAL that the toggle does
    /// NOT rescue: an adjustment sitting inside a `current_and_below`
    /// scope still refuses with the toggle on, because on that arm the
    /// toggle is not consulted at all.
    ///
    /// §45.2 disables adjustments **ABOVE THE TARGET LAYER**, not every
    /// adjustment in the scope. The distinction is the whole point of the
    /// rule — an adjustment BELOW the target is one the clone result will
    /// NOT re-composite through, so disabling it would change what the
    /// user sampled for no reason.
    ///
    /// This is the guard against an OVER-BROAD implementation, on the arm
    /// where the toggle IS consulted (`source:"composite"`): an
    /// adjustment below the target must still REFUSE typed with the
    /// toggle ON. Its sibling test above covers the other half (the
    /// toggle is not consulted at all off the composite arm), and both
    /// are paired with the green control that the SAME fixture with the
    /// adjustment moved ABOVE the target serves.
    #[test]
    fn ignore_adjustments_does_not_reach_adjustments_below_the_target() {
        let composite_ignoring = json!({
            "source": "composite",
            "offset": [40.0, 0.0],
            "sampleAllLayers": true,
            "ignoreAdjustments": true,
            "sizePx": 24.0,
            "seed": 7,
        });

        // An adjustment just above `bg` — well BELOW the target (`top`).
        let (mut e, ids, asset) = layered_with_ops();
        add_adjustment(&mut e, 284, ids[0]);
        let before = e.state_hash();
        let err = e
            .apply_json(&env(
                285,
                "clone.stroke",
                ids[3],
                asset,
                composite_ignoring.clone(),
            ))
            .expect_err(
                "ignoreAdjustments disables adjustments ABOVE the target only (§45.2); \
                 one BELOW it is still in the sampled scope and must refuse typed",
            );
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("adjustment layer"),
            "the refusal must name its row; got: {}",
            err.message
        );
        assert_eq!(before, e.state_hash(), "a refused command mutates nothing");

        // GREEN control: the SAME toggle, the SAME op, the same fixture —
        // only the adjustment's POSITION changes, to above the target.
        // So the refusal above is positional and nothing else.
        let (mut e, ids, asset) = layered_with_ops();
        add_adjustment(&mut e, 286, ids[3]);
        e.apply_json(&env(287, "clone.stroke", ids[3], asset, composite_ignoring))
            .expect("control: an adjustment ABOVE the target is disabled by the toggle");
    }

    /// (The `warn` log itself needs a minted MessageId for
    /// `ApplyReceipt.warnings`; not minted in this lane, recorded in the
    /// clone impl. The IGNORING is what is normative and it is served.)
    #[test]
    fn ignore_adjustments_is_not_consulted_off_the_composite_arm() {
        let armed = |ignore: bool| {
            json!({
                "source": {"layerId": HIGH_ID},
                "offset": [40.0, 0.0],
                "sampleAllLayers": true,
                "ignoreAdjustments": ignore,
                "sizePx": 24.0,
                "seed": 7,
            })
        };
        // An adjustment just above `bg` — BELOW the armed layer, so it is
        // inside the current_and_below scope.
        for ignore in [false, true] {
            let (mut e, ids, asset) = layered_with_ops();
            add_adjustment(&mut e, 280, ids[0]);
            let err = e
                .apply_json(&env(281, "clone.stroke", ids[3], asset, armed(ignore)))
                .expect_err("current_and_below reaches the adjustment and must refuse");
            assert_eq!(
                err.code,
                ErrorCode::NotImplemented,
                "ignoreAdjustments:{ignore} must not change the `current_and_below` \
                 arm — the toggle is composite-only (§45.2)"
            );
        }
        // GREEN control: the same scope with no adjustment in it serves,
        // so the refusals above are the adjustment's doing.
        let (mut e, ids, asset) = layered_with_ops();
        e.apply_json(&env(282, "clone.stroke", ids[3], asset, armed(true)))
            .expect("control: current_and_below over plain rasters serves");
    }
}
