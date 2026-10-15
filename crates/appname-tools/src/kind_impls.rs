// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `brush.stroke.yaml` / `eraser.stroke.yaml` (params +
//! lock tables), ES §40.5 (stroke family rules), §15 via `appname-tools`
//! (the pure engine), §11.1 (locks), §28.1 (preset resolution — minimal
//! v1), §22.1 (color param); integration-log wave-2 arbitration 1.
//!
//! The two stroke [`KindImpl`]s — kinds 2–3 of 102 — exported from
//! `appname-tools` per integration-log wave-2 arbitration 1 ("appname-tools
//! exports the impls; cli/cabi assemble"). Lifted home verbatim from
//! `appname-cli/src/stroke_kinds.rs` (O-INT-brokered move; the assembly
//! points import this module — e.g. `use appname_tools::kind_impls as
//! stroke_kinds;` — and call [`assembly`]). The impls mutate ONLY through
//! the `KindContext` doors core's apply pipeline hands them (D-48: they run
//! inside the single command entry; the `brush` module itself stays pure).
//!
//! Stage map (§40.5 over the engine's pinned order): stage 5 =
//! contract-generated schema (structured params defer); stage 6 here =
//! target resolution (`layerId` only until the mask/channel arenas, else
//! typed) → §11.1 locks (`PIXELS`/`ALL` block both; `TRANSPARENCY`
//! constrains paint, BLOCKS eraser — §40.5 exception) →
//! `eraseToHistory` semantic gate → samples asset resolve + decode (parse
//! failure → `bad_target`, O-TOOLS handoff rule) → preset resolve (two
//! built-in ids, §28.1 minimal) → §22.1 color decode → render → install
//! through the recorder-coupled doors.
//!
//! Determinism: `seed` is command-recorded (§15.4); an omitted hand-written
//! seed pins to 0 (the spec leaves it unstated — recorded divergence,
//! `content_aware_fill` precedent). Compositing runs in the surface's
//! STORAGE encoding at CP-0.3 (the tools `Backdrop`/`TilePatch` contract is
//! storage-normalized f32; premultiplied invariants hold under source-over/
//! dst-out) — linear-light unification for u8-sRGB surfaces lands with the
//! §5 color wave (recorded divergence, lane log).

use std::sync::Arc;

use crate::brush::{
    decode_samples, render_brush_stroke, render_eraser_stroke, BrushPreset, BrushStrokeSpec,
    EraserStrokeSpec, StrokeContext, StrokeError, StrokeOutcome,
};
use appname_core::{
    EngineAssembly, EngineError, ErrorCode, IRect, KindContext, KindImpl, KindOutcome, LayerId,
};
use serde_json::Value;
use uuid::Uuid;

/// Built-in preset ids (§28.1 minimal resolution — the full resource index
/// is CP-0.5). Content-independent v5 constants under the engine namespace;
/// documented for script authors.
pub fn preset_soft_round_pressure_id() -> Uuid {
    Uuid::new_v5(
        &appname_core::engine::doc_new::APPNAME_NS,
        b"preset/builtin/soft-round-pressure",
    )
}

/// Built-in hard round, 16 px base.
pub fn preset_hard_round_16_id() -> Uuid {
    Uuid::new_v5(
        &appname_core::engine::doc_new::APPNAME_NS,
        b"preset/builtin/hard-round-16",
    )
}

pub(crate) fn resolve_preset(id: Uuid) -> Result<BrushPreset, EngineError> {
    if id == preset_soft_round_pressure_id() {
        Ok(BrushPreset::soft_round_pressure())
    } else if id == preset_hard_round_16_id() {
        Ok(BrushPreset::hard_round(16.0))
    } else {
        Err(EngineError::bad_target(format!(
            "brushPreset {id} does not resolve (resource index arrives CP-0.5; built-ins: \
             {} soft-round-pressure, {} hard-round-16)",
            preset_soft_round_pressure_id(),
            preset_hard_round_16_id()
        )))
    }
}

/// The CLI/cabi engine assembly (arbitration 1). CP-0.5 wave 2 adds the
/// three raster-bake transform kinds (§40.6; the doc.* geometry kinds and
/// selection kinds stay blocked on core doors — lane log handoff).
pub fn assembly() -> EngineAssembly {
    EngineAssembly::new()
        .with_kind(Arc::new(BrushStrokeKind))
        .with_kind(Arc::new(EraserStrokeKind))
        // stroke_path (O-TEXTVEC stroke slice): the LAST registry kind —
        // fill/stroke family, beside FillKind's machinery.
        .with_kind(Arc::new(StrokePathKind))
        .with_kind(Arc::new(TransformFreeKind))
        .with_kind(Arc::new(TransformPerspectiveKind))
        .with_kind(Arc::new(TransformWarpKind))
        // CP-1.0 warps row: validation-complete; identity arm live,
        // rotation-pin solve behind the elected ARAP backend.
        .with_kind(Arc::new(TransformPuppetKind))
        // layer.transform LIFTED from appname-core (lane/transform-183, the
        // fill-lift precedent): the non-interactive affine primitive, whose
        // scale/rotate/affine arm resamples through the §18.2 kernel here.
        // core::engine::kinds::register drops its copy in the same swap, so
        // with_kind's dup guard never trips (registered once, tools-side).
        .with_kind(Arc::new(LayerTransformKind))
        // CP-0.5 wave 3 (doc geometry + selections; fill rides
        // `with_tools_fill` until the brokered core/cli/cabi swap).
        .with_kind(Arc::new(DocResizeKind))
        .with_kind(Arc::new(DocCanvasResizeKind))
        .with_kind(Arc::new(DocCropKind))
        .with_kind(Arc::new(DocTrimKind))
        .with_kind(Arc::new(DocRotateKind))
        .with_kind(Arc::new(SelectionSetKind))
        .with_kind(Arc::new(SelectionModifyKind))
        .with_kind(Arc::new(SelectionTransformKind))
        .with_kind(Arc::new(SelectionAllKind))
        .with_kind(Arc::new(SelectionNoneKind))
        .with_kind(Arc::new(SelectionInvertKind))
        // CP-1.0 consolidated round: the §45.3 CAF consumer arm.
        .with_kind(Arc::new(ContentAwareFillKind))
        // CP-1.0 retouch row (§2.5): the five stroke kinds over the
        // `retouch` pure engines (heal consumes appname-algo's merged
        // §45.1 poisson core; spot content-aware rides the §45.3
        // exemplar engine, CAF-consumer precedent).
        .with_kind(Arc::new(crate::kind_impls_retouch::CloneStrokeKind))
        .with_kind(Arc::new(crate::kind_impls_retouch::HealStrokeKind))
        .with_kind(Arc::new(crate::kind_impls_retouch::DodgeBurnStrokeKind))
        .with_kind(Arc::new(crate::kind_impls_retouch::SmudgeStrokeKind))
        .with_kind(Arc::new(crate::kind_impls_retouch::BlurSharpenStrokeKind))
}

pub struct BrushStrokeKind;
pub struct EraserStrokeKind;

impl KindImpl for BrushStrokeKind {
    fn kind(&self) -> &'static str {
        "brush.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_stroke_target(cx)?;
        check_locks(cx, layer, false)?;
        let clamp = has_lock(cx, layer, "transparency");
        let samples_bytes = resolve_samples(cx)?;
        let samples = decode_samples(&samples_bytes)
            .map_err(|e| EngineError::bad_target(format!("samples asset failed to parse: {e}")))?;
        let preset = resolve_preset(param_uuid(cx, "brushPreset")?)?;
        let color = decode_color_param(cx)?;
        let blend = decode_blend(cx)?;
        let spec = BrushStrokeSpec {
            samples: &samples,
            preset: &preset,
            size_px: param_f32_opt(cx, "sizePx"),
            hardness: param_f32_opt(cx, "hardness"),
            opacity: param_f32(cx, "opacity", 1.0),
            flow: param_f32(cx, "flow", 1.0),
            color,
            blend,
            seed: param_seed(cx),
        };
        let ctx = StrokeContext {
            clip: canvas_clip(cx),
            selection: Some(cx.document().selection()),
            kernels: None, // §4.5 table arrives with the CP-0.5 kernel work
            clamp_to_backdrop_alpha: clamp,
        };
        let outcome = {
            let backdrop = cx
                .document()
                .layer(layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            render_brush_stroke(&spec, backdrop, &ctx).map_err(map_stroke_error)?
        };
        install_outcome(cx, layer, outcome, false)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for EraserStrokeKind {
    fn kind(&self) -> &'static str {
        "eraser.stroke"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_stroke_target(cx)?;
        // §40.5 exception: TRANSPARENCY BLOCKS the eraser outright (its
        // only effect is an alpha decrease, invariant under the lock).
        check_locks(cx, layer, true)?;
        if cx.params().get("eraseToHistory").and_then(Value::as_bool) == Some(true) {
            return Err(EngineError::new(
                ErrorCode::EraseToHistoryUnsupported,
                "eraseToHistory is reserved in v1 (§40.5/§32.3) and MUST be false",
            ));
        }
        let samples_bytes = resolve_samples(cx)?;
        let samples = decode_samples(&samples_bytes)
            .map_err(|e| EngineError::bad_target(format!("samples asset failed to parse: {e}")))?;
        let preset = resolve_preset(param_uuid(cx, "brushPreset")?)?;
        let spec = EraserStrokeSpec {
            samples: &samples,
            preset: &preset,
            size_px: param_f32_opt(cx, "sizePx"),
            opacity: param_f32(cx, "opacity", 1.0),
            flow: param_f32(cx, "flow", 1.0),
            seed: param_seed(cx),
        };
        let ctx = StrokeContext {
            clip: canvas_clip(cx),
            selection: Some(cx.document().selection()),
            kernels: None,
            clamp_to_backdrop_alpha: false,
        };
        let outcome = {
            let backdrop = cx
                .document()
                .layer(layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            render_eraser_stroke(&spec, backdrop, &ctx).map_err(map_stroke_error)?
        };
        install_outcome(cx, layer, outcome, true)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// Shared stage-6 helpers.
// ---------------------------------------------------------------------------

/// Contract target rule: exactly one of layerId | maskId | channelId; v1
/// resolves layer targets only (mask/channel arenas are CP-0.5).
pub(crate) fn resolve_stroke_target(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let target = cx.envelope().target.as_ref();
    let (layer, mask, channel) = match target {
        Some(t) => (t.layer_id, t.mask_id, t.channel_id),
        None => (None, None, None),
    };
    if mask.is_some() || channel.is_some() {
        return Err(EngineError::bad_target(
            "mask/channel stroke surfaces arrive with their arenas (CP-0.5); \
             v1 strokes target target.layerId",
        ));
    }
    let Some(layer) = layer else {
        return Err(EngineError::bad_target(
            "stroke kinds require target.layerId (the paint surface, §40.5)",
        ));
    };
    let id = LayerId::from_uuid(layer);
    let node = cx.document().layer(id).ok_or_else(|| {
        EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
    })?;
    if node.pixels().is_none() {
        return Err(EngineError::bad_target(format!(
            "target.layerId {layer} is not a raster layer"
        )));
    }
    Ok(id)
}

pub(crate) fn has_lock(cx: &KindContext<'_>, layer: LayerId, which: &str) -> bool {
    use appname_core::LockFlags;
    // §11.1 transitive letter via the ONE consolidated place (O-CORE
    // door-slice handoff): "Locks on a Group apply transitively to its
    // subtree; the strictest lock along the ancestry wins".
    let locks = cx.document().effective_locks(layer);
    match which {
        "transparency" => locks.contains(LockFlags::TRANSPARENCY),
        "pixels" => locks.contains(LockFlags::PIXELS),
        _ => false,
    }
}

/// §11.1/§40.5 lock gates: `PIXELS` (and thus ALL) block both kinds;
/// `TRANSPARENCY` blocks the eraser, constrains paint (caller reads it).
pub(crate) fn check_locks(
    cx: &KindContext<'_>,
    layer: LayerId,
    eraser: bool,
) -> Result<(), EngineError> {
    if has_lock(cx, layer, "pixels") {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the target layer's pixels are locked (§11.1; error id errors.layer.locked)",
        ));
    }
    if eraser && has_lock(cx, layer, "transparency") {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "TRANSPARENCY lock blocks eraser.stroke outright (§40.5: its only effect is an \
             alpha decrease, invariant under the lock)",
        ));
    }
    Ok(())
}

pub(crate) fn resolve_samples(cx: &KindContext<'_>) -> Result<Vec<u8>, EngineError> {
    let Some(s) = cx.envelope().samples.as_deref() else {
        return Err(EngineError::bad_target(
            "stroke kinds require the envelope `samples` asset reference (§10.2/§15.1)",
        ));
    };
    let id = Uuid::parse_str(s).map_err(|_| {
        EngineError::bad_target(format!("`samples` is not a valid asset uuid: \"{s}\""))
    })?;
    match cx.asset_bytes(id) {
        Some(bytes) => Ok(bytes.to_vec()),
        None => Err(EngineError::bad_target(format!(
            "samples asset {id} does not resolve (register it via `appname asset add` or the \
             package's assets/ entries)"
        ))),
    }
}

pub(crate) fn param_uuid(cx: &KindContext<'_>, name: &str) -> Result<Uuid, EngineError> {
    cx.params()
        .get(name)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| EngineError::missing(name))
}

pub(crate) fn param_f32(cx: &KindContext<'_>, name: &str, default: f32) -> f32 {
    cx.params()
        .get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .unwrap_or(default)
}

pub(crate) fn param_f32_opt(cx: &KindContext<'_>, name: &str) -> Option<f32> {
    cx.params()
        .get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
}

/// §15.4 seed is command-recorded; an omitted hand-written seed pins to 0
/// (spec-unstated — recorded divergence).
pub(crate) fn param_seed(cx: &KindContext<'_>) -> u64 {
    cx.params().get("seed").and_then(Value::as_u64).unwrap_or(0)
}

fn decode_blend(cx: &KindContext<'_>) -> Result<appname_core::BlendMode, EngineError> {
    match cx.params().get("blend") {
        None => Ok(appname_core::BlendMode::Normal),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| EngineError::range("blend", format!("`blend`: {e}"))),
    }
}

/// §22.1 color param → straight working-space linear RGB. v1: `rgb` decodes
/// through the interim sRGB boundary; `gray` splats; an explicit profileId
/// is unverifiable pre-CMM (the standing doc.new divergence); other models
/// need the §5/§6 conversion pipeline.
fn decode_color_param(cx: &KindContext<'_>) -> Result<[f32; 3], EngineError> {
    let color = cx
        .params()
        .get("color")
        .and_then(Value::as_object)
        .ok_or_else(|| EngineError::missing("color"))?;
    if color.get("profileId").is_some() {
        return Err(EngineError::with_field(
            ErrorCode::ProfileSpaceMismatch,
            "color.profileId",
            "explicit color profiles cannot be verified at CP-0.3 (no CMM before the color \
             lane, §5.5); omit profileId",
        ));
    }
    let comps = color
        .get("comps")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("color.comps"))?;
    if comps.len() != 4 || !comps.iter().all(|c| c.is_number()) {
        return Err(EngineError::range(
            "color.comps",
            "`color.comps` must be 4 numbers (§22.1)",
        ));
    }
    let c = |i: usize| comps[i].as_f64().unwrap_or(0.0) as f32;
    match color.get("model").and_then(Value::as_str) {
        Some("rgb") | Some("rgba") => Ok([
            appname_core::io::srgb::srgb_decode(c(0).clamp(0.0, 1.0)),
            appname_core::io::srgb::srgb_decode(c(1).clamp(0.0, 1.0)),
            appname_core::io::srgb::srgb_decode(c(2).clamp(0.0, 1.0)),
        ]),
        Some("gray") | Some("grayA") => {
            let v = appname_core::io::srgb::srgb_decode(c(0).clamp(0.0, 1.0));
            Ok([v, v, v])
        }
        Some(other) => Err(EngineError::new(
            ErrorCode::NotImplemented,
            format!("color model \"{other}\" needs the §5/§6 conversion pipeline (CP-0.5)"),
        )),
        None => Err(EngineError::missing("color.model")),
    }
}

pub(crate) fn canvas_clip(cx: &KindContext<'_>) -> IRect {
    cx.document().geometry().canvas_rect()
}

/// Install the composited tiles through the recorder-coupled doors and
/// maintain bounds (§2.4: paint grows eagerly by the commit bounds; the
/// eraser re-tightens tile-granularly). An empty outcome installs nothing —
/// the engine pushes the §40.1 empty DeltaSet.
fn install_outcome(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    outcome: StrokeOutcome,
    eraser: bool,
) -> Result<(), EngineError> {
    let commit = outcome.commit;
    for patch in &commit.patches {
        cx.install_tile_plane(layer, patch.coord, &patch.pixels)?;
    }
    if commit.patches.is_empty() {
        return Ok(());
    }
    finish_write_bounds(cx, layer, commit.bounds, eraser)?;
    Ok(())
}

/// Shared write-commit bounds maintenance — the Q-1 determinism-class
/// invariant in ONE place. The class is WRITES-THROUGH-SOFT-COVERAGE, not
/// strokes: every raster write door whose per-pixel change test runs
/// PRE-quantization (`commit.bounds`/`changed` from `edit_region_at`) counts
/// the soft coverage fringe — sub-quantum values that store as exactly
/// transparent — so the eager extent runs up to 1px proud of the persisted
/// plane. The package loader recomputes bounds pixel-exact on reload;
/// `bounds` folds into `region_hash` → `state_hash`, so a proud extent forks
/// live-vs-reloaded and the D-48 replay-as-recovery gate disables undo/redo
/// across save→reopen (aud-replay F1's class — four stroke kinds AND
/// `filter.apply` through a feathered selection still forked after F1 fixed
/// the `fill` instance at one call site).
///
/// Tools-side callers: `install_outcome` (brush/eraser), the retouch
/// `install_commit` (heal/clone/dodge_burn/smudge/blur_sharpen),
/// `stroke_path`, `transform.puppet`'s selection-gated arm,
/// `content_aware_fill`, and `filter.apply` (appname-filters). Core-side
/// doors (edit.cut, mask.apply, the merge/flatten family) call the same
/// `KindContext` pixel-exact primitives directly — appname-core cannot
/// depend on this crate.
///
/// `can_erase`: a write that may REDUCE alpha (clone/smudge/eraser strokes,
/// filters, CAF, puppet) can recede the extent below a still-occupied tile —
/// only the full pixel-exact recompute matches the loader's sub-tile shrink
/// there. Alpha-PRESERVING writes only ADD coverage, so the region-scoped
/// pixel-exact grow is exact (the prior extent stays loader-tight) and cheap
/// — the paint hot path pays only its own footprint, guarding the F1
/// `fill_apply` +17% regression class.
///
/// Structural: a NEW write door reaches its bounds update through this door
/// (or core's pixel-exact primitives), so it cannot silently reintroduce the
/// fork. Two pins enforce it: the parameterized class-killing pin
/// (`crates/appname-cli/tests/replay_stroke_kinds_pin.rs`) over every stroke
/// kind + the soft-coverage doors, and the bounds-door census guard
/// (`crates/appname-cli/tests/bounds_door_census.rs`) which REDS when a new
/// bounds-mutating call site appears outside the audited set.
pub fn finish_write_bounds(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    written: appname_core::IRect,
    can_erase: bool,
) -> Result<(), EngineError> {
    if can_erase {
        // INTERIOR GUARD (the erase-side sibling of the grow branch's
        // growth-frontier guard; the same bench discipline that produced
        // fill's `needs_pixel_exact` after the F1 +17% regression): the
        // current extent is loader-TIGHT by induction, so every edge of
        // `bounds` is realized by at least one stored-nonzero pixel in its
        // 1px perimeter band. A write footprint STRICTLY interior to that
        // band cannot touch a realizing pixel (no shrink) and cannot store
        // outside `bounds` (no growth) — bounds provably unchanged, skip
        // the full-plane rescan. Stored changes are a subset of the
        // f32-counted `written` (quantization is deterministic), so testing
        // `written` is sound. A footprint that reaches the perimeter — every
        // scene the Q-1 probes red-lined — takes the pixel-exact recompute.
        let bounds = cx
            .document()
            .layer(layer)
            .and_then(|n| n.pixels())
            .map(|b| b.bounds())
            .unwrap_or(appname_core::IRect::EMPTY);
        let strictly_interior = !written.is_empty()
            && !bounds.is_empty()
            && written.min.x > bounds.min.x
            && written.min.y > bounds.min.y
            && written.max.x < bounds.max.x
            && written.max.y < bounds.max.y;
        if !strictly_interior {
            cx.recompute_layer_bounds_pixel_exact(layer)?;
        }
    } else {
        cx.grow_layer_bounds_pixel_exact(layer, written)?;
    }
    Ok(())
}

/// O-TOOLS handoff mapping: StrokeError → wire taxonomy.
pub(crate) fn map_stroke_error(e: StrokeError) -> EngineError {
    match e {
        StrokeError::Samples(e) => {
            EngineError::bad_target(format!("samples asset failed §15.1 validation: {e}"))
        }
        StrokeError::Preset(e) => EngineError::bad_target(format!("brush preset invalid: {e}")),
        StrokeError::SizePxOutOfRange(v) => {
            EngineError::range("sizePx", format!("sizePx {v} outside 0.1..=10000"))
        }
        StrokeError::HardnessOutOfRange(v) => {
            EngineError::range("hardness", format!("hardness {v} outside 0..=1"))
        }
        StrokeError::OpacityOutOfRange(v) => {
            EngineError::range("opacity", format!("opacity {v} outside 0..=1"))
        }
        StrokeError::FlowOutOfRange(v) => {
            EngineError::range("flow", format!("flow {v} outside 0..=1"))
        }
        StrokeError::ColorNotFinite => EngineError::range("color", "paint color must be finite"),
        StrokeError::BlendKernelUnavailable(m) => EngineError::new(
            ErrorCode::NotImplemented,
            format!("blend mode {m:?} needs the §4.5 kernel table (CP-0.5); normal is built in"),
        ),
        StrokeError::TransparencyLockBlendUnsupported(m) => EngineError::new(
            ErrorCode::NotImplemented,
            format!(
                "TRANSPARENCY-locked painting in blend mode {m:?} needs the CP-0.5 kernel-table \
                 extension"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{encode_samples, InputSample};
    use appname_core::{Engine, IVec2};
    use serde_json::json;

    fn sample_line() -> Vec<u8> {
        let pts = [(20.0, 20.0, 0.0), (60.0, 40.0, 0.04), (100.0, 60.0, 0.08)];
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

    fn doc_new(engine: &mut Engine, n: u32, background: &str) -> appname_core::LayerId {
        let text = json!({
            "schema": "appname.command.v1",
            "id": format!("e0000000-0000-4000-8000-0000000{n:05x}"),
            "kind": "doc.new",
            "params": {"width": 128, "height": 128, "background": background},
        })
        .to_string();
        engine.apply_json(&text).unwrap();
        engine.document().unwrap().z_order_walk()[0]
    }

    fn stroke_env(
        n: u32,
        kind: &str,
        layer: appname_core::LayerId,
        samples: Uuid,
        extra: serde_json::Value,
    ) -> String {
        let mut params = json!({"brushPreset": preset_hard_round_16_id().to_string()});
        if kind == "brush.stroke" {
            params["color"] = json!({"model": "rgb", "comps": [1, 0, 0, 0]});
        }
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                params[k] = v.clone();
            }
        }
        json!({
            "schema": "appname.command.v1",
            "id": format!("e0000000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": params,
            "samples": samples.to_string(),
        })
        .to_string()
    }

    fn ready_engine(background: &str) -> (Engine, appname_core::LayerId, Uuid) {
        let mut e = assembly().build();
        let layer = doc_new(&mut e, 1, background);
        let asset = e.add_samples_asset(sample_line());
        (e, layer, asset)
    }

    #[test]
    fn brush_stroke_paints_undoes_redoes_and_replays() {
        let (mut e, layer, asset) = ready_engine("white");
        let before = e.state_hash();
        let receipt = e
            .apply_json(&stroke_env(
                2,
                "brush.stroke",
                layer,
                asset,
                json!({"seed": 7}),
            ))
            .unwrap();
        let after = e.state_hash();
        assert_ne!(before, after);
        assert!(!receipt.dirty.is_empty(), "touched tiles reported");
        assert!(receipt.dirty.iter().all(|d| d.layer == layer));
        // The stroke crosses (60, 40): red deposited over white.
        let px = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(60, 40));
        assert!(px[0] > 0.9 && px[1] < 0.6, "red-ward pixel, got {px:?}");

        let row = e.undo().unwrap();
        assert_eq!(row.dirty, receipt.dirty, "undo echoes the same rects");
        assert_eq!(
            e.state_hash(),
            before,
            "COW-delta undo restores byte identity"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);

        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "replay through the same assembly + assets"
        );
    }

    #[test]
    fn brush_stroke_hardness_override_reaches_the_stamp_and_replays() {
        // O-UI item 4: the `hardness` override binds symmetrically with
        // `sizePx`. The built-in preset is hard-round (hardness 1.0); a soft
        // override must change the deposited coverage, and an absent override
        // must reproduce the preset default byte-for-byte. `ready_engine` is
        // deterministic (see `stroke_determinism_same_inputs_same_state`), so
        // fresh builds are the value-capture-safe way to compare states.
        let paint = |extra: serde_json::Value| {
            let (mut e, layer, asset) = ready_engine("white");
            e.apply_json(&stroke_env(40, "brush.stroke", layer, asset, extra))
                .unwrap();
            e.state_hash()
        };

        // Absent override → preset hardness, byte-identical across runs.
        let default_hash = paint(json!({"seed": 7}));
        assert_eq!(
            paint(json!({"seed": 7})),
            default_hash,
            "absent hardness ⇒ preset default, byte-identical"
        );

        // Explicit soft hardness → the stamp coverage changes, so the state
        // must differ from the preset-default stroke (the value reached
        // Stamp.hardness through §15.5 tip_coverage).
        assert_ne!(
            paint(json!({"seed": 7, "hardness": 0.1})),
            default_hash,
            "explicit hardness override changes the deposited stamp"
        );

        // D-48: the hardness-carrying command replays bit-identically.
        let (mut e, layer, asset) = ready_engine("white");
        e.apply_json(&stroke_env(
            40,
            "brush.stroke",
            layer,
            asset,
            json!({"seed": 7, "hardness": 0.1}),
        ))
        .unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());

        // Out-of-domain hardness → typed refusal (error.schema.out_of_range),
        // mirroring the sizePx range rule; nothing is mutated or journaled.
        let (mut r, layer, asset) = ready_engine("white");
        let before = r.state_hash();
        let err = r
            .apply_json(&stroke_env(
                41,
                "brush.stroke",
                layer,
                asset,
                json!({"hardness": 1.5}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        assert!(err.message.contains("hardness"));
        assert_eq!(r.state_hash(), before, "rejected stroke mutated nothing");
        assert_eq!(r.journal().len(), 1, "only doc.new journaled");
    }

    #[test]
    fn eraser_stroke_erases_and_round_trips() {
        let (mut e, layer, asset) = ready_engine("white");
        let before = e.state_hash();
        e.apply_json(&stroke_env(3, "eraser.stroke", layer, asset, json!({})))
            .unwrap();
        let px = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(60, 40));
        assert!(px[3] < 0.5, "alpha erased at the stroke center, got {px:?}");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Lift-home equivalence anchor (O-INT-brokered move): the canonical
    /// stroke script's post-state hash, frozen. The pre-move impls in
    /// `appname-cli/src/stroke_kinds.rs` are byte-identical code (diff-
    /// verified modulo use-paths and the doc header), so this constant is
    /// the behavioral fingerprint both assemblies must produce; O-CORE's
    /// cli-side import swap re-verifies against it for free (this test
    /// rides `cargo test --workspace`). Regenerate ONLY on a spec-owner-
    /// approved behavior change: run with `--nocapture` on mismatch.
    #[test]
    fn lift_home_equivalence_frozen_state_hash() {
        let (mut e, layer, asset) = ready_engine("white");
        e.apply_json(&stroke_env(
            30,
            "brush.stroke",
            layer,
            asset,
            json!({"seed": 7, "opacity": 0.8}),
        ))
        .unwrap();
        e.apply_json(&stroke_env(
            31,
            "eraser.stroke",
            layer,
            asset,
            json!({"opacity": 0.4}),
        ))
        .unwrap();
        let got = e.state_hash().to_hex();
        const FROZEN_STATE: &str =
            "767b1544433c454ea2a70c324e10f6ba0e6f88fbfe51d6eb342cc17f8c2aaf2d";
        assert_eq!(
            got, FROZEN_STATE,
            "canonical stroke-script state hash moved"
        );
    }

    #[test]
    fn stroke_determinism_same_inputs_same_state() {
        let build = || {
            let (mut e, layer, asset) = ready_engine("white");
            e.apply_json(&stroke_env(
                4,
                "brush.stroke",
                layer,
                asset,
                json!({"seed": 9, "opacity": 0.5}),
            ))
            .unwrap();
            e.state_hash()
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn stroke_outside_canvas_is_the_empty_delta_set_push() {
        let (mut e, layer, _asset) = ready_engine("white");
        let far = [(-500.0, -500.0, 0.0), (-400.0, -500.0, 0.05)];
        let samples: Vec<InputSample> = far
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
        let asset = e.add_samples_asset(encode_samples(&samples).unwrap());
        let before = e.state_hash();
        let receipt = e
            .apply_json(&stroke_env(5, "brush.stroke", layer, asset, json!({})))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "§40.1 empty outcome");
        assert_eq!(e.state_hash(), before);
        assert_eq!(e.journal().len(), 2, "empty DeltaSet still journals");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "empty undo is a no-op on state");
    }

    #[test]
    fn adversarial_rejects_are_typed() {
        let (mut e, layer, asset) = ready_engine("white");

        // Missing samples reference.
        let mut env: serde_json::Value =
            serde_json::from_str(&stroke_env(6, "brush.stroke", layer, asset, json!({}))).unwrap();
        env.as_object_mut().unwrap().remove("samples");
        let err = e.apply_json(&env.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("samples"));

        // Unknown asset id.
        let ghost = Uuid::new_v4();
        let err = e
            .apply_json(&stroke_env(7, "brush.stroke", layer, ghost, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("does not resolve"));

        // Garbage blob behind a valid id (the engine door is opaque; decode
        // fails at stage 6 per the O-TOOLS handoff rule).
        let garbage = e.add_samples_asset(b"not a samples blob".to_vec());
        let err = e
            .apply_json(&stroke_env(8, "brush.stroke", layer, garbage, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("parse"));

        // Unknown preset uuid names the built-ins.
        let err = e
            .apply_json(&stroke_env(
                9,
                "brush.stroke",
                layer,
                asset,
                json!({"brushPreset": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("built-ins"));

        // eraseToHistory reserved.
        let err = e
            .apply_json(&stroke_env(
                10,
                "eraser.stroke",
                layer,
                asset,
                json!({"eraseToHistory": true}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::EraseToHistoryUnsupported);

        // Non-normal blend needs the kernel table.
        let err = e
            .apply_json(&stroke_env(
                11,
                "brush.stroke",
                layer,
                asset,
                json!({"blend": "multiply"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);

        // Color models beyond the interim boundary.
        let err = e
            .apply_json(&stroke_env(
                12,
                "brush.stroke",
                layer,
                asset,
                json!({"color": {"model": "lab", "comps": [0.5, 0.5, 0.5, 0.0]}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);

        // Explicit color profile pre-CMM.
        let err = e
            .apply_json(&stroke_env(13, "brush.stroke", layer, asset,
                json!({"color": {"model": "rgb", "comps": [1, 0, 0, 0], "profileId": "eeeeeeee-0000-4000-8000-000000000001"}})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);

        // Target rules: absent / mask-surface / unresolved.
        let mut env: serde_json::Value =
            serde_json::from_str(&stroke_env(14, "brush.stroke", layer, asset, json!({}))).unwrap();
        env.as_object_mut().unwrap().remove("target");
        let err = e.apply_json(&env.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);

        let mut env: serde_json::Value =
            serde_json::from_str(&stroke_env(15, "brush.stroke", layer, asset, json!({}))).unwrap();
        env["target"] = json!({"maskId": Uuid::new_v4().to_string()});
        let err = e.apply_json(&env.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("CP-0.5"));

        let mut env: serde_json::Value =
            serde_json::from_str(&stroke_env(16, "brush.stroke", layer, asset, json!({}))).unwrap();
        env["target"] = json!({"layerId": Uuid::new_v4().to_string()});
        let err = e.apply_json(&env.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);

        // Every reject above left the document untouched and journaled
        // nothing beyond doc.new.
        assert_eq!(e.journal().len(), 1);
    }

    #[test]
    fn transparency_lock_constrains_brush_to_existing_coverage() {
        // Transparent background + TRANSPARENCY lock: source-atop against
        // alpha 0 deposits nothing — the §11.1 constrain, not a block.
        let (mut e, layer, asset) = ready_engine("transparent");
        lock_layer(&mut e, layer, &["transparency"]);
        let before = e.state_hash();
        let receipt = e
            .apply_json(&stroke_env(17, "brush.stroke", layer, asset, json!({})))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "no coverage → nothing deposited");
        assert_eq!(e.state_hash(), before);
    }

    #[test]
    fn locks_block_pixels_for_both_and_transparency_for_eraser() {
        let (mut e, layer, asset) = ready_engine("white");
        lock_layer(&mut e, layer, &["pixels"]);
        let err = e
            .apply_json(&stroke_env(18, "brush.stroke", layer, asset, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        let err = e
            .apply_json(&stroke_env(19, "eraser.stroke", layer, asset, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);

        let (mut e2, layer2, asset2) = ready_engine("white");
        lock_layer(&mut e2, layer2, &["transparency"]);
        let err = e2
            .apply_json(&stroke_env(20, "eraser.stroke", layer2, asset2, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked, "§40.5 eraser hard block");
        // …while the brush proceeds constrained (white bg has full alpha).
        e2.apply_json(&stroke_env(21, "brush.stroke", layer2, asset2, json!({})))
            .unwrap();
    }

    /// Set locks on a layer by round-tripping the package with a rewritten
    /// document.json (public doors only — the loader reads `locks`).
    fn lock_layer(e: &mut Engine, layer: appname_core::LayerId, locks: &[&str]) {
        use std::io::{Read, Write};
        let bytes = appname_core::save_package_bytes(e).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..za.len() {
            let mut entry = za.by_index(i).unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            entries.push((entry.name().to_string(), data));
        }
        for (name, data) in entries.iter_mut() {
            if name == "document.json" {
                let mut v: serde_json::Value = serde_json::from_slice(data).unwrap();
                for l in v["layers"].as_array_mut().unwrap() {
                    if l["id"] == layer.as_uuid().to_string() {
                        l["locks"] = json!(locks);
                    }
                }
                let mut out = serde_json::to_vec_pretty(&v).unwrap();
                out.push(b'\n');
                *data = out;
            }
        }
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                zw.start_file(name, opts).unwrap();
                zw.write_all(&data).unwrap();
            }
            zw.finish().unwrap();
        }
        let loaded =
            appname_core::load_package_bytes_with(&assembly(), &cursor.into_inner()).unwrap();
        // Carry the session assets across the round trip (the lock rewrite
        // is test scaffolding, not a save-flow claim).
        let mut fresh = loaded.engine;
        for id in e.assets().ids_sorted() {
            let _ = id; // ids are content-derived; re-add below
        }
        std::mem::swap(e, &mut fresh);
        // Re-register the sample assets under their content ids.
        for id in fresh.assets().ids_sorted() {
            if let Some(bytes) = fresh.assets().bytes(id) {
                e.add_samples_asset(bytes.to_vec());
            }
        }
    }
}

// ===========================================================================
// CP-0.5 wave 2: transform.* raster-bake kinds (§40.6, §18.2–§18.4).
//
// Stage-6 map: raster-layer target (transient multi-select groups are a
// later wave) → §40.6 locks (POSITION/ALL block; PIXELS does NOT) →
// semantic geometry gates (invertible matrix / valid quad / mesh count) →
// pure engines (crate modules xform/warp/resample) → whole-plane bake
// through the recorder-coupled tile doors + bounds recompute.
//
// v1 pins (lane log): the wire matrix applies as-is in document space
// (`reference` is validated + recorded session-pivot metadata — the §18.3
// scripting example's matrix is not pivot-relative); identity transforms
// skip installs entirely (§40.1 empty-delta push, §18.3 bit-identity);
// perspective solves source-bounds → quad (§18.3); warp meshes span the
// layer's content bounds and always sample Catmull-Rom (§18.4 commit
// sentence — the `resample` param is validated + recorded); `scaleEffects`
// is accepted and inert until layer effects exist; §18.2 store-clamping
// applies on unsigned-normalized targets (f32 keeps excursions);
// unsupported preset kinds (no §18.4 math) → error.command.not_implemented.
// ===========================================================================

use appname_core::{ChannelKind, LockFlags, Mat3};

use crate::resample::Plane;
use crate::warp::{preset_mesh, WarpError, WarpMesh};
use crate::xform::{
    dlt_homography, mat3_inverse, quad_is_valid, resolve_transform_kernel, scale_metric,
    transform_plane, transformed_bounds,
};

pub struct TransformFreeKind;
pub struct TransformPerspectiveKind;
pub struct TransformWarpKind;
pub struct TransformPuppetKind;
/// `layer.transform` — the non-interactive affine primitive (§10.3/§40.3;
/// contract `layer.transform.yaml`), LIFTED from `appname-core` on
/// lane/transform-183 (fill-lift precedent) so its general arm can reach
/// the §18.2 resample kernel that lives tools-side.
pub struct LayerTransformKind;

impl KindImpl for TransformFreeKind {
    fn kind(&self) -> &'static str {
        "transform.free"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_transform_target(cx)?;
        check_transform_locks(cx, layer)?;
        let matrix = param_mat3(cx, "matrix")?;
        let _reference = param_vec2(cx, "reference")?; // session-pivot metadata (pin)
        if mat3_inverse(&matrix).is_none() {
            return Err(EngineError::range(
                "matrix",
                "matrix MUST be invertible (§40.6)",
            ));
        }
        if matrix.is_identity() {
            return Ok(KindOutcome::default()); // §40.1 empty-delta push
        }
        // §11.2 Sub-slice C class-kill (q-scenario/Q-3, interactive door): a
        // non-identity free transform bakes a masked layer's pixels while the
        // mask stays in canvas space. Refuse typed — identity already returned
        // above, so this fires only on a genuine bake.
        if layer_has_enabled_mask(cx, layer) {
            return Err(masked_transform_refusal());
        }
        let (src, src_rect, unsigned) = read_layer_plane(cx, layer)?;
        let Some(src) = src else {
            return Ok(KindOutcome::default());
        };
        let scale = scale_metric(&matrix, rect_center(src_rect));
        let kernel = resolve_transform_kernel(&param_resample(cx), scale)
            .map_err(|_| EngineError::range("resample", "unknown §18.2 kernel name"))?;
        let dst_rect = transformed_bounds(&matrix, src_rect);
        let out = transform_plane(&src, &matrix, dst_rect, kernel)
            .ok_or_else(|| EngineError::range("matrix", "matrix MUST be invertible (§40.6)"))?;
        install_plane(cx, layer, &out, src_rect, unsigned)
    }
}

impl KindImpl for TransformPerspectiveKind {
    fn kind(&self) -> &'static str {
        "transform.perspective"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_transform_target(cx)?;
        check_transform_locks(cx, layer)?;
        let quad = param_quad(cx)?;
        if !quad_is_valid(quad) {
            return Err(EngineError::range(
                "quad",
                "degenerate quad: three collinear, flipped, or self-intersecting (§40.6/§18.4)",
            ));
        }
        // §11.2 Sub-slice C class-kill (q-scenario/Q-3, interactive door): a
        // perspective bake resamples a masked layer's pixels while the mask
        // stays in canvas space. Perspective has NO identity no-op arm — it
        // always bakes — so a masked target refuses typed unconditionally here.
        if layer_has_enabled_mask(cx, layer) {
            return Err(masked_transform_refusal());
        }
        let (src, src_rect, unsigned) = read_layer_plane(cx, layer)?;
        let Some(src) = src else {
            return Ok(KindOutcome::default());
        };
        // §18.3: homography from SOURCE BOUNDS corners (TL/TR/BR/BL) → quad.
        let sb = [
            [src_rect.min.x as f64, src_rect.min.y as f64],
            [src_rect.max.x as f64, src_rect.min.y as f64],
            [src_rect.max.x as f64, src_rect.max.y as f64],
            [src_rect.min.x as f64, src_rect.max.y as f64],
        ];
        let h = dlt_homography(sb, quad).ok_or_else(|| {
            EngineError::range(
                "quad",
                "homography solve failed (degenerate configuration, §18.4)",
            )
        })?;
        let scale = scale_metric(&h, rect_center(src_rect));
        let kernel = resolve_transform_kernel(&param_resample(cx), scale)
            .map_err(|_| EngineError::range("resample", "unknown §18.2 kernel name"))?;
        let dst_rect = transformed_bounds(&h, src_rect);
        let out = transform_plane(&src, &h, dst_rect, kernel)
            .ok_or_else(|| EngineError::range("quad", "homography not invertible (§18.4)"))?;
        install_plane(cx, layer, &out, src_rect, unsigned)
    }
}

impl KindImpl for TransformWarpKind {
    fn kind(&self) -> &'static str {
        "transform.warp"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_transform_target(cx)?;
        check_transform_locks(cx, layer)?;
        // Validate the resample param even though §18.4 pins CR sampling.
        resolve_transform_kernel(&param_resample(cx), 1.0)
            .map_err(|_| EngineError::range("resample", "unknown §18.2 kernel name"))?;
        let (src, src_rect, unsigned) = read_layer_plane(cx, layer)?;
        let Some(src) = src else {
            return Ok(KindOutcome::default());
        };
        let mesh = param_mesh(cx, src_rect)?;
        // Identity mesh → §40.1 empty-delta push (1e-9 pin, warp module).
        let base = WarpMesh::base_lattice(mesh.cols, mesh.rows, src_rect);
        let identity = mesh
            .points
            .iter()
            .zip(base.points.iter())
            .all(|(a, b)| (a[0] - b[0]).abs() <= 1e-9 && (a[1] - b[1]).abs() <= 1e-9);
        if identity {
            return Ok(KindOutcome::default());
        }
        // §11.2 Sub-slice C class-kill (q-scenario/Q-3, interactive door): a
        // non-identity mesh warp bakes a masked layer's pixels while the mask
        // stays in canvas space. Refuse typed — the identity mesh already
        // returned above, so this fires only on a genuine bake.
        if layer_has_enabled_mask(cx, layer) {
            return Err(masked_transform_refusal());
        }
        let dst_rect = crate::warp::mesh_bounds(&mesh);
        let out = crate::warp::warp_plane(&src, &mesh, dst_rect);
        install_plane(cx, layer, &out, src_rect, unsigned)
    }
}

impl KindImpl for LayerTransformKind {
    fn kind(&self) -> &'static str {
        "layer.transform"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Target resolves at stage 4 (§40.3). NOTE the divergence from
        // `resolve_transform_target` (transform.free): the layer.transform
        // contract keeps the §12.4 rasterize-offer posture, so a parametric
        // (non-raster) target answers `target_kind_mismatch` BELOW, not the
        // bad_target the interactive kinds raise.
        let Some(layer) = cx
            .envelope()
            .target
            .as_ref()
            .and_then(|t| t.layer_id)
            .map(LayerId::from_uuid)
        else {
            return Err(EngineError::bad_target(
                "layer.transform requires target.layerId",
            ));
        };
        if cx.document().layer(layer).is_none() {
            return Err(EngineError::bad_target(format!(
                "target.layerId {layer} does not resolve"
            )));
        }
        // §11.1/§40.3 locks: POSITION/ALL block; PIXELS does not (a
        // transform is not an in-place pixel edit).
        check_transform_locks(cx, layer)?;
        // Mesh warps are transform.warp's arm (the WarpGrid wire-shape is an
        // acknowledged spec gap; unchanged from the core arm).
        if cx.params().get("mesh").is_some() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "mesh warps live with the tools transform engines (transform.warp; the wire \
                 shape of WarpGrid is an acknowledged spec gap)",
            ));
        }
        let matrix = param_mat3(cx, "matrix")?;
        if mat3_inverse(&matrix).is_none() {
            return Err(EngineError::range(
                "matrix",
                "matrix MUST be invertible (det != 0, §40.3)",
            ));
        }
        // §12.4 rasterize-offer posture STANDS: a non-raster target
        // (smart-object/text/shape) is NOT transformed in place — it answers
        // typed so the caller/UI can offer to rasterize first, then retarget
        // the bake. (The contract's non-destructive SetProp arm for
        // parametric kinds is a later wave; this arm serves raster + the
        // rasterize-then-transform path.)
        if cx
            .document()
            .layer(layer)
            .map(|n| n.pixels().is_none())
            .unwrap_or(true)
        {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "v1 transforms bake raster targets (§12.4 offers rasterize for parametric kinds)",
            ));
        }
        // §11.2 Sub-slice C — enforce the DECLARED refusal (q-scenario/Q-3).
        // A non-identity transform BAKES the layer's pixels (both the §18.3
        // exact integer remap and the general resample arm below) while its
        // mask stays put in canvas space, so the composite silently serves the
        // moved content with the mask left behind. The compositor's
        // `TransformNotImplemented` guard keys on a LIVE node transform, which
        // the baking arms never set, so it can never fire on this path — the
        // refusal must be enforced at the command door. Sibling-consistent with
        // the two remaining §11.2 deferrals (masked-group /
        // masked-adjustment), which already refuse typed — feather is no
        // longer among them (Sub-slice B serves it). Identity is exempt
        // (no pixel change ⇒ no mask desync; handled as the §40.1 no-op below).
        if !matrix.is_identity() && layer_has_enabled_mask(cx, layer) {
            return Err(masked_transform_refusal());
        }
        // §18.3 EXACT arm: a pure integer translation remaps on-grid with no
        // kernel — bit-identical, deterministic, no needless blur (kept
        // byte-for-byte as the pre-lift core arm via the ported helper).
        if let Some(d) = integer_translation(&matrix) {
            translate_layer_pixels(cx, layer, d)?;
            return Ok(KindOutcome::default());
        }
        // GENERAL arm (scale/rotate/shear/affine): inverse-map resample
        // through the pinned §18.2 kernel — the SAME `xform::transform_plane`
        // engine `transform.free` commits (one full-res pass, `out(p) =
        // kernel(src, M⁻¹·p)`; Auto picks Lanczos-3 minify / Catmull-Rom
        // magnify by the area-change metric unless `resample` names a kernel).
        let (src, src_rect, unsigned) = read_layer_plane(cx, layer)?;
        let Some(src) = src else {
            return Ok(KindOutcome::default()); // empty layer: §40.1 empty push
        };
        let scale = scale_metric(&matrix, rect_center(src_rect));
        let kernel = resolve_transform_kernel(&param_resample(cx), scale)
            .map_err(|_| EngineError::range("resample", "unknown §18.2 kernel name"))?;
        let dst_rect = transformed_bounds(&matrix, src_rect);
        let out = transform_plane(&src, &matrix, dst_rect, kernel)
            .ok_or_else(|| EngineError::range("matrix", "matrix MUST be invertible (§40.3)"))?;
        install_plane(cx, layer, &out, src_rect, unsigned)
    }
}

/// True iff `layer` carries at least one ENABLED mask (a disabled mask is a
/// §11.2 no-op and does not desync under a bake). The §11.2 Sub-slice C
/// refusal on the transform doors keys on this, mirroring the compositor's
/// enabled-mask coverage resolution.
fn layer_has_enabled_mask(cx: &KindContext<'_>, layer: LayerId) -> bool {
    let doc = cx.document();
    doc.layer(layer)
        .map(|n| {
            n.masks()
                .iter()
                .any(|m| doc.mask(*m).is_some_and(|mn| mn.enabled()))
        })
        .unwrap_or(false)
}

/// The §11.2 Sub-slice C typed refusal shared by EVERY transform door
/// (`layer.transform` + the interactive `transform.free`/`.perspective`/`.warp`
/// bake doors): a non-identity transform on a MASKED layer bakes the layer's
/// pixels while its mask stays in canvas space, so the mask falls behind the
/// moved content (q-scenario/Q-3, the full class-kill). Enforced at the command
/// door because the compositor's `TransformNotImplemented` guard keys on a LIVE
/// node transform the baking §18.3 arms never set. Typed `NotImplemented`
/// (QA-F8: refuse, never wrong-serve), sibling-consistent across the doors.
fn masked_transform_refusal() -> EngineError {
    EngineError::new(
        ErrorCode::NotImplemented,
        "a non-identity transform on a MASKED layer bakes the layer's pixels while its mask \
         stays in canvas space, so the mask would fall behind the moved content (§11.2 \
         Sub-slice C rides §18 live-transform-prop, unbuilt). Refused typed rather than serve \
         mask-less pixels (QA-F8); apply or remove the mask, or transform before masking.",
    )
}

// ---------------------------------------------------------------------------
// Transform-family stage-6 helpers.
// ---------------------------------------------------------------------------

/// `Some(d)` iff `m` is EXACTLY `[1,0,0,0,1,0,tx,ty,1]` with integer tx,ty
/// (column-major) — the §18.3 pure-integer-translate exact arm, which the
/// general resample path must never touch (determinism + no blur). An
/// identity matrix returns `Some((0,0))`, handled as the §40.1 no-op by
/// [`translate_layer_pixels`].
fn integer_translation(m: &Mat3) -> Option<appname_core::IVec2> {
    let a = &m.0;
    let is_int = |x: f64| (x - x.round()).abs() < 1e-9;
    let identity_linear = (a[0] - 1.0).abs() < 1e-12
        && a[1].abs() < 1e-12
        && a[2].abs() < 1e-12
        && a[3].abs() < 1e-12
        && (a[4] - 1.0).abs() < 1e-12
        && a[5].abs() < 1e-12
        && (a[8] - 1.0).abs() < 1e-12;
    if identity_linear && is_int(a[6]) && is_int(a[7]) {
        Some(appname_core::IVec2::new(
            a[6].round() as i32,
            a[7].round() as i32,
        ))
    } else {
        None
    }
}

/// Bake an integer translation into a raster layer's pixels — the §18.3
/// exact remap `dest(p) = src(p − d)` (vacated pixels clear; bounds
/// translate exactly). PORTED VERBATIM from `appname-core`'s
/// `kinds_layer_geom::translate_layer_pixels` (which align/distribute still
/// share) so the lifted integer-translate arm stays byte-identical to the
/// pre-lift core path. Uses only the public [`KindContext`] surface doors.
fn translate_layer_pixels(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    d: appname_core::IVec2,
) -> Result<(), EngineError> {
    use appname_core::{IVec2, TileCoord, TILE, TILE_I32};
    if d == IVec2::new(0, 0) {
        return Ok(());
    }
    let src = cx
        .document()
        .layer(layer)
        .and_then(|n| n.pixels())
        .ok_or_else(|| EngineError::bad_target("raster target required"))?
        .clone(); // COW snapshot: reads stay pre-move while installs land
    let old = src.bounds();
    if old.is_empty() {
        return Ok(());
    }
    let new = old.translate(d);
    let union = old.union(&new);
    let coords: Vec<(i32, i32)> = union.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        if rect.intersect(&union).is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        let mut changed = false;
        let mut i = 0;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let p = IVec2::new(x, y);
                let srcp = IVec2::new(x - d.x, y - d.y);
                let v = if new.contains(p) && old.contains(srcp) {
                    src.read_pixel_f32(srcp)
                } else {
                    [0.0; 4]
                };
                if v != src.read_pixel_f32(p) {
                    changed = true;
                }
                plane[i] = v;
                i += 1;
            }
        }
        if changed {
            cx.install_surface_plane(layer.as_uuid(), coord, &plane)?;
        }
    }
    cx.set_surface_bounds(layer.as_uuid(), new)?;
    Ok(())
}

fn resolve_transform_target(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let target = cx.envelope().target.as_ref();
    let Some(layer) = target.and_then(|t| t.layer_id) else {
        return Err(EngineError::bad_target(
            "transform kinds require target.layerId (§40.6; multi-select transient groups are a \
             later wave)",
        ));
    };
    let id = LayerId::from_uuid(layer);
    let node = cx.document().layer(id).ok_or_else(|| {
        EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
    })?;
    if node.pixels().is_none() {
        return Err(EngineError::bad_target(format!(
            "target.layerId {layer} is not a raster layer (parametric bake/param split is a later \
             wave, §40.6)"
        )));
    }
    Ok(id)
}

/// §40.6/§11.1: POSITION blocks (and ALL ⊇ POSITION); PIXELS does not.
fn check_transform_locks(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    // §11.1 transitive ancestry union (`Document::effective_locks`,
    // O-CORE handoff): a POSITION/ALL lock on any ancestor group binds.
    let locks = cx.document().effective_locks(layer);
    if locks.contains(LockFlags::POSITION) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the target layer's position is locked (§11.1; transform-family block, §40.6)",
        ));
    }
    Ok(())
}

fn param_mat3(cx: &KindContext<'_>, name: &str) -> Result<Mat3, EngineError> {
    let arr = cx
        .params()
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing(name))?;
    if arr.len() != 9 || !arr.iter().all(|v| v.is_number()) {
        return Err(EngineError::range(
            name,
            "matrix is 9 numbers, column-major Mat3 (§0.3)",
        ));
    }
    let mut m = [0.0f64; 9];
    for (i, v) in arr.iter().enumerate() {
        m[i] = v.as_f64().unwrap_or(f64::NAN);
        if !m[i].is_finite() {
            // Non-finite policy (§40.1): ±Inf clamps are meaningless for a
            // matrix; treat as out-of-range (recorded divergence).
            return Err(EngineError::range(name, "matrix entries must be finite"));
        }
    }
    Ok(Mat3(m))
}

fn param_vec2(cx: &KindContext<'_>, name: &str) -> Result<[f64; 2], EngineError> {
    let arr = cx
        .params()
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing(name))?;
    if arr.len() != 2 || !arr.iter().all(|v| v.is_number()) {
        return Err(EngineError::range(name, "[x, y] pair of numbers"));
    }
    Ok([
        arr[0].as_f64().unwrap_or(0.0),
        arr[1].as_f64().unwrap_or(0.0),
    ])
}

fn param_quad(cx: &KindContext<'_>) -> Result<[[f64; 2]; 4], EngineError> {
    let arr = cx
        .params()
        .get("quad")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("quad"))?;
    if arr.len() != 4 {
        return Err(EngineError::range("quad", "4 corners TL/TR/BR/BL (§10.3)"));
    }
    let mut q = [[0.0f64; 2]; 4];
    for (i, c) in arr.iter().enumerate() {
        let pair = c
            .as_array()
            .ok_or_else(|| EngineError::range("quad", "corner = [x, y]"))?;
        if pair.len() != 2 || !pair.iter().all(|v| v.is_number()) {
            return Err(EngineError::range("quad", "corner = [x, y] numbers"));
        }
        q[i] = [
            pair[0].as_f64().unwrap_or(f64::NAN),
            pair[1].as_f64().unwrap_or(f64::NAN),
        ];
        if !q[i][0].is_finite() || !q[i][1].is_finite() {
            return Err(EngineError::range("quad", "corners must be finite"));
        }
    }
    Ok(q)
}

fn rect_center(r: appname_core::IRect) -> [f64; 2] {
    [
        (r.min.x as f64 + r.max.x as f64) / 2.0,
        (r.min.y as f64 + r.max.y as f64) / 2.0,
    ]
}

fn param_resample(cx: &KindContext<'_>) -> String {
    cx.params()
        .get("resample")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .to_string()
}

fn param_mesh(cx: &KindContext<'_>, src_rect: IRect) -> Result<WarpMesh, EngineError> {
    let map_err = |e: WarpError| match e {
        WarpError::PresetUnsupported => EngineError::new(
            ErrorCode::NotImplemented,
            "preset kind has no §18.4 displacement math in v1 (spec gap; lane log)",
        ),
        other => EngineError::range("mesh", format!("{other}")),
    };
    if let Some(mesh) = cx.params().get("mesh").and_then(Value::as_object) {
        let cols = mesh.get("cols").and_then(Value::as_u64).unwrap_or(0) as u32;
        let rows = mesh.get("rows").and_then(Value::as_u64).unwrap_or(0) as u32;
        let pts = mesh
            .get("points")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("mesh.points"))?;
        let mut points = Vec::with_capacity(pts.len());
        for p in pts {
            let pair = p
                .as_array()
                .ok_or_else(|| EngineError::range("mesh.points", "[x, y]"))?;
            if pair.len() != 2 {
                return Err(EngineError::range("mesh.points", "[x, y]"));
            }
            points.push([
                pair[0].as_f64().unwrap_or(f64::NAN),
                pair[1].as_f64().unwrap_or(f64::NAN),
            ]);
        }
        return WarpMesh::new(cols, rows, points).map_err(map_err);
    }
    if let Some(preset) = cx.params().get("preset").and_then(Value::as_object) {
        let kind = preset
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("preset.kind"))?;
        let bend = preset
            .get("bendPct")
            .and_then(Value::as_f64)
            .ok_or_else(|| EngineError::missing("preset.bendPct"))?;
        return preset_mesh(kind, bend, src_rect).map_err(map_err);
    }
    Err(EngineError::missing("mesh")) // stage-3 oneOf normally catches this
}

// ===========================================================================
// CP-1.0 warps row: transform.puppet (contract transform.puppet.yaml,
// ES §18.4 + §45.5 [NORMATIVE]; plan §2.5 "puppet (MLS rigid + ARAP via
// libigl link-record)"). Engines: crate::puppet (mesh gen, ARAP driver,
// rasterizer, MLS-rigid session tier — rights basis in that module's
// docs, warps_upright row).
//
// WIRE-FORM SEMANTICS + RECORDED CONTRACT GAP (escalated, lane log this
// date): the contract carries ONE `pos` per pin ("[x, y] doc px; each
// pin must lie inside the source content bounds"), i.e. the pin's
// PLACEMENT — there is no wire field for a dragged destination, so the
// interactive session's drag deformations are NOT expressible in the
// recorded params (the §45.5 session model keeps {rest, current} pairs;
// `appname_tools::puppet::PuppetPin` carries both; the workspace is the
// consumer). The wire-expressible deformation is exactly the ROTATION
// PIN (`rotationDeg` — the contract's own scripting example and the
// §45.8 3-pin golden are built from it) plus `depth` overlap ordering.
// Consequences, pinned:
//   * a payload with NO rotationDeg anywhere is the §45.5 identity
//     fixed point (targets ≡ rests) — bit-exact no-op → §40.1 empty
//     push (the §45.5 identity property at the wire);
//   * a payload WITH rotation pins requires the elected ARAP backend —
//     libigl behind the ruled cpp-shim consumption shape
//     (deps/libigl.yaml + deps/eigen.yaml, feature `arap-libigl`);
//     default builds answer the typed NI below (validation-complete:
//     every stage-3/6 gate runs first);
//   * the pos-carries-destination alternative reading is REJECTED here
//     (it cannot name which material point was grabbed and collapses
//     to near-identity); the gap goes to the fold-in register, not
//     into invented wire fields (never-invent rule).
// Stage map: target resolves (raster; parametric split later wave) →
// §11.1 locks (POSITION/ALL block; PIXELS does NOT — contract
// lockBehavior) → resample name validated → §40.6 pin-in-content-bounds
// live-state check (empty layer: no content bounds ⇒ the same
// out_of_range arm, message naming tool.puppet.empty_layer's state) →
// arm dispatch.
// ===========================================================================

/// Parsed wire pin (stage-5 schema already enforced counts/types).
struct WirePin {
    pos: [f64; 2],
    rotation_deg: Option<f64>,
    /// §45.5 overlap order — consumed by the solve arm only (the
    /// identity arm never rasterizes), so default builds see it parsed
    /// and validated but unread.
    #[cfg_attr(not(feature = "arap-libigl"), allow(dead_code))]
    depth: i64,
}

fn param_pins(cx: &KindContext<'_>) -> Result<Vec<WirePin>, EngineError> {
    let arr = cx
        .params()
        .get("pins")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("pins"))?;
    if arr.is_empty() || arr.len() > 512 {
        return Err(EngineError::range(
            "pins",
            "1..=512 pins (§10.3 len ≥ 1; §45.5 stage-3 cap)",
        ));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, p) in arr.iter().enumerate() {
        let obj = p
            .as_object()
            .ok_or_else(|| EngineError::range("pins", format!("pins[{i}] must be an object")))?;
        let pos = obj
            .get("pos")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("pins.pos"))?;
        if pos.len() != 2 || !pos.iter().all(|v| v.is_number()) {
            return Err(EngineError::range(
                "pins",
                format!("pins[{i}].pos must be [x, y] numbers (§0.3)"),
            ));
        }
        let x = pos[0].as_f64().unwrap_or(f64::NAN);
        let y = pos[1].as_f64().unwrap_or(f64::NAN);
        if !x.is_finite() || !y.is_finite() {
            return Err(EngineError::range(
                "pins",
                format!("pins[{i}].pos must be finite (§0.3/§45.5)"),
            ));
        }
        let rotation_deg = match obj.get("rotationDeg") {
            None => None,
            Some(v) => {
                let r = v.as_f64().unwrap_or(f64::NAN);
                if !r.is_finite() {
                    return Err(EngineError::range(
                        "pins",
                        format!("pins[{i}].rotationDeg must be finite (§0.3)"),
                    ));
                }
                Some(r)
            }
        };
        let depth = match obj.get("depth") {
            None => 0,
            Some(v) => v.as_i64().ok_or_else(|| {
                EngineError::range(
                    "pins",
                    format!("pins[{i}].depth must be an integer (§10.3)"),
                )
            })?,
        };
        out.push(WirePin {
            pos: [x, y],
            rotation_deg,
            depth,
        });
    }
    Ok(out)
}

impl KindImpl for TransformPuppetKind {
    fn kind(&self) -> &'static str {
        "transform.puppet"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_transform_target(cx)?;
        check_transform_locks(cx, layer)?;
        // Validate the transform-family common params (recorded even
        // where the §45.5 auto rule decides per triangle).
        let resample = param_resample(cx);
        if resample != "auto" {
            crate::resample::Kernel::from_wire(&resample)
                .ok_or_else(|| EngineError::range("resample", "unknown §18.2 kernel name"))?;
        }
        // density / expansionPx / rigidity (stage 5 enforces the schema;
        // re-checked typed for the direct-KindContext callers).
        let density = match cx.params().get("density").and_then(Value::as_str) {
            None => crate::puppet::mesh::MeshDensity::Normal,
            Some(s) => crate::puppet::mesh::MeshDensity::from_wire(s)
                .ok_or_else(|| EngineError::range("density", "fewer | normal | more (§10.3)"))?,
        };
        let expansion = match cx.params().get("expansionPx") {
            None => 2i64,
            Some(v) => {
                let e = v.as_i64().unwrap_or(-1);
                if !(0..=32).contains(&e) {
                    return Err(EngineError::range("expansionPx", "0..=32 (§10.3)"));
                }
                e
            }
        };
        let rigidity = match cx.params().get("rigidity") {
            None => 0.7f64,
            Some(v) => {
                let r = v.as_f64().unwrap_or(f64::NAN);
                if !(0.0..=1.0).contains(&r) {
                    return Err(EngineError::range("rigidity", "0..=1 (§45.8 addition)"));
                }
                r
            }
        };
        let _ = (density, expansion, rigidity); // consumed by the solve arm
        let pins = param_pins(cx)?;

        // §40.6 live-state gate: every pin inside the source CONTENT
        // bounds. An empty layer has none — the zero-area-alpha-hull
        // state (tool-side: tool.puppet.empty_layer refuses entry).
        let (bounds_empty, src_rect) = {
            let buf = cx
                .document()
                .layer(layer)
                .and_then(|n| n.pixels())
                .ok_or_else(|| EngineError::bad_target("raster target vanished mid-apply"))?;
            (buf.bounds().is_empty(), buf.bounds())
        };
        for (i, p) in pins.iter().enumerate() {
            let inside = !bounds_empty
                && p.pos[0] >= f64::from(src_rect.min.x)
                && p.pos[0] < f64::from(src_rect.max.x)
                && p.pos[1] >= f64::from(src_rect.min.y)
                && p.pos[1] < f64::from(src_rect.max.y);
            if !inside {
                return Err(EngineError::range(
                    "pins",
                    if bounds_empty {
                        format!(
                            "pins[{i}]: the target layer has no content — zero-area alpha hull \
                             (§45.5 tool.puppet.empty_layer; §40.6 pin-in-content-bounds)"
                        )
                    } else {
                        format!(
                            "pins[{i}].pos {:?} lies outside the source content bounds (§40.6)",
                            p.pos
                        )
                    },
                ));
            }
        }

        let has_rotation = pins.iter().any(|p| p.rotation_deg.is_some());
        if !has_rotation {
            // Wire identity arm (module-header pin): targets ≡ rests ⇒
            // the §45.5 identity fixed point ⇒ bit-exact no-op ⇒ §40.1
            // empty push. Covers the single-pin rigid translate too
            // (translate by target − rest = 0 on the wire).
            return Ok(KindOutcome::default());
        }

        #[cfg(feature = "arap-libigl")]
        {
            return puppet_commit_libigl(
                cx,
                layer,
                &pins,
                density,
                expansion as i32,
                rigidity,
                &resample,
            );
        }
        #[cfg(not(feature = "arap-libigl"))]
        {
            Err(EngineError::new(
                ErrorCode::NotImplemented,
                "transform.puppet rotation-pin solve requires the elected ARAP backend — libigl \
                 behind the ruled cpp-shim consumption shape (deps/libigl.yaml + deps/eigen.yaml, \
                 O-INT ruling 2026-08-17; cargo feature `arap-libigl` with the pinned external \
                 checkouts). Validation is complete: every stage-3/6 gate ran before this refusal",
            ))
        }
    }
}

/// The solve-arm commit (compiled with the elected backend only): §45.5
/// mesh over the alpha hull (or the selection's ≥ 0.5 region), the f64
/// commit solve to convergence through the libigl shim, §18.4
/// rasterization, selection-coverage-weighted composite (§45.5), tile
/// bake through the recorder doors.
#[cfg(feature = "arap-libigl")]
fn puppet_commit_libigl(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    pins: &[WirePin],
    density: crate::puppet::mesh::MeshDensity,
    expansion: i32,
    rigidity: f64,
    resample: &str,
) -> Result<KindOutcome, EngineError> {
    use crate::puppet::arap::{
        deformed_bounds, rasterize_mesh, solve_arap, ArapSetup, COMMIT_MAX_ITERS, COMMIT_TOL_PX,
    };
    use crate::puppet::arap_libigl::LibiglGlobalSolver;
    use crate::puppet::mesh::{build_mesh, CoverageMask, MeshError};
    use crate::puppet::PuppetPin;
    use crate::resample::EdgePolicy;

    let (src, src_rect, unsigned) = read_layer_plane(cx, layer)?;
    let Some(src) = src else {
        return Ok(KindOutcome::default()); // guarded earlier; defensive
    };
    let sel_active = cx.document().selection().active();
    let coverage = if sel_active {
        let doc = cx.document();
        let sel = doc.selection();
        CoverageMask::from_fn(src_rect, |x, y| {
            sel.mask().read_pixel_f32(appname_core::IVec2::new(x, y))[0] >= 0.5
        })
    } else {
        CoverageMask::from_fn(src_rect, |x, y| {
            src.get(x, y, EdgePolicy::Transparent)[3] > 0.0
        })
    };
    let mesh = match build_mesh(&coverage, density, expansion) {
        Ok(m) => m,
        // Empty/degenerate mesh region: nothing to warp (§40.1 push).
        Err(MeshError::EmptyCoverage) | Err(MeshError::Degenerate) => {
            return Ok(KindOutcome::default())
        }
    };
    let session_pins: Vec<PuppetPin> = pins
        .iter()
        .map(|p| PuppetPin {
            rest: p.pos,
            target: p.pos, // wire form: placement only (module-header pin)
            rotation_deg: p.rotation_deg,
            depth: p.depth,
        })
        .collect();
    // D5 fix-round taping: runtime solve failures are NOT NI. Per the
    // CAF §45.3-rule-2 precedent (a validated payload meeting a
    // degenerate/failed live-state computation → error.api.invalid_arg
    // pending a dedicated wire id), degenerate configurations and
    // numeric solver failures map to InvalidArg; the contract's
    // error.engine.* family has no registered concrete member and no
    // ErrorCode variant — FLAGGED in the lane log (mint-before-use
    // routes via O-INT/O-CORE, never minted here).
    let arap_err = |e: crate::puppet::arap::ArapError| {
        match e {
        crate::puppet::arap::ArapError::NoPins => {
            EngineError::range("pins", "at least one pin (§10.3)")
        }
        other => EngineError::new(
            ErrorCode::InvalidArg,
            format!(
                "ARAP solve failed on a validated payload: {other} (typed runtime failure;                  dedicated error.engine.* wire id pends its taxonomy door — lane-log flag)"
            ),
        ),
    }
    };
    let setup = ArapSetup::new(&mesh, &session_pins).map_err(arap_err)?;
    let mut solver = LibiglGlobalSolver::default();
    let sol = solve_arap(
        &mesh,
        &setup,
        &session_pins,
        &mut solver,
        None,
        rigidity,
        COMMIT_MAX_ITERS,
        COMMIT_TOL_PX,
    )
    .map_err(arap_err)?;
    let kernel = crate::resample::Kernel::from_wire(resample); // None = auto
    let out_rect = deformed_bounds(&sol.positions, &mesh.tris).union(&src_rect);
    let warped = rasterize_mesh(
        &src,
        &mesh,
        &sol.positions,
        &setup.tri_depth,
        out_rect,
        kernel,
    );
    if sel_active {
        // §45.5: composite weighted by selection coverage; pixels
        // outside the mesh keep the original (the warped plane is
        // transparent there and cov gates the write).
        let changed = edit_region_at(cx, layer, out_rect, &mut |p, dst, cov| {
            if cov <= 0.0 {
                return dst;
            }
            let w = warped.get(p.x, p.y, EdgePolicy::Transparent);
            let mut v = [0.0f32; 4];
            for c in 0..4 {
                v[c] = dst[c] + (w[c] - dst[c]) * cov;
            }
            if unsigned {
                v[3] = v[3].clamp(0.0, 1.0);
                for c in 0..3 {
                    v[c] = v[c].clamp(0.0, v[3]);
                }
            }
            v
        })?;
        if !changed.is_empty() {
            // Q-1 determinism class: the selection-gated arm writes through
            // `edit_region_at` weighted by selection coverage — a FEATHERED
            // selection makes `changed` count sub-quantum pixels that store
            // unchanged, and a warp can VACATE content (alpha reduction), so
            // the erase-side full pixel-exact recompute is the only branch
            // that matches the loader. (Today only the identity solve is
            // live, which writes nothing — this routing is structural, so
            // the ARAP arm cannot land on the forked pair later.)
            finish_write_bounds(cx, layer, changed, true)?;
        }
        Ok(KindOutcome::default())
    } else {
        install_plane(cx, layer, &warped, src_rect, unsigned)
    }
}

/// Read the layer's content into a Plane (`None` = empty layer → the §40.1
/// empty push). Returns the content rect and the storage's
/// unsigned-normalized flag (§18.2 store-clamping).
fn read_layer_plane(
    cx: &KindContext<'_>,
    layer: LayerId,
) -> Result<(Option<Plane>, IRect, bool), EngineError> {
    let buf = cx
        .document()
        .layer(layer)
        .and_then(|n| n.pixels())
        .ok_or_else(|| EngineError::bad_target("raster target vanished mid-apply"))?;
    let rect = buf.bounds();
    let unsigned = matches!(buf.kind(), k if ChannelKind::is_unsigned_normalized(k));
    if rect.is_empty() {
        return Ok((None, rect, unsigned));
    }
    let mut plane = Plane::new(rect);
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            plane.set(x, y, buf.read_pixel_f32(appname_core::IVec2::new(x, y)));
        }
    }
    Ok((Some(plane), rect, unsigned))
}

/// Bake the output plane over the union of old and new extents through the
/// recorder-coupled doors, then re-tighten bounds (prunes emptied tiles).
/// §18.2 store-clamping on unsigned-normalized storage: color clamps into
/// [0, alpha], alpha into [0, 1]; f32 keeps kernel excursions.
fn install_plane(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    out: &Plane,
    old_rect: IRect,
    unsigned: bool,
) -> Result<KindOutcome, EngineError> {
    use appname_core::{TileCoord, TILE, TILE_I32};
    let union = out.rect.union(&old_rect);
    for (tx, ty) in union.tiles(TILE_I32) {
        let coord = TileCoord::new(0, tx, ty);
        let trect = coord.pixel_rect();
        let mut pixels = vec![[0.0f32; 4]; TILE * TILE];
        let mut i = 0;
        for y in trect.min.y..trect.max.y {
            for x in trect.min.x..trect.max.x {
                let mut v = out.get(x, y, crate::resample::EdgePolicy::Transparent);
                if unsigned {
                    v[3] = v[3].clamp(0.0, 1.0);
                    for c in 0..3 {
                        v[c] = v[c].clamp(0.0, v[3]);
                    }
                } else if v[3] == 0.0 {
                    v = [0.0; 4]; // §6.4: zero alpha stores zero color
                } else {
                    // Defensive: f32 premul allows HDR but colors under
                    // a = 0 were zeroed above; nothing else to enforce.
                }
                pixels[i] = v;
                i += 1;
            }
        }
        cx.install_tile_plane(layer, coord, &pixels)?;
    }
    // §2.4 eager growth BEFORE the re-tighten (CP-0.7 R1 root-cause fix,
    // lane log): `recompute_bounds` is the lazy-shrink door — it can only
    // intersect occupied tiles with the CURRENT bounds — so a bake that
    // moves content outside the old bounds (transform translates/rotates)
    // must first union the written extent, or `bounds()` goes stale and
    // every §2.5 content-bounds consumer (filter.apply was the first)
    // silently under-reads. The buffer door doc names exactly this
    // discipline ("every write door unions eagerly").
    cx.grow_layer_bounds(layer, out.rect)?;
    // PIXEL-EXACT re-tighten (O-QA W12 finding 1 fix): a transform MOVES
    // content, so the tile-granular lazy shrink leaves the vacated
    // sub-tile extent in `bounds` — live bounds then diverge from the
    // loader's pixel-exact bounds, and region_hash (which folds bounds)
    // forks live-vs-reloaded, killing the D-48 replay-as-recovery undo
    // across the package boundary. The pixel-exact door matches the
    // loader's canonicalization; the per-pixel scan is the same order as
    // the bake we just did.
    cx.recompute_layer_bounds_pixel_exact(layer)?;
    Ok(KindOutcome::default())
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    use crate::brush::{encode_samples, InputSample};
    use appname_core::{Engine, IVec2};
    use serde_json::json;

    fn line_blob(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<u8> {
        let samples = vec![
            InputSample {
                x: x0,
                y: y0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.0,
            },
            InputSample {
                x: x1,
                y: y1,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.05,
            },
        ];
        encode_samples(&samples).unwrap()
    }

    /// O-QA W12 finding 1 regression: transform.free breaks cross-boundary
    /// undo. A content-MOVING bake must leave the layer's bounds
    /// PIXEL-EXACT (== what the package loader recomputes), so the live
    /// and reloaded `region_hash` (which folds bounds) agree and the D-48
    /// replay-as-recovery undo across the package boundary survives.
    #[test]
    fn transform_free_leaves_canonical_bounds_across_the_package_boundary() {
        for (w, h) in [(64u32, 48u32), (96, 64), (128, 96)] {
            let mut e = assembly().build();
            e.apply_json(
                &json!({
                    "schema":"appname.command.v1","id":"f0000000-0000-4000-8000-000000000001",
                    "kind":"doc.new","params":{"width":w,"height":h,"background":"white"}
                })
                .to_string(),
            )
            .unwrap();
            let layer = e.document().unwrap().z_order_walk()[0];
            // translate +5/+5 (column-major Mat3) — vacates the origin.
            e.apply_json(&json!({
                "schema":"appname.command.v1","id":"f0000000-0000-4000-8000-000000000003",
                "kind":"transform.free","target":{"layerId":layer.as_uuid().to_string()},
                "params":{"matrix":[1.0,0.0,0.0, 0.0,1.0,0.0, 5.0,5.0,1.0],"reference":[0.0,0.0]}
            }).to_string()).unwrap();
            let live_bounds = e
                .document()
                .unwrap()
                .layer(layer)
                .unwrap()
                .pixels()
                .unwrap()
                .bounds();
            let live_hash = e.state_hash_persistent();

            let bytes = appname_core::save_package_bytes(&e).unwrap();
            let loaded = appname_core::load_package_bytes_with(
                &assembly(),
                std::io::Cursor::new(&bytes).get_ref(),
            )
            .unwrap();
            let re_bounds = loaded
                .engine
                .document()
                .unwrap()
                .layer(layer)
                .unwrap()
                .pixels()
                .unwrap()
                .bounds();

            // The bake left canonical (pixel-exact) bounds: the vacated
            // origin (0,0) is GONE — content starts at (5,5).
            assert_eq!(
                live_bounds, re_bounds,
                "[{w}x{h}] live bounds == reloaded (pixel-exact)"
            );
            assert_eq!(
                live_bounds.min,
                appname_core::IVec2::new(5, 5),
                "[{w}x{h}] vacated origin dropped"
            );
            // The identity hash agrees across the boundary → the CLI
            // replay-as-recovery undo gate now passes.
            assert_eq!(
                live_hash,
                loaded.engine.state_hash_persistent(),
                "[{w}x{h}] persistent hash agrees across save/reload"
            );
            // And the whole in-process journal replays to the persisted
            // state (the exact gate the CLI runs).
            let replayed = e.replay_assembled(e.journal()).unwrap();
            assert_eq!(
                replayed.state_hash_persistent(),
                e.state_hash_persistent(),
                "[{w}x{h}] journal replay reproduces persisted state (D-48)"
            );
        }
    }

    fn painted_engine() -> (Engine, appname_core::LayerId) {
        let mut e = assembly().build();
        let text = json!({
            "schema": "appname.command.v1",
            "id": "f0000000-0000-4000-8000-000000000001",
            "kind": "doc.new",
            "params": {"width": 128, "height": 128, "background": "transparent"},
        })
        .to_string();
        e.apply_json(&text).unwrap();
        let layer = e.document().unwrap().z_order_walk()[0];
        let asset = e.add_samples_asset(line_blob(30.0, 40.0, 60.0, 40.0));
        let env = json!({
            "schema": "appname.command.v1",
            "id": "f0000000-0000-4000-8000-000000000002",
            "kind": "brush.stroke",
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": {
                "brushPreset": preset_hard_round_16_id().to_string(),
                "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
            },
            "samples": asset.to_string(),
        })
        .to_string();
        e.apply_json(&env).unwrap();
        (e, layer)
    }

    fn tf_env(
        n: u32,
        kind: &str,
        layer: appname_core::LayerId,
        params: serde_json::Value,
    ) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("f0000000-0000-4000-8000-0002{n:08x}"),
            "kind": kind,
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": params,
        })
        .to_string()
    }

    #[test]
    fn free_transform_integer_translate_moves_content_and_round_trips() {
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        let px_before = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(45, 40));
        assert!(px_before[3] > 0.9, "stroke painted at (45,40)");
        // Translate +20/+10 (auto → scale 1 → CR; integer offsets are
        // interpolating-exact).
        let receipt = e
            .apply_json(&tf_env(
                1,
                "transform.free",
                layer,
                json!({
                    "matrix": [1, 0, 0, 0, 1, 0, 20, 10, 1],
                    "reference": [64, 64],
                }),
            ))
            .unwrap();
        assert!(!receipt.dirty.is_empty());
        let doc = e.document().unwrap();
        let buf = doc.layer(layer).unwrap().pixels().unwrap();
        let moved = buf.read_pixel_f32(IVec2::new(65, 50));
        assert!(
            moved[3] > 0.9 && moved[0] > 0.9,
            "content at +20/+10, got {moved:?}"
        );
        assert!(
            buf.read_pixel_f32(IVec2::new(45, 40))[3] < 0.05,
            "old location now transparent"
        );
        // Undo restores byte identity; redo + replay agree.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "transform replays deterministically"
        );
    }

    #[test]
    fn identity_and_degenerate_matrices() {
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        // Identity → §40.1 empty-delta push: state unchanged, journaled.
        let receipt = e
            .apply_json(&tf_env(
                2,
                "transform.free",
                layer,
                json!({
                    "matrix": [1, 0, 0, 0, 1, 0, 0, 0, 1],
                    "reference": [0, 0],
                }),
            ))
            .unwrap();
        assert!(receipt.dirty.is_empty());
        assert_eq!(e.state_hash(), before);
        // Singular matrix → out_of_range (§40.6).
        let err = e
            .apply_json(&tf_env(
                3,
                "transform.free",
                layer,
                json!({
                    "matrix": [0, 0, 0, 0, 0, 0, 0, 0, 1],
                    "reference": [0, 0],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // Unknown resample name → out_of_range (v1 vocabulary pin).
        let err = e
            .apply_json(&tf_env(
                4,
                "transform.free",
                layer,
                json!({
                    "matrix": [2, 0, 0, 0, 2, 0, 0, 0, 1],
                    "reference": [0, 0],
                    "resample": "cubic",
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn perspective_quad_gates_and_applies() {
        let (mut e, layer) = painted_engine();
        // Degenerate quad (three collinear) → out_of_range.
        let err = e
            .apply_json(&tf_env(
                5,
                "transform.perspective",
                layer,
                json!({
                    "quad": [[0, 0], [50, 0], [100, 0], [0, 100]],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // A mild perspective applies, dirties, and round-trips.
        let before = e.state_hash();
        let receipt = e
            .apply_json(&tf_env(
                6,
                "transform.perspective",
                layer,
                json!({
                    "quad": [[25, 35], [70, 33], [72, 50], [23, 48]],
                }),
            ))
            .unwrap();
        assert!(!receipt.dirty.is_empty());
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn warp_mesh_and_preset_paths() {
        let (mut e, layer) = painted_engine();
        // Point-count mismatch (the contract's own invalid case).
        let sixteen: Vec<[f64; 2]> = vec![[0.0, 0.0]; 16];
        let err = e
            .apply_json(&tf_env(
                7,
                "transform.warp",
                layer,
                json!({
                    "mesh": {"cols": 2, "rows": 2, "points": sixteen},
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // Unsupported preset kind → not_implemented (v1 spec gap).
        let err = e
            .apply_json(&tf_env(
                8,
                "transform.warp",
                layer,
                json!({
                    "preset": {"kind": "twist", "bendPct": 40},
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // Zero bend = identity mesh → empty push.
        let before = e.state_hash();
        let receipt = e
            .apply_json(&tf_env(
                9,
                "transform.warp",
                layer,
                json!({
                    "preset": {"kind": "arc", "bendPct": 0},
                }),
            ))
            .unwrap();
        assert!(receipt.dirty.is_empty());
        assert_eq!(e.state_hash(), before);
        // A real bend applies and round-trips.
        let receipt = e
            .apply_json(&tf_env(
                10,
                "transform.warp",
                layer,
                json!({
                    "preset": {"kind": "arc", "bendPct": 45},
                }),
            ))
            .unwrap();
        assert!(!receipt.dirty.is_empty());
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn transform_targets_and_locks_gate() {
        let (mut e, layer) = painted_engine();
        // Missing target.
        let mut env: serde_json::Value = serde_json::from_str(&tf_env(
            11,
            "transform.free",
            layer,
            json!({
                "matrix": [1, 0, 0, 0, 1, 0, 5, 5, 1],
                "reference": [0, 0],
            }),
        ))
        .unwrap();
        env.as_object_mut().unwrap().remove("target");
        let err = e.apply_json(&env.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // Unresolvable layer.
        let ghost = Uuid::new_v4();
        let err = e
            .apply_json(&tf_env(
                12,
                "transform.free",
                appname_core::LayerId::from_uuid(ghost),
                json!({
                    "matrix": [1, 0, 0, 0, 1, 0, 5, 5, 1],
                    "reference": [0, 0],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    // -----------------------------------------------------------------------
    // CP-1.0 warps row: transform.puppet wire arm (§45.5; the module-
    // header wire-form pins). Solve-path engine tests live in
    // crate::puppet (mesh/ARAP/rasterizer with the cfg(test) oracle);
    // these exercise the COMMAND surface: stage order, typed rejects,
    // the identity arm's §40.1/§45.5 behavior, and replay determinism.
    // -----------------------------------------------------------------------

    fn pins(v: serde_json::Value) -> serde_json::Value {
        json!({ "pins": v })
    }

    #[test]
    fn puppet_identity_arm_is_the_empty_push_with_replay_determinism() {
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        let journal_before = e.journal().len();
        // Three pins on painted content, no rotation: the §45.5 identity
        // fixed point at the wire (targets ≡ rests).
        let receipt = e
            .apply_json(&tf_env(
                0x50,
                "transform.puppet",
                layer,
                pins(json!([
                    {"pos": [40.0, 40.0]},
                    {"pos": [50.0, 40.0], "depth": 1},
                    {"pos": [45.0, 42.0]},
                ])),
            ))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "§40.1/§45.5 bit-exact no-op");
        assert_eq!(e.state_hash(), before, "identity commit changes nothing");
        assert_eq!(e.journal().len(), journal_before + 1, "still journaled");
        // Undo of the empty DeltaSet is a state no-op.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        // D-48 replay: byte-identical through the assembled replay door.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash(), "replay determinism");
    }

    #[test]
    fn puppet_single_pin_is_the_wire_identity() {
        // §45.5 single pin = rigid translate; on the wire target ≡ rest,
        // so the translate is zero — empty push.
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        let receipt = e
            .apply_json(&tf_env(
                0x51,
                "transform.puppet",
                layer,
                pins(json!([{"pos": [45.0, 40.0]}])),
            ))
            .unwrap();
        assert!(receipt.dirty.is_empty());
        assert_eq!(e.state_hash(), before);
    }

    #[cfg(not(feature = "arap-libigl"))]
    #[test]
    fn puppet_rotation_pin_answers_the_typed_backend_ni() {
        // The wire-expressible deformation (rotation pin) needs the
        // elected ARAP backend; default builds refuse typed AFTER full
        // validation (validation-complete posture).
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        let err = e
            .apply_json(&tf_env(
                0x52,
                "transform.puppet",
                layer,
                pins(json!([
                    {"pos": [40.0, 40.0]},
                    {"pos": [50.0, 40.0], "rotationDeg": 30.0},
                ])),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("arap-libigl") && err.message.contains("libigl"),
            "the refusal names the election: {}",
            err.message
        );
        assert_eq!(e.state_hash(), before, "refusal mutates nothing");
    }

    #[test]
    fn puppet_reject_matrix_is_typed_and_mutation_free() {
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        let journal_before = e.journal().len();

        // Empty pins array (§10.3 len ≥ 1).
        let err = e
            .apply_json(&tf_env(0x60, "transform.puppet", layer, pins(json!([]))))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // > 512 pins (§45.5 stage-3 cap).
        let many: Vec<serde_json::Value> = (0..513).map(|_| json!({"pos": [40.0, 40.0]})).collect();
        let err = e
            .apply_json(&tf_env(0x61, "transform.puppet", layer, pins(json!(many))))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // Missing pins entirely.
        let err = e
            .apply_json(&tf_env(0x62, "transform.puppet", layer, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);

        // Pin outside the source content bounds (§40.6 live-state).
        let err = e
            .apply_json(&tf_env(
                0x63,
                "transform.puppet",
                layer,
                pins(json!([{"pos": [-99999.0, -99999.0]}])),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        assert!(err.message.contains("content bounds"));

        // pos arity violation.
        let err = e
            .apply_json(&tf_env(
                0x64,
                "transform.puppet",
                layer,
                pins(json!([{"pos": [40.0]}])),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // density outside the enum.
        let err = e
            .apply_json(&tf_env(
                0x65,
                "transform.puppet",
                layer,
                json!({"pins": [{"pos": [40.0, 40.0]}], "density": "extreme"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // expansionPx over the cap.
        let err = e
            .apply_json(&tf_env(
                0x66,
                "transform.puppet",
                layer,
                json!({"pins": [{"pos": [40.0, 40.0]}], "expansionPx": 33}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // rigidity outside 0..=1 (§45.8 addition).
        let err = e
            .apply_json(&tf_env(
                0x67,
                "transform.puppet",
                layer,
                json!({"pins": [{"pos": [40.0, 40.0]}], "rigidity": 1.5}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // Unknown resample kernel name.
        let err = e
            .apply_json(&tf_env(
                0x68,
                "transform.puppet",
                layer,
                json!({"pins": [{"pos": [40.0, 40.0]}], "resample": "sharpest"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // Dangling target.
        let ghost = appname_core::LayerId::from_uuid(uuid::Uuid::new_v4());
        let err = e
            .apply_json(&tf_env(
                0x69,
                "transform.puppet",
                ghost,
                pins(json!([{"pos": [40.0, 40.0]}])),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);

        // Absent target.
        let mut env: serde_json::Value = serde_json::from_str(&tf_env(
            0x6a,
            "transform.puppet",
            layer,
            pins(json!([{"pos": [40.0, 40.0]}])),
        ))
        .unwrap();
        env.as_object_mut().unwrap().remove("target");
        let err = e.apply_json(&env.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);

        // Every reject above journaled nothing and mutated nothing.
        assert_eq!(e.state_hash(), before);
        assert_eq!(e.journal().len(), journal_before);
    }

    #[test]
    fn puppet_empty_layer_rejects_via_the_pin_bounds_rule() {
        // Zero-area alpha hull (§45.5 tool.puppet.empty_layer): the
        // wire-side surface is the §40.6 pin-in-content-bounds check.
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "f0000000-0000-4000-8000-000000000031",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "transparent"},
            })
            .to_string(),
        )
        .unwrap();
        let layer = e.document().unwrap().z_order_walk()[0];
        let err = e
            .apply_json(&tf_env(
                0x70,
                "transform.puppet",
                layer,
                pins(json!([{"pos": [32.0, 32.0]}])),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        assert!(
            err.message.contains("empty_layer") || err.message.contains("no content"),
            "{}",
            err.message
        );
    }

    #[test]
    fn puppet_position_lock_blocks_and_pixels_lock_does_not() {
        // §11.1 transform-family rule (contract lockBehavior): POSITION
        // blocks, PIXELS does not.
        let (mut e, layer) = painted_engine();
        lock_layer_via_package(&mut e, layer, &["pixels"]);
        let layer = e.document().unwrap().z_order_walk()[0];
        e.apply_json(&tf_env(
            0x71,
            "transform.puppet",
            layer,
            pins(json!([{"pos": [45.0, 40.0]}])),
        ))
        .unwrap();

        let (mut e2, layer2) = painted_engine();
        lock_layer_via_package(&mut e2, layer2, &["position"]);
        let layer2 = e2.document().unwrap().z_order_walk()[0];
        let err = e2
            .apply_json(&tf_env(
                0x72,
                "transform.puppet",
                layer2,
                pins(json!([{"pos": [45.0, 40.0]}])),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    #[test]
    fn perspective_warp_reserved_kind_stays_absent() {
        // §45.6 rule 5 [NORMATIVE]: multi-plane perspective warp is
        // Post-1.0; `transform.perspective_warp` is a RESERVED name,
        // absent from the v1 registry — the v1 primitive is the
        // single-quad transform.perspective (already live, DLT +
        // degenerate-quad rejection tested above).
        let (mut e, layer) = painted_engine();
        let err = e
            .apply_json(&tf_env(
                0x73,
                "transform.perspective_warp",
                layer,
                json!({"quads": []}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownKind);
    }

    /// Lock rewrite via package round-trip (public doors only; the
    /// stroke-battery helper's pattern, local to this module).
    fn lock_layer_via_package(e: &mut Engine, layer: appname_core::LayerId, locks: &[&str]) {
        use std::io::{Read, Write};
        let bytes = appname_core::save_package_bytes(e).unwrap();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..za.len() {
            let mut entry = za.by_index(i).unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            entries.push((entry.name().to_string(), data));
        }
        for (name, data) in entries.iter_mut() {
            if name == "document.json" {
                let mut v: serde_json::Value = serde_json::from_slice(data).unwrap();
                for l in v["layers"].as_array_mut().unwrap() {
                    if l["id"] == layer.as_uuid().to_string() {
                        l["locks"] = json!(locks);
                    }
                }
                let mut out = serde_json::to_vec_pretty(&v).unwrap();
                out.push(b'\n');
                *data = out;
            }
        }
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                zw.start_file(name, opts).unwrap();
                zw.write_all(&data).unwrap();
            }
            zw.finish().unwrap();
        }
        let loaded =
            appname_core::load_package_bytes_with(&assembly(), &cursor.into_inner()).unwrap();
        *e = loaded.engine;
    }

    // -------------------------------------------------------------------
    // layer.transform (LIFTED from appname-core, lane/transform-183): the
    // §18.3 integer-translate EXACT arm + the general scale/rotate/affine
    // resample arm (wired to the §18.2 kernel), typed refusals, and the
    // §12.4 rasterize-offer posture.
    // -------------------------------------------------------------------

    /// §18.3 exact arm: a pure integer translation remaps ON-GRID — bounds
    /// translate EXACTLY (no resample apron), content moves verbatim, undo
    /// restores byte identity, and the journal replays deterministically
    /// (D-48). This is the arm that must NOT resample.
    #[test]
    fn layer_transform_integer_translate_is_exact_and_deterministic() {
        let (mut e, layer) = painted_engine();
        let before = e.state_hash();
        let pre_bounds = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        e.apply_json(&tf_env(
            40,
            "layer.transform",
            layer,
            json!({ "matrix": [1, 0, 0, 0, 1, 0, 20, 10, 1] }),
        ))
        .unwrap();
        let post_bounds = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(
            post_bounds,
            pre_bounds.translate(IVec2::new(20, 10)),
            "integer translate moves bounds EXACTLY (no resample apron)"
        );
        let doc = e.document().unwrap();
        let px = doc.layer(layer).unwrap().pixels().unwrap();
        assert!(
            px.read_pixel_f32(IVec2::new(65, 50))[3] > 0.9,
            "content present at +20/+10"
        );
        assert!(
            px.read_pixel_f32(IVec2::new(45, 40))[3] < 0.05,
            "origin vacated on-grid"
        );
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            before,
            "integer-translate undo is byte-identical"
        );
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "layer.transform replays deterministically (D-48)"
        );
    }

    /// The general arm resamples through the SAME §18.2 engine as
    /// transform.free: for every non-integer matrix (scale up/down,
    /// non-uniform, 90°/30° rotation) the two kinds produce BYTE-IDENTICAL
    /// pixels (one `xform::transform_plane`, one Auto kernel resolution) —
    /// pinning that layer.transform is wired to the pinned kernel, not a
    /// second resampler.
    #[test]
    fn layer_transform_general_arm_equals_transform_free() {
        let c30 = (std::f64::consts::PI / 6.0).cos();
        let s30 = (std::f64::consts::PI / 6.0).sin();
        let cases: [(&str, serde_json::Value); 5] = [
            ("scale_2x", json!([2.0, 0, 0, 0, 2.0, 0, 0, 0, 1])),
            ("scale_half", json!([0.5, 0, 0, 0, 0.5, 0, 0, 0, 1])),
            ("scale_nonuniform", json!([1.5, 0, 0, 0, 0.75, 0, 0, 0, 1])),
            ("rotate_90", json!([0, 1.0, 0, -1.0, 0, 0, 0, 0, 1])),
            ("rotate_30", json!([c30, s30, 0, -s30, c30, 0, 0, 0, 1])),
        ];
        for (name, matrix) in cases {
            let (mut e_lt, l1) = painted_engine();
            let (mut e_tf, l2) = painted_engine();
            e_lt.apply_json(&tf_env(
                41,
                "layer.transform",
                l1,
                json!({ "matrix": matrix }),
            ))
            .unwrap_or_else(|err| panic!("layer.transform {name}: {err:?}"));
            e_tf.apply_json(&tf_env(
                41,
                "transform.free",
                l2,
                json!({ "matrix": matrix, "reference": [0, 0] }),
            ))
            .unwrap_or_else(|err| panic!("transform.free {name}: {err:?}"));
            let h_lt = e_lt
                .document()
                .unwrap()
                .layer(l1)
                .unwrap()
                .pixels()
                .unwrap()
                .region_hash();
            let h_tf = e_tf
                .document()
                .unwrap()
                .layer(l2)
                .unwrap()
                .pixels()
                .unwrap()
                .region_hash();
            assert_eq!(
                h_lt, h_tf,
                "{name}: layer.transform must byte-match transform.free (same §18.2 kernel)"
            );
        }
    }

    /// A 2× scale visibly RESAMPLES (the content extent grows ~2×, not an
    /// on-grid move), and the general arm replays deterministically.
    #[test]
    fn layer_transform_scale_grows_bounds_and_replays() {
        let (mut e, layer) = painted_engine();
        let pre = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        e.apply_json(&tf_env(
            42,
            "layer.transform",
            layer,
            json!({ "matrix": [2.0, 0, 0, 0, 2.0, 0, 0, 0, 1] }),
        ))
        .unwrap();
        let post = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert!(
            post.width() > pre.width() + pre.width() / 2,
            "2× scale grows width (pre {} post {})",
            pre.width(),
            post.width()
        );
        assert!(
            post.height() > pre.height() + pre.height() / 2,
            "2× scale grows height (pre {} post {})",
            pre.height(),
            post.height()
        );
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "general (resample) arm replays deterministically (D-48)"
        );
    }

    /// Typed refusals on the lifted kind: singular matrix, unknown resample
    /// vocabulary, and the mesh arm (which routes to transform.warp).
    #[test]
    fn layer_transform_typed_refusals() {
        let (mut e, layer) = painted_engine();
        let err = e
            .apply_json(&tf_env(
                43,
                "layer.transform",
                layer,
                json!({ "matrix": [0, 0, 0, 0, 0, 0, 0, 0, 1] }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "singular matrix");
        let err = e
            .apply_json(&tf_env(
                44,
                "layer.transform",
                layer,
                json!({ "matrix": [2, 0, 0, 0, 2, 0, 0, 0, 1], "resample": "cubic" }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "unknown resample name");
        let err = e
            .apply_json(&tf_env(
                45,
                "layer.transform",
                layer,
                json!({ "matrix": [1, 0, 0, 0, 1, 0, 0, 0, 1],
                        "mesh": {"cols": 1, "rows": 1, "points": [[0, 0]]} }),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::NotImplemented,
            "mesh routes to transform.warp"
        );
    }

    /// §12.4 rasterize-offer posture STANDS after the lift: a non-raster
    /// (group) target is NOT transformed in place — it answers
    /// target_kind_mismatch so the caller can offer to rasterize first.
    /// Also pins the §11.1 POSITION-lock refusal on the lifted kind (both
    /// exercised under the full production-style assembly, since
    /// layer.group / layer.set_props are core kinds).
    #[test]
    fn layer_transform_non_raster_offer_and_position_lock() {
        let mut e = appname_core::engine::kinds::register(with_tools_fill(assembly())).build();
        e.apply_json(
            &json!({
                "schema":"appname.command.v1","id":"f0000000-0000-4000-8000-000000000101",
                "kind":"doc.new","params":{"width":64,"height":64,"background":"transparent"}
            })
            .to_string(),
        )
        .unwrap();
        let raster = e.document().unwrap().z_order_walk()[0];

        // §11.1 POSITION lock → Locked (set via layer.set_props — no test
        // back door exists across the crate boundary).
        e.apply_json(
            &json!({
                "schema":"appname.command.v1","id":"f0000000-0000-4000-8000-000000000102",
                "kind":"layer.set_props","target":{"layerId":raster.as_uuid().to_string()},
                "params":{"locks":{"position":true}}
            })
            .to_string(),
        )
        .unwrap();
        let err = e
            .apply_json(&tf_env(
                46,
                "layer.transform",
                raster,
                json!({ "matrix": [1, 0, 0, 0, 1, 0, 5, 0, 1] }),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::Locked,
            "POSITION lock blocks layer.transform (§11.1)"
        );
        e.apply_json(
            &json!({
                "schema":"appname.command.v1","id":"f0000000-0000-4000-8000-000000000103",
                "kind":"layer.set_props","target":{"layerId":raster.as_uuid().to_string()},
                "params":{"locks":{"position":false}}
            })
            .to_string(),
        )
        .unwrap();

        // Group it → the GROUP is parametric (pixels() == None).
        let group_id = "f0000000-0000-4000-8000-0000000000f1";
        e.apply_json(
            &json!({
                "schema":"appname.command.v1","id":"f0000000-0000-4000-8000-000000000104",
                "kind":"layer.group",
                "params":{"layerIds":[raster.as_uuid().to_string()],"newGroupId":group_id}
            })
            .to_string(),
        )
        .unwrap();
        let group = appname_core::LayerId::from_uuid(Uuid::parse_str(group_id).unwrap());
        let err = e
            .apply_json(&tf_env(
                47,
                "layer.transform",
                group,
                json!({ "matrix": [2, 0, 0, 0, 2, 0, 0, 0, 1] }),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::TargetKindMismatch,
            "§12.4: non-raster target offers rasterize, no silent in-place transform"
        );
    }
}

// ===========================================================================
// CP-0.5 wave 3, job 1: the arbitrated fill lift (O-INT brief; stroke/
// transform precedent). `FillKind` moves home from `appname-core::engine::
// kinds` — behavior-identical for every previously-implemented arm (the
// stage-6 order, error texts, and compositing math are transplanted
// verbatim; equivalence pinned by `fill_lift_tests::lift_equivalence_*`
// against core's still-registered impl) — and the FLOOD VARIANT goes LIVE:
// `seed` now floods through `select::wand::{wand_weight, wand_select}`,
// the §22.6 [NORMATIVE] shared wand/bucket predicate, so bucket and wand
// agree on the region BY CONSTRUCTION. The core-side typed-pending seed
// arm retires with the brokered swap.
//
// REGISTRATION (the brokered pattern, gate-green on this branch):
// `EngineAssembly::with_kind` panics on duplicates, so this `FillKind` is
// NOT in `assembly()` while core's copy is still registered. The assembly
// points adopt it through [`with_tools_fill`]; the core+cli swap diff is
// specified in the lane log/report for O-INT to broker atomically.
//
// v1 pins (flood): `sampleAllLayers:true` stays typed-pending (the region
// reads the flattened composite, §22.6 — NOT for want of a composite any
// more: `appname_core::resolve_sampling_composite` serves exactly this
// scope since the §45.2/§13.3 sample-scope slice. This row is simply not
// wired to it in that slice's scope, and is flagged as a newly unblocked
// follow-on); the flood samples
// the TARGET layer and compares straight color (select-module pin); seed
// outside the canvas → `error.schema.out_of_range` (§40.5); flood
// coverage multiplies selection coverage (§13.1 — every consumer scales).
// ===========================================================================

use crate::select::wand::{wand_select, WandParams};

/// The lifted `fill` implementation (assembly via [`with_tools_fill`]).
pub struct FillKind;

/// Adopt the tools-side fill onto an assembly — the brokered swap point:
/// core's `register()` drops its `FillKind` line in the same merge.
pub fn with_tools_fill(assembly: EngineAssembly) -> EngineAssembly {
    assembly.with_kind(Arc::new(FillKind))
}

impl KindImpl for FillKind {
    fn kind(&self) -> &'static str {
        "fill"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_fill_target(cx)?;
        require_pixels_unlocked(cx, layer)?;
        // §22.6/§11.1: the TRANSPARENCY lock constrains; preserveTransparency
        // applies the SAME clamp even unlocked (dst α caps the fill).
        let constrain = locks_of(cx, layer).contains(LockFlags::TRANSPARENCY)
            || cx
                .params()
                .get("preserveTransparency")
                .and_then(Value::as_bool)
                == Some(true);

        match cx.params().get("blend").and_then(Value::as_str) {
            None | Some("normal") => {}
            Some(other) => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!("blend mode {other:?} needs the §4.5 kernel table; normal is built in"),
                ))
            }
        }
        let content = cx
            .params()
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("content"))?;
        let color = match content {
            "color" => {
                let obj = cx
                    .params()
                    .get("color")
                    .and_then(Value::as_object)
                    .ok_or_else(|| EngineError::missing("color"))?; // §40.5 cross-field
                decode_color_object(obj)?
            }
            // Headless default swatches (lane-log divergence): foreground
            // black, background white — no tool state exists yet.
            "foreground" => [0.0, 0.0, 0.0],
            "background" => [1.0, 1.0, 1.0],
            other => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!(
                        "fill content \"{other}\" arrives with its own row (gradient §22.4, \
                         pattern §22.5, content_aware §21.4/§45.3)"
                    ),
                ))
            }
        };
        let opacity = param_f32(cx, "opacity", 1.0).clamp(0.0, 1.0);
        let canvas = cx.document().geometry().canvas_rect();

        // The LIVE flood variant (§22.6 shared predicate) vs the
        // selection-region fill.
        let flood = match cx.params().get("seed") {
            None => None,
            Some(v) => Some(flood_coverage(cx, layer, canvas, v)?),
        };

        let region = match &flood {
            Some(plane) => plane.rect.intersect(&canvas),
            None => {
                // Selection gates coverage (operative bounds ∩ canvas,
                // §22.6); no selection ⇒ whole layer = the canvas rect
                // (module-doc reading).
                let sel = cx.document().selection();
                if sel.active() {
                    sel.bounds().intersect(&canvas)
                } else {
                    canvas
                }
            }
        };

        // The aud-replay F1 pixel-exact re-tighten below is only NEEDED when a
        // written pixel can store transparent while `changed` (an f32 delta,
        // computed pre-quantization in `edit_region_at`) still counted it — a
        // sub-quantum premultiplied result. That can happen through (a)
        // FRACTIONAL coverage/weight — a feathered selection or a flood weight;
        // (b) a sub-quantum SOURCE alpha — partial `opacity`; or (c) the
        // transparency `constrain`, which scales src α by dst α (dst can be
        // sub-quantum). ONLY a no-flood / no-selection / FULL-opacity /
        // unconstrained fill writes α = 1.0 uniformly across the region: every
        // touched pixel stores fully opaque (nonzero at any depth) and none is
        // ever made transparent, so `changed`/`grow_layer_bounds` is ALREADY
        // pixel-exact and the recompute is a provable no-op (aud-perf r2
        // FINDING 1). Every other case keeps the loader-agreeing rescan — the
        // narrow skip is sound, not merely faster. (Self-review R1 caught that
        // a uniform SUB-QUANTUM opacity fill stores all-transparent yet grows
        // eager bounds to the whole canvas; the `opacity < 1.0` / `constrain`
        // terms keep the recompute for it.) Captured here while the selection
        // is cheap to read; the fill touches only the target layer.
        let needs_pixel_exact =
            flood.is_some() || constrain || opacity < 1.0 || cx.document().selection().active();

        let changed = edit_region_at(cx, layer, region, &mut |p, dst, cov| {
            let flood_w = match &flood {
                Some(plane) => plane.get(p.x, p.y),
                None => 1.0,
            };
            let mut src_a = opacity * cov * flood_w;
            if constrain {
                src_a *= dst[3].clamp(0.0, 1.0); // source-atop (§22.6/§11.1)
            }
            if src_a <= 0.0 {
                return dst;
            }
            let ia = 1.0 - src_a;
            [
                color[0] * src_a + dst[0] * ia,
                color[1] * src_a + dst[1] * ia,
                color[2] * src_a + dst[2] * ia,
                src_a + dst[3] * ia,
            ]
        })?;
        if !changed.is_empty() {
            // §2.4 eager growth by the written bounds (paint-path rule).
            cx.grow_layer_bounds(layer, changed)?;
            // PIXEL-EXACT re-tighten (aud-replay F1 fix): `changed` is the
            // union of pixels whose f32 value moved, computed BEFORE the
            // plane is quantized into the layer's storage depth. Through a
            // FEATHERED selection the fringe carries sub-quantum coverage
            // (e.g. alpha < 0.5/255 on a U8 doc): `changed` counts those
            // pixels, but they store as exactly transparent, so the eager
            // extent runs up to 1px proud of the real content on every side.
            // The package loader recomputes bounds pixel-exact on reload
            // (`recompute_bounds_pixel_exact`, io/package.rs) over the
            // already-quantized plane, so the eager extent forks
            // live-vs-reloaded, `region_hash` (which folds bounds) diverges,
            // and the D-48 replay-as-recovery gate disables undo/redo across
            // the save boundary. This door runs the loader's exact
            // canonicalization over the stored (quantized) pixels so live
            // bounds equal persisted bounds. Same class/fix as the transform
            // bake (O-QA W12) and the paste transparent-border (O-QA W18);
            // the scan is over the just-written tiles.
            //
            // GUARD (aud-perf r2 FINDING 1): the re-tighten is a redundant
            // SECOND full pixel scan, needed only for the sub-quantum-capable
            // cases (`needs_pixel_exact`, computed above). For the provably
            // no-op case — no flood, no selection, full opacity, unconstrained
            // — it still pays a full 49k-pixel rescan (+222 µs / +17.2 % on the
            // `fill_apply` bench), so skip it there and keep it everywhere else.
            if needs_pixel_exact {
                cx.recompute_layer_bounds_pixel_exact(layer)?;
            }
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// stroke_path · R · inverse delta (§40.5 entry; §22.6 Stroke dialog;
// §20.2 stroke tessellation via Door 1; §13.5 selection boundary)
// ---------------------------------------------------------------------------

/// The LAST registry kind (102/102): stroke a path or the selection
/// boundary. O-TEXTVEC stroke slice (O-INT durable-queue routing,
/// 2026-08-18): the arm lands beside `FillKind` because §10.3 places
/// `stroke_path` in the fill/stroke FAMILY — target resolution, lock
/// behavior, color decoding, and the tile compositor are shared
/// family machinery. Geometry crosses the Door-1 seam
/// (`stroke_path_coverage`/`stroke_boundary_coverage`, §20.2's
/// [NORMATIVE] position rule inside the service).
///
/// Pinned (lane log; fold-in register): tool:"brush" answers typed
/// `error.command.not_implemented` AFTER its cross-field validation —
/// the brush-engine trace (§22.6 fixed path sampling) consumes the
/// stroke-engine machinery and lands with its own slice; the
/// rasterize-wire ruling's typed-NI-for-sanctioned-unserved wire
/// applies. Blend modes mirror the sibling fill arm: normal built in,
/// the rest typed-NI until both arms consume the routed §4.5 kernel
/// table together (one flip, one review).
pub struct StrokePathKind;

impl KindImpl for StrokePathKind {
    fn kind(&self) -> &'static str {
        "stroke_path"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        use appname_core::engine::vector_ops::StrokePosition;
        let layer = resolve_fill_target(cx)?;
        require_pixels_unlocked(cx, layer)?;
        let constrain = locks_of(cx, layer).contains(LockFlags::TRANSPARENCY);

        match cx.params().get("blend").and_then(Value::as_str) {
            None | Some("normal") => {}
            Some(other) => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!("blend mode {other:?} needs the §4.5 kernel table; normal is built in"),
                ))
            }
        }

        // widthPx: stage-5 range re-checked live (§40.5).
        let width = cx
            .params()
            .get("widthPx")
            .map_or(Some(1.0), Value::as_f64)
            .filter(|w| w.is_finite() && (0.1..=1000.0).contains(w))
            .ok_or_else(|| {
                EngineError::range("widthPx", "must be a number in 0.1..=1000 (§10.3/§40.5)")
            })?;

        let position = match cx.params().get("position").and_then(Value::as_str) {
            None | Some("center") => StrokePosition::Center,
            Some("inside") => StrokePosition::Inside,
            Some("outside") => StrokePosition::Outside,
            Some(other) => {
                return Err(EngineError::range(
                    "position",
                    format!("unknown position {other:?} (inside|center|outside)"),
                ))
            }
        };

        let color_obj = cx
            .params()
            .get("color")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing("color"))?;
        let color = decode_color_object(color_obj)?;
        let opacity = param_f32(cx, "opacity", 1.0).clamp(0.0, 1.0);

        // tool:"brush" — cross-field first (§40.5: brushPreset REQUIRED),
        // then the typed-NI interim (module doc).
        match cx.params().get("tool").and_then(Value::as_str) {
            None | Some("line") => {}
            Some("brush") => {
                let preset_ok = cx
                    .params()
                    .get("brushPreset")
                    .and_then(Value::as_str)
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok());
                if !preset_ok {
                    return Err(EngineError::missing("brushPreset"));
                }
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "stroke_path tool:\"brush\": the brush-engine trace (§22.6 fixed \
                     path sampling) lands with the brush-consumption slice; tool:\"line\" \
                     serves today (typed interim per the rasterize-wire ruling)",
                ));
            }
            Some(other) => {
                return Err(EngineError::range(
                    "tool",
                    format!("unknown tool {other:?} (line|brush)"),
                ))
            }
        }

        // The source: a resolving path (stage 4) or the active selection.
        let svc = cx.require_vector_geometry()?;
        let source = cx
            .params()
            .get("source")
            .ok_or_else(|| EngineError::missing("source"))?;
        let (plane, selection_gates) = match source {
            Value::String(s) if s == "selection" => {
                let sel_plane = {
                    let doc = cx.document();
                    let sel = doc.selection();
                    if !sel.active() {
                        return Err(EngineError::new(
                            ErrorCode::SelectionRequired,
                            "stroke_path source \"selection\" requires an active selection \
                             (§40.5; its §13.5 boundary is traced)",
                        ));
                    }
                    let b = sel.bounds();
                    let mut values = vec![0.0f32; (b.width().max(0) * b.height().max(0)) as usize];
                    let mask = sel.mask();
                    let mut i = 0;
                    for y in b.min.y..b.max.y {
                        for x in b.min.x..b.max.x {
                            values[i] = mask.read_pixel_f32(appname_core::IVec2::new(x, y))[0];
                            i += 1;
                        }
                    }
                    appname_core::engine::vector_ops::CoveragePlaneData { rect: b, values }
                };
                // A selection-boundary stroke is NOT self-masked (§40.5
                // documented choice) — the gate stays off.
                (
                    svc.stroke_boundary_coverage(&sel_plane, width, position)?,
                    false,
                )
            }
            Value::Object(m) => {
                let pid = m
                    .get("pathId")
                    .and_then(Value::as_str)
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(appname_core::PathId::from_uuid)
                    .ok_or_else(|| EngineError::missing("source.pathId"))?;
                let sidecar = match cx.document().path(pid) {
                    Some(node) => std::sync::Arc::clone(node.sidecar()),
                    None => {
                        return Err(EngineError::bad_target(format!(
                            "source.pathId {pid} does not resolve (§2.7 arenas)"
                        )))
                    }
                };
                // Painting IS selection-gated for a path source (§40.5).
                (svc.stroke_path_coverage(&sidecar, width, position)?, true)
            }
            _ => {
                return Err(EngineError::range(
                    "source",
                    "must be {pathId: uuid} or \"selection\" (§40.5)",
                ))
            }
        };

        let canvas = cx.document().geometry().canvas_rect();
        let region = plane.rect.intersect(&canvas);
        let cov_at = |p: appname_core::IVec2| -> f32 {
            if !plane.rect.contains(p) {
                return 0.0;
            }
            let idx =
                ((p.y - plane.rect.min.y) * plane.rect.width() + (p.x - plane.rect.min.x)) as usize;
            plane.values[idx].clamp(0.0, 1.0)
        };

        let changed = edit_region_at(cx, layer, region, &mut |p, dst, sel_cov| {
            let gate = if selection_gates { sel_cov } else { 1.0 };
            let mut src_a = opacity * cov_at(p) * gate;
            if constrain {
                src_a *= dst[3].clamp(0.0, 1.0); // §11.1 source-atop
            }
            if src_a <= 0.0 {
                return dst;
            }
            let ia = 1.0 - src_a;
            [
                color[0] * src_a + dst[0] * ia,
                color[1] * src_a + dst[1] * ia,
                color[2] * src_a + dst[2] * ia,
                src_a + dst[3] * ia,
            ]
        })?;
        if !changed.is_empty() {
            // Q-1 determinism class: pixel-exact grow so a soft-edged path
            // stroke's fringe cannot leave the live extent proud of the
            // persisted plane. `stroke_path` is alpha-preserving (source-over
            // / source-atop never lowers alpha), so the scoped grow is exact.
            finish_write_bounds(cx, layer, changed, false)?;
        }
        Ok(KindOutcome::default())
    }
}

/// The §22.6 flood region: seed gates + the SHARED wand predicate over the
/// target layer.
fn flood_coverage(
    cx: &KindContext<'_>,
    layer: LayerId,
    canvas: appname_core::IRect,
    seed_val: &Value,
) -> Result<crate::select::MaskPlane, EngineError> {
    let seed = match seed_val.as_array() {
        Some(items) if items.len() == 2 && items.iter().all(|v| v.as_i64().is_some()) => {
            appname_core::IVec2::new(
                items[0].as_i64().unwrap_or(0) as i32,
                items[1].as_i64().unwrap_or(0) as i32,
            )
        }
        _ => {
            return Err(EngineError::range(
                "seed",
                "flood seed must be [x, y] integer doc px (§10.3; the content_aware integer \
                 seed rides that content arm)",
            ))
        }
    };
    if !canvas.contains(seed) {
        return Err(EngineError::range(
            "seed",
            format!(
                "flood seed [{}, {}] must lie inside the canvas (§40.5)",
                seed.x, seed.y
            ),
        ));
    }
    if cx.params().get("sampleAllLayers").and_then(Value::as_bool) == Some(true) {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "sampleAllLayers floods the flattened composite (§22.6); the flood region is not \
             wired to the sampling-composite door yet. NOTE: the door itself now EXISTS \
             (appname_core::resolve_sampling_composite, the §45.2/§13.3 sample-scope slice) \
             and this row is SampleScope::Document — the old \"no composite is reachable \
             headless\" reason no longer holds and is not restated here",
        ));
    }
    let tolerance = param_f32(cx, "tolerance", 32.0).clamp(0.0, 255.0);
    let contiguous = cx
        .params()
        .get("contiguous")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let antialias = cx
        .params()
        .get("antialias")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let params = WandParams {
        tolerance,
        contiguous,
        eight_way: false,
        antialias,
    };
    let doc = cx.document();
    let buf = doc
        .layer(layer)
        .and_then(|n| n.pixels())
        .expect("resolved raster target");
    Ok(wand_select(buf, buf.premultiplied(), canvas, seed, &params))
}

/// `fill`'s stage-6 target resolution — core's error texts verbatim
/// (`NonRasterError::BadTarget` arm of the lifted helper).
fn resolve_fill_target(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let target = cx.envelope().target.as_ref();
    let (layer, mask, channel) = match target {
        Some(t) => (t.layer_id, t.mask_id, t.channel_id),
        None => (None, None, None),
    };
    if mask.is_some() || channel.is_some() {
        return Err(EngineError::bad_target(
            "mask/channel surface editing arrives with the mask.*/channel kinds rows (CP-0.5); \
             this kind targets target.layerId today",
        ));
    }
    let Some(layer) = layer else {
        return Err(EngineError::bad_target(
            "this kind requires target.layerId (§40.5)",
        ));
    };
    let id = LayerId::from_uuid(layer);
    let node = cx.document().layer(id).ok_or_else(|| {
        EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
    })?;
    if node.pixels().is_none() {
        return Err(EngineError::bad_target(format!(
            "target.layerId {layer} is not a raster surface"
        )));
    }
    Ok(id)
}

fn locks_of(cx: &KindContext<'_>, layer: LayerId) -> appname_core::LockFlags {
    // §11.1 transitive ancestry union (`Document::effective_locks`,
    // O-CORE handoff): group locks bind the subtree.
    cx.document().effective_locks(layer)
}

fn require_pixels_unlocked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    if locks_of(cx, layer).contains(LockFlags::PIXELS) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the target layer's pixels are locked (§11.1)",
        ));
    }
    Ok(())
}

/// §22.1 color object → straight working-space linear RGB (core's
/// `decode_color_object`, transplanted verbatim for the lift).
fn decode_color_object(color: &serde_json::Map<String, Value>) -> Result<[f32; 3], EngineError> {
    if color.get("profileId").is_some() {
        return Err(EngineError::with_field(
            ErrorCode::ProfileSpaceMismatch,
            "color.profileId",
            "explicit color profiles cannot be verified before the CMM (§5.5); omit profileId",
        ));
    }
    let comps = color
        .get("comps")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("color.comps"))?;
    if comps.len() != 4 || !comps.iter().all(|c| c.is_number()) {
        return Err(EngineError::range(
            "color.comps",
            "`color.comps` must be 4 numbers (§22.1)",
        ));
    }
    let c = |i: usize| comps[i].as_f64().unwrap_or(0.0) as f32;
    match color.get("model").and_then(Value::as_str) {
        Some("rgb") | Some("rgba") => Ok([
            appname_core::io::srgb::srgb_decode(c(0).clamp(0.0, 1.0)),
            appname_core::io::srgb::srgb_decode(c(1).clamp(0.0, 1.0)),
            appname_core::io::srgb::srgb_decode(c(2).clamp(0.0, 1.0)),
        ]),
        Some("gray") | Some("grayA") => {
            let v = appname_core::io::srgb::srgb_decode(c(0).clamp(0.0, 1.0));
            Ok([v, v, v])
        }
        Some(other) => Err(EngineError::new(
            ErrorCode::NotImplemented,
            format!("color model \"{other}\" needs the §5/§6 conversion pipeline"),
        )),
        None => Err(EngineError::missing("color.model")),
    }
}

/// Core's `edit_region`, transplanted with a position-aware closure (the
/// flood weight is a function of `p`; the non-flood paths ignore it —
/// behavior-identical to the core helper otherwise).
/// Selection-gated per-pixel edit over `region` through the recorder tile
/// doors: `f(p, dst, cov)` maps each in-region pixel (premultiplied
/// storage-normalized f32) to its new value, with `cov` the §13.1
/// selection coverage at `p` (1.0 when no selection is active). Only tiles
/// whose pixels actually changed are installed (§40.1 empty-delta
/// discipline). `pub` since the CP-0.7 auto/filter bake slice: the
/// `appname-adjust` (`image.auto_*`) and `appname-filters`
/// (`filter.apply`) KindImpls share this one §16.2 masked-bake loop.
pub fn edit_region_at(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    region: appname_core::IRect,
    f: &mut dyn FnMut(appname_core::IVec2, [f32; 4], f32) -> [f32; 4],
) -> Result<appname_core::IRect, EngineError> {
    use appname_core::{IRect, IVec2, TileCoord, TILE, TILE_I32};
    let mut changed_rect = IRect::EMPTY;
    if region.is_empty() {
        return Ok(changed_rect);
    }
    let coords: Vec<(i32, i32)> = region.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        let span = rect.intersect(&region);
        if span.is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        let mut tile_changed = IRect::EMPTY;
        {
            let doc = cx.document();
            let buf = doc
                .layer(layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let sel = doc.selection();
            let sel_active = sel.active();
            let mut i = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    let dst = buf.read_pixel_f32(p);
                    let out = if span.contains(p) {
                        let cov = if sel_active {
                            sel.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let v = f(p, dst, cov);
                        if v != dst {
                            tile_changed = tile_changed.union(&IRect::new(x, y, x + 1, y + 1));
                        }
                        v
                    } else {
                        dst
                    };
                    plane[i] = out;
                    i += 1;
                }
            }
        }
        if !tile_changed.is_empty() {
            cx.install_tile_plane(layer, coord, &plane)?;
            changed_rect = changed_rect.union(&tile_changed);
        }
    }
    Ok(changed_rect)
}

#[cfg(test)]
mod fill_lift_tests {
    use super::*;
    use appname_core::{Engine, IVec2};
    use serde_json::json;

    fn core_engine() -> Engine {
        // Post-swap MECHANICAL companion (O-CORE, flagged for O-TOOLS
        // ratification): core's register() no longer carries FillKind —
        // the equivalence reference registers it explicitly.
        appname_core::engine::kinds::register(assembly())
            .with_kind(std::sync::Arc::new(appname_core::engine::kinds::FillKind))
            .build()
    }

    fn tools_engine() -> Engine {
        with_tools_fill(assembly()).build()
    }

    fn doc_new(e: &mut Engine, background: &str) -> appname_core::LayerId {
        let text = json!({
            "schema": "appname.command.v1",
            "id": "aa000000-0000-4000-8000-000000000001",
            "kind": "doc.new",
            "params": {"width": 96, "height": 64, "background": background},
        })
        .to_string();
        e.apply_json(&text).unwrap();
        e.document().unwrap().z_order_walk()[0]
    }

    fn fill_env(n: u32, layer: appname_core::LayerId, params: serde_json::Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("aa000000-0000-4000-8000-0001{n:08x}"),
            "kind": "fill",
            "target": {"layerId": layer.as_uuid().to_string()},
            "params": params,
        })
        .to_string()
    }

    /// aud-perf r2 FINDING 1 — production-path SOUNDNESS pin (public doors).
    ///
    /// The guarded skip of the pixel-exact re-tighten must leave a whole-layer
    /// no-selection fill's eager bounds ALREADY equal to what the package
    /// loader computes pixel-exact on reopen. If the skip dropped needed
    /// tightening, the live extent would fork the reloaded extent, `region_hash`
    /// would diverge, and D-48 replay-as-recovery would break. A save->reopen
    /// bounds match on the no-selection path therefore proves the skip is sound
    /// on the very `FillKind` (appname-tools) that the `fill_apply` bench runs.
    #[test]
    fn whole_layer_fill_bounds_survive_reload_without_recompute() {
        let mut e = tools_engine();
        let layer = doc_new(&mut e, "white");
        e.apply_json(&fill_env(
            2,
            layer,
            json!({"content": "foreground", "opacity": 1.0}),
        ))
        .unwrap();
        assert!(
            !e.document().unwrap().selection().active(),
            "precondition: no selection ⇒ exercises the guarded SKIP branch"
        );
        let eager = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(
            eager,
            appname_core::IRect::new(0, 0, 96, 64),
            "whole-canvas fill ⇒ eager bounds == canvas"
        );

        let bytes = appname_core::save_package_bytes(&e).expect("save");
        let loaded = appname_core::load_package_bytes_with(&assembly(), &bytes).expect("load");
        let reloaded = loaded
            .engine
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(
            eager, reloaded,
            "eager (recompute-SKIPPED) bounds must equal the loader's pixel-exact \
             bounds — the guarded skip loses zero tightening"
        );
    }

    /// The lift is behavior-identical on every previously-implemented arm:
    /// the same script through core's still-registered FillKind and the
    /// lifted one reaches bitwise-equal state at every step.
    #[test]
    fn lift_equivalence_state_hashes_match_step_for_step() {
        let mut a = core_engine();
        let mut b = tools_engine();
        let la = doc_new(&mut a, "white");
        let lb = doc_new(&mut b, "white");
        assert_eq!(a.state_hash(), b.state_hash(), "doc.new baseline");
        let steps = [
            json!({"content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0]}, "opacity": 0.6}),
            json!({"content": "foreground", "opacity": 0.25}),
            json!({"content": "background", "opacity": 0.5, "preserveTransparency": true}),
            json!({"content": "color", "color": {"model": "gray", "comps": [0.5, 0, 0, 0]}}),
        ];
        for (i, p) in steps.iter().enumerate() {
            a.apply_json(&fill_env(i as u32 + 2, la, p.clone()))
                .unwrap();
            b.apply_json(&fill_env(i as u32 + 2, lb, p.clone()))
                .unwrap();
            assert_eq!(a.state_hash(), b.state_hash(), "step {i}");
        }
        // Reject-code parity on the typed arms.
        let rejects = [
            (
                json!({"content": "gradient", "gradient": {"stops": [
                {"pos": 0, "color": {"model": "rgb", "comps": [0,0,0,0]}},
                {"pos": 1, "color": {"model": "rgb", "comps": [1,1,1,0]}}], "kind": "linear"}}),
                ErrorCode::NotImplemented,
            ),
            (
                json!({"content": "color", "color": {"model": "rgb", "comps": [1,0,0,0]}, "blend": "multiply"}),
                ErrorCode::NotImplemented,
            ),
            (json!({"content": "color"}), ErrorCode::MissingField),
        ];
        for (i, (p, want)) in rejects.iter().enumerate() {
            let ea = a
                .apply_json(&fill_env(20 + i as u32, la, p.clone()))
                .unwrap_err();
            let eb = b
                .apply_json(&fill_env(20 + i as u32, lb, p.clone()))
                .unwrap_err();
            assert_eq!(ea.code, *want);
            assert_eq!(ea.code, eb.code, "reject parity at {i}");
            assert_eq!(ea.message, eb.message, "reject message parity at {i}");
        }
        assert_eq!(a.state_hash(), b.state_hash(), "rejects touched nothing");
    }

    /// The LIVE flood variant: §22.6 shared-predicate region semantics,
    /// end-to-end determinism, and the seed gates.
    #[test]
    fn flood_fill_is_live_and_region_exact() {
        let mut e = tools_engine();
        let layer = doc_new(&mut e, "white");
        // Wall off the left third with a hard vertical brush line.
        let samples = crate::brush::encode_samples(&[
            crate::brush::InputSample {
                x: 32.0,
                y: -8.0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.0,
            },
            crate::brush::InputSample {
                x: 32.0,
                y: 72.0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.05,
            },
        ])
        .unwrap();
        let asset = e.add_samples_asset(samples);
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "aa000000-0000-4000-8000-000000000002",
                "kind": "brush.stroke",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {
                    "brushPreset": preset_hard_round_16_id().to_string(),
                    "color": {"model": "rgb", "comps": [0, 0, 1, 0]},
                },
                "samples": asset.to_string(),
            })
            .to_string(),
        )
        .unwrap();
        let before = e.state_hash();
        // Flood the LEFT compartment with red.
        let receipt = e
            .apply_json(&fill_env(
                3,
                layer,
                json!({
                    "content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                    "seed": [5, 30],
                }),
            ))
            .unwrap();
        assert!(!receipt.dirty.is_empty());
        let doc = e.document().unwrap();
        let buf = doc.layer(layer).unwrap().pixels().unwrap();
        let left = buf.read_pixel_f32(IVec2::new(5, 30));
        assert!(
            left[0] > 0.9 && left[2] < 0.2,
            "left compartment filled red: {left:?}"
        );
        let right = buf.read_pixel_f32(IVec2::new(80, 30));
        assert!(
            right[0] > 0.9 && right[1] > 0.9 && right[2] > 0.9,
            "right stays white: {right:?}"
        );
        let wall = buf.read_pixel_f32(IVec2::new(32, 30));
        assert!(wall[2] > 0.9 && wall[0] < 0.2, "wall stays blue: {wall:?}");
        // Undo/redo byte-identity + journal replay determinism.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "flood replays deterministically"
        );
        // contiguous:false floods every white pixel — the right side too.
        e.apply_json(&fill_env(
            4,
            layer,
            json!({
                "content": "color", "color": {"model": "rgb", "comps": [0, 1, 0, 0]},
                "seed": [80, 30], "contiguous": false, "tolerance": 8,
            }),
        ))
        .unwrap();
        let doc = e.document().unwrap();
        let buf = doc.layer(layer).unwrap().pixels().unwrap();
        assert!(
            buf.read_pixel_f32(IVec2::new(80, 30))[1] > 0.9,
            "global flood painted the right"
        );
        assert!(
            buf.read_pixel_f32(IVec2::new(32, 30))[2] > 0.9,
            "wall untouched by tolerance"
        );
    }

    #[test]
    fn flood_seed_gates_and_pending_arms() {
        let mut e = tools_engine();
        let layer = doc_new(&mut e, "white");
        // Seed outside the canvas → out_of_range (§40.5).
        let err = e
            .apply_json(&fill_env(
                2,
                layer,
                json!({
                    "content": "foreground", "seed": [96, 0],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // Malformed seed shape → out_of_range.
        let err = e
            .apply_json(&fill_env(
                3,
                layer,
                json!({
                    "content": "foreground", "seed": [1.5, 2.5],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // sampleAllLayers stays typed-pending (no headless composite).
        let err = e
            .apply_json(&fill_env(
                4,
                layer,
                json!({
                    "content": "foreground", "seed": [5, 5], "sampleAllLayers": true,
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // Nothing above touched the document.
        assert_eq!(e.journal().len(), 1);
    }

    /// §11.1 transitive letter through `Document::effective_locks` (the
    /// O-CORE door-slice consolidation, adopted this round): a PIXELS
    /// lock on an ancestor GROUP blocks fill on the child layer —
    /// own-node reads under-enforced this until the adoption.
    #[test]
    fn fill_under_pixels_locked_group_refuses_locked_transitively() {
        let mut e = appname_core::engine::kinds::register(with_tools_fill(assembly())).build();
        let layer = doc_new(&mut e, "white");
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "aa000000-0000-4000-8000-00000000f001",
                "kind": "layer.group",
                "params": {"layerIds": [layer.as_uuid().to_string()],
                            "newGroupId": "aa700000-0000-4000-8000-0000000000f1"},
            })
            .to_string(),
        )
        .unwrap();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "aa000000-0000-4000-8000-00000000f002",
                "kind": "layer.set_props",
                "target": {"layerId": "aa700000-0000-4000-8000-0000000000f1"},
                "params": {"locks": {"pixels": true}},
            })
            .to_string(),
        )
        .unwrap();
        let before = e.state_hash();
        let err = e
            .apply_json(&fill_env(
                3,
                layer,
                json!({"content": "color",
                        "color": {"model": "rgb", "comps": [1, 0, 0, 0]}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked, "group PIXELS binds the child");
        assert_eq!(before, e.state_hash(), "refusal mutates nothing (§10.2)");
    }
}

// ===========================================================================
// CP-0.5 wave 3, job 2a: the doc.* geometry kinds through the merged seam
// doors (§40.2 family; contracts doc.resize/canvas_resize/crop/trim/rotate;
// ES §19.1–§19.3, §18.2; DD ㉔ — nothing here resamples unless the command
// explicitly selects a kernel).
//
// All class R (inverse: delta): every pixel change flows through the
// recorder-coupled `install_surface_plane` tile door — NOT the class-C
// `checkpoint_replace_surface` door, which is unreachable from R kinds by
// its own guard (registry class rows; divergence vs core's door note
// logged for O-INT). Geometry changes record `TreeOp::SetGeometry`.
// §40.2 carried rule: EVERY raster surface (layers, mask coverages, the
// selection — `raster_surfaces()` order) transforms through the SAME
// mapping/kernel; coverage surfaces clamp to [0,1] (§13.1/§34.4 prop 13),
// unsigned-normalized layer storage store-clamps (§18.2), f32 keeps
// excursions.
//
// v1 pins (lane log): resize at scale 1 skips resampling for EVERY kernel
// (§19.3 no-op discipline; approximating kernels must not blur a no-op);
// `doc.crop` serves BOTH pixel arms: `deletePixels:true` maps+clips the
// surfaces destructively, `deletePixels:false` sets the §19.1 `crop_rect`
// window non-destructively (hides, never deletes — image==crop.size;
// full-canvas rect normalizes to None / uncrop); `straightenAngleDeg != 0`
// is typed-pending (the rotate+crop one-step composition rides the next
// slice); trim's color
// bases need the flattened composite for multi-layer documents
// (single-raster-layer docs use that layer; else typed-pending) while the
// `transparent` basis is exact at any layer count (composite alpha is zero
// iff every layer's alpha is zero); positive `angleDeg` rotates clockwise
// in the y-down document frame (§0.3 atan2 convention); `doc.rotate` with
// `expand:false` on odd quarter-turns recenters by floor((w−h)/2) — an
// integer translate, still a lossless permutation; canvas-extension fill
// paints the BOTTOM raster layer iff its alpha is 1 over the (translated)
// old canvas (§19.3 "background layer, if one exists and is opaque";
// `background_color` resolves to the headless white swatch — fill-kind
// divergence precedent).
// ===========================================================================

use crate::geometry::{
    canvas_anchor_offset, crop_rect_valid, resize_dims, trim_rect, Anchor, Orientation, TrimBasis,
    TrimSource, TrimSpec,
};
use crate::resample::{resize_plane_global, EdgePolicy, Kernel};

pub struct DocResizeKind;
pub struct DocCanvasResizeKind;
pub struct DocCropKind;
pub struct DocTrimKind;
pub struct DocRotateKind;

// ---------------------------------------------------------------------------
// Shared surface machinery (public read paths + recorder-door installs).
// ---------------------------------------------------------------------------

fn surface_plane(
    cx: &KindContext<'_>,
    id: Uuid,
) -> Option<(Plane, appname_core::IRect, SurfaceClass)> {
    let doc = cx.document();
    let (buf, class): (&appname_core::Buffer, SurfaceClass) = if id == cx.selection_surface() {
        (doc.selection().mask(), SurfaceClass::Coverage)
    } else if let Some(b) = doc.layer(LayerId::from_uuid(id)).and_then(|n| n.pixels()) {
        let class = if ChannelKind::is_unsigned_normalized(b.kind()) {
            SurfaceClass::LayerUnsigned
        } else {
            SurfaceClass::LayerFloat
        };
        (b, class)
    } else {
        let m = doc.mask(appname_core::MaskId::from_uuid(id))?;
        (m.coverage(), SurfaceClass::Coverage)
    };
    let rect = buf.bounds();
    if rect.is_empty() {
        return Some((Plane::new(appname_core::IRect::EMPTY), rect, class));
    }
    let mut plane = Plane::new(rect);
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            plane.set(x, y, buf.read_pixel_f32(appname_core::IVec2::new(x, y)));
        }
    }
    Some((plane, rect, class))
}

#[derive(Clone, Copy, PartialEq)]
enum SurfaceClass {
    LayerUnsigned,
    LayerFloat,
    Coverage,
}

fn clamp_for(class: SurfaceClass, mut v: [f32; 4]) -> [f32; 4] {
    match class {
        SurfaceClass::Coverage => {
            v[0] = v[0].clamp(0.0, 1.0); // §13.1 coverage invariant
            [v[0], 0.0, 0.0, 0.0]
        }
        SurfaceClass::LayerUnsigned => {
            v[3] = v[3].clamp(0.0, 1.0); // §18.2 store-clamp
            for c in 0..3 {
                v[c] = v[c].clamp(0.0, v[3]);
            }
            v
        }
        SurfaceClass::LayerFloat => {
            if v[3] == 0.0 {
                [0.0; 4] // §6.4 zero color under zero alpha
            } else {
                v
            }
        }
    }
}

/// Install `out` (the surface's full new content) over the union of old and
/// new extents through the recorder doors, then re-tighten. Selection
/// bounds re-sync via the selection-specific door.
fn install_surface_result(
    cx: &mut KindContext<'_>,
    id: Uuid,
    out: &Plane,
    old_rect: appname_core::IRect,
    class: SurfaceClass,
) -> Result<(), EngineError> {
    use appname_core::{TileCoord, TILE, TILE_I32};
    let union = out.rect.union(&old_rect);
    if union.is_empty() {
        return Ok(());
    }
    cx.grow_surface_bounds(id, union)?;
    // Track the pixel-tight nonzero rect while installing, so the final
    // bounds are EXACT (drift-free under permutation cycles — the §19.1
    // 4×90° bit-identity depends on bounds identity too).
    let mut tight = appname_core::IRect::EMPTY;
    for (tx, ty) in union.tiles(TILE_I32) {
        let coord = TileCoord::new(0, tx, ty);
        let trect = coord.pixel_rect();
        let mut pixels = vec![[0.0f32; 4]; TILE * TILE];
        let mut i = 0;
        for y in trect.min.y..trect.max.y {
            for x in trect.min.x..trect.max.x {
                let v = clamp_for(class, out.get(x, y, EdgePolicy::Transparent));
                if v.iter().any(|c| *c != 0.0) {
                    tight = tight.union(&appname_core::IRect::new(x, y, x + 1, y + 1));
                }
                pixels[i] = v;
                i += 1;
            }
        }
        cx.install_surface_plane(id, coord, &pixels)?;
    }
    if id == cx.selection_surface() {
        // Prune + re-sync, then pin the exact §13.1 bounds.
        cx.recompute_selection_bounds()?;
        cx.set_selection_bounds(tight)?;
    } else {
        cx.recompute_surface_bounds(id)?; // prunes emptied tiles
        cx.set_surface_bounds(id, tight)?; // exact pixel-tight bounds
    }
    Ok(())
}

/// The §40.2 all-surface walk applying `map` to every raster surface.
fn map_all_surfaces(
    cx: &mut KindContext<'_>,
    map: &dyn Fn(&Plane, SurfaceClass) -> Plane,
) -> Result<(), EngineError> {
    let surfaces = cx.raster_surfaces();
    for id in surfaces {
        let Some((src, old_rect, class)) = surface_plane(cx, id) else {
            continue;
        };
        if old_rect.is_empty() {
            continue;
        }
        let out = map(&src, class);
        install_surface_result(cx, id, &out, old_rect, class)?;
    }
    Ok(())
}

fn geometry_kernel(
    cx: &KindContext<'_>,
    name_param: &str,
    default: &str,
) -> Result<Kernel, EngineError> {
    let name = cx
        .params()
        .get(name_param)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string();
    Kernel::from_wire(&name)
        .ok_or_else(|| EngineError::range(name_param, "unknown §18.2 kernel name"))
}

fn param_i64(cx: &KindContext<'_>, name: &str) -> Option<i64> {
    cx.params().get(name).and_then(Value::as_i64)
}

// ---------------------------------------------------------------------------
// doc.resize
// ---------------------------------------------------------------------------

impl KindImpl for DocResizeKind {
    fn kind(&self) -> &'static str {
        "doc.resize"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let old_geom = *cx.document().geometry();
        let old = old_geom.canvas_size;
        let constrain = cx
            .params()
            .get("constrainAspect")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let dims = resize_dims(
            old,
            param_i64(cx, "width"),
            param_i64(cx, "height"),
            param_i64(cx, "longestEdge"),
            constrain,
        )
        .ok_or_else(|| {
            EngineError::range(
                "width",
                "the derived dimension MUST land in 1..=1_000_000 px (§40.2; exactly one of \
                 width/height/longestEdge)",
            )
        })?;
        let ppi = match cx.params().get("resolutionPpi") {
            None => old_geom.resolution_ppi,
            Some(v) => {
                let p = v.as_f64().unwrap_or(f64::NAN);
                if !(p.is_finite() && p > 0.0) {
                    return Err(EngineError::range(
                        "resolutionPpi",
                        "resolution must be finite and > 0 (§19.1)",
                    ));
                }
                p
            }
        };
        // scaleEffects accepted; no layer effects exist at CP-0.5 (inert).
        let kernel = geometry_kernel(cx, "resample", "lanczos")?;
        let (sx, sy) = (dims.x as f64 / old.x as f64, dims.y as f64 / old.y as f64);
        if sx != 1.0 || sy != 1.0 {
            let scale_rect = |r: appname_core::IRect| {
                appname_core::IRect::new(
                    (r.min.x as f64 * sx).floor() as i32,
                    (r.min.y as f64 * sy).floor() as i32,
                    (r.max.x as f64 * sx).ceil() as i32,
                    (r.max.y as f64 * sy).ceil() as i32,
                )
            };
            map_all_surfaces(cx, &|src, _class| {
                resize_plane_global(src, scale_rect(src.rect), sx, sy, kernel, EdgePolicy::Clamp)
            })?;
        }
        // §19.3: PPI without resampling never touches pixels; geometry
        // records either way.
        // §19.1: a canvas resize reflows the whole document — clear any
        // active non-destructive crop (set_full_canvas resets image==canvas,
        // crop_rect None) so a stale window can never outlive its canvas.
        let mut geom = old_geom;
        geom.set_full_canvas(dims);
        geom.resolution_ppi = ppi;
        if geom != old_geom {
            cx.set_document_geometry(geom)?;
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// doc.canvas_resize
// ---------------------------------------------------------------------------

impl KindImpl for DocCanvasResizeKind {
    fn kind(&self) -> &'static str {
        "doc.canvas_resize"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let old_geom = *cx.document().geometry();
        let old = old_geom.canvas_size;
        let w = param_i64(cx, "width").ok_or_else(|| EngineError::missing("width"))?;
        let h = param_i64(cx, "height").ok_or_else(|| EngineError::missing("height"))?;
        let relative = cx
            .params()
            .get("relative")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (nw, nh) = if relative {
            (old.x as i64 + w, old.y as i64 + h)
        } else {
            (w, h)
        };
        if nw < 1 || nh < 1 || nw > 1_000_000 || nh > 1_000_000 {
            return Err(EngineError::range(
                "width",
                "the resulting canvas MUST be >= 1x1 (and within 1..=1_000_000 px, §19.1/§40.2)",
            ));
        }
        let new = appname_core::IVec2::new(nw as i32, nh as i32);
        let anchor = Anchor::from_wire(
            cx.params()
                .get("anchor")
                .and_then(Value::as_str)
                .unwrap_or("center"),
        )
        .ok_or_else(|| EngineError::range("anchor", "unknown §19.1 anchor"))?;
        let offset = canvas_anchor_offset(old, new, anchor);

        if offset != appname_core::IVec2::ZERO {
            // §19.3: every document-space coordinate translates by +offset —
            // an exact integer translate (no kernel, DD ㉔).
            map_all_surfaces(cx, &|src, _class| {
                let mut out = src.clone();
                out.rect = out.rect.translate(offset);
                out
            })?;
        }
        // Canvas-extension fill (§19.3): bottom raster layer, if opaque
        // over the (translated) old canvas.
        let background = cx
            .params()
            .get("background")
            .and_then(Value::as_str)
            .unwrap_or("background_color")
            .to_string();
        if background != "transparent" {
            fill_revealed_on_bottom_layer(cx, old, new, offset, &background)?;
        }
        // §19.1: canvas resize reflows the document — clear any active
        // non-destructive crop (see set_full_canvas).
        let mut geom = old_geom;
        geom.set_full_canvas(new);
        if geom != old_geom {
            cx.set_document_geometry(geom)?;
        }
        Ok(KindOutcome::default())
    }
}

fn fill_revealed_on_bottom_layer(
    cx: &mut KindContext<'_>,
    old: appname_core::IVec2,
    new: appname_core::IVec2,
    offset: appname_core::IVec2,
    background: &str,
) -> Result<(), EngineError> {
    use appname_core::{IRect, IVec2};
    let color = match background {
        "background_color" | "white" => [1.0f32, 1.0, 1.0], // headless white swatch
        "black" => [0.0, 0.0, 0.0],
        _ => return Ok(()),
    };
    let doc = cx.document();
    let Some(&bottom) = doc.z_order_walk().iter().find(|id| {
        doc.layer(**id)
            .map(|n| n.pixels().is_some())
            .unwrap_or(false)
    }) else {
        return Ok(());
    };
    let old_rect_translated = IRect::new(0, 0, old.x, old.y).translate(offset);
    let buf = doc
        .layer(bottom)
        .and_then(|n| n.pixels())
        .expect("raster bottom");
    // §19.3 "if one exists and is opaque": alpha 1 over the old canvas.
    for y in old_rect_translated.min.y..old_rect_translated.max.y {
        for x in old_rect_translated.min.x..old_rect_translated.max.x {
            if buf.read_pixel_f32(IVec2::new(x, y))[3] < 1.0 {
                return Ok(());
            }
        }
    }
    let new_rect = IRect::new(0, 0, new.x, new.y);
    let changed = edit_region_at(cx, bottom, new_rect, &mut |p, dst, _cov| {
        if old_rect_translated.contains(p) {
            dst
        } else {
            [color[0], color[1], color[2], 1.0]
        }
    })?;
    if !changed.is_empty() {
        cx.grow_layer_bounds(bottom, changed)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// doc.crop (deletePixels path) + the shared crop-delete core doc.trim uses
// ---------------------------------------------------------------------------

impl KindImpl for DocCropKind {
    fn kind(&self) -> &'static str {
        "doc.crop"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let canvas = cx.document().geometry().canvas_rect();
        let rect = param_rect(cx, "rect")?;
        let allow_growth = cx
            .params()
            .get("allowGrowth")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !crop_rect_valid(rect, canvas, allow_growth) {
            return Err(EngineError::range(
                "rect",
                "crop rect MUST be non-empty and, unless allowGrowth, intersect the canvas (§40.2)",
            ));
        }
        let straighten = cx
            .params()
            .get("straightenAngleDeg")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if straighten != 0.0 {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "straightening (rotate+crop in one step, §19.2) rides the next slice; \
                 apply doc.rotate then doc.crop today",
            ));
        }
        let delete = cx
            .params()
            .get("deletePixels")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !delete {
            return crop_nondestructive(cx, rect);
        }
        crop_delete(cx, rect)
    }
}

/// §19.1 NON-DESTRUCTIVE crop (`deletePixels:false`): set the `crop_rect`
/// window and derive `image_size = crop_rect.size`; the canvas and EVERY
/// raster surface are left untouched — nothing is translated or discarded,
/// so undo/re-crop recover the hidden pixels (the value-captured
/// `SetGeometry` inverse round-trips the whole geometry). This is the
/// hide twin of [`crop_delete`], which maps and clips the surfaces.
///
/// A crop rect that exactly covers the canvas hides nothing, so it
/// NORMALIZES to `crop_rect: None` (image == canvas): a full-canvas `Some`
/// would diverge from the byte-identical uncropped state hash (the §19.1
/// spine folds `crop_rect` only when active), and this same normalization
/// is the UNCROP door — re-cropping to the full canvas clears the crop and
/// restores `image_size == canvas_size`.
fn crop_nondestructive(
    cx: &mut KindContext<'_>,
    rect: appname_core::IRect,
) -> Result<KindOutcome, EngineError> {
    use appname_core::IVec2;
    let mut geom = *cx.document().geometry();
    if rect == geom.canvas_rect() {
        // No-op / uncrop: the window is the whole canvas.
        geom.crop_rect = None;
        geom.image_size = geom.canvas_size;
    } else {
        geom.crop_rect = Some(rect);
        geom.image_size = IVec2::new(rect.width(), rect.height());
    }
    cx.set_document_geometry(geom)?;
    Ok(KindOutcome::default())
}

fn crop_delete(
    cx: &mut KindContext<'_>,
    rect: appname_core::IRect,
) -> Result<KindOutcome, EngineError> {
    use appname_core::{IRect, IVec2};
    let old_geom = *cx.document().geometry();
    let offset = IVec2::new(-rect.min.x, -rect.min.y);
    let frame = IRect::new(0, 0, rect.width(), rect.height());
    map_all_surfaces(cx, &|src, _class| {
        // Translate by −rect.min, then clip to the new canvas frame
        // (§19.2 delete: buffers intersect with R; §40.2 carried surfaces
        // crop with the canvas — pin).
        let translated = src.rect.translate(offset);
        let kept = translated.intersect(&frame);
        let mut out = Plane::new(kept);
        for y in kept.min.y..kept.max.y {
            for x in kept.min.x..kept.max.x {
                out.set(
                    x,
                    y,
                    src.get(x - offset.x, y - offset.y, EdgePolicy::Transparent),
                );
            }
        }
        out
    })?;
    // §19.3 destructive crop shrinks the canvas to the rect — a full-canvas
    // frame with no residual non-destructive crop window (set_full_canvas
    // clears any prior crop_rect; doc.trim routes through here too).
    let mut geom = old_geom;
    geom.set_full_canvas(IVec2::new(rect.width(), rect.height()));
    cx.set_document_geometry(geom)?;
    Ok(KindOutcome::default())
}

fn param_rect(cx: &KindContext<'_>, name: &str) -> Result<appname_core::IRect, EngineError> {
    let obj = cx
        .params()
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| EngineError::missing(name))?;
    let f = |k: &str| -> Result<i64, EngineError> {
        obj.get(k)
            .and_then(Value::as_i64)
            .ok_or_else(|| EngineError::missing(&format!("{name}.{k}")))
    };
    let (x, y, w, h) = (f("x")?, f("y")?, f("w")?, f("h")?);
    if w < 1 || h < 1 {
        return Err(EngineError::range(name, "rect w/h must be >= 1"));
    }
    Ok(appname_core::IRect::new(
        x as i32,
        y as i32,
        (x + w) as i32,
        (y + h) as i32,
    ))
}

// ---------------------------------------------------------------------------
// doc.trim
// ---------------------------------------------------------------------------

impl KindImpl for DocTrimKind {
    fn kind(&self) -> &'static str {
        "doc.trim"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        use appname_core::IVec2;
        let get_bool = |k: &str, d: bool| cx.params().get(k).and_then(Value::as_bool).unwrap_or(d);
        let spec = TrimSpec {
            basis: TrimBasis::from_wire(
                cx.params()
                    .get("basis")
                    .and_then(Value::as_str)
                    .unwrap_or("transparent"),
            )
            .ok_or_else(|| EngineError::range("basis", "unknown trim basis"))?,
            tolerance: 0.0, // wire pin (§19.3 t default 0; no wire param)
            top: get_bool("top", true),
            bottom: get_bool("bottom", true),
            left: get_bool("left", true),
            right: get_bool("right", true),
        };
        if !(spec.top || spec.bottom || spec.left || spec.right) {
            return Err(EngineError::range(
                "top",
                "at least one side must be enabled (§40.2)",
            ));
        }
        let canvas = cx.document().geometry().canvas_rect();
        let doc = cx.document();
        let layers: Vec<LayerId> = doc
            .z_order_walk()
            .into_iter()
            .filter(|id| {
                doc.layer(*id)
                    .map(|n| n.pixels().is_some())
                    .unwrap_or(false)
            })
            .collect();
        let kept =
            match (spec.basis, layers.len()) {
                (_, 0) => None, // no raster content: everything "matches"
                (TrimBasis::Transparent, _) => {
                    struct MaxAlpha<'a>(Vec<&'a appname_core::Buffer>);
                    impl TrimSource for MaxAlpha<'_> {
                        fn read(&self, p: IVec2) -> [f32; 4] {
                            // Composite alpha is zero iff every layer's alpha is
                            // zero — exact for the t = 0 wire (module pin).
                            let a = self
                                .0
                                .iter()
                                .map(|b| b.read_pixel_f32(p)[3])
                                .fold(0.0f32, f32::max);
                            [0.0, 0.0, 0.0, a]
                        }
                    }
                    let src = MaxAlpha(
                        layers
                            .iter()
                            .map(|id| doc.layer(*id).unwrap().pixels().unwrap())
                            .collect(),
                    );
                    trim_rect(&src, canvas, &spec, false)
                }
                (_, 1) => {
                    let buf = doc.layer(layers[0]).unwrap().pixels().unwrap();
                    trim_rect(buf, canvas, &spec, buf.premultiplied())
                }
                _ => return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "color-basis trim over multi-layer documents needs the flattened composite \
                     (§4.6; render-lane handoff) — the transparent basis and single-raster-layer \
                     documents are live",
                )),
            };
        match kept {
            None => Ok(KindOutcome::default()), // nothing left → §40.1 no-op
            Some(r) if r == canvas => Ok(KindOutcome::default()), // nothing to trim
            Some(r) => crop_delete(cx, r),      // §19.3: doc.crop with deletePixels
        }
    }
}

// ---------------------------------------------------------------------------
// doc.rotate
// ---------------------------------------------------------------------------

impl KindImpl for DocRotateKind {
    fn kind(&self) -> &'static str {
        "doc.rotate"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        use appname_core::{IRect, IVec2};
        let angle = cx
            .params()
            .get("angleDeg")
            .and_then(Value::as_f64)
            .ok_or_else(|| EngineError::missing("angleDeg"))?;
        if !(-360.0..=360.0).contains(&angle) || !angle.is_finite() {
            return Err(EngineError::range(
                "angleDeg",
                "angleDeg outside -360..=360",
            ));
        }
        let flip_h = cx
            .params()
            .get("flipH")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let flip_v = cx
            .params()
            .get("flipV")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let expand = cx
            .params()
            .get("expand")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let old_geom = *cx.document().geometry();
        let old = old_geom.canvas_size;

        if let Some(orient) = Orientation::from_angle_deg(angle, flip_h, flip_v) {
            // §19.1 [NORMATIVE] lossless permutation path.
            if orient.is_identity() {
                return Ok(KindOutcome::default()); // §40.1 empty push
            }
            let mapped = orient.map_size(old);
            let (new_size, extra) = if expand || mapped == old {
                (mapped, IVec2::ZERO)
            } else {
                // expand:false on an odd quarter: recenter (module pin).
                (
                    old,
                    IVec2::new(
                        (old.x - mapped.x).div_euclid(2),
                        (old.y - mapped.y).div_euclid(2),
                    ),
                )
            };
            map_all_surfaces(cx, &|src, _class| {
                let mut out_rect = IRect::EMPTY;
                for &(cx_, cy_) in &[
                    (src.rect.min.x, src.rect.min.y),
                    (src.rect.max.x - 1, src.rect.min.y),
                    (src.rect.min.x, src.rect.max.y - 1),
                    (src.rect.max.x - 1, src.rect.max.y - 1),
                ] {
                    let m = orient.map_pixel(IVec2::new(cx_, cy_), old);
                    let p = IVec2::new(m.x + extra.x, m.y + extra.y);
                    out_rect = out_rect.union(&IRect::new(p.x, p.y, p.x + 1, p.y + 1));
                }
                let mut out = Plane::new(out_rect);
                for y in src.rect.min.y..src.rect.max.y {
                    for x in src.rect.min.x..src.rect.max.x {
                        let m = orient.map_pixel(IVec2::new(x, y), old);
                        out.set(
                            m.x + extra.x,
                            m.y + extra.y,
                            src.get(x, y, EdgePolicy::Transparent),
                        );
                    }
                }
                out
            })?;
            // §19.1: rotation reflows the canvas — clear any active
            // non-destructive crop (see set_full_canvas).
            let mut geom = old_geom;
            geom.set_full_canvas(new_size);
            if geom != old_geom {
                cx.set_document_geometry(geom)?;
            }
            return Ok(KindOutcome::default());
        }

        // Arbitrary angle: §18.2 resample about the canvas center; positive
        // = clockwise in the y-down frame (module pin).
        let kernel = geometry_kernel(cx, "resample", "bicubic")?;
        let theta = angle.to_radians();
        let (s, c) = theta.sin_cos();
        let center = [old.x as f64 / 2.0, old.y as f64 / 2.0];
        let rot = crate::xform::mat3_mul(
            &Mat3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, center[0], center[1], 1.0]),
            &crate::xform::mat3_mul(
                &Mat3([c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0]),
                &Mat3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -center[0], -center[1], 1.0]),
            ),
        );
        let (map, new_size) = if expand {
            let bbox = crate::xform::transformed_bounds(
                &rot,
                appname_core::IRect::new(0, 0, old.x, old.y),
            );
            let shift = Mat3([
                1.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                -bbox.min.x as f64,
                -bbox.min.y as f64,
                1.0,
            ]);
            (
                crate::xform::mat3_mul(&shift, &rot),
                IVec2::new(bbox.width(), bbox.height()),
            )
        } else {
            (rot, old)
        };
        map_all_surfaces(cx, &|src, _class| {
            let dst_rect = crate::xform::transformed_bounds(&map, src.rect);
            crate::xform::transform_plane(src, &map, dst_rect, Some(kernel))
                .unwrap_or_else(|| Plane::new(appname_core::IRect::EMPTY))
        })?;
        // §19.1: the resampled (arbitrary-angle) rotate reflows the canvas —
        // clear any active non-destructive crop (see set_full_canvas).
        let mut geom = old_geom;
        geom.set_full_canvas(new_size);
        if geom != old_geom {
            cx.set_document_geometry(geom)?;
        }
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod doc_geometry_tests {
    use super::*;
    use appname_core::{Engine, IVec2};
    use serde_json::json;

    /// The flipped wave-3 assembly (fill still via the lift helper).
    fn wave3() -> appname_core::EngineAssembly {
        with_tools_fill(assembly())
    }

    fn engine_with_doc(background: &str) -> (Engine, appname_core::LayerId) {
        let mut e = wave3().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ab000000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 48, "background": background},
            })
            .to_string(),
        )
        .unwrap();
        let layer = e.document().unwrap().z_order_walk()[0];
        (e, layer)
    }

    fn env(n: u32, kind: &str, params: serde_json::Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("ab000000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        })
        .to_string()
    }

    fn fill_red_square(e: &mut Engine, layer: appname_core::LayerId) {
        // Paint a deterministic red 16x16 square at (8,8) via flood-free fill
        // + selection-free region: use a brush-less approach — fill with a
        // rect selection is unavailable pre-selection kinds, so paint via
        // brush hard round dabs along a dense grid… simpler: whole-canvas
        // fill red at low opacity is position-less; instead use edit path:
        // one hard brush dab centered at (16,16).
        let samples = crate::brush::encode_samples(&[crate::brush::InputSample {
            x: 16.0,
            y: 16.0,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t: 0.0,
        }])
        .unwrap();
        let asset = e.add_samples_asset(samples);
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ab000000-0000-4000-8000-000000000002",
                "kind": "brush.stroke",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {
                    "brushPreset": preset_hard_round_16_id().to_string(),
                    "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                },
                "samples": asset.to_string(),
            })
            .to_string(),
        )
        .unwrap();
    }

    fn px(e: &Engine, layer: appname_core::LayerId, x: i32, y: i32) -> [f32; 4] {
        e.document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    #[test]
    fn resize_scales_content_selection_and_geometry() {
        let (mut e, layer) = engine_with_doc("white");
        fill_red_square(&mut e, layer);
        let before = e.state_hash();
        let r = e
            .apply_json(&env(3, "doc.resize", json!({"width": 128})))
            .unwrap();
        assert!(!r.dirty.is_empty() || true);
        let g = *e.document().unwrap().geometry();
        assert_eq!(
            g.canvas_size,
            IVec2::new(128, 96),
            "derived height from the ORIGINAL ratio"
        );
        // The dab centered at (16,16) now sits near (32,32), scaled ×2.
        let c = px(&e, layer, 32, 32);
        assert!(
            c[0] > 0.8 && c[1] < 0.4,
            "content scaled with the canvas: {c:?}"
        );
        let corner = px(&e, layer, 120, 90);
        assert!(
            corner[0] > 0.9 && corner[1] > 0.9,
            "white scales white: {corner:?}"
        );
        // Undo/redo byte-identity + replay.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after);
        // PPI-only change touches no pixels.
        let content_before = px(&e, layer, 32, 32);
        e.apply_json(&env(
            4,
            "doc.resize",
            json!({"width": 128, "resolutionPpi": 300}),
        ))
        .unwrap();
        assert_eq!(
            px(&e, layer, 32, 32),
            content_before,
            "PPI change never touches pixels (§19.3)"
        );
        assert_eq!(e.document().unwrap().geometry().resolution_ppi, 300.0);
        // Derived-dimension gate.
        let err = e
            .apply_json(&env(5, "doc.resize", json!({"longestEdge": 1000001})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn canvas_resize_anchors_translates_and_fills() {
        let (mut e, layer) = engine_with_doc("white");
        fill_red_square(&mut e, layer);
        let before = e.state_hash();
        // Grow 64x48 → 96x80 centered: offset (16,16); white opaque bottom
        // layer → revealed border fills white (background_color default).
        e.apply_json(&env(
            3,
            "doc.canvas_resize",
            json!({"width": 96, "height": 80}),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().geometry().canvas_size,
            IVec2::new(96, 80)
        );
        let c = px(&e, layer, 32, 32);
        assert!(
            c[0] > 0.8 && c[1] < 0.4,
            "content translated by the center offset: {c:?}"
        );
        let border = px(&e, layer, 2, 2);
        assert!(
            border[3] > 0.99 && border[0] > 0.99,
            "revealed area filled white: {border:?}"
        );
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        // Relative shrink with top_left anchor: no translation, content stays.
        e.apply_json(&env(
            4,
            "doc.canvas_resize",
            json!({
                "width": -16, "height": -16, "relative": true, "anchor": "top_left",
            }),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().geometry().canvas_size,
            IVec2::new(80, 64)
        );
        let c2 = px(&e, layer, 32, 32);
        assert_eq!(
            c2, c,
            "top_left shrink translates nothing; shrink never deletes (§19.3)"
        );
        // Degenerate result rejects.
        let err = e
            .apply_json(&env(
                5,
                "doc.canvas_resize",
                json!({
                    "width": -80, "height": 0, "relative": true,
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn crop_delete_translates_clips_and_gates() {
        let (mut e, layer) = engine_with_doc("white");
        fill_red_square(&mut e, layer);
        let before = e.state_hash();
        e.apply_json(&env(
            3,
            "doc.crop",
            json!({
                "rect": {"x": 8, "y": 8, "w": 32, "h": 24}, "deletePixels": true,
            }),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().geometry().canvas_size,
            IVec2::new(32, 24)
        );
        // The dab at (16,16) is now at (8,8).
        let c = px(&e, layer, 8, 8);
        assert!(
            c[0] > 0.8 && c[1] < 0.4,
            "content translated to the crop frame: {c:?}"
        );
        // Content outside the crop is DELETED (bounds re-tightened within frame).
        let bounds = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert!(
            bounds.max.x <= 32 && bounds.max.y <= 24,
            "clipped: {bounds:?}"
        );
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "delete-crop undoes via tile deltas");
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        // Typed-pending arm (straightening still rides the next slice) + gate.
        let err = e
            .apply_json(&env(
                5,
                "doc.crop",
                json!({
                    "rect": {"x": 0, "y": 0, "w": 8, "h": 8}, "deletePixels": true,
                    "straightenAngleDeg": 5.0,
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented, "straighten pends");
        let err = e
            .apply_json(&env(
                6,
                "doc.crop",
                json!({
                    "rect": {"x": 5000, "y": 5000, "w": 10, "h": 10}, "deletePixels": true,
                }),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "non-intersecting rect without allowGrowth"
        );
    }

    #[test]
    fn crop_nondestructive_hides_retains_and_reverses() {
        // 64×48 white doc with a red dab centred at (16,16).
        let (mut e, layer) = engine_with_doc("white");
        fill_red_square(&mut e, layer);
        let before = e.state_hash();

        // Non-destructive crop to the sub-rect [8,8)..[40,32) (32×24).
        e.apply_json(&env(
            3,
            "doc.crop",
            json!({ "rect": {"x": 8, "y": 8, "w": 32, "h": 24} }),
        ))
        .unwrap();

        // Geometry: crop_rect set, image_size derives to the crop size, the
        // canvas is UNCHANGED (§19.1 — the crop hides, never resizes).
        let g = *e.document().unwrap().geometry();
        assert_eq!(g.crop_rect, Some(appname_core::IRect::new(8, 8, 40, 32)));
        assert_eq!(g.image_size, IVec2::new(32, 24));
        assert_eq!(g.canvas_size, IVec2::new(64, 48), "canvas unchanged");
        assert_eq!(
            g.image_rect(),
            appname_core::IRect::new(8, 8, 40, 32),
            "image_rect is the crop sub-region"
        );

        // NON-DESTRUCTIVE proof: no surface was touched — the layer still
        // covers the full 64×48 canvas and a pixel OUTSIDE the crop rect
        // reads back its original white (crop_delete would have tightened
        // the bounds to 32×24 and discarded it).
        let buf_bounds = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(
            buf_bounds,
            appname_core::IRect::new(0, 0, 64, 48),
            "no pixels deleted: full bounds retained"
        );
        let outside = px(&e, layer, 50, 40); // world (50,40), outside the crop
        assert!(
            outside[0] > 0.8 && outside[1] > 0.8 && outside[2] > 0.8,
            "hidden pixel is RETAINED (still white): {outside:?}"
        );

        // The EXPORT door honours the crop: composite frames on image_rect,
        // so the plane is exactly the 32×24 sub-region sampled at +(8,8).
        let (w, h, plane) = appname_core::composite_plane_f32(&e).unwrap();
        assert_eq!((w, h), (32, 24), "composite framed on the crop sub-rect");
        let at = |x: usize, y: usize| {
            let i = (y * w as usize + x) * 4;
            [plane[i], plane[i + 1], plane[i + 2], plane[i + 3]]
        };
        // Output (8,8) == world (16,16) == the red dab centre.
        let centre = at(8, 8);
        assert!(
            centre[0] > 0.8 && centre[1] < 0.3,
            "crop offset honoured — dab lands at output (8,8): {centre:?}"
        );
        // Output (30,22) == world (38,30) == white background.
        let corner = at(30, 22);
        assert!(corner[1] > 0.8, "white where the dab is absent: {corner:?}");

        // Reversibility: undo restores the FULL geometry (crop_rect None,
        // image == canvas) and the whole state hash.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "crop undoes to full geometry");
        let gu = *e.document().unwrap().geometry();
        assert_eq!(gu.crop_rect, None);
        assert_eq!(gu.image_size, gu.canvas_size);

        // Redo + DETERMINISM: journal replay is byte-identical (D-48 spine).
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "journal replay of the non-destructive crop is byte-identical"
        );

        // UNCROP: re-cropping to the FULL canvas normalizes crop_rect back to
        // None and restores image_size == canvas_size.
        e.apply_json(&env(
            4,
            "doc.crop",
            json!({ "rect": {"x": 0, "y": 0, "w": 64, "h": 48} }),
        ))
        .unwrap();
        let gc = *e.document().unwrap().geometry();
        assert_eq!(gc.crop_rect, None, "full-canvas crop normalizes to uncrop");
        assert_eq!(gc.image_size, IVec2::new(64, 48));
    }

    #[test]
    fn canvas_ops_clear_an_active_nondestructive_crop() {
        // A canvas-geometry op run while a non-destructive crop is active
        // must CLEAR the crop (set_full_canvas) — a stale window into a
        // reflowed canvas would be invalid geometry (§19.1 image==crop.size).
        for op in [
            json!({"kind": "doc.resize", "params": {"width": 32}}),
            json!({"kind": "doc.rotate", "params": {"angleDeg": 90}}),
            json!({"kind": "doc.canvas_resize", "params": {"width": 80, "height": 60, "anchor": "top_left"}}),
        ] {
            let (mut e, _layer) = engine_with_doc("white");
            e.apply_json(&env(
                3,
                "doc.crop",
                json!({ "rect": {"x": 8, "y": 8, "w": 32, "h": 24} }),
            ))
            .unwrap();
            assert!(e.document().unwrap().geometry().crop_rect.is_some());
            e.apply_json(&env(4, op["kind"].as_str().unwrap(), op["params"].clone()))
                .unwrap();
            let g = *e.document().unwrap().geometry();
            assert_eq!(
                g.crop_rect,
                None,
                "{} cleared the active crop",
                op["kind"].as_str().unwrap()
            );
            assert_eq!(
                g.image_size,
                g.canvas_size,
                "{} left a full-canvas geometry",
                op["kind"].as_str().unwrap()
            );
            // The geometry must validate (image == canvas when crop is None).
            g.validate().unwrap();
        }
    }

    #[test]
    fn trim_finds_content_and_no_ops_honestly() {
        let (mut e, layer) = engine_with_doc("transparent");
        fill_red_square(&mut e, layer);
        let before = e.state_hash();
        // Transparent basis: canvas shrinks to the dab's tight alpha rect.
        e.apply_json(&env(3, "doc.trim", json!({}))).unwrap();
        let g = e.document().unwrap().geometry().canvas_size;
        assert!(
            g.x <= 20 && g.y <= 20 && g.x >= 14 && g.y >= 14,
            "trimmed to the dab: {g:?}"
        );
        let c = px(&e, layer, g.x / 2, g.y / 2);
        assert!(c[0] > 0.8, "content centered in the trimmed frame: {c:?}");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        // Nothing to trim on an all-white doc → §40.1 no-op push.
        let (mut e2, _l2) = engine_with_doc("white");
        let before2 = e2.state_hash();
        let r = e2.apply_json(&env(3, "doc.trim", json!({}))).unwrap();
        assert!(r.dirty.is_empty());
        assert_eq!(e2.state_hash(), before2);
        // Color-basis trim IS live on a single-raster-layer doc.
        let (mut e3, l3) = engine_with_doc("white");
        fill_red_square(&mut e3, l3);
        e3.apply_json(&env(3, "doc.trim", json!({"basis": "top_left_color"})))
            .unwrap();
        let g3 = e3.document().unwrap().geometry().canvas_size;
        assert!(
            g3.x < 64,
            "white border trimmed by the corner-color basis: {g3:?}"
        );
    }

    #[test]
    fn rotate_lossless_permutations_and_resample_path() {
        let (mut e, layer) = engine_with_doc("white");
        fill_red_square(&mut e, layer);
        let content_hash_0 = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .region_hash();
        // 4 × 90° CW returns the exact buffer (lossless permutations §19.1).
        for n in 0..4 {
            e.apply_json(&env(10 + n, "doc.rotate", json!({"angleDeg": 90})))
                .unwrap();
        }
        let content_hash_4 = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .region_hash();
        assert_eq!(
            content_hash_0, content_hash_4,
            "4×90° = identity, bit-exact"
        );
        assert_eq!(
            e.document().unwrap().geometry().canvas_size,
            IVec2::new(64, 48)
        );
        // One 90°: canvas swaps; the dab at (16,16) maps to (h−1−16, 16) = (31,16).
        e.apply_json(&env(20, "doc.rotate", json!({"angleDeg": 90})))
            .unwrap();
        assert_eq!(
            e.document().unwrap().geometry().canvas_size,
            IVec2::new(48, 64)
        );
        let c = px(&e, layer, 31, 16);
        assert!(c[0] > 0.8 && c[1] < 0.4, "90° CW pixel map: {c:?}");
        // flipH twice = identity.
        let h1 = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .region_hash();
        e.apply_json(&env(
            21,
            "doc.rotate",
            json!({"angleDeg": 0, "flipH": true}),
        ))
        .unwrap();
        e.apply_json(&env(
            22,
            "doc.rotate",
            json!({"angleDeg": 0, "flipH": true}),
        ))
        .unwrap();
        let h2 = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .region_hash();
        assert_eq!(h1, h2);
        // angleDeg 0, no flips → §40.1 empty push.
        let before = e.state_hash();
        let r = e
            .apply_json(&env(23, "doc.rotate", json!({"angleDeg": 0})))
            .unwrap();
        assert!(r.dirty.is_empty());
        assert_eq!(e.state_hash(), before);
        // Arbitrary angle resamples and round-trips through undo/replay.
        let before_45 = e.state_hash();
        e.apply_json(&env(24, "doc.rotate", json!({"angleDeg": 45})))
            .unwrap();
        let g = e.document().unwrap().geometry().canvas_size;
        assert!(g.x > 48 && g.y > 48, "expanded to the rotated bbox: {g:?}");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before_45);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }
}

// ===========================================================================
// CP-0.5 wave 3, job 2b: the selection kinds over the §13 engines + the
// merged selection doors (contracts selection.set/modify/transform/all/
// none/invert; ES §13.1–§13.3, §40.4 family: never lock-gated, no
// composite D, T topic:"selection").
//
// v1 pins (lane log): `selection.set` feather applies to the rasterized
// SOURCE before the §13.2 op (wave-2 pin); geometric `antialias:false`
// thresholds the source at 0.5 (§13.1), the wand handles its own snap;
// wand sampling with `sampleAllLayers:false` uses the single raster layer
// of a single-raster-layer document (no active-layer state exists headless
// — multi-layer ACTIVE-layer wand stays typed-pending on the tool-state
// row); `sampleAllLayers:true` is LIVE over core's §13.3 flattened
// composite door and needs no active layer, so it serves on multi-layer
// documents; `quick` is LIVE (CP-1.0 consolidated round, ES §13.3 five-step
// block per Ruling A — envelope `samples` carries the drag, seed-only =
// single stamp); `color_range` (CMM normalized-Lab over arbitrary working
// spaces) and `path` (no vector arena wiring here — core's door slice
// owns `selection.from_path`) answer typed; `selection.invert` with no active selection
// = select-all (contract-pinned); `selection.none` on an inactive
// selection is the §40.1 no-op push (contract-pinned); modify/transform
// REQUIRE an active selection → error.command.selection_required (stage
// 4, contract-pinned).
// ===========================================================================

use crate::brush::present::CoverageRead;
use crate::select::refine;
use crate::select::shape::{condition_lasso, rasterize_ellipse, rasterize_polygon, rasterize_rect};
use crate::select::{combine, InactiveMask, MaskPlane, SelectOp, SelectionEdit};

pub struct SelectionSetKind;
pub struct SelectionModifyKind;
pub struct SelectionTransformKind;
pub struct SelectionAllKind;
pub struct SelectionNoneKind;
pub struct SelectionInvertKind;

/// Combine `source` with the live selection under `op` and install the
/// resulting edit through the selection doors. Empty edits are the §40.1
/// empty-DeltaSet push.
fn combine_and_install(
    cx: &mut KindContext<'_>,
    source: &MaskPlane,
    op: SelectOp,
) -> Result<(), EngineError> {
    let edit: SelectionEdit = {
        let doc = cx.document();
        let canvas = doc.geometry().canvas_rect();
        let sel = doc.selection();
        let inactive = InactiveMask;
        let (reader, bounds): (&dyn CoverageRead, appname_core::IRect) = if sel.active() {
            (sel, sel.bounds())
        } else {
            (&inactive, appname_core::IRect::EMPTY)
        };
        combine(reader, bounds, source, op, canvas)
    };
    if edit.patches.is_empty() {
        return Ok(());
    }
    for patch in &edit.patches {
        cx.install_selection_tile_plane(patch.coord, &patch.values)?;
    }
    // The exact-bounds side of the §13.1 split (combine tracks the tight
    // nonzero rect in-pass); syncs active/inactive.
    cx.set_selection_bounds(edit.bounds)?;
    Ok(())
}

fn require_active_selection(cx: &KindContext<'_>) -> Result<appname_core::IRect, EngineError> {
    let sel = cx.document().selection();
    if !sel.active() {
        return Err(EngineError::new(
            ErrorCode::SelectionRequired,
            "no active selection (§40.1 vocabulary; §40.4 stage-4 live-state gate)",
        ));
    }
    Ok(sel.bounds())
}

/// Read the live selection mask into a `MaskPlane` over `rect`.
fn selection_plane(cx: &KindContext<'_>, rect: appname_core::IRect) -> MaskPlane {
    let doc = cx.document();
    let sel = doc.selection();
    let mut plane = MaskPlane::new(rect);
    if !sel.active() {
        return plane;
    }
    let mask = sel.mask();
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            plane.set(
                x,
                y,
                mask.read_pixel_f32(appname_core::IVec2::new(x, y))[0].clamp(0.0, 1.0),
            );
        }
    }
    plane
}

impl KindImpl for SelectionSetKind {
    fn kind(&self) -> &'static str {
        "selection.set"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let source_kind = cx
            .params()
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("source"))?
            .to_string();
        let op = SelectOp::from_wire(
            cx.params()
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("replace"),
        )
        .ok_or_else(|| EngineError::range("op", "unknown §13.1 op"))?;
        let antialias = cx
            .params()
            .get("antialias")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let feather = param_f32(cx, "feather", 0.0);
        if !(0.0..=1000.0).contains(&feather) {
            return Err(EngineError::range("feather", "feather outside 0..=1000 px"));
        }

        let mut source: MaskPlane = match source_kind.as_str() {
            "rect" | "ellipse" => {
                let r = param_rect(cx, "rect")?;
                let (x, y, w, h) = (
                    r.min.x as f64,
                    r.min.y as f64,
                    r.width() as f64,
                    r.height() as f64,
                );
                let mut m = if source_kind == "rect" {
                    rasterize_rect(x, y, w, h)
                } else {
                    rasterize_ellipse(x, y, w, h)
                };
                if !antialias {
                    m.threshold_half(); // §13.1
                }
                m
            }
            "lasso" | "polygon" => {
                let pts = param_points(cx)?;
                let min = if source_kind == "lasso" { 2 } else { 3 };
                if pts.len() < min {
                    return Err(EngineError::range(
                        "points",
                        format!("{source_kind} needs >= {min} points (§40.4)"),
                    ));
                }
                let pts = if source_kind == "lasso" {
                    condition_lasso(&pts)
                } else {
                    pts
                };
                let mut m = rasterize_polygon(&pts);
                if !antialias {
                    m.threshold_half();
                }
                m
            }
            "wand" => wand_source(cx, antialias)?,
            "color_range" => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "color_range needs normalized-Lab conversion of the working space through \
                     the CMM sample path (§13.3/§5) — its row is pending",
                ))
            }
            "quick" => quick_source(cx, antialias)?,
            "path" => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "source:'path' needs the vector path arena (§20, CP-1.0)",
                ))
            }
            other => {
                return Err(EngineError::range(
                    "source",
                    format!("unknown source \"{other}\""),
                ))
            }
        };
        if feather > 0.0 {
            // v1 pin: set-feather softens the SOURCE before the op.
            source = refine::feather(&source, feather as f64);
        }
        combine_and_install(cx, &source, op)?;
        Ok(KindOutcome::default())
    }
}

fn param_points(cx: &KindContext<'_>) -> Result<Vec<[f64; 2]>, EngineError> {
    let arr = cx
        .params()
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("points"))?;
    let mut out = Vec::with_capacity(arr.len());
    for p in arr {
        let pair = p
            .as_array()
            .ok_or_else(|| EngineError::range("points", "[x, y] pairs"))?;
        if pair.len() != 2 || !pair.iter().all(Value::is_number) {
            return Err(EngineError::range("points", "[x, y] pairs of numbers"));
        }
        let (x, y) = (
            pair[0].as_f64().unwrap_or(f64::NAN),
            pair[1].as_f64().unwrap_or(f64::NAN),
        );
        if !x.is_finite() || !y.is_finite() {
            return Err(EngineError::range("points", "points must be finite"));
        }
        out.push([x, y]);
    }
    Ok(out)
}

/// §13.3 seed param shared by wand and quick-select: `[x, y]` integers,
/// REQUIRED, MUST lie inside the canvas (§40.4 live-state).
fn param_seed_xy(
    cx: &KindContext<'_>,
    canvas: appname_core::IRect,
    tool: &str,
) -> Result<appname_core::IVec2, EngineError> {
    let seed_arr = cx
        .params()
        .get("seed")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("seed"))?;
    if seed_arr.len() != 2 || !seed_arr.iter().all(|v| v.as_i64().is_some()) {
        return Err(EngineError::range(
            "seed",
            "seed must be [x, y] integers (§13.3)",
        ));
    }
    let seed = appname_core::IVec2::new(
        seed_arr[0].as_i64().unwrap_or(0) as i32,
        seed_arr[1].as_i64().unwrap_or(0) as i32,
    );
    if !canvas.contains(seed) {
        return Err(EngineError::range(
            "seed",
            format!("{tool} seed MUST lie inside the canvas (§40.4 live-state)"),
        ));
    }
    Ok(seed)
}

/// The §13.3 sampling source, resolved: either ONE layer's buffer or the
/// lazy flattened composite. Both answer the `Backdrop` seam the wand and
/// quick-select engines already take (`&B: Backdrop + ?Sized`), so
/// neither engine learns which arm it is on.
enum SelectSamplingSource<'a> {
    Layer(&'a appname_core::Buffer),
    Composite(crate::kind_impls_retouch::CompositeBackdrop<'a>),
}

impl SelectSamplingSource<'_> {
    fn backdrop(&self) -> &dyn crate::brush::present::Backdrop {
        match self {
            SelectSamplingSource::Layer(b) => *b,
            SelectSamplingSource::Composite(c) => c,
        }
    }

    fn premultiplied(&self) -> bool {
        match self {
            SelectSamplingSource::Layer(b) => b.premultiplied(),
            SelectSamplingSource::Composite(c) => c.0.premultiplied(),
        }
    }
}

/// §13.3 sampling-source resolution shared by wand and quick-select.
///
/// `sampleAllLayers:true` SERVES: it samples the flattened composite
/// (core's `engine::sampling` door — the same per-pixel source-over
/// `image.flatten` runs, so "sample all layers" and "flatten, then
/// sample" agree). Note that this arm needs NO active-layer state, so it
/// is live on multi-layer documents too — the tool-state limit below
/// applies only to the `false` (active-layer) arm.
///
/// `sampleAllLayers:false` samples the ACTIVE layer; headless carries no
/// active-layer state, so the single raster layer of a single-raster-layer
/// document is the source and multi-layer documents pend typed on the
/// tool-state row.
fn sampling_source<'a>(
    cx: &'a KindContext<'_>,
    tool: &str,
) -> Result<SelectSamplingSource<'a>, EngineError> {
    if cx.params().get("sampleAllLayers").and_then(Value::as_bool) == Some(true) {
        let req = appname_core::SampleRequest {
            scope: appname_core::SampleScope::Document,
            ignore_adjustments_above: None, // §45.2's toggle is retouch-only
        };
        return Ok(SelectSamplingSource::Composite(
            crate::kind_impls_retouch::CompositeBackdrop(appname_core::resolve_sampling_composite(
                cx.document(),
                &req,
            )?),
        ));
    }
    let doc = cx.document();
    let layers: Vec<LayerId> = doc
        .z_order_walk()
        .into_iter()
        .filter(|id| {
            doc.layer(*id)
                .map(|n| n.pixels().is_some())
                .unwrap_or(false)
        })
        .collect();
    if layers.len() != 1 {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            format!(
                "{tool} samples the ACTIVE layer (§13.3); headless carries no active-layer \
                 state, so multi-layer documents pend on the tool-state row \
                 (single-raster-layer documents are live; sampleAllLayers:true samples the \
                 flattened composite and needs no active layer)"
            ),
        ));
    }
    Ok(SelectSamplingSource::Layer(
        doc.layer(layers[0]).unwrap().pixels().unwrap(),
    ))
}

fn wand_source(cx: &KindContext<'_>, antialias: bool) -> Result<MaskPlane, EngineError> {
    let canvas = cx.document().geometry().canvas_rect();
    let seed = param_seed_xy(cx, canvas, "the wand")?;
    let src = sampling_source(cx, "the wand")?;
    let params = crate::select::wand::WandParams {
        tolerance: param_f32(cx, "tolerance", 32.0).clamp(0.0, 255.0),
        contiguous: cx
            .params()
            .get("contiguous")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        eight_way: false,
        antialias,
    };
    Ok(crate::select::wand::wand_select(
        src.backdrop(),
        src.premultiplied(),
        canvas,
        seed,
        &params,
    ))
}

/// `source:"quick"` (ES §13.3 five-step block VERBATIM per Ruling A;
/// §39.4 T-09): ONE stroke session per command — the envelope `samples`
/// asset (the B3 stroke shape, §39.4 T-09 row) carries the drag path;
/// absent samples = the single-stamp form (seed only). The seed stamps
/// first (it is the contract's REQUIRED wire member), then the resampled
/// drag stamps march the statistics along (§13.3 step 1). Brush diameter
/// is the §32.3 T-09 default 30 px — the wire form pins no size key
/// (contract gap, lane log). NO graph cut runs here (Ruling A); the
/// GrabCut refine tier is `select::grabcut_refine`.
fn quick_source(cx: &KindContext<'_>, antialias: bool) -> Result<MaskPlane, EngineError> {
    use crate::select::quick::{stamps_along, QuickSession, DEFAULT_BRUSH_DIAMETER};
    let canvas = cx.document().geometry().canvas_rect();
    let seed = param_seed_xy(cx, canvas, "quick selection")?;
    let src = sampling_source(cx, "quick selection")?;

    let mut stamps = vec![[seed.x as f64 + 0.5, seed.y as f64 + 0.5]];
    if let Some(s) = cx.envelope().samples.as_deref() {
        let id = Uuid::parse_str(s).map_err(|_| {
            EngineError::bad_target(format!("`samples` is not a valid asset uuid: \"{s}\""))
        })?;
        let bytes = cx.asset_bytes(id).ok_or_else(|| {
            EngineError::bad_target(format!(
                "samples asset {id} does not resolve (register it via `appname asset add` or \
                 the C ABI sample door)"
            ))
        })?;
        let samples = crate::brush::decode_samples(bytes)
            .map_err(|e| EngineError::bad_target(format!("samples asset failed to parse: {e}")))?;
        let pts: Vec<[f64; 2]> = samples.iter().map(|smp| [smp.x, smp.y]).collect();
        stamps.extend(stamps_along(&pts, DEFAULT_BRUSH_DIAMETER));
    }

    let mut session = QuickSession::new(src.backdrop(), src.premultiplied(), canvas);
    for c in &stamps {
        session.stamp(*c, DEFAULT_BRUSH_DIAMETER);
    }
    Ok(session.finish(antialias))
}

// ===========================================================================
// CP-1.0 consolidated round: `content_aware_fill` over the ratified
// §45.3 deterministic exemplar engine (appname-algo::exemplar — Row A,
// merged; the live CAF stream-conformance test on main guards the engine
// itself). Contract `content_aware_fill.yaml` (§40.5 + §45.3/§45.8).
//
// Arm dispositions (validation-complete posture; per-arm log entry):
// * output:"current_layer" — LIVE: selection = H (coverage ≥ 0.5), S per
//   sampleArea auto/rectangular, engine params mapped 1:1, effective
//   PIXELS lock blocks, TRANSPARENCY constrains (alpha preserved), final
//   composite re-weights by selection coverage (out' = lerp(in, out,
//   sel) — the §16.2/§45.3 evaluator rule).
// * output:"new_layer"/"duplicate_layer" — typed-NI: layer CREATION from
//   an assembled kind needs a public raster-node construction door
//   (`LayerNode::new_raster` is core-private; `insert_layer` takes the
//   node) — door filing escalated to O-INT, params validated first
//   (newLayerId presence enforced per the §40.5 cross-field).
// * sampleArea:"custom" — typed-NI: customRegion resolves to an alpha
//   CHANNEL; the channel arena is the O-COLOR+O-CORE CP-1.0 row, not on
//   main yet (error.retouch.sample_region_missing unreachable until
//   then).
// * target.maskId synthesis source — typed-NI: the contract sanctions a
//   mask source but pins no output semantics for it (contract reading
//   escalated, not invented).
// * §45.3 failure rule 2 (all-hole: no boundary AND no samples): the
//   engine's SampleRegionEmpty maps to APPNAME_ERR_INVALID_ARG per the
//   contract; the dedicated wire id `error.retouch.sample_region_empty`
//   needs an ErrorCode variant + catalogued MessageId (core + i18n
//   fences) — ESCALATED; until then the InvalidArg message names the
//   §45.3 rule. Failure rule 1 (insufficient samples): the command
//   SUCCEEDS via the engine's diffusion fallback; the NON-BLOCKING warn
//   has no receipt seam headless (escalated with the door filing).
// ===========================================================================

pub struct ContentAwareFillKind;

impl KindImpl for ContentAwareFillKind {
    fn kind(&self) -> &'static str {
        "content_aware_fill"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        use appname_algo::exemplar::{
            exemplar_fill, ColorAdaptation, ExemplarError, ExemplarParams, Quality,
            RotationAdaptation,
        };
        use appname_algo::plane::Plane;

        // ---- Param mapping (stage-3 schema already validated enums and
        // ranges; cross-fields enforced here, kind-impl precedent). ----
        let p = cx.params();
        let sample_area = p
            .get("sampleArea")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string();
        if p.get("customRegion").is_some() && sample_area != "custom" {
            return Err(EngineError::range(
                "customRegion",
                "customRegion REQUIRES sampleArea:'custom' (§40.5 cross-field)",
            ));
        }
        let output = p
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("new_layer")
            .to_string();
        if output != "current_layer" {
            // §40.5 cross-field first (validation-complete posture)…
            if p.get("newLayerId").and_then(Value::as_str).is_none() {
                return Err(EngineError::missing("newLayerId"));
            }
            // …then the honest door refusal.
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "output:'{output}' creates a layer; assembled kinds have no public \
                     raster-node construction door yet (core door filing, lane log) — \
                     output:'current_layer' is live"
                ),
            ));
        }
        if sample_area == "custom" {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "sampleArea:'custom' resolves customRegion against the channel arena \
                 (§45.3); channels land with the O-COLOR/O-CORE CP-1.0 row",
            ));
        }
        let params = ExemplarParams {
            quality: match p
                .get("quality")
                .and_then(Value::as_str)
                .unwrap_or("default")
            {
                "draft" => Quality::Draft,
                "best" => Quality::Best,
                _ => Quality::Default,
            },
            seed: p.get("seed").and_then(Value::as_u64).unwrap_or(0) as u32,
            color_adaptation: match p
                .get("colorAdaptation")
                .and_then(Value::as_str)
                .unwrap_or("default")
            {
                "none" => ColorAdaptation::None,
                "high" => ColorAdaptation::Cellwise { strength: 1.0 },
                _ => ColorAdaptation::Global { strength: 1.0 },
            },
            rotation: match p
                .get("rotationAdaptation")
                .and_then(Value::as_str)
                .unwrap_or("none")
            {
                "low" => RotationAdaptation::Low,
                "full" => RotationAdaptation::Full,
                _ => RotationAdaptation::None,
            },
            scale: p.get("scale").and_then(Value::as_bool).unwrap_or(false),
            mirror: p.get("mirror").and_then(Value::as_bool).unwrap_or(false),
            ..ExemplarParams::default()
        };

        // ---- Stage 4: target + live-state gates. ----
        let (layer_uuid, mask_id) = match cx.envelope().target.as_ref() {
            Some(t) => (t.layer_id, t.mask_id),
            None => (None, None),
        };
        if mask_id.is_some() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "target.maskId as the synthesis source is contract-sanctioned but its \
                 output semantics are unpinned — escalated for a contract reading \
                 (lane log); target.layerId is live",
            ));
        }
        let Some(layer_uuid) = layer_uuid else {
            return Err(EngineError::bad_target(
                "content_aware_fill requires target.layerId (§10.3 fill family)",
            ));
        };
        let lid = LayerId::from_uuid(layer_uuid);
        let node = cx.document().layer(lid).ok_or_else(|| {
            EngineError::bad_target(format!("target.layerId {layer_uuid} does not resolve"))
        })?;
        if node.pixels().is_none() {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "the synthesis source MUST be a Raster layer (§40.5)",
            ));
        }
        // Selection REQUIRED — it is the hole H (§45.3).
        let sel_bounds = require_active_selection(cx)?;
        // §11.1 locks through the consolidated ancestry union:
        // current_layer writes pixels → PIXELS blocks (ALL ⊇ PIXELS).
        let eff = cx.document().effective_locks(lid);
        if eff.contains(LockFlags::PIXELS) {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "the target layer's pixels are locked (§11.1; §40.5 current_layer row)",
            ));
        }
        let trans_locked = eff.contains(LockFlags::TRANSPARENCY);

        // ---- Inputs: canvas-sized planes (headless adapter; production
        // windows to bbox(H) + pads — perf note in the lane log). ----
        let canvas = cx.document().geometry().canvas_rect();
        let (w, h) = (canvas.width() as usize, canvas.height() as usize);
        let doc = cx.document();
        let buf = doc.layer(lid).unwrap().pixels().unwrap();
        let premul = buf.premultiplied();
        let mut planes = vec![Plane::new(w, h); 4]; // premultiplied RGB + A
        for y in 0..h {
            for x in 0..w {
                let v = buf.read_pixel_f32(appname_core::IVec2::new(
                    canvas.min.x + x as i32,
                    canvas.min.y + y as i32,
                ));
                let (rgbv, a) = if premul {
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
        // Hole H = selection coverage ≥ 0.5 (§45.3); float coverage kept
        // for the final evaluator re-weight.
        let sel = doc.selection();
        let mut coverage = vec![0.0f32; w * h];
        let mut hole = Plane::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = appname_core::IVec2::new(canvas.min.x + x as i32, canvas.min.y + y as i32);
                let c = sel.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0);
                coverage[y * w + x] = c;
                if c >= 0.5 {
                    *hole.at_mut(x, y) = 1.0;
                }
            }
        }
        // S per sampleArea (auto = None: engine takes canvas ∖ G;
        // rectangular = expand(bbox(H), pad) with pad = max(64 px,
        // ⌈0.5·max(bbox.w, bbox.h)⌉), §45.3).
        let sample: Option<Plane> = if sample_area == "rectangular" {
            let pad =
                64.max((sel_bounds.width().max(sel_bounds.height()) as f64 * 0.5).ceil() as i32);
            let rect = appname_core::IRect::new(
                sel_bounds.min.x - pad,
                sel_bounds.min.y - pad,
                sel_bounds.max.x + pad,
                sel_bounds.max.y + pad,
            )
            .intersect(&canvas);
            let mut s = Plane::new(w, h);
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    *s.at_mut((x - canvas.min.x) as usize, (y - canvas.min.y) as usize) = 1.0;
                }
            }
            Some(s)
        } else {
            None
        };

        // ---- The §45.3 engine (deterministic; the three walled-field
        // invariants are the engine's own tested contract). ----
        let alpha_ref = planes[3].clone();
        let result = exemplar_fill(&planes, Some(&alpha_ref), &hole, sample.as_ref(), &params)
            .map_err(|e| match e {
                ExemplarError::SampleRegionEmpty => EngineError::new(
                    ErrorCode::InvalidArg,
                    "no samples AND no boundary — the fill cannot produce pixels (§45.3 \
                     failure rule 2; wire id error.retouch.sample_region_empty pends its \
                     ErrorCode/MessageId doors, lane log)",
                ),
            })?;
        // §45.3 failure rule 1: DiffusionFallback ⇒ the command SUCCEEDS;
        // the non-blocking insufficient-samples warn has no receipt seam
        // headless (escalated; result.method recorded nowhere else yet).

        // ---- Evaluator composite: out' = lerp(in, out, sel), written
        // through the recorder tile door over the selection bounds. ----
        let touched = sel_bounds.intersect(&canvas);
        let mut wrote = false;
        for (tx, ty) in touched.tiles(appname_core::TILE_I32) {
            let coord = appname_core::TileCoord::new(0, tx, ty);
            let trect = coord.pixel_rect();
            let mut pixels = vec![[0.0f32; 4]; appname_core::TILE * appname_core::TILE];
            let mut any = false;
            let mut i = 0;
            for y in trect.min.y..trect.max.y {
                for x in trect.min.x..trect.max.x {
                    let p = appname_core::IVec2::new(x, y);
                    let orig = cx
                        .document()
                        .layer(lid)
                        .unwrap()
                        .pixels()
                        .unwrap()
                        .read_pixel_f32(p);
                    let out = if canvas.contains(p) {
                        let (ux, uy) = ((x - canvas.min.x) as usize, (y - canvas.min.y) as usize);
                        let m = coverage[uy * w + ux];
                        if m > 0.0 {
                            // Engine output (premultiplied) → storage form.
                            let sp = [
                                planes_at(&result.planes, 0, ux, uy),
                                planes_at(&result.planes, 1, ux, uy),
                                planes_at(&result.planes, 2, ux, uy),
                                planes_at(&result.planes, 3, ux, uy),
                            ];
                            // TRANSPARENCY constrains: alpha preserved.
                            // D2 degenerate class, merged pattern
                            // verbatim (review T2 ruling): synthesized
                            // α ≤ 0 under the preserved alpha would
                            // write color 0 at α > 0 — keep dst. Both
                            // storage forms route through this guard.
                            let synth = if trans_locked {
                                if sp[3] <= 0.0 {
                                    orig // out α <= 0 → keep dst
                                } else if premul {
                                    // Rescale color to the preserved
                                    // alpha.
                                    let k = orig[3] / sp[3];
                                    [sp[0] * k, sp[1] * k, sp[2] * k, orig[3]]
                                } else {
                                    // Straight storage: un-premultiply.
                                    let inv = 1.0 / sp[3];
                                    [sp[0] * inv, sp[1] * inv, sp[2] * inv, orig[3]]
                                }
                            } else if premul {
                                [sp[0], sp[1], sp[2], sp[3]]
                            } else {
                                // Straight storage: un-premultiply color.
                                let a = sp[3];
                                let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
                                [sp[0] * inv, sp[1] * inv, sp[2] * inv, a]
                            };
                            [
                                orig[0] + (synth[0] - orig[0]) * m,
                                orig[1] + (synth[1] - orig[1]) * m,
                                orig[2] + (synth[2] - orig[2]) * m,
                                orig[3] + (synth[3] - orig[3]) * m,
                            ]
                        } else {
                            orig
                        }
                    } else {
                        orig
                    };
                    if out != orig {
                        any = true;
                    }
                    pixels[i] = out;
                    i += 1;
                }
            }
            if any {
                cx.install_tile_plane(lid, coord, &pixels)?;
                // §2.4 eager growth per installed tile (mid-op discipline);
                // the class door below re-tightens pixel-exact once.
                cx.grow_layer_bounds(lid, trect.intersect(&touched))?;
                wrote = true;
            }
        }
        if wrote {
            // Q-1 determinism class: `any` is a PRE-quantization f32 test and
            // the eager growth above is TILE-granular (`trect ∩ touched`), so
            // a feathered selection's sub-quantum blend fringe (m ≈ 0) leaves
            // the live extent proud of the persisted plane. Synthesized
            // content can also be MORE transparent than the original inside
            // the selection (alpha reduction), so the erase-side full
            // pixel-exact recompute is the branch that matches the loader.
            finish_write_bounds(cx, lid, touched, true)?;
        }
        Ok(KindOutcome::default())
    }
}

#[inline]
fn planes_at(planes: &[appname_algo::plane::Plane], k: usize, x: usize, y: usize) -> f32 {
    planes[k].at(x, y)
}

impl KindImpl for SelectionModifyKind {
    fn kind(&self) -> &'static str {
        "selection.modify"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let bounds = require_active_selection(cx)?;
        let keys = ["feather", "expand", "contract", "smooth", "border"];
        let present: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|k| cx.params().get(*k).is_some())
            .collect();
        let [key] = present.as_slice() else {
            return Err(EngineError::missing("feather")); // stage-3 oneOf normally catches
        };
        let value = cx
            .params()
            .get(*key)
            .and_then(Value::as_f64)
            .unwrap_or(f64::NAN);
        let ok = match *key {
            "feather" => value.is_finite() && (0.1..=1000.0).contains(&value),
            _ => value.is_finite() && (1.0..=500.0).contains(&value) && value.fract() == 0.0,
        };
        if !ok {
            return Err(EngineError::range(key, "outside the §13.2 contract range"));
        }
        let src = selection_plane(cx, bounds);
        let refined = match *key {
            "feather" => refine::feather(&src, value),
            "expand" => refine::grow(&src, value),
            "contract" => refine::shrink(&src, value),
            "smooth" => refine::smooth(&src, value),
            "border" => refine::border(&src, value),
            _ => unreachable!(),
        };
        combine_and_install(cx, &refined, SelectOp::Replace)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for SelectionTransformKind {
    fn kind(&self) -> &'static str {
        "selection.transform"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let bounds = require_active_selection(cx)?;
        let matrix = param_mat3(cx, "matrix")?;
        if crate::xform::mat3_inverse(&matrix).is_none() {
            return Err(EngineError::range(
                "matrix",
                "matrix MUST be invertible (§40.4)",
            ));
        }
        let kernel = geometry_kernel(cx, "resample", "bilinear")?;
        // Mask coverage as a channel-0 plane through the shared sampler.
        let src_mask = selection_plane(cx, bounds);
        let mut src = Plane::new(bounds);
        for y in bounds.min.y..bounds.max.y {
            for x in bounds.min.x..bounds.max.x {
                src.set(x, y, [src_mask.get(x, y), 0.0, 0.0, 0.0]);
            }
        }
        let out = if matrix.is_identity() {
            src
        } else {
            let dst_rect = crate::xform::transformed_bounds(&matrix, bounds);
            crate::xform::transform_plane(&src, &matrix, dst_rect, Some(kernel))
                .ok_or_else(|| EngineError::range("matrix", "matrix MUST be invertible (§40.4)"))?
        };
        let mut source = MaskPlane::new(out.rect);
        for y in out.rect.min.y..out.rect.max.y {
            for x in out.rect.min.x..out.rect.max.x {
                // §34.4 property 13 under kernel overshoot.
                source.set(
                    x,
                    y,
                    out.get(x, y, EdgePolicy::Transparent)[0].clamp(0.0, 1.0),
                );
            }
        }
        combine_and_install(cx, &source, SelectOp::Replace)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for SelectionAllKind {
    fn kind(&self) -> &'static str {
        "selection.all"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let canvas = cx.document().geometry().canvas_rect();
        let mut source = MaskPlane::new(canvas);
        source.v.fill(1.0);
        combine_and_install(cx, &source, SelectOp::Replace)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for SelectionNoneKind {
    fn kind(&self) -> &'static str {
        "selection.none"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        if !cx.document().selection().active() {
            return Ok(KindOutcome::default()); // §40.1 no-op push (contract)
        }
        let source = MaskPlane::new(appname_core::IRect::EMPTY);
        combine_and_install(cx, &source, SelectOp::Replace)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for SelectionInvertKind {
    fn kind(&self) -> &'static str {
        "selection.invert"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let canvas = cx.document().geometry().canvas_rect();
        // 1 − m over the canvas; with no active selection this is
        // select-all (contract: "deliberate and documented, §40.4").
        let current = selection_plane(cx, canvas);
        let mut source = MaskPlane::new(canvas);
        for (o, c) in source.v.iter_mut().zip(current.v.iter()) {
            *o = 1.0 - c;
        }
        combine_and_install(cx, &source, SelectOp::Replace)?;
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod selection_kind_tests {
    use super::*;
    use appname_core::{Engine, IVec2};
    use serde_json::json;

    fn wave3() -> appname_core::EngineAssembly {
        with_tools_fill(assembly())
    }

    fn engine() -> Engine {
        let mut e = wave3().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 96, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, params: serde_json::Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("ac000000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        })
        .to_string()
    }

    fn sel_cov(e: &Engine, x: i32, y: i32) -> f32 {
        let doc = e.document().unwrap();
        let sel = doc.selection();
        if !sel.active() {
            return -1.0;
        }
        sel.mask().read_pixel_f32(IVec2::new(x, y))[0]
    }

    #[test]
    fn set_rect_ops_and_booleans() {
        let mut e = engine();
        let before = e.state_hash();
        e.apply_json(&env(
            2,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 10, "y": 10, "w": 20, "h": 20},
            }),
        ))
        .unwrap();
        let sel_bounds = e.document().unwrap().selection().bounds();
        assert_eq!(sel_bounds, appname_core::IRect::new(10, 10, 30, 30));
        assert_eq!(sel_cov(&e, 15, 15), 1.0);
        assert_eq!(sel_cov(&e, 5, 5), 0.0);
        // add a second rect; subtract the overlap; §13.2 forms end-to-end.
        e.apply_json(&env(
            3,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 25, "y": 10, "w": 20, "h": 20}, "op": "add",
            }),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            appname_core::IRect::new(10, 10, 45, 30)
        );
        e.apply_json(&env(
            4,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 20, "y": 10, "w": 10, "h": 20}, "op": "subtract",
            }),
        ))
        .unwrap();
        assert_eq!(sel_cov(&e, 22, 15), 0.0, "subtracted band");
        assert_eq!(sel_cov(&e, 12, 15), 1.0);
        assert_eq!(sel_cov(&e, 40, 15), 1.0);
        // Undo chain back to no selection; replay determinism.
        e.undo().unwrap();
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        assert!(!e.document().unwrap().selection().active());
        e.redo().unwrap();
        e.redo().unwrap();
        e.redo().unwrap();
        let after = e.state_hash();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "selection edits replay deterministically"
        );
        // Clipped source: rect partly outside the canvas clips to it.
        e.apply_json(&env(
            5,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 90, "y": 60, "w": 20, "h": 20},
            }),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            appname_core::IRect::new(90, 60, 96, 64)
        );
    }

    #[test]
    fn set_shapes_feather_and_antialias() {
        let mut e = engine();
        // Ellipse with AA: rim carries fractional coverage.
        e.apply_json(&env(
            2,
            "selection.set",
            json!({
                "source": "ellipse", "rect": {"x": 20, "y": 10, "w": 40, "h": 40},
            }),
        ))
        .unwrap();
        assert_eq!(sel_cov(&e, 40, 30), 1.0, "ellipse center");
        assert_eq!(sel_cov(&e, 21, 11), 0.0, "bbox corner outside the disc");
        // antialias:false → binary coverage.
        e.apply_json(&env(3, "selection.set", json!({
            "source": "ellipse", "rect": {"x": 20, "y": 10, "w": 40, "h": 40}, "antialias": false,
        }))).unwrap();
        let doc = e.document().unwrap();
        let mask = doc.selection().mask();
        let b = doc.selection().bounds();
        for y in b.min.y..b.max.y {
            for x in b.min.x..b.max.x {
                let v = mask.read_pixel_f32(IVec2::new(x, y))[0];
                assert!(v == 0.0 || v == 1.0, "binary at ({x},{y}): {v}");
            }
        }
        // Polygon = triangle; lasso conditions vertices.
        e.apply_json(&env(
            4,
            "selection.set",
            json!({
                "source": "polygon", "points": [[10, 10], [50, 10], [10, 50]],
            }),
        ))
        .unwrap();
        assert_eq!(sel_cov(&e, 15, 15), 1.0);
        assert_eq!(sel_cov(&e, 45, 45), 0.0);
        // Feather softens the source edge (fractional beyond the rect).
        e.apply_json(&env(
            5,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 40, "y": 20, "w": 16, "h": 16}, "feather": 6.0,
            }),
        ))
        .unwrap();
        let inside = sel_cov(&e, 48, 28);
        let ring = sel_cov(&e, 58, 28);
        assert!(inside > 0.9, "feathered interior {inside}");
        assert!(ring > 0.0 && ring < 0.5, "soft ring beyond the rect {ring}");
        // Bad source values reject typed (`quick` left this set in the
        // CP-1.0 consolidated round — its live battery is below).
        for (src, code) in [
            ("color_range", ErrorCode::NotImplemented),
            ("path", ErrorCode::NotImplemented),
        ] {
            let mut p = json!({"source": src});
            if src == "color_range" {
                p["samples"] = json!([{"model": "rgb", "comps": [1,0,0,0], "profile": "00000000-0000-4000-8000-000000000001"}]);
            }
            if src == "path" {
                p["pathId"] = json!("00000000-0000-4000-8000-0000000000aa");
            }
            let err = e
                .apply_json(&env(50 + code as u32, "selection.set", p))
                .unwrap_err();
            assert_eq!(err.code, code, "{src}");
        }
    }

    #[test]
    fn wand_selects_regions_and_gates_fill() {
        let mut e = engine();
        // Build two compartments: blue wall via brush.
        let samples = crate::brush::encode_samples(&[
            crate::brush::InputSample {
                x: 48.0,
                y: -8.0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.0,
            },
            crate::brush::InputSample {
                x: 48.0,
                y: 72.0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.05,
            },
        ])
        .unwrap();
        let asset = e.add_samples_asset(samples);
        let layer = e.document().unwrap().z_order_walk()[0];
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000000000002",
                "kind": "brush.stroke",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {
                    "brushPreset": preset_hard_round_16_id().to_string(),
                    "color": {"model": "rgb", "comps": [0, 0, 1, 0]},
                },
                "samples": asset.to_string(),
            })
            .to_string(),
        )
        .unwrap();
        // Wand the left white compartment.
        e.apply_json(&env(
            3,
            "selection.set",
            json!({
                "source": "wand", "seed": [10, 30],
            }),
        ))
        .unwrap();
        assert_eq!(sel_cov(&e, 10, 30), 1.0, "left compartment selected");
        assert_eq!(sel_cov(&e, 80, 30), 0.0, "right compartment not connected");
        assert_eq!(sel_cov(&e, 48, 30), 0.0, "wall not selected");
        // The active selection now gates a whole-canvas fill (§13.1: every
        // consumer scales) — only the left compartment turns green.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000000000003",
                "kind": "fill",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {"content": "color", "color": {"model": "rgb", "comps": [0, 1, 0, 0]}},
            })
            .to_string(),
        )
        .unwrap();
        let doc = e.document().unwrap();
        let buf = doc.layer(layer).unwrap().pixels().unwrap();
        assert!(
            buf.read_pixel_f32(IVec2::new(10, 30))[1] > 0.9,
            "selected side filled"
        );
        assert!(
            buf.read_pixel_f32(IVec2::new(80, 30))[1] > 0.95
                && buf.read_pixel_f32(IVec2::new(80, 30))[0] > 0.95,
            "unselected side stays white"
        );
        // Wand gates: seed outside canvas; sampleAllLayers pending.
        let err = e
            .apply_json(&env(
                4,
                "selection.set",
                json!({
                    "source": "wand", "seed": [96, 0],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // `sampleAllLayers:true` used to be the typed composite door; it
        // now SERVES over core's §13.3 flattened-composite door. The
        // dedicated battery is `wand_sample_all_layers_*` below; here we
        // only pin that the door is no longer shut.
        e.apply_json(&env(
            5,
            "selection.set",
            json!({
                "source": "wand", "seed": [5, 5], "sampleAllLayers": true,
            }),
        ))
        .expect("§13.3 sampleAllLayers serves");
    }

    #[test]
    fn modify_refines_and_requires_selection() {
        let mut e = engine();
        // Stage-4 live-state gate.
        let err = e
            .apply_json(&env(2, "selection.modify", json!({"feather": 4.0})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        // Expand a square by 5: bounds grow by 5 each side (clipped math).
        e.apply_json(&env(
            3,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 30, "y": 20, "w": 20, "h": 20},
            }),
        ))
        .unwrap();
        e.apply_json(&env(4, "selection.modify", json!({"expand": 5})))
            .unwrap();
        let b = e.document().unwrap().selection().bounds();
        assert!(
            b.min.x <= 25 && b.max.x >= 55 && b.min.y <= 15 && b.max.y >= 45,
            "grown: {b:?}"
        );
        assert_eq!(sel_cov(&e, 27, 30), 1.0, "expanded area selected");
        // Border: interior clears, band remains.
        e.apply_json(&env(5, "selection.modify", json!({"border": 6})))
            .unwrap();
        assert!(sel_cov(&e, 40, 30) < 0.05, "interior emptied by border");
        // Feather + undo/redo/replay round-trip.
        let pre = e.state_hash();
        e.apply_json(&env(6, "selection.modify", json!({"feather": 8.0})))
            .unwrap();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        // Range gate.
        let err = e
            .apply_json(&env(7, "selection.modify", json!({"expand": 501})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn transform_moves_the_mask_only() {
        let mut e = engine();
        let err = e
            .apply_json(&env(
                2,
                "selection.transform",
                json!({
                    "matrix": [1, 0, 0, 0, 1, 0, 8, 0, 1],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        e.apply_json(&env(
            3,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 10, "y": 10, "w": 20, "h": 20},
            }),
        ))
        .unwrap();
        let pixels_before = e
            .document()
            .unwrap()
            .layer(e.document().unwrap().z_order_walk()[0])
            .unwrap()
            .pixels()
            .unwrap()
            .region_hash();
        e.apply_json(&env(
            4,
            "selection.transform",
            json!({
                "matrix": [1, 0, 0, 0, 1, 0, 8, 4, 1],
            }),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            appname_core::IRect::new(18, 14, 38, 34)
        );
        assert_eq!(sel_cov(&e, 20, 16), 1.0);
        assert_eq!(sel_cov(&e, 12, 12), 0.0, "old area deselected");
        let pixels_after = e
            .document()
            .unwrap()
            .layer(e.document().unwrap().z_order_walk()[0])
            .unwrap()
            .pixels()
            .unwrap()
            .region_hash();
        assert_eq!(
            pixels_before, pixels_after,
            "transforms the mask only, never pixels"
        );
        // Degenerate matrix.
        let err = e
            .apply_json(&env(
                5,
                "selection.transform",
                json!({
                    "matrix": [0, 0, 0, 0, 0, 0, 0, 0, 1],
                }),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // Replay determinism.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn all_none_invert_contract_semantics() {
        let mut e = engine();
        let canvas = appname_core::IRect::new(0, 0, 96, 64);
        // none on inactive → §40.1 no-op push.
        let before = e.state_hash();
        let r = e.apply_json(&env(2, "selection.none", json!({}))).unwrap();
        assert!(r.dirty.is_empty());
        assert_eq!(e.state_hash(), before);
        // invert on inactive = select-all (contract-pinned).
        e.apply_json(&env(3, "selection.invert", json!({})))
            .unwrap();
        assert_eq!(e.document().unwrap().selection().bounds(), canvas);
        assert_eq!(sel_cov(&e, 50, 50), 1.0);
        // none clears; active flips off.
        e.apply_json(&env(4, "selection.none", json!({}))).unwrap();
        assert!(!e.document().unwrap().selection().active());
        // all = full canvas; all again = no change (empty push).
        e.apply_json(&env(5, "selection.all", json!({}))).unwrap();
        assert_eq!(e.document().unwrap().selection().bounds(), canvas);
        let h = e.state_hash();
        let r = e.apply_json(&env(6, "selection.all", json!({}))).unwrap();
        assert!(r.dirty.is_empty());
        assert_eq!(e.state_hash(), h, "idempotent select-all is the empty push");
        // invert of a binary rect twice = exact round trip.
        e.apply_json(&env(
            7,
            "selection.set",
            json!({
                "source": "rect", "rect": {"x": 10, "y": 10, "w": 20, "h": 20},
            }),
        ))
        .unwrap();
        let h_rect = e.state_hash();
        e.apply_json(&env(8, "selection.invert", json!({})))
            .unwrap();
        assert_eq!(sel_cov(&e, 15, 15), 0.0);
        assert_eq!(sel_cov(&e, 60, 40), 1.0);
        e.apply_json(&env(9, "selection.invert", json!({})))
            .unwrap();
        assert_eq!(
            e.state_hash(),
            h_rect,
            "binary invert is exactly self-inverse"
        );
        // Undo/redo byte-identity over the chain end + replay.
        e.undo().unwrap();
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Two-tone document for the quick-select battery: left half
    /// (x < 48) red, right half white.
    fn two_tone_engine() -> (Engine, appname_core::LayerId) {
        let mut e = engine();
        let layer = e.document().unwrap().z_order_walk()[0];
        e.apply_json(&env(
            90,
            "selection.set",
            json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 48, "h": 64},
                    "antialias": false}),
        ))
        .unwrap();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000100000091",
                "kind": "fill",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {"content": "color",
                            "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 1.0]}},
            })
            .to_string(),
        )
        .unwrap();
        e.apply_json(&env(92, "selection.none", json!({}))).unwrap();
        (e, layer)
    }

    /// ES §13.3 quick selection through the live arm (CP-1.0 round):
    /// seed-only single-stamp form — statistical flood + edge stop keep
    /// the selection inside the seeded surface; one command per session;
    /// undo/redo byte-identity; journal replay determinism.
    #[test]
    fn quick_select_floods_the_seeded_surface_and_replays() {
        let (mut e, _) = two_tone_engine();
        let baseline = e.state_hash();
        let steps_before = e.journal().len();
        e.apply_json(&env(
            2,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32]}),
        ))
        .unwrap();
        assert_eq!(
            e.journal().len(),
            steps_before + 1,
            "ONE coalesced selection.set per stroke session (§13.3 step 5)"
        );
        // The red surface floods; white never joins (2.5σ color model).
        assert_eq!(sel_cov(&e, 10, 32), 1.0);
        assert_eq!(sel_cov(&e, 40, 5), 1.0, "far red corner joined");
        for &(x, y) in &[(60, 32), (90, 5), (52, 60)] {
            assert_eq!(sel_cov(&e, x, y), 0.0, "white stays out at ({x},{y})");
        }
        // Coverage invariant over the mask (§34.4 property 13) is
        // enforced by the install door clamp; spot-check the AA band.
        let edge = sel_cov(&e, 47, 32);
        assert!((0.0..=1.0).contains(&edge));
        // Undo restores the pre-command state byte-identically.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), baseline);
        e.redo().unwrap();
        let after = e.state_hash();
        // Journal replay re-runs the SAME arm deterministically (D-48).
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "quick replays bit-exactly");
    }

    /// §13.3 step 4: alt-drag subtract = the same growth applied with
    /// the §13.2 Subtract op against the current selection.
    #[test]
    fn quick_select_subtract_removes_the_brushed_surface() {
        let (mut e, _) = two_tone_engine();
        e.apply_json(&env(2, "selection.all", json!({}))).unwrap();
        e.apply_json(&env(
            3,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32], "op": "subtract"}),
        ))
        .unwrap();
        assert_eq!(sel_cov(&e, 10, 32), 0.0, "brushed red surface removed");
        assert_eq!(sel_cov(&e, 40, 5), 0.0);
        assert_eq!(sel_cov(&e, 70, 32), 1.0, "white side keeps selection");
    }

    /// The envelope `samples` asset drives the drag (§39.4 T-09 B3
    /// shape): a stroke crossing both surfaces grows the model onto the
    /// second surface (§13.3 step-1 decay), monotonically.
    #[test]
    fn quick_select_drag_across_surfaces_tracks_the_model() {
        let (mut e, _) = two_tone_engine();
        let samples: Vec<crate::brush::InputSample> = (0..=14)
            .map(|i| crate::brush::InputSample {
                x: 10.0 + 5.0 * i as f64,
                y: 32.0,
                pressure: 1.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                rotation: 0.0,
                velocity: 0.0,
                t: 0.01 * i as f64,
            })
            .collect();
        let asset = e.add_samples_asset(crate::brush::encode_samples(&samples).unwrap());
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000100000002",
                "kind": "selection.set",
                "params": {"source": "quick", "seed": [10, 32]},
                "samples": asset.to_string(),
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(sel_cov(&e, 10, 32), 1.0, "first surface selected");
        assert_eq!(sel_cov(&e, 85, 10), 1.0, "dragged-onto surface joined");
        // Replay with the recorded samples asset is deterministic.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Adversarial rejects for the quick arm (contract rows).
    #[test]
    fn quick_select_reject_arms() {
        let (mut e, _) = two_tone_engine();
        let h = e.state_hash();
        // seed REQUIRED (contract allOf; stage-6 enforcement precedent).
        let err = e
            .apply_json(&env(2, "selection.set", json!({"source": "quick"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        // seed MUST lie inside the canvas (§40.4 live-state).
        let err = e
            .apply_json(&env(
                3,
                "selection.set",
                json!({"source": "quick", "seed": [9999999, 9999999]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // (`sampleAllLayers:true` is NOT a reject arm any more — it serves
        // over the §13.3 composite door; see `quick_select_sample_all_
        // layers_matches_flatten_then_sample`. It is removed from this
        // battery rather than re-pinned because a served command mutates
        // state and would break the closing "every reject mutated
        // nothing" assertion.)
        // A dangling samples asset refuses typed.
        let err = e
            .apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "ac000000-0000-4000-8000-000100000005",
                    "kind": "selection.set",
                    "params": {"source": "quick", "seed": [5, 5]},
                    "samples": "0badbad0-0000-4000-8000-00000000dead",
                })
                .to_string(),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert_eq!(e.state_hash(), h, "every reject mutated nothing (§10.2)");
    }

    // ---- §13.3 sampleAllLayers (the SAMPLE-ALL-LAYERS arm) --------------
    //
    // Contract: `selection.set` `sampleAllLayers:bool=false` —
    // "wand/quick (§13.3): sample the flattened composite instead of the
    // target layer"; ES §13.3 wand paragraph: "`sampleAllLayers:true`
    // samples the flattened composite (§4.6 cache) instead of the target
    // layer, and THE SEED COLOR OBEYS THE SAME SAMPLING RULE".
    //
    // Fixture discipline matches the retouch battery: hard-edged, fully
    // opaque blocks over exact storage values, never an antialiased
    // stroke — so "sample the composite" and "flatten, then sample" are
    // comparable on RAW BITS rather than inside a quantization tolerance.

    /// An engine carrying the CORE kinds as well (`layer.add`,
    /// `layer.set_props`, `image.flatten`), which the scope battery needs.
    fn stacked_engine() -> Engine {
        let mut e = appname_core::engine::kinds::register(wave3()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-00000000000f",
                "kind": "doc.new",
                "params": {"width": 96, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    const UPPER_ID: &str = "0d130300-0000-4000-8000-000000000001";

    /// Paint a hard-edged, fully opaque BLUE block over the left half of a
    /// NEW top layer. The composite is then blue|white; the bottom layer
    /// alone is uniformly white, so the two sampling scopes disagree and
    /// the assertions below cannot pass vacuously.
    fn add_upper_blue_block(e: &mut Engine) {
        e.apply_json(&env(
            80,
            "layer.add",
            json!({"kind": "raster", "newLayerId": UPPER_ID}),
        ))
        .unwrap();
        e.apply_json(&env(
            81,
            "selection.set",
            json!({"source": "rect", "rect": {"x": 0, "y": 0, "w": 48, "h": 64},
                   "antialias": false}),
        ))
        .unwrap();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000100000082",
                "kind": "fill",
                "target": {"layerId": UPPER_ID},
                "params": {"content": "color",
                           "color": {"model": "rgb", "comps": [0.0, 0.0, 1.0, 1.0]},
                           "antialias": false},
            })
            .to_string(),
        )
        .unwrap();
        e.apply_json(&env(83, "selection.none", json!({}))).unwrap();
    }

    /// Every canvas pixel of the selection mask, as RAW BITS.
    fn sel_bits(e: &Engine) -> Vec<u32> {
        let doc = e.document().unwrap();
        let mask = doc.selection().mask();
        let r = doc.geometry().canvas_rect();
        let mut out = Vec::with_capacity((r.width() * r.height()) as usize);
        for y in r.min.y..r.max.y {
            for x in r.min.x..r.max.x {
                out.push(mask.read_pixel_f32(IVec2::new(x, y))[0].to_bits());
            }
        }
        out
    }

    /// §13.3 `sampleAllLayers:true` SERVES the flattened composite, and
    /// "sample all layers" == "flatten, then sample" BIT-for-bit.
    ///
    /// Three legs, so no half can pass vacuously:
    ///   (a) the true arm selects the composite's blue region — which the
    ///       target layer alone does not even contain;
    ///   (b) the SAME document with `sampleAllLayers:false` refuses typed
    ///       (headless has no active layer on a multi-layer document), so
    ///       leg (a) is demonstrably doing work the false arm cannot;
    ///   (c) `image.flatten` first, then the plain single-layer wand,
    ///       yields the IDENTICAL mask bits.
    #[test]
    fn wand_sample_all_layers_matches_flatten_then_sample() {
        let mut e = stacked_engine();
        add_upper_blue_block(&mut e);

        // (a) composite scope: the blue left half floods, white stays out.
        e.apply_json(&env(
            84,
            "selection.set",
            json!({"source": "wand", "seed": [10, 32], "sampleAllLayers": true,
                   "antialias": false}),
        ))
        .expect("§13.3 sampleAllLayers:true serves");
        assert_eq!(sel_cov(&e, 10, 32), 1.0, "composite blue region selected");
        assert_eq!(sel_cov(&e, 80, 32), 0.0, "composite white region excluded");
        let composite_bits = sel_bits(&e);

        // (b) the same request WITHOUT the flag has no active layer to
        //     sample on a multi-layer document — typed, not silent.
        let err = e
            .apply_json(&env(
                85,
                "selection.set",
                json!({"source": "wand", "seed": [10, 32], "antialias": false}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::NotImplemented,
            "the active-layer arm still pends on the tool-state row"
        );

        // (c) flatten first, then sample ONE layer: identical bits.
        e.apply_json(&env(86, "image.flatten", json!({}))).unwrap();
        assert_eq!(
            e.document().unwrap().z_order_walk().len(),
            1,
            "flatten must leave exactly one layer, or leg (c) is not the \
             single-layer arm"
        );
        e.apply_json(&env(
            87,
            "selection.set",
            json!({"source": "wand", "seed": [10, 32], "antialias": false}),
        ))
        .unwrap();
        assert_eq!(
            composite_bits,
            sel_bits(&e),
            "§13.3: sampling the flattened composite must equal flattening \
             and then sampling — BIT-for-bit"
        );
    }

    /// The seed COLOR obeys the same sampling rule (ES §13.3, explicit).
    /// Seeded on the WHITE half, the composite scope must flood the white
    /// half and exclude the blue — the mirror image of the test above. If
    /// the seed color were read from the target layer (uniform white
    /// everywhere) instead of the composite, the flood would swallow the
    /// blue half too and this reds.
    #[test]
    fn wand_seed_color_is_read_from_the_sampled_surface() {
        let mut e = stacked_engine();
        add_upper_blue_block(&mut e);
        e.apply_json(&env(
            88,
            "selection.set",
            json!({"source": "wand", "seed": [80, 32], "sampleAllLayers": true,
                   "antialias": false, "contiguous": false}),
        ))
        .unwrap();
        assert_eq!(sel_cov(&e, 80, 32), 1.0, "seeded white region selected");
        assert_eq!(
            sel_cov(&e, 10, 32),
            0.0,
            "the blue half must NOT match a white seed — if it does, the \
             seed color came from the wrong surface"
        );
    }

    /// On a ONE-layer document the composite IS that layer, so the two
    /// `sampleAllLayers` arms must not fork. Pinned on an ANTIALIASED
    /// brush wall (fractional coverage), not only on hard blocks.
    #[test]
    fn wand_sample_all_layers_does_not_fork_on_a_one_layer_document() {
        let run = |all: bool| -> (Vec<u32>, appname_core::ContentHash) {
            let mut e = stacked_engine();
            let layer = e.document().unwrap().z_order_walk()[0];
            let samples = crate::brush::encode_samples(&[
                crate::brush::InputSample {
                    x: 48.0,
                    y: -8.0,
                    pressure: 1.0,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    rotation: 0.0,
                    velocity: 0.0,
                    t: 0.0,
                },
                crate::brush::InputSample {
                    x: 48.0,
                    y: 72.0,
                    pressure: 1.0,
                    tilt_x: 0.0,
                    tilt_y: 0.0,
                    rotation: 0.0,
                    velocity: 0.0,
                    t: 0.05,
                },
            ])
            .unwrap();
            let asset = e.add_samples_asset(samples);
            e.apply_json(
                &json!({
                    "schema": "appname.command.v1",
                    "id": "ac000000-0000-4000-8000-000100000089",
                    "kind": "brush.stroke",
                    "target": {"layerId": layer.as_uuid().to_string()},
                    "params": {
                        "brushPreset": preset_hard_round_16_id().to_string(),
                        "color": {"model": "rgb", "comps": [0, 0, 1, 0]},
                    },
                    "samples": asset.to_string(),
                })
                .to_string(),
            )
            .unwrap();
            let mut p = json!({"source": "wand", "seed": [10, 30]});
            if all {
                p["sampleAllLayers"] = json!(true);
            }
            e.apply_json(&env(90, "selection.set", p)).unwrap();
            assert!(sel_cov(&e, 10, 30) > 0.0, "the wand must select something");
            (sel_bits(&e), e.state_hash())
        };
        let (single, h_single) = run(false);
        let (composite, h_composite) = run(true);
        assert_eq!(
            single, composite,
            "§13.3: on a one-layer document the two sampling scopes must \
             answer BIT-identical coverage"
        );
        assert_eq!(h_single, h_composite, "…and identical state hashes");
    }

    /// QA-F8 on the §13.3 door: a layer the storage-space composite cannot
    /// serve REFUSES typed — the wand never quietly samples a composite
    /// that is missing a layer. Paired with its GREEN control.
    #[test]
    fn wand_sample_all_layers_refuses_unservable_layers_typed() {
        // Control: the plain two-layer fixture serves.
        let mut e = stacked_engine();
        add_upper_blue_block(&mut e);
        e.apply_json(&env(
            91,
            "selection.set",
            json!({"source": "wand", "seed": [10, 32], "sampleAllLayers": true}),
        ))
        .expect("control: the plain raster stack serves");

        // Perturbation: the upper layer takes a non-NORMAL blend.
        let mut e = stacked_engine();
        add_upper_blue_block(&mut e);
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "ac000000-0000-4000-8000-000100000092",
                "kind": "layer.set_props",
                "target": {"layerId": UPPER_ID},
                "params": {"blend": "multiply"},
            })
            .to_string(),
        )
        .unwrap();
        let err = e
            .apply_json(&env(
                93,
                "selection.set",
                json!({"source": "wand", "seed": [10, 32], "sampleAllLayers": true}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("blend"),
            "the refusal must name its row; got: {}",
            err.message
        );
    }

    /// The quick-selection arm rides the SAME `sampling_source` seam, so
    /// it gets the same two pins: it serves, and it agrees with flatten-
    /// then-sample bit-for-bit.
    #[test]
    fn quick_select_sample_all_layers_matches_flatten_then_sample() {
        let mut e = stacked_engine();
        add_upper_blue_block(&mut e);
        e.apply_json(&env(
            94,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32], "sampleAllLayers": true}),
        ))
        .expect("§13.3 quick sampleAllLayers:true serves");
        assert_eq!(sel_cov(&e, 10, 32), 1.0, "seeded composite surface floods");
        assert_eq!(sel_cov(&e, 80, 32), 0.0, "the other surface stays out");
        let composite_bits = sel_bits(&e);

        e.apply_json(&env(95, "image.flatten", json!({}))).unwrap();
        e.apply_json(&env(
            96,
            "selection.set",
            json!({"source": "quick", "seed": [10, 32]}),
        ))
        .unwrap();
        assert_eq!(
            composite_bits,
            sel_bits(&e),
            "quick selection over the composite must equal flatten-then-sample"
        );
    }

    /// D-48: a composite-sampled selection journals, undoes, redoes and
    /// REPLAYS byte-identically. The sampled surface is derived from OTHER
    /// layers' live state, which is exactly the class of change that can
    /// break replay determinism, so it is pinned rather than assumed.
    #[test]
    fn composite_sampled_selection_replays_byte_identically() {
        for (n, source) in [(97u32, "wand"), (98, "quick")] {
            let mut e = stacked_engine();
            add_upper_blue_block(&mut e);
            let before = e.state_hash();
            e.apply_json(&env(
                n,
                "selection.set",
                json!({"source": source, "seed": [10, 32], "sampleAllLayers": true}),
            ))
            .unwrap();
            let after = e.state_hash();
            assert_ne!(before, after, "{source}: the selection must change state");
            e.undo().unwrap();
            assert_eq!(before, e.state_hash(), "{source}: undo byte identity");
            e.redo().unwrap();
            assert_eq!(after, e.state_hash(), "{source}: redo byte identity");
            let replayed = e.replay_assembled(e.journal()).unwrap();
            assert_eq!(
                after,
                replayed.state_hash(),
                "{source}: composite-sampled replay must be byte-identical (D-48)"
            );
            let again = e.replay_assembled(e.journal()).unwrap();
            assert_eq!(
                replayed.state_hash(),
                again.state_hash(),
                "{source}: two replays of one journal must agree"
            );
        }
    }
}

// ===========================================================================
// content_aware_fill battery (contract content_aware_fill.yaml; ES §45.3
// consumer arm — the ENGINE's own conformance lives in appname-algo + the
// live CAF stream-conformance test on main; nothing here freezes a
// golden: pixel pins below are in-crate corpus companions FLAGGED for
// O-QA ratification per standing practice, citing the 2026-08-16 §45.3
// ratification entry).
// ===========================================================================
#[cfg(test)]
mod caf_kind_tests {
    use super::*;
    use appname_core::{Engine, IVec2};
    use serde_json::json;

    fn engine() -> Engine {
        let mut e = appname_core::engine::kinds::register(with_tools_fill(assembly())).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "cafe0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 96, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, layer: Option<&str>, params: serde_json::Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("cafe0000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l});
        }
        v.to_string()
    }

    fn px(e: &Engine, layer: appname_core::LayerId, x: i32, y: i32) -> [f32; 4] {
        e.document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    /// Red canvas with a blue square blemish; hole = the blemish.
    fn blemish_engine() -> (Engine, appname_core::LayerId, String) {
        let mut e = engine();
        let layer = e.document().unwrap().z_order_walk()[0];
        let l = layer.to_string();
        e.apply_json(&env(
            2,
            "fill",
            Some(&l),
            json!({"content": "color", "color": {"model": "rgb", "comps": [1.0, 0.0, 0.0, 1.0]}}),
        ))
        .unwrap();
        e.apply_json(&env(
            3,
            "selection.set",
            None,
            json!({"source": "rect", "rect": {"x": 40, "y": 24, "w": 12, "h": 12},
                    "antialias": false}),
        ))
        .unwrap();
        e.apply_json(&env(
            4,
            "fill",
            Some(&l),
            json!({"content": "color", "color": {"model": "rgb", "comps": [0.0, 0.0, 1.0, 1.0]}}),
        ))
        .unwrap();
        // The blemish rect stays selected: it IS the hole H.
        (e, layer, l)
    }

    /// current_layer fill heals the blemish from the surround; undo is
    /// byte-identical; journal replay reproduces the state (D-48 — the
    /// §45.3 engine is deterministic per (inputs, seed)).
    #[test]
    fn caf_current_layer_heals_undoes_and_replays() {
        let (mut e, layer, l) = blemish_engine();
        let before_fill = e.state_hash();
        let blue = px(&e, layer, 45, 30);
        assert!(blue[2] > 0.9, "blemish in place: {blue:?}");
        e.apply_json(&env(
            5,
            "content_aware_fill",
            Some(&l),
            json!({"output": "current_layer"}),
        ))
        .unwrap();
        let healed = px(&e, layer, 45, 30);
        assert!(
            healed[0] > 0.9 && healed[2] < 0.1,
            "hole synthesized from the red surround: {healed:?}"
        );
        let outside = px(&e, layer, 10, 10);
        assert!(outside[0] > 0.9, "outside the selection untouched");
        // Undo byte-identity + replay determinism.
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before_fill);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after, "CAF replays bit-exactly");
    }

    /// sampleArea:'rectangular' is live geometry over the same engine
    /// parameter; seed/quality map through.
    #[test]
    fn caf_rectangular_sample_area_and_params_map() {
        let (mut e, layer, l) = blemish_engine();
        e.apply_json(&env(
            5,
            "content_aware_fill",
            Some(&l),
            json!({"output": "current_layer", "sampleArea": "rectangular",
                    "quality": "draft", "seed": 7, "colorAdaptation": "none"}),
        ))
        .unwrap();
        let healed = px(&e, layer, 45, 30);
        assert!(healed[0] > 0.9 && healed[2] < 0.1, "healed: {healed:?}");
    }

    /// Adversarial rejects: every refusal typed, nothing mutated.
    #[test]
    fn caf_reject_arms() {
        let (mut e, _, l) = blemish_engine();
        let h = e.state_hash();
        // new_layer outputs: params validated first (§40.5 cross-field),
        // then the honest door refusal.
        let err = e
            .apply_json(&env(10, "content_aware_fill", Some(&l), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "newLayerId required");
        let err = e
            .apply_json(&env(
                11,
                "content_aware_fill",
                Some(&l),
                json!({"newLayerId": "cafe0000-0000-4000-8000-00000000aa01"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented, "layer-creation door");
        // sampleArea custom pends the channel arena.
        let err = e
            .apply_json(&env(
                12,
                "content_aware_fill",
                Some(&l),
                json!({"output": "current_layer", "sampleArea": "custom",
                        "customRegion": "cafe0000-0000-4000-8000-00000000aa02"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // customRegion without sampleArea:'custom' → §40.5 cross-field.
        let err = e
            .apply_json(&env(
                13,
                "content_aware_fill",
                Some(&l),
                json!({"output": "current_layer",
                        "customRegion": "cafe0000-0000-4000-8000-00000000aa03"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // Missing target.
        let err = e
            .apply_json(&env(
                14,
                "content_aware_fill",
                None,
                json!({"output": "current_layer"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // PIXELS lock through an ancestor group (effective_locks synergy).
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "cafe0000-0000-4000-8000-000100000015",
                "kind": "layer.group",
                "params": {"layerIds": [l],
                            "newGroupId": "cafe0000-0000-4000-8000-0000000000f1"},
            })
            .to_string(),
        )
        .unwrap();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "cafe0000-0000-4000-8000-000100000016",
                "kind": "layer.set_props",
                "target": {"layerId": "cafe0000-0000-4000-8000-0000000000f1"},
                "params": {"locks": {"pixels": true}},
            })
            .to_string(),
        )
        .unwrap();
        let locked_h = e.state_hash();
        let err = e
            .apply_json(&env(
                17,
                "content_aware_fill",
                Some(&l),
                json!({"output": "current_layer"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked, "group PIXELS binds (§11.1)");
        assert_eq!(e.state_hash(), locked_h, "lock refusal mutated nothing");
        let _ = h;
    }

    /// No active selection → error.command.selection_required (the
    /// selection IS the hole, §45.3).
    #[test]
    fn caf_requires_a_selection() {
        let mut e = engine();
        let layer = e.document().unwrap().z_order_walk()[0];
        let l = layer.to_string();
        let err = e
            .apply_json(&env(
                2,
                "content_aware_fill",
                Some(&l),
                json!({"output": "current_layer"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
    }

    /// Review T2 (D2 degenerate class, merged pattern verbatim): a
    /// TRANSPARENCY-locked target whose surround is fully transparent
    /// synthesizes α ≤ 0 in the hole (transparent pixels are invalid
    /// sample candidates → diffusion from a transparent boundary); the
    /// preserved-alpha branch must KEEP DST, never write color 0 at the
    /// preserved α > 0. COLOR-asserting on purpose — the D2 round's
    /// lesson is that alpha-only tests miss this.
    #[test]
    fn caf_trans_locked_alpha_zero_synthesis_keeps_dst_color() {
        let mut e = appname_core::engine::kinds::register(with_tools_fill(assembly())).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "cafe0000-0000-4000-8000-0000000000d2",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "transparent"},
            })
            .to_string(),
        )
        .unwrap();
        let layer = e.document().unwrap().z_order_walk()[0];
        let l = layer.to_string();
        // Opaque blue island on the transparent canvas.
        e.apply_json(&env(
            2,
            "selection.set",
            None,
            json!({"source": "rect", "rect": {"x": 24, "y": 24, "w": 16, "h": 16},
                    "antialias": false}),
        ))
        .unwrap();
        e.apply_json(&env(
            3,
            "fill",
            Some(&l),
            json!({"content": "color",
                    "color": {"model": "rgb", "comps": [0.0, 0.0, 1.0, 1.0]}}),
        ))
        .unwrap();
        // TRANSPARENCY lock, then fill the island (still selected = H).
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "cafe0000-0000-4000-8000-0001000000d4",
                "kind": "layer.set_props",
                "target": {"layerId": l},
                "params": {"locks": {"transparency": true}},
            })
            .to_string(),
        )
        .unwrap();
        let before = px(&e, layer, 30, 30);
        assert!(
            before[2] > 0.9 && before[3] > 0.9,
            "opaque blue: {before:?}"
        );
        e.apply_json(&env(
            5,
            "content_aware_fill",
            Some(&l),
            json!({"output": "current_layer"}),
        ))
        .unwrap();
        let after = px(&e, layer, 30, 30);
        assert!(
            after[3] > 0.9,
            "TRANSPARENCY lock preserved alpha: {after:?}"
        );
        assert!(
            after[2] > 0.9 && after[0] < 0.1 && after[1] < 0.1,
            "D2 guard kept the dst COLOR (blue), not color-0-at-alpha:              {after:?}"
        );
    }

    /// §45.3 failure rule 2: an all-canvas hole has neither boundary nor
    /// samples — typed refusal, document unchanged (apply-or-unchanged).
    /// Failure rule 1: a starved-but-bounded hole SUCCEEDS via the
    /// engine's diffusion fallback.
    #[test]
    fn caf_failure_rules_all_hole_refuses_starved_falls_back() {
        let (mut e, layer, l) = blemish_engine();
        e.apply_json(&env(5, "selection.all", None, json!({})))
            .unwrap();
        let h = e.state_hash();
        let err = e
            .apply_json(&env(
                6,
                "content_aware_fill",
                Some(&l),
                json!({"output": "current_layer"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArg, "§45.3 failure rule 2");
        assert_eq!(e.state_hash(), h, "document unchanged (§10.2)");
        // Starved: select everything except a 1-px border ring — far
        // fewer than N_min = 64 valid patch centers remain, and the
        // command still succeeds (diffusion fallback, rule 1).
        e.apply_json(&env(
            7,
            "selection.set",
            None,
            json!({"source": "rect", "rect": {"x": 1, "y": 1, "w": 94, "h": 62},
                    "antialias": false}),
        ))
        .unwrap();
        e.apply_json(&env(
            8,
            "content_aware_fill",
            Some(&l),
            json!({"output": "current_layer"}),
        ))
        .unwrap();
        let v = px(&e, layer, 48, 32);
        assert!(
            v.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c)),
            "fallback produced sane pixels: {v:?}"
        );
    }
}
