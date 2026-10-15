// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `image.auto_tone.yaml` / `image.auto_contrast.yaml` /
//! `image.auto_color.yaml` (params, §40.2 family rules, lock tables,
//! selection behavior), ES §17.1 (histogram service), §17.2 (the Auto
//! analyses — the ACCEPTED CP-0.7 wiring design: analyses emit ordinary
//! Levels params, baked destructively through the existing edit
//! machinery; the kinds are OpNode-INDEPENDENT), §16.2 (`out' = lerp(in,
//! out, sel(p))` masking rule), §11.1 (locks), §40.1 (validation-stage
//! order, no-op empty push); integration-log 2026-08-13 "auto_* wiring
//! offer ACCEPTED" + "§17.2 gamma ruling FINAL" (defining-property
//! election stands — implemented in `auto.rs`; the printed reciprocal is
//! fold-in).
//!
//! The three `image.auto_*` [`KindImpl`]s, exported for the assembly
//! points via [`register`] (arbitration-1 pattern: this crate exports the
//! impls; cli/cabi assemble). All mutation flows through the
//! `KindContext` doors + the shared selection-gated bake loop
//! (`appname_tools::kind_impls::edit_region_at`), so D-48 holds: replay
//! recomputes the same histogram from the same document state and bakes
//! identical bytes.
//!
//! Stage map (§40.2 over the pinned order): stage 5 = contract-generated
//! schema (defaults filled); stage 6 here = clip params re-checked
//! (range + the `clipLow + clipHigh < 1` cross-field — defensive echo of
//! the schema's not-(0.5,0.5) corner) → target resolution → §11.1 locks
//! (PIXELS and ALL block; TRANSPARENCY unaffected — levels never touch
//! alpha) → §17.1 histogram over the selection support → §17.2 analysis
//! → levels bake masked per §16.2.
//!
//! Recorded divergences (lane log; never silent):
//! * "active layer" default: the engine is headless and carries no
//!   active-layer state (the standing fill/paste divergence class). The
//!   default target resolves to the TOPMOST raster layer in z-order
//!   (deterministic pure function of the document; matches paste's
//!   insert-at-top reading). No raster layer ⇒
//!   `error.command.target_kind_mismatch` (the contract's non-Raster
//!   active-layer case). `image.auto_tone` accepts the contract's
//!   explicit `params.targetLayer` override; the other two kinds' schemas
//!   omit it (contract gap note carried verbatim).
//! * Identity analyses (already full-range histograms) return the §40.1
//!   empty push — pinned, so the 1024-entry LUT's re-quantization never
//!   dirties pixels a no-op analysis would not change.

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use appname_core::{
    EngineAssembly, EngineError, ErrorCode, IRect, IVec2, KindContext, KindImpl, KindOutcome,
    LayerId, LockFlags,
};
use appname_tools::kind_impls::edit_region_at;
use appname_tools::resample::Plane;

use crate::auto::{auto_color, auto_contrast, auto_tone, midtone_means, AutoLevels};
use crate::histogram::Histogram;
use crate::kernel::{apply_kernel_pixel, bake_display_lut, display_closed, AdjKernel};
use crate::ops::LevelsTuple;

pub struct AutoToneKind;
pub struct AutoContrastKind;
pub struct AutoColorKind;

/// Register the three auto kinds + the §12.1 adjustments-corpus ops onto
/// an assembly (the cli/cabi assembly points wrap their chain in this —
/// one call carries the whole appname-adjust seam contribution).
pub fn register(assembly: EngineAssembly) -> EngineAssembly {
    crate::op_bridge::register_ops(
        assembly
            .with_kind(Arc::new(AutoToneKind))
            .with_kind(Arc::new(AutoContrastKind))
            .with_kind(Arc::new(AutoColorKind)),
    )
}

// ---------------------------------------------------------------------------
// Shared stage-6 plumbing.
// ---------------------------------------------------------------------------

/// Stage-6 defensive clip read: stage 5 filled defaults and checked the
/// 0..=0.5 bounds; the cross-field sum rule is re-checked here (§40.2 —
/// within the per-field bounds only the (0.5, 0.5) corner violates).
fn clip_params(cx: &KindContext<'_>) -> Result<(f64, f64), EngineError> {
    let read = |name: &'static str| -> Result<f64, EngineError> {
        match cx.params().get(name) {
            None => Ok(0.001),
            Some(v) => {
                let x = v.as_f64().unwrap_or(f64::NAN);
                // Non-finite policy (§40.1): NaN → default, ±Inf → bound
                // (unreachable through standard JSON; kept for the door).
                let x = if x.is_nan() {
                    0.001
                } else if x == f64::INFINITY {
                    0.5
                } else if x == f64::NEG_INFINITY {
                    0.0
                } else {
                    x
                };
                if !(0.0..=0.5).contains(&x) {
                    return Err(EngineError::range(name, "clip fraction outside 0..=0.5"));
                }
                Ok(x)
            }
        }
    };
    let lo = read("clipLow")?;
    let hi = read("clipHigh")?;
    if lo + hi >= 1.0 {
        return Err(EngineError::range(
            "clipHigh",
            "clipLow + clipHigh must be < 1.0 (§40.2 cross-field)",
        ));
    }
    Ok((lo, hi))
}

/// Default-target resolution (recorded divergence): the topmost raster
/// layer in z-order; `param_target` = the contract's explicit
/// `params.targetLayer` (auto_tone only).
fn resolve_auto_target(
    cx: &KindContext<'_>,
    accept_target_param: bool,
) -> Result<LayerId, EngineError> {
    if accept_target_param {
        if let Some(v) = cx.params().get("targetLayer") {
            let id = v
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::range("targetLayer", "uuid string (§40.2)"))?;
            let lid = LayerId::from_uuid(id);
            let node = cx.document().layer(lid).ok_or_else(|| {
                EngineError::bad_target(format!("params.targetLayer {id} does not resolve"))
            })?;
            if node.pixels().is_none() {
                return Err(EngineError::with_field(
                    ErrorCode::TargetKindMismatch,
                    "targetLayer",
                    "params.targetLayer MUST be a Raster layer (§40.2)",
                ));
            }
            return Ok(lid);
        }
    }
    // Topmost raster in composite order (walk lists bottom-first).
    cx.document()
        .z_order_walk()
        .into_iter()
        .rev()
        .find(|lid| {
            cx.document()
                .layer(*lid)
                .map(|n| n.pixels().is_some())
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            EngineError::new(
                ErrorCode::TargetKindMismatch,
                "no raster layer resolves as the active layer (§40.2; headless active-layer \
                 divergence, lane log)",
            )
        })
}

/// §40.2 lock table: PIXELS and ALL block (ALL transitively per §11.1);
/// TRANSPARENCY is unaffected — levels never touch alpha. Reads go
/// through `Document::effective_locks` (the O-CORE door-slice
/// consolidation): the ancestry union makes a group's PIXELS bind the
/// subtree, and subsumes the former local ALL walk (ALL ⊇ PIXELS, so
/// the PIXELS gate already refuses every ALL-locked ancestry).
fn check_auto_locks(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    if cx
        .document()
        .effective_locks(layer)
        .contains(LockFlags::PIXELS)
    {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the target layer's pixels are locked (§11.1; §40.2 lock table)",
        ));
    }
    Ok(())
}

/// The §17.1 statistics support: layer bounds ∩ selection bounds when a
/// selection is active (else the full layer extent). Pixels outside the
/// selection SUPPORT (coverage 0) are zeroed in the returned plane —
/// `a = 0` pixels are excluded from the histogram by the §17.1 rule, so
/// the plane doubles as the exact statistics domain for both the
/// histogram and the midtone-mean scan.
fn support_plane(cx: &KindContext<'_>, layer: LayerId) -> Option<(Plane, IRect)> {
    let doc = cx.document();
    let buf = doc.layer(layer).and_then(|n| n.pixels())?;
    let sel = doc.selection();
    let region = if sel.active() {
        buf.bounds().intersect(&sel.bounds())
    } else {
        buf.bounds()
    };
    if region.is_empty() {
        return None;
    }
    let mut plane = Plane::new(region);
    for y in region.min.y..region.max.y {
        for x in region.min.x..region.max.x {
            let p = IVec2::new(x, y);
            let cov = if sel.active() {
                sel.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0)
            } else {
                1.0
            };
            if cov > 0.0 {
                plane.set(x, y, buf.read_pixel_f32(p));
            } // else: stays (0,0,0,0) — excluded (§17.1 a = 0 rule)
        }
    }
    Some((plane, region))
}

fn is_identity(levels: &AutoLevels) -> bool {
    let id = LevelsTuple::default();
    levels.r == id && levels.g == id && levels.b == id
}

/// Compile the emitted per-channel Levels params exactly as
/// `adjustment.levels` does (§17.2 shared math: per-channel eval, no
/// composite pass; 1024-entry display-referred LUT + closed-form
/// out-of-domain fallback) and bake them masked per §16.2 through the
/// shared edit loop.
fn bake_levels(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    region: IRect,
    levels: &AutoLevels,
) -> Result<KindOutcome, EngineError> {
    let chans = [levels.r, levels.g, levels.b];
    let form = move |v: f32, c: usize| chans[c].eval(v);
    let kernel = AdjKernel::Lut1D {
        tables: bake_display_lut(form),
        closed: display_closed(form),
    };
    let unsigned = {
        let buf = cx
            .document()
            .layer(layer)
            .and_then(|n| n.pixels())
            .ok_or_else(|| EngineError::bad_target("raster target vanished mid-apply"))?;
        matches!(buf.kind(), k if appname_core::ChannelKind::is_unsigned_normalized(k))
    };
    edit_region_at(cx, layer, region, &mut |_p, dst, cov| {
        if cov <= 0.0 {
            return dst;
        }
        let (out, _fallback) = apply_kernel_pixel(&kernel, dst);
        // §16.2 masking rule on the premultiplied working values.
        let mut v = [
            dst[0] + (out[0] - dst[0]) * cov,
            dst[1] + (out[1] - dst[1]) * cov,
            dst[2] + (out[2] - dst[2]) * cov,
            dst[3], // TRANSPARENCY unaffected: alpha untouched (§40.2)
        ];
        if unsigned {
            // §18.2 store discipline (install_plane parity): premul color
            // clamps into [0, α] on unsigned-normalized storage.
            v[3] = v[3].clamp(0.0, 1.0);
            for c in 0..3 {
                v[c] = v[c].clamp(0.0, v[3]);
            }
        }
        v
    })?;
    // Alpha is untouched, so bounds never grow or shrink: no §2.4 pass.
    Ok(KindOutcome::default())
}

fn apply_auto(
    cx: &mut KindContext<'_>,
    accept_target_param: bool,
    analyze: impl FnOnce(&Histogram, &Plane, f64, f64) -> AutoLevels,
) -> Result<KindOutcome, EngineError> {
    let (f_lo, f_hi) = clip_params(cx)?;
    let layer = resolve_auto_target(cx, accept_target_param)?;
    check_auto_locks(cx, layer)?;
    let Some((plane, region)) = support_plane(cx, layer) else {
        return Ok(KindOutcome::default()); // §40.1 empty push
    };
    let hist = Histogram::of_plane(&plane);
    if hist.total == 0 {
        return Ok(KindOutcome::default()); // fully transparent support
    }
    let levels = analyze(&hist, &plane, f_lo, f_hi);
    if is_identity(&levels) {
        return Ok(KindOutcome::default()); // pinned no-op (module doc)
    }
    bake_levels(cx, layer, region, &levels)
}

// ---------------------------------------------------------------------------
// The three kinds.
// ---------------------------------------------------------------------------

impl KindImpl for AutoToneKind {
    fn kind(&self) -> &'static str {
        "image.auto_tone"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        apply_auto(cx, true, |hist, _plane, lo, hi| auto_tone(hist, lo, hi))
    }
}

impl KindImpl for AutoContrastKind {
    fn kind(&self) -> &'static str {
        "image.auto_contrast"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        apply_auto(cx, false, |hist, _plane, lo, hi| {
            auto_contrast(hist, lo, hi)
        })
    }
}

impl KindImpl for AutoColorKind {
    fn kind(&self) -> &'static str {
        "image.auto_color"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let neutralize = cx
            .params()
            .get("neutralizeMidtones")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        apply_auto(cx, false, move |hist, plane, lo, hi| {
            let midtones = if neutralize {
                midtone_means(plane)
            } else {
                None
            };
            auto_color(hist, midtones, lo, hi, neutralize)
        })
    }
}
