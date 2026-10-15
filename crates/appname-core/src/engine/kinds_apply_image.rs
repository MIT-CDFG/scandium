// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `channel.apply_image` / `channel.calculations`; ES
//! §14.4 (conditioning `S′`/`S″`, application lerp, coverage arm m(p),
//! legacy add/subtract, the output table), §4.5 (kernel catalog —
//! render's, injected), §13.1/§13.2 (selection/coverage), §40.4 (family
//! rules); door C4 of reports/O-COLOR-log.md **in the SUPERSEDING
//! addendum shape** ("D1 fix: §14.4 injection seam made total"):
//! dispatch on render's kernel routing — separable →
//! `apply_blend_plane`; non-separable/whole-pixel (hue, saturation,
//! color, luminosity, darkerColor, lighterColor) → `apply_blend_planes3`
//! over the three component planes; `blend:"dissolve"` →
//! `dissolve_reserved()` mapped to the not-implemented taxonomy;
//! add/subtract → the legacy planes. Conditioning is NEVER re-derived
//! core-side (`ApplyShape::condition_source` / `source_prime` are the
//! exported single source); kernels are NEVER re-derived core-side
//! (`BlendKernelRouting`, the assembly-injected
//! `render_separable_kernel`/`render_triple_kernel` pair — the
//! one-math-library clause; the §14.4 bit-identity test lands core-side
//! against render's kernel, dev-dep leg).
//!
//! CP-1.0 door slice 3, group 7 (sequenced last per O-INT). Recorded
//! divergences/postures (lane log):
//!
//! * `source.docId`: single-document engine — equal to this document's
//!   id it resolves (dims trivially identical); any other id answers
//!   `bad_target` naming the posture (`error.command.size_mismatch`
//!   minted, reachable with the session host — the channel.duplicate
//!   record, shared).
//! * Mode-gated component selectors (`c`/`m`/`y`/`k` CMYK-only,
//!   `l`/`a`/`b_lab` Lab-only, `r`/`g`/`b`/`rgb` RGB-only) answer
//!   `target_kind_mismatch` against a wrong-mode source document —
//!   §14.4 "channel selector must exist for the source document's
//!   mode". CMYK documents cannot exist yet (C5 substrate note), so the
//!   c/m/y/k arms are mode-gated refusals in practice.
//! * `layer:"merged"` = the flattened composite via the EXACT
//!   `composite_plane_f32` math (both arms — reference walk +
//!   parametric re-render; a parametric node the §12.1 arm cannot
//!   represent refuses typed, QA-F8 posture inherited).
//! * Triple modes require a LAYER target under `channel:"rgb"`
//!   (three-component doc mode); a Gray/channel target — or a
//!   single-channel source — with a triple mode answers the §40.4
//!   `target_kind_mismatch` family (the addendum's rule verbatim).
//! * The `mask` arm resolves like the main source but must land ONE
//!   coverage plane — `channel:"rgb"` under `mask.source` answers
//!   `target_kind_mismatch` (a §14.4 m(p) is scalar; recorded).
//! * `channel.calculations` combines SINGLE channels (T = source2,
//!   S = source1); `result:"new_document"` answers typed NI (the
//!   single-document posture again — §40.4's empty-DeltaSet-undo arm
//!   needs the session host); `result:"new_channel"` arm-enforces
//!   `newChannelId` (the schema's if/then, outside the stage-3
//!   interpreter subset — the selection.save precedent).
//! * Integer-depth targets marshal through the standing
//!   storage-encoding interim (u8 sRGB-encoded straight ↔ linear f32
//!   working), the mode-convert reading verbatim.

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use appname_color::{
    apply_add_plane, apply_blend_plane, apply_blend_planes3, apply_subtract_plane,
    dissolve_reserved, ApplyShape, CoverageArm,
};

use crate::engine::color_glue;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::geom::{IRect, IVec2};
use crate::graph::id::{ChannelId, LayerId};
use crate::graph::node::{BlendMode, LockFlags};
use crate::tile::map::TileCoord;
use crate::tile::model::ColorModelKind;
use crate::tile::{TILE, TILE_I32};

pub struct ChannelApplyImageKind;
pub struct ChannelCalculationsKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(ChannelApplyImageKind),
        Arc::new(ChannelCalculationsKind),
    ]
}

// ---------------------------------------------------------------------------
// Blend dispatch (the addendum's three-way routing + the two legacies).
// ---------------------------------------------------------------------------

enum BlendDispatch {
    Separable(fn(f32, f32) -> f32),
    Triple(fn([f32; 3], [f32; 3]) -> [f32; 3]),
    LegacyAdd,
    LegacySubtract,
}

fn dispatch_blend(cx: &KindContext<'_>, wire: &str) -> Result<BlendDispatch, EngineError> {
    match wire {
        // §14.4 legacy arithmetic — no kernel, always available.
        "add" => return Ok(BlendDispatch::LegacyAdd),
        // The contract note: 'subtract' takes the §14.4 legacy
        // arithmetic definition IN THIS COMMAND (not the §4.5 kernel).
        "subtract" => return Ok(BlendDispatch::LegacySubtract),
        "dissolve" => return Err(color_glue::map_color_error(dissolve_reserved())),
        _ => {}
    }
    let mode: BlendMode = serde_json::from_value(Value::String(wire.to_string()))
        .map_err(|_| EngineError::range("blend", format!("unknown blend \"{wire}\"")))?;
    let routing = cx.require_blend_kernels()?;
    if let Some(f) = (routing.separable)(mode) {
        return Ok(BlendDispatch::Separable(f));
    }
    if let Some(f) = (routing.triple)(mode) {
        return Ok(BlendDispatch::Triple(f));
    }
    // Dissolve is the only both-None mode and is handled above.
    Err(EngineError::range(
        "blend",
        format!("blend \"{wire}\" resolves no §4.5 kernel routing"),
    ))
}

// ---------------------------------------------------------------------------
// Source grammar (§14.4/§10.3).
// ---------------------------------------------------------------------------

enum LayerRef {
    Merged,
    Layer(LayerId),
}

enum ChannelSel {
    /// The composite color triple (`rgb`; RGB-mode documents only).
    Rgb,
    /// One color component of the layer/merged planes (index into the
    /// mode's color components; mode-gated).
    Comp(usize),
    /// The document selection's coverage.
    Selection,
    /// The layer source's alpha.
    Transparency,
    /// A §14.1 named channel's coverage.
    Alpha(ChannelId),
}

struct SourceSpec {
    layer: LayerRef,
    channel: ChannelSel,
}

/// Straight LINEAR planes of a source's color components + alpha over the
/// canvas (values domain, §14.4).
struct SourcePlanes {
    comps: Vec<Vec<f32>>,
    alpha: Vec<f32>,
}

fn parse_source(cx: &KindContext<'_>, v: &Value, field: &str) -> Result<SourceSpec, EngineError> {
    let obj = v
        .as_object()
        .ok_or_else(|| EngineError::range(field, "source must be an object"))?;
    // docId: single-document posture (module docs).
    if let Some(d) = obj.get("docId").and_then(Value::as_str) {
        let id = Uuid::parse_str(d)
            .map_err(|_| EngineError::range(field, format!("docId is not a UUID: \"{d}\"")))?;
        if id != cx.document().id().as_uuid() {
            return Err(EngineError::bad_target(format!(
                "{field}.docId {id} does not resolve to an open document (the engine hosts \
                 one document; §14.4 cross-document sources need the session host — \
                 size_mismatch reachable there; divergence recorded)"
            )));
        }
    }
    let layer = match obj.get("layer") {
        Some(Value::String(s)) if s == "merged" => LayerRef::Merged,
        Some(Value::String(s)) => {
            let id = Uuid::parse_str(s).map_err(|_| {
                EngineError::range(
                    field,
                    format!("layer must be a layerId or \"merged\": \"{s}\""),
                )
            })?;
            let lid = LayerId::from_uuid(id);
            let node = cx.document().layer(lid).ok_or_else(|| {
                EngineError::bad_target(format!("{field}.layer {id} does not resolve"))
            })?;
            if node.pixels().is_none() {
                return Err(EngineError::bad_target(format!(
                    "{field}.layer {id} is not a raster surface (§14.4 sources read pixels)"
                )));
            }
            LayerRef::Layer(lid)
        }
        _ => return Err(EngineError::missing(&format!("{field}.layer"))),
    };
    let mode = cx.document().color().mode;
    let channel = match obj.get("channel").and_then(Value::as_str).unwrap_or("rgb") {
        "rgb" => {
            if mode != ColorModelKind::Rgba {
                return Err(EngineError::new(
                    ErrorCode::TargetKindMismatch,
                    "channel 'rgb' requires an RGB-mode source document (§14.4)",
                ));
            }
            ChannelSel::Rgb
        }
        c @ ("r" | "g" | "b") => {
            if mode != ColorModelKind::Rgba {
                return Err(EngineError::new(
                    ErrorCode::TargetKindMismatch,
                    format!("channel '{c}' exists only in RGB mode (§14.4)"),
                ));
            }
            ChannelSel::Comp(match c {
                "r" => 0,
                "g" => 1,
                _ => 2,
            })
        }
        c @ ("l" | "a" | "b_lab") => {
            if mode != ColorModelKind::LabA {
                return Err(EngineError::new(
                    ErrorCode::TargetKindMismatch,
                    format!("channel '{c}' exists only in Lab mode (§14.4)"),
                ));
            }
            ChannelSel::Comp(match c {
                "l" => 0,
                "a" => 1,
                _ => 2,
            })
        }
        c @ ("c" | "m" | "y" | "k") => {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                format!("channel '{c}' exists only in CMYK mode (§14.4; the document is {mode:?})"),
            ))
        }
        "selection" => ChannelSel::Selection,
        "transparency" => ChannelSel::Transparency,
        other => {
            let id = Uuid::parse_str(other).map_err(|_| {
                EngineError::range(field, format!("unknown channel selector \"{other}\""))
            })?;
            let cid = ChannelId::from_uuid(id);
            if cx.document().channel(cid).is_none() {
                return Err(EngineError::bad_target(format!(
                    "{field}.channel {id} does not resolve to a named channel (§14.1)"
                )));
            }
            ChannelSel::Alpha(cid)
        }
    };
    Ok(SourceSpec { layer, channel })
}

/// Number of color components (alpha excluded) of the document mode.
fn doc_comps(cx: &KindContext<'_>) -> usize {
    cx.document().color().mode.channels() - 1
}

/// Read a source's straight-linear component planes + alpha over the
/// canvas (§14.4 values domain; interim storage-encoding decode).
fn layer_planes(cx: &KindContext<'_>, layer: &LayerRef) -> Result<SourcePlanes, EngineError> {
    let canvas = cx.document().geometry().canvas_rect();
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let n = w * h;
    let comps_n = doc_comps(cx);
    let mut comps = vec![vec![0.0f32; n]; comps_n];
    let mut alpha = vec![0.0f32; n];
    match layer {
        LayerRef::Merged => {
            // The composite door's exact math (QA-F8 posture inherited:
            // unrepresentable parametric content refuses typed).
            let doc = cx.document();
            let plane = if crate::io::png_io::has_visible_parametric_content(doc) {
                crate::io::png_io::parametric_composite_plane(
                    doc,
                    cx.ops,
                    cx.text,
                    cx.vector,
                    cx.fill_convert,
                    cx.blend_kernels,
                    canvas,
                )?
            } else {
                let layers: Vec<&crate::buffer::Buffer> = doc
                    .z_order_walk()
                    .into_iter()
                    .filter_map(|id| doc.layer(id))
                    .filter(|nd| nd.visible())
                    .filter_map(|nd| nd.pixels())
                    .collect();
                crate::io::png_io::composite_plane(&layers, w as u32, h as u32)
            };
            // Linear premultiplied composite → straight values.
            for i in 0..n {
                let a = plane[i * 4 + 3];
                alpha[i] = a;
                let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
                for (c, comp) in comps.iter_mut().enumerate().take(comps_n.min(3)) {
                    comp[i] = (plane[i * 4 + c] * inv).clamp(0.0, 1.0);
                }
            }
        }
        LayerRef::Layer(lid) => {
            let doc = cx.document();
            let buf = doc.layer(*lid).expect("resolved").pixels().expect("raster");
            let a_idx = doc.color().mode.channels() - 1;
            let mut i = 0;
            for y in canvas.min.y..canvas.max.y {
                for x in canvas.min.x..canvas.max.x {
                    let v = buf.read_pixel_f32(IVec2::new(x, y));
                    let a = v[a_idx];
                    alpha[i] = a;
                    // PREMULTIPLIED-normalized at every depth (the §6.4
                    // fix-round ruling; integer content rides its
                    // storage encoding through the §14.4 kernels — the
                    // stroke-family working-domain interim, recorded).
                    let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
                    for (c, comp) in comps.iter_mut().enumerate() {
                        comp[i] = (v[c] * inv).clamp(0.0, 1.0);
                    }
                    i += 1;
                }
            }
        }
    }
    Ok(SourcePlanes { comps, alpha })
}

/// Read one coverage-style plane over the canvas from a non-layer
/// selector.
fn coverage_plane(cx: &KindContext<'_>, read: impl Fn(IVec2) -> f32) -> Vec<f32> {
    let canvas = cx.document().geometry().canvas_rect();
    let mut out = Vec::with_capacity((canvas.width() * canvas.height()) as usize);
    for y in canvas.min.y..canvas.max.y {
        for x in canvas.min.x..canvas.max.x {
            out.push(read(IVec2::new(x, y)).clamp(0.0, 1.0));
        }
    }
    out
}

/// Resolve a parsed source to its plane(s): `Ok(Err(triple))` never —
/// returns either one plane or the rgb triple.
enum Resolved {
    One(Vec<f32>),
    Triple([Vec<f32>; 3]),
}

fn resolve_source(cx: &KindContext<'_>, spec: &SourceSpec) -> Result<Resolved, EngineError> {
    Ok(match &spec.channel {
        ChannelSel::Rgb => {
            let p = layer_planes(cx, &spec.layer)?;
            let [r, g, b]: [Vec<f32>; 3] = p
                .comps
                .try_into()
                .map_err(|_| EngineError::bad_target("rgb triple needs three components"))?;
            Resolved::Triple([r, g, b])
        }
        ChannelSel::Comp(c) => {
            let mut p = layer_planes(cx, &spec.layer)?;
            Resolved::One(std::mem::take(&mut p.comps[*c]))
        }
        ChannelSel::Transparency => {
            let p = layer_planes(cx, &spec.layer)?;
            Resolved::One(p.alpha)
        }
        ChannelSel::Selection => {
            let sel = cx.document().selection();
            Resolved::One(coverage_plane(cx, |p| sel.mask().read_pixel_f32(p)[0]))
        }
        ChannelSel::Alpha(cid) => {
            let buf = cx.document().channel(*cid).expect("resolved").buffer();
            Resolved::One(coverage_plane(cx, |p| buf.read_pixel_f32(p)[0]))
        }
    })
}

/// The optional §14.4 mask arm: resolve to ONE coverage plane.
fn resolve_mask_arm(cx: &KindContext<'_>) -> Result<(Option<Vec<f32>>, bool), EngineError> {
    let Some(m) = cx.params().get("mask").cloned() else {
        return Ok((None, false));
    };
    let obj = m
        .as_object()
        .ok_or_else(|| EngineError::range("mask", "`mask` must be an object"))?;
    let src_v = obj
        .get("source")
        .ok_or_else(|| EngineError::missing("mask.source"))?;
    let spec = parse_source(cx, src_v, "mask.source")?;
    let plane = match resolve_source(cx, &spec)? {
        Resolved::One(p) => p,
        Resolved::Triple(_) => {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "mask.source must resolve ONE coverage plane — 'rgb' is not a scalar m(p) \
                 (§14.4; recorded reading)",
            ))
        }
    };
    let invert = obj.get("invert").and_then(Value::as_bool).unwrap_or(false);
    Ok((Some(plane), invert))
}

fn shape_from_params(cx: &KindContext<'_>) -> ApplyShape {
    let f = |k: &str, d: f64| cx.params().get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
    ApplyShape {
        invert_source: cx
            .params()
            .get("invertSource")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        opacity: f("opacity", 1.0),
        scale: f("scale", 1.0),
        offset: f("offset", 0.0),
    }
}

/// The active selection's canvas plane, if any (m(p) factor).
fn selection_plane(cx: &KindContext<'_>) -> Option<Vec<f32>> {
    if !cx.document().selection().active() {
        return None;
    }
    let sel = cx.document().selection();
    Some(coverage_plane(cx, |p| sel.mask().read_pixel_f32(p)[0]))
}

/// Write blended straight-linear component planes + untouched alpha back
/// to a raster layer over the canvas (interim storage re-encode).
///
/// Posture note (review MINOR-3, superseded at the §6.4 fix round):
/// pixels with `a == 0` write all-zero — canonical for premultiplied
/// storage at EVERY depth (§6.4: no hidden color under zero alpha).
/// The first cut's integer-depth STRAIGHT-encoded write was the §6.4
/// breach class the O-UI filing exposed (encode(c) > a under fractional
/// alpha) — integer depths now re-premultiply exactly like f32, values
/// riding the storage encoding (the recorded working-domain interim).
fn write_layer_planes(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    comps: &[Vec<f32>],
    alpha: &[f32],
) -> Result<(), EngineError> {
    let canvas = cx.document().geometry().canvas_rect();
    let (w, _h) = (canvas.width() as usize, canvas.height() as usize);
    let a_idx = cx.document().color().mode.channels() - 1;
    let coords: Vec<(i32, i32)> = canvas.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        if rect.intersect(&canvas).is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        let mut any = false;
        {
            let buf = cx
                .document()
                .layer(layer)
                .expect("resolved")
                .pixels()
                .expect("raster");
            let mut ti = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    if canvas.contains(p) {
                        let i = (y - canvas.min.y) as usize * w + (x - canvas.min.x) as usize;
                        let a = alpha[i];
                        let mut px = [0.0f32; 4];
                        if a > 0.0 {
                            for (c, comp) in comps.iter().enumerate() {
                                let s = comp[i].clamp(0.0, 1.0);
                                // Re-premultiply at EVERY depth: §6.4
                                // c ≤ a by construction (fix-round
                                // ruling; a = 0 stays all-zero).
                                px[c] = (s * a).clamp(0.0, a);
                            }
                            px[a_idx] = a;
                        }
                        if px != buf.read_pixel_f32(p) {
                            any = true;
                        }
                        plane[ti] = px;
                    } else {
                        plane[ti] = buf.read_pixel_f32(p);
                    }
                    ti += 1;
                }
            }
        }
        if any {
            cx.install_surface_plane(layer.as_uuid(), coord, &plane)?;
        }
    }
    Ok(())
}

/// Write a blended coverage plane back to a Gray coverage surface (a
/// §14.1 named channel or the selection), returning the nonzero rect.
///
/// Every canvas-intersecting tile installs UNCONDITIONALLY (review
/// DEFECT-1 ruled fix): an all-zero blended result tile must OVERWRITE
/// prior nonzero coverage — the §14.4 in-place [NORMATIVE] semantics
/// and the calculations result:"selection" replace both die on a
/// skip-empty install (the §2.4 commit prune still drops tiles whose
/// bytes did not change, so the §40.1 no-op push survives).
fn write_coverage_plane(
    cx: &mut KindContext<'_>,
    surface: Uuid,
    values: &[f32],
) -> Result<IRect, EngineError> {
    let canvas = cx.document().geometry().canvas_rect();
    let w = canvas.width() as usize;
    let mut nonzero = IRect::EMPTY;
    let coords: Vec<(i32, i32)> = canvas.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        if rect.intersect(&canvas).is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        {
            let mut ti = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    let v = if canvas.contains(p) {
                        let i = (y - canvas.min.y) as usize * w + (x - canvas.min.x) as usize;
                        values[i].clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    if v > 0.0 {
                        nonzero = nonzero.union(&IRect::new(x, y, x + 1, y + 1));
                    }
                    plane[ti] = [v, 0.0, 0.0, 0.0];
                    ti += 1;
                }
            }
        }
        cx.install_surface_plane(surface, coord, &plane)?;
    }
    Ok(nonzero)
}

/// Run one §14.4 blend over single planes (T mutated in place).
fn blend_single(
    dispatch: &BlendDispatch,
    t: &mut [f32],
    s: &[f32],
    shape: &ApplyShape,
    cov: &CoverageArm<'_>,
) -> Result<(), EngineError> {
    match dispatch {
        BlendDispatch::Separable(f) => {
            apply_blend_plane(f, t, s, shape, cov).map_err(color_glue::map_color_error)
        }
        BlendDispatch::LegacyAdd => {
            apply_add_plane(t, s, shape, cov).map_err(color_glue::map_color_error)
        }
        BlendDispatch::LegacySubtract => {
            apply_subtract_plane(t, s, shape, cov).map_err(color_glue::map_color_error)
        }
        BlendDispatch::Triple(_) => Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "triple (non-separable/whole-pixel) modes require a layer target under \
             channel:'rgb' — single-channel targets cannot host them (§40.4 family; the \
             addendum rule)",
        )),
    }
}

// ---------------------------------------------------------------------------
// channel.apply_image
// ---------------------------------------------------------------------------

impl KindImpl for ChannelApplyImageKind {
    fn kind(&self) -> &'static str {
        "channel.apply_image"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Target: exactly one of layerId / channelId (§10.3).
        let (t_layer, t_channel) = match cx.envelope().target.as_ref() {
            Some(t) => (t.layer_id, t.channel_id),
            None => (None, None),
        };
        let blend_wire = cx
            .params()
            .get("blend")
            .and_then(Value::as_str)
            .unwrap_or("multiply")
            .to_string();
        let shape = shape_from_params(cx);
        shape.validate().map_err(color_glue::map_color_error)?;
        let source_v = cx
            .params()
            .get("source")
            .cloned()
            .ok_or_else(|| EngineError::missing("source"))?;
        let preserve = cx
            .params()
            .get("preserveTransparency")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        match (t_layer, t_channel) {
            (Some(layer), None) => {
                let layer = LayerId::from_uuid(layer);
                let node = cx.document().layer(layer).ok_or_else(|| {
                    EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
                })?;
                if node.pixels().is_none() {
                    return Err(EngineError::new(
                        ErrorCode::TargetKindMismatch,
                        "the apply-image layer target must be raster (§14.4 output table)",
                    ));
                }
                // §40.4 locks: PIXELS/ALL block; TRANSPARENCY constrains.
                let locks = cx.document().effective_locks(layer);
                if locks.contains(LockFlags::PIXELS) || locks.contains(LockFlags::ALL) {
                    return Err(EngineError::new(
                        ErrorCode::Locked,
                        "the target layer's pixels are locked (§11.1)",
                    ));
                }
                let preserve = preserve || locks.contains(LockFlags::TRANSPARENCY);
                let dispatch = dispatch_blend(cx, &blend_wire)?;
                let spec = parse_source(cx, &source_v, "source")?;
                let src = resolve_source(cx, &spec)?;
                let (mask, invert_mask) = resolve_mask_arm(cx)?;
                let sel = selection_plane(cx);
                let mut target = layer_planes(cx, &LayerRef::Layer(layer))?;
                let alpha = target.alpha.clone();
                let cov = CoverageArm {
                    mask: mask.as_deref(),
                    invert_mask,
                    selection: sel.as_deref(),
                    target_alpha: if preserve { Some(&alpha) } else { None },
                };
                match (&dispatch, src) {
                    (BlendDispatch::Triple(f), Resolved::Triple([sr, sg, sb])) => {
                        if target.comps.len() != 3 {
                            return Err(EngineError::new(
                                ErrorCode::TargetKindMismatch,
                                "triple modes need a three-component target mode (§40.4)",
                            ));
                        }
                        let (head, tail) = target.comps.split_at_mut(1);
                        let (mid, last) = tail.split_at_mut(1);
                        apply_blend_planes3(
                            f,
                            [&mut head[0], &mut mid[0], &mut last[0]],
                            [&sr, &sg, &sb],
                            &shape,
                            &cov,
                        )
                        .map_err(color_glue::map_color_error)?;
                    }
                    (BlendDispatch::Triple(_), Resolved::One(_)) => {
                        return Err(EngineError::new(
                            ErrorCode::TargetKindMismatch,
                            "triple modes require channel:'rgb' on the source (§40.4 family; \
                             the addendum rule)",
                        ))
                    }
                    (_, Resolved::Triple([sr, sg, sb])) => {
                        // Separable/legacy over the rgb triple: per
                        // component plane (§14.4).
                        if target.comps.len() != 3 {
                            return Err(EngineError::new(
                                ErrorCode::TargetKindMismatch,
                                "channel 'rgb' is valid only against a three-component \
                                 layer target (§14.4 output table)",
                            ));
                        }
                        for (t, s) in target.comps.iter_mut().zip([&sr, &sg, &sb]) {
                            blend_single(&dispatch, t, s, &shape, &cov)?;
                        }
                    }
                    (_, Resolved::One(s)) => {
                        // Single-channel source rides as Gray: every
                        // target component blends against it (§14.4).
                        for t in target.comps.iter_mut() {
                            blend_single(&dispatch, t, &s, &shape, &cov)?;
                        }
                    }
                }
                write_layer_planes(cx, layer, &target.comps, &alpha)?;
                Ok(KindOutcome::default())
            }
            (None, Some(channel)) => {
                let cid = ChannelId::from_uuid(channel);
                if cx.document().channel(cid).is_none() {
                    return Err(EngineError::bad_target(format!(
                        "target.channelId {channel} does not resolve to a named channel \
                         (§14.1)"
                    )));
                }
                let dispatch = dispatch_blend(cx, &blend_wire)?;
                let spec = parse_source(cx, &source_v, "source")?;
                let src = match resolve_source(cx, &spec)? {
                    Resolved::One(p) => p,
                    Resolved::Triple(_) => {
                        return Err(EngineError::new(
                            ErrorCode::TargetKindMismatch,
                            "an rgb source is valid only against a layer target (§14.4 \
                             output table)",
                        ))
                    }
                };
                let (mask, invert_mask) = resolve_mask_arm(cx)?;
                let sel = selection_plane(cx);
                let buf_read = {
                    let buf = cx.document().channel(cid).expect("resolved").buffer();
                    coverage_plane(cx, |p| buf.read_pixel_f32(p)[0])
                };
                let mut t = buf_read;
                let cov = CoverageArm {
                    mask: mask.as_deref(),
                    invert_mask,
                    selection: sel.as_deref(),
                    target_alpha: None, // channel targets never pass one
                };
                blend_single(&dispatch, &mut t, &src, &shape, &cov)?;
                let nonzero = write_coverage_plane(cx, cid.as_uuid(), &t)?;
                cx.set_surface_bounds(cid.as_uuid(), nonzero)?;
                Ok(KindOutcome::default())
            }
            _ => Err(EngineError::bad_target(
                "channel.apply_image targets exactly one of target.layerId or \
                 target.channelId (§10.3)",
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// channel.calculations
// ---------------------------------------------------------------------------

impl KindImpl for ChannelCalculationsKind {
    fn kind(&self) -> &'static str {
        "channel.calculations"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let blend_wire = cx
            .params()
            .get("blend")
            .and_then(Value::as_str)
            .unwrap_or("multiply")
            .to_string();
        let shape = shape_from_params(cx);
        shape.validate().map_err(color_glue::map_color_error)?;
        let result = cx
            .params()
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("new_channel")
            .to_string();
        let one_plane = |cx: &KindContext<'_>, key: &str| -> Result<Vec<f32>, EngineError> {
            let v = cx
                .params()
                .get(key)
                .cloned()
                .ok_or_else(|| EngineError::missing(key))?;
            let spec = parse_source(cx, &v, key)?;
            match resolve_source(cx, &spec)? {
                Resolved::One(p) => Ok(p),
                Resolved::Triple(_) => Err(EngineError::new(
                    ErrorCode::TargetKindMismatch,
                    format!(
                        "{key} must resolve ONE channel — calculations combine single \
                             channels (§14.4)"
                    ),
                )),
            }
        };
        let dispatch = dispatch_blend(cx, &blend_wire)?;
        let s1 = one_plane(cx, "source1")?;
        let mut t = one_plane(cx, "source2")?; // T = source2 (backdrop)
        let (mask, invert_mask) = resolve_mask_arm(cx)?;
        let sel = selection_plane(cx);
        let cov = CoverageArm {
            mask: mask.as_deref(),
            invert_mask,
            selection: sel.as_deref(),
            target_alpha: None,
        };
        blend_single(&dispatch, &mut t, &s1, &shape, &cov)?;

        match result.as_str() {
            "new_channel" => {
                // The schema's if/then: newChannelId REQUIRED here
                // (arm-enforced — stage-3 interpreter subset).
                let new_id = cx
                    .params()
                    .get("newChannelId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| EngineError::missing("newChannelId"))?;
                let new_id = Uuid::parse_str(new_id).map_err(|_| {
                    EngineError::range("newChannelId", format!("not a valid UUID: \"{new_id}\""))
                })?;
                let name = format!("Alpha {}", cx.document().channel_count() + 1);
                let cid = cx.insert_named_channel(ChannelId::from_uuid(new_id), name)?;
                let nonzero = write_coverage_plane(cx, cid.as_uuid(), &t)?;
                cx.set_surface_bounds(cid.as_uuid(), nonzero)?;
                Ok(KindOutcome {
                    created_ids: vec![cid.as_uuid()],
                })
            }
            "selection" => {
                let surface = cx.selection_surface();
                let nonzero = write_coverage_plane(cx, surface, &t)?;
                // Replace semantics also zero what the plane left out.
                let canvas = cx.document().geometry().canvas_rect();
                let stale: Vec<TileCoord> = {
                    let buf = cx.document().selection().mask();
                    buf.tiles()
                        .iter()
                        .map(|(c, _)| c)
                        .filter(|c| c.pixel_rect().intersect(&canvas).is_empty())
                        .collect()
                };
                for c in stale {
                    let zeros = vec![[0.0f32; 4]; TILE * TILE];
                    cx.install_surface_plane(surface, c, &zeros)?;
                }
                cx.set_selection_bounds(nonzero)?; // §13.1 bounds maintenance
                Ok(KindOutcome::default())
            }
            "new_document" => Err(EngineError::new(
                ErrorCode::NotImplemented,
                "result:'new_document' needs the session host (single-document engine; the \
                 §40.4 empty-DeltaSet-undo arm rides it — divergence recorded)",
            )),
            other => Err(EngineError::range(
                "result",
                format!("unknown result \"{other}\""),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use uuid::Uuid;

    use crate::engine::error::ErrorCode;
    use crate::engine::kind_impl::{BlendKernelRouting, EngineAssembly};
    use crate::engine::Engine;
    use crate::geom::IVec2;
    use crate::graph::id::ChannelId;
    use crate::graph::node::BlendMode;

    /// Dev-dep type adapter: the cyclic dev-dependency builds core TWICE
    /// (once as render's dependency, once as this test crate), so the two
    /// `BlendMode` types do not unify — bridge by the serde wire form
    /// (identical camelCase catalog by construction; serde_json is a
    /// single shared build). The PRODUCTION assembly point injects
    /// render's fns directly (one core build there — no adapter).
    fn adapt_sep(m: crate::graph::node::BlendMode) -> Option<fn(f32, f32) -> f32> {
        let dep = serde_json::from_value(serde_json::to_value(m).unwrap()).unwrap();
        appname_render::render_separable_kernel(dep)
    }
    fn adapt_triple(
        m: crate::graph::node::BlendMode,
    ) -> Option<crate::engine::kind_impl::TripleKernel> {
        let dep = serde_json::from_value(serde_json::to_value(m).unwrap()).unwrap();
        appname_render::render_triple_kernel(dep)
    }

    /// The seam injected in the production SHAPE: render's routing table
    /// (the fns under test are render's own; render pins fn-pointer
    /// identity between this table and its compositor dispatch, so
    /// kernel-level identity here IS composite identity at opaque alpha).
    ///
    /// DELIBERATE-PARTIAL RIG (allowlisted by exact path in
    /// appname-assembly's workspace-walk guard): core cannot fold
    /// PRODUCTION_CHAIN (dependency direction) and the dev-dep type
    /// split forces the serde-bridge adapters — this rig never claims
    /// production parity; the production injection lives in
    /// appname-assembly's manifest row.
    fn kernel_assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new()).with_blend_kernels(
            BlendKernelRouting {
                separable: adapt_sep,
                triple: adapt_triple,
            },
        )
    }

    fn ready() -> Engine {
        let mut e = kernel_assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "a1b10000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 32, "height": 32, "background": "white", "depth": "f32"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, target: Value, params: Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("a1b10000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if !target.is_null() {
            v["target"] = target;
        }
        v.to_string()
    }

    const CHAN_A: &str = "feedfeed-0000-4000-8000-00000000000a";
    const CHAN_B: &str = "feedfeed-0000-4000-8000-00000000000b";

    fn add_channel_filled(e: &mut Engine, n: u32, id: &str, v: f64) {
        e.apply_json(&env(
            n,
            "channel.add",
            Value::Null,
            json!({"newChannelId": id}),
        ))
        .unwrap();
        e.apply_json(&env(
            n + 1,
            "channel.edit",
            json!({"channelId": id}),
            json!({"edit": {"op": "fill", "value": v}}),
        ))
        .unwrap();
    }

    fn chan_at(e: &Engine, id: &str, x: i32, y: i32) -> f32 {
        let cid = ChannelId::from_uuid(Uuid::parse_str(id).unwrap());
        e.document()
            .unwrap()
            .channel(cid)
            .unwrap()
            .buffer()
            .read_pixel_f32(IVec2::new(x, y))[0]
    }

    fn layer_px(e: &Engine, x: i32, y: i32) -> [f32; 4] {
        let layer = e.document().unwrap().z_order_walk()[0];
        e.document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    /// §14.4 bit-identity clause: apply multiply at opacity 1, no mask,
    /// no selection ≡ render's Multiply kernel bit-for-bit (the lerp
    /// w = 1 endpoint is exact by construction; render's fn-pointer
    /// identity pin extends this to the Multiply-layer composite at
    /// opaque alpha).
    #[test]
    fn apply_multiply_is_bit_identical_to_renders_kernel() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.6371);
        add_channel_filled(&mut e, 3, CHAN_B, 0.2843);
        let t_before = chan_at(&e, CHAN_A, 5, 5);
        let s_before = chan_at(&e, CHAN_B, 5, 5);
        e.apply_json(&env(
            5,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_B}, "blend": "multiply"}),
        ))
        .unwrap();
        let kernel = adapt_sep(BlendMode::Multiply).unwrap();
        assert_eq!(
            chan_at(&e, CHAN_A, 5, 5).to_bits(),
            kernel(t_before, s_before).to_bits(),
            "§14.4 bit-identity against render's kernel (one math library)"
        );
        // Undo restores the exact prior bytes; replay reproduces.
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(chan_at(&e, CHAN_A, 5, 5), t_before);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// §14.4 conditioning + coverage: S″ = clamp(S′/scale + offset) and
    /// the application lerp under opacity, via the color-crate arms.
    #[test]
    fn conditioning_scale_offset_opacity_and_legacy_arithmetic() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.5);
        add_channel_filled(&mut e, 3, CHAN_B, 0.8);
        // multiply with scale 2, offset -0.1, opacity 0.5:
        // S″ = clamp(0.8/2 − 0.1) = 0.3; B = 0.5·0.3 = 0.15;
        // R = 0.5 + (0.15 − 0.5)·0.5 = 0.325.
        e.apply_json(&env(
            5,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_B},
                   "blend": "multiply", "scale": 2, "offset": -0.1, "opacity": 0.5}),
        ))
        .unwrap();
        assert!((chan_at(&e, CHAN_A, 5, 5) - 0.325).abs() < 1e-6);
        e.undo().unwrap();
        // Legacy add: scale/offset INSIDE the arithmetic, S″ skipped:
        // R = clamp((0.5 + 0.8)/2 + 0.0) = 0.65.
        e.apply_json(&env(
            6,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_B},
                   "blend": "add", "scale": 2}),
        ))
        .unwrap();
        assert!((chan_at(&e, CHAN_A, 5, 5) - 0.65).abs() < 1e-6);
        e.undo().unwrap();
        // Legacy subtract with invertSource: S′ = 0.2;
        // R = clamp((0.5 − 0.2)/1 + 0) = 0.3.
        e.apply_json(&env(
            7,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_B},
                   "blend": "subtract", "invertSource": true}),
        ))
        .unwrap();
        assert!((chan_at(&e, CHAN_A, 5, 5) - 0.3).abs() < 1e-6);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Triple routing (the addendum rule): hue on a layer target under
    /// channel:'rgb' matches render's triple kernel; single-channel
    /// targets/sources refuse typed.
    #[test]
    fn triple_modes_route_and_gate_per_the_addendum() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.4);
        let t = layer_px(&e, 5, 5); // white background, straight = 1.0
        let layer = e.document().unwrap().z_order_walk()[0];
        // hue: layer target, rgb source (self-merged): B(T, S″) with the
        // whole-pixel weight; source = merged = the same white — the
        // kernel triple at w = 1.
        e.apply_json(&env(
            5,
            "channel.apply_image",
            json!({"layerId": layer.as_uuid().to_string()}),
            json!({"source": {"layer": "merged", "channel": "rgb"}, "blend": "hue"}),
        ))
        .unwrap();
        let kernel = adapt_triple(BlendMode::Hue).unwrap();
        let expect = kernel([t[0], t[1], t[2]], [1.0, 1.0, 1.0]);
        let got = layer_px(&e, 5, 5);
        for c in 0..3 {
            assert_eq!(
                got[c].to_bits(),
                // straight expected re-premultiplied at a = 1: identical.
                expect[c].to_bits(),
                "hue triple bit-identity, component {c}"
            );
        }
        // Triple on a channel target → target_kind_mismatch.
        let err = e
            .apply_json(&env(
                6,
                "channel.apply_image",
                json!({"channelId": CHAN_A}),
                json!({"source": {"layer": "merged", "channel": CHAN_A}, "blend": "hue"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        // Triple with a single-channel source → target_kind_mismatch.
        let err = e
            .apply_json(&env(
                7,
                "channel.apply_image",
                json!({"layerId": layer.as_uuid().to_string()}),
                json!({"source": {"layer": "merged", "channel": CHAN_A}, "blend": "hue"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
    }

    #[test]
    fn dissolve_reserved_and_bare_seam_answer_typed() {
        // Dissolve: typed-reserved through the color crate's ruling.
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.5);
        let err = e
            .apply_json(&env(
                5,
                "channel.apply_image",
                json!({"channelId": CHAN_A}),
                json!({"source": {"layer": "merged", "channel": CHAN_A}, "blend": "dissolve"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // A kernel-less assembly (no with_blend_kernels): §4.5 modes
        // answer the typed seam mirror; legacy add still runs.
        let mut bare = crate::engine::kinds::register(EngineAssembly::new()).build();
        bare.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "a1b10000-0000-4000-8000-000000000002",
                "kind": "doc.new",
                "params": {"width": 16, "height": 16, "depth": "f32"},
            })
            .to_string(),
        )
        .unwrap();
        bare.apply_json(&env(
            1,
            "channel.add",
            Value::Null,
            json!({"newChannelId": CHAN_A}),
        ))
        .unwrap();
        let err = bare
            .apply_json(&env(
                2,
                "channel.apply_image",
                json!({"channelId": CHAN_A}),
                json!({"source": {"layer": "merged", "channel": CHAN_A}, "blend": "multiply"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented, "typed seam mirror");
        bare.apply_json(&env(
            3,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_A}, "blend": "add"}),
        ))
        .unwrap();
    }

    #[test]
    fn apply_image_adversarial_rejects() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.5);
        let layer = e.document().unwrap().z_order_walk()[0];
        let lt = json!({"layerId": layer.as_uuid().to_string()});
        // Missing source → stage-3 required.
        let err = e
            .apply_json(&env(
                5,
                "channel.apply_image",
                lt.clone(),
                json!({"blend": "multiply"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        // scale 3 → stage-3 bounds.
        let err = e
            .apply_json(&env(
                6,
                "channel.apply_image",
                lt.clone(),
                json!({"source": {"layer": "merged"}, "scale": 3}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // channel 'c' against an RGB document → target_kind_mismatch.
        let err = e
            .apply_json(&env(
                7,
                "channel.apply_image",
                lt.clone(),
                json!({"source": {"layer": "merged", "channel": "c"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        // Foreign docId → bad_target (single-document posture).
        let err = e
            .apply_json(&env(
                8,
                "channel.apply_image",
                lt.clone(),
                json!({"source": {"docId": "abababab-0000-4000-8000-000000000001",
                                   "layer": "merged"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // rgb source against a channel target → target_kind_mismatch.
        let err = e
            .apply_json(&env(
                9,
                "channel.apply_image",
                json!({"channelId": CHAN_A}),
                json!({"source": {"layer": "merged", "channel": "rgb"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        // No target at all → bad_target.
        let err = e
            .apply_json(&env(
                10,
                "channel.apply_image",
                Value::Null,
                json!({"source": {"layer": "merged"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // PIXELS lock on the layer target → locked.
        {
            let doc = e.doc_mut_for_test();
            doc.layer_mut(layer).unwrap().locks = crate::graph::node::LockFlags::PIXELS;
        }
        let err = e
            .apply_json(&env(
                11,
                "channel.apply_image",
                lt,
                json!({"source": {"layer": "merged"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    #[test]
    fn mask_arm_and_selection_gate_coverage() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.5);
        add_channel_filled(&mut e, 3, CHAN_B, 1.0);
        // Mask = channel B inverted (1 → 0): m(p) = 0 everywhere ⇒ the
        // apply is the identity (lerp w = 0 endpoint exact).
        let before = chan_at(&e, CHAN_A, 5, 5);
        e.apply_json(&env(
            5,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_B},
                   "blend": "multiply",
                   "mask": {"source": {"layer": "merged", "channel": CHAN_B},
                            "invert": true}}),
        ))
        .unwrap();
        assert_eq!(
            chan_at(&e, CHAN_A, 5, 5),
            before,
            "m(p) = 0 is exact identity"
        );
        // mask.source with 'rgb' → target_kind_mismatch (scalar m(p)).
        let err = e
            .apply_json(&env(
                6,
                "channel.apply_image",
                json!({"channelId": CHAN_A}),
                json!({"source": {"layer": "merged", "channel": CHAN_B},
                       "mask": {"source": {"layer": "merged", "channel": "rgb"}}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
    }

    /// Review DEFECT-1 regression, arm (1a): an ALL-ZERO blended result
    /// must OVERWRITE prior nonzero coverage on the in-place channel
    /// target (§14.4 [NORMATIVE]) — the skip-empty install left 0.5.
    #[test]
    fn all_zero_result_overwrites_prior_channel_coverage() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.5);
        add_channel_filled(&mut e, 3, CHAN_B, 1.0);
        let before_hash = e.state_hash();
        // Legacy subtract: R = clamp(0.5 − 1.0) = 0 everywhere.
        e.apply_json(&env(
            5,
            "channel.apply_image",
            json!({"channelId": CHAN_A}),
            json!({"source": {"layer": "merged", "channel": CHAN_B}, "blend": "subtract"}),
        ))
        .unwrap();
        assert_eq!(
            chan_at(&e, CHAN_A, 5, 5),
            0.0,
            "in-place zero result sticks"
        );
        assert_eq!(chan_at(&e, CHAN_A, 31, 31), 0.0);
        assert_eq!(
            e.document()
                .unwrap()
                .channel(ChannelId::from_uuid(Uuid::parse_str(CHAN_A).unwrap()))
                .unwrap()
                .buffer()
                .bounds(),
            crate::geom::IRect::EMPTY,
            "bounds tighten to empty"
        );
        // Undo restores the 0.5 coverage byte-exactly; replay agrees.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before_hash);
        assert_eq!(chan_at(&e, CHAN_A, 5, 5), 0.5);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// Review DEFECT-1 regression, arm (1b): calculations
    /// result:"selection" with an all-zero result REPLACES prior
    /// selection coverage — mask zero, active=false, bounds empty, and
    /// the state hash agrees with replay (the inconsistent
    /// mask-1.0/active-false state is dead).
    #[test]
    fn all_zero_calculations_result_replaces_the_selection() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 1.0);
        add_channel_filled(&mut e, 3, CHAN_B, 0.5);
        // Establish a NONZERO selection first (result:'selection').
        e.apply_json(&env(
            5,
            "channel.calculations",
            Value::Null,
            json!({"source1": {"layer": "merged", "channel": CHAN_B},
                   "source2": {"layer": "merged", "channel": CHAN_B},
                   "blend": "add", "result": "selection"}),
        ))
        .unwrap();
        assert!(e.document().unwrap().selection().active());
        // Now an all-zero result: subtract, T = source2 (0.5), S′ =
        // source1 (1.0) → clamp(0.5 − 1.0) = 0 everywhere.
        e.apply_json(&env(
            6,
            "channel.calculations",
            Value::Null,
            json!({"source1": {"layer": "merged", "channel": CHAN_A},
                   "source2": {"layer": "merged", "channel": CHAN_B},
                   "blend": "subtract", "result": "selection"}),
        ))
        .unwrap();
        let doc = e.document().unwrap();
        assert_eq!(
            doc.selection().mask().read_pixel_f32(IVec2::new(5, 5))[0],
            0.0,
            "replace overwrote the prior coverage"
        );
        assert!(
            !doc.selection().active(),
            "empty result deactivates (§13.1)"
        );
        assert_eq!(doc.selection().bounds(), crate::geom::IRect::EMPTY);
        // Consistency: undo/redo byte identity + replay equality (the
        // reviewer's state-hash divergence is the regression target).
        let after = e.state_hash();
        e.undo().unwrap();
        assert!(e.document().unwrap().selection().active());
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn calculations_results_and_rejects() {
        let mut e = ready();
        add_channel_filled(&mut e, 1, CHAN_A, 0.6);
        add_channel_filled(&mut e, 3, CHAN_B, 0.5);
        // new_channel: T = source2 (B = 0.5), S = source1 (A = 0.6);
        // multiply → 0.3.
        let new_c = "feedfeed-0000-4000-8000-00000000000c";
        let receipt = e
            .apply_json(&env(
                5,
                "channel.calculations",
                Value::Null,
                json!({"source1": {"layer": "merged", "channel": CHAN_A},
                       "source2": {"layer": "merged", "channel": CHAN_B},
                       "blend": "multiply", "newChannelId": new_c}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids.len(), 1);
        assert!(
            (chan_at(&e, new_c, 5, 5) - 0.3).abs() < 1e-6,
            "T=s2, S=s1, multiply"
        );
        assert_eq!(e.document().unwrap().channel_count(), 3);
        // result:'selection' writes the §13.1 surface.
        e.apply_json(&env(
            6,
            "channel.calculations",
            Value::Null,
            json!({"source1": {"layer": "merged", "channel": CHAN_A},
                   "source2": {"layer": "merged", "channel": CHAN_B},
                   "blend": "add", "result": "selection"}),
        ))
        .unwrap();
        let sel = e.document().unwrap().selection();
        assert!(sel.active());
        assert!((sel.mask().read_pixel_f32(IVec2::new(5, 5))[0] - 1.0).abs() < 1e-6);
        // new_channel without newChannelId → the if/then cross-field.
        let err = e
            .apply_json(&env(
                7,
                "channel.calculations",
                Value::Null,
                json!({"source1": {"layer": "merged", "channel": CHAN_A},
                       "source2": {"layer": "merged", "channel": CHAN_B}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        // rgb source → single-channel rule.
        let err = e
            .apply_json(&env(
                8,
                "channel.calculations",
                Value::Null,
                json!({"source1": {"layer": "merged", "channel": "rgb"},
                       "source2": {"layer": "merged", "channel": CHAN_B},
                       "newChannelId": "feedfeed-0000-4000-8000-00000000000d"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        // new_document → typed NI (session-host posture).
        let err = e
            .apply_json(&env(
                9,
                "channel.calculations",
                Value::Null,
                json!({"source1": {"layer": "merged", "channel": CHAN_A},
                       "source2": {"layer": "merged", "channel": CHAN_B},
                       "result": "new_document"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }
}
