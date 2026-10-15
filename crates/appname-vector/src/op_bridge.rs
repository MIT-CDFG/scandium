// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! TEXTVEC Door 1 implementation: [`appname_core::engine::vector_ops::VectorGeometry`]
//! over this crate's landed, reviewed geometry (wire/path/primitives/
//! boolean/raster/fit/anchor).
//!
//! Spec: ES §20.1–§20.4, §40.7 (vector kind rows); DD ㉕. Pattern: the
//! established `op_bridge` shape — `register(EngineAssembly) ->
//! EngineAssembly`, construction-only, crates→core, core geometry-blind
//! behind the version-stamped `appname.vector.v1` seam (the assembly
//! asserts the stamp at registration).
//!
//! Param-ingestion posture (§40.7 "as vector.path_new payload rules"):
//! the inline `vectorPath` param is parsed STRICTLY — the §20.1 live
//! invariants (bitwise contiguity, closed-loop weld, no degenerate
//! segments) apply verbatim, no lenient re-weld — while SIDECAR bytes
//! from storage go through the lenient §7.5 reader (re-weld ≤ 1e−6,
//! degenerate removal) exactly as the wire module defines.
//!
//! Pinned elections (fold-in register; recorded in the lane log):
//! * `primitive.geometry` is the `{x, y, w, h}` rect hull of every
//!   primitive (the contract enumerates no per-kind fields; the script
//!   example shows this shape for ellipse): rect/rounded_rect fill it,
//!   ellipse inscribes it, polygon inscribes its circumcircle in it
//!   (circumradius = min(w,h)/2, center = rect center; optional open
//!   `geometry.inset` ∈ [0,1) engages the §20.4 star arm and
//!   `geometry.rotation` radians rotates the apex), line runs
//!   corner-to-corner (x,y)→(x+w,y+h) with optional `geometry.weight`
//!   (px, default 1).
//! * `vector.edit` close appends a straight closing segment when the
//!   endpoints differ, then sets `closed`; open clears the flag and
//!   REMOVES NO geometry (the loop's return segment stays as a normal
//!   segment — deterministic, reversible by close).
//! * `set_handles` applies coordinates verbatim; the optional `kind`
//!   field is accepted but classification DERIVES from geometry
//!   (§20.1: anchors derive from stored segments).
//! * `add_anchor` with kind smooth/symmetric synthesizes collinear
//!   handles at ⅓ of neighbor distances (the §20.4 convert-point
//!   rule); corner (default) inserts with retracted handles.
//! * `custom` primitives answer typed `error.command.not_implemented`:
//!   shape ASSETS resolve core-side (§28.1) — the asset-payload door is
//!   a follow-up filing, not a geometry call.

use std::sync::Arc;

use serde_json::{Map, Value};

use appname_core::engine::error::{EngineError, ErrorCode};
use appname_core::engine::kind_impl::EngineAssembly;
use appname_core::engine::vector_ops::{
    CoveragePlaneData, GeometryResult, ShapeStrokeStyle, StrokePosition, VectorBoolOp,
    VectorGeometry, VECTOR_SIDECAR_SCHEMA,
};
use appname_core::geom::IRect;
use appname_core::PathId;

use crate::anchor::{anchors, Anchor2};
use crate::boolean::{path_boolean as geo_boolean, BoolOp};
use crate::fit::fit_single;
use crate::flatten::TOL_BOOLEAN;
use crate::geom::Vec2;
use crate::path::{CubicSeg, FillRule, PathError, SubPath, VectorPath};
use crate::raster::rasterize_path;
use crate::wire::{path_from_sidecar_json, path_to_sidecar_json};

/// The Door-1 service (zero-sized; all state is per-call).
pub struct VectorGeometryService;

/// Register this crate's geometry service on an engine assembly
/// (construction-only; the assembly points wrap their existing chain).
pub fn register(assembly: EngineAssembly) -> EngineAssembly {
    assembly.with_vector_geometry(Arc::new(VectorGeometryService))
}

fn oob(msg: impl Into<String>) -> EngineError {
    EngineError::new(ErrorCode::OutOfRange, msg)
}

fn path_err(e: PathError) -> EngineError {
    match e {
        PathError::SchemaMismatch { got } => EngineError::new(
            ErrorCode::WrongSchema,
            format!("vector sidecar: wrong schema stamp {got:?} (want {VECTOR_SIDECAR_SCHEMA})"),
        ),
        other => oob(format!("vector path invalid: {other:?}")),
    }
}

/// Integer hull of the path's tight bounds (§40.7 dirty rects).
fn hull(path: &VectorPath) -> IRect {
    match crate::bounds::tight_bounds(path) {
        Some(r) => IRect::new(
            r.min.x.floor() as i32,
            r.min.y.floor() as i32,
            r.max.x.ceil() as i32,
            r.max.y.ceil() as i32,
        ),
        None => IRect::EMPTY,
    }
}

fn emit(path: &VectorPath) -> Result<GeometryResult, EngineError> {
    let sidecar = path_to_sidecar_json(path).map_err(path_err)?;
    Ok(GeometryResult {
        sidecar,
        bounds: hull(path),
    })
}

/// Lenient sidecar parse (storage-side §7.5 reader).
fn parse(bytes: &[u8]) -> Result<VectorPath, EngineError> {
    let (path, _notes) = path_from_sidecar_json(PathId::nil(), bytes).map_err(path_err)?;
    Ok(path)
}

/// ENGINE CEILING on one §20.2 stroke band's coverage plane, in cells.
///
/// This is NOT a spec range — `vector.shape_add` states outright that
/// `widthPx` has "no range pinned for VectorStroke (§20.2 width:f32)", so
/// an arbitrarily large width is a CONTRACT-VALID document. The band's
/// plane is the path's bounds INFLATED BY THE WIDTH, so — unlike the fill
/// leg, whose plane is bounded by the path the user actually drew — a
/// single wire number scales this allocation without limit:
/// `widthPx: 1e9` on a 64 × 48 canvas asks for a ~2e9 × 2e9 plane. Left
/// unguarded that is not a slow render, it is an allocation abort (and,
/// past `i32::MAX`, a saturating cast whose `w * h` wraps to a SILENTLY
/// WRONG plane). QA-F8's posture is refuse typed, never crash.
///
/// 1 << 26 = 67,108,864 cells — an 8192 × 8192 band. The rasterizer holds
/// ~20 bytes per cell (two f64 accumulators plus the f32 plane), so the
/// ceiling caps one band near 1.3 GB: deliberately NO STRICTER than the
/// fill leg already accepts for an 8192 × 8192 path today, so nothing that
/// composites now can start refusing. FOLD-IN CANDIDATE: the spec pinning
/// a `widthPx` range would make this ceiling a stage-5 schema bound
/// instead of an engine one.
const MAX_STROKE_BAND_CELLS: f64 = (1u64 << 26) as f64;

/// Refuse a stroke whose band plane would exceed [`MAX_STROKE_BAND_CELLS`]
/// BEFORE any geometry runs. Control bounds (⊇ tight bounds) keep the
/// estimate conservative in the safe direction, and the whole computation
/// stays in f64 so no cast can overflow on the way to the check.
fn band_plane_budget(path: &VectorPath, style: ShapeStrokeStyle) -> Result<(), EngineError> {
    let Some(b) = crate::bounds::control_bounds(path) else {
        return Ok(()); // empty path — no band at all
    };
    // `shape_stroke_coverage`'s own half-width election, mirrored: the
    // Inside/Outside arms build the DOUBLE band before gating it.
    let h = match style.align {
        StrokePosition::Center => style.width / 2.0,
        StrokePosition::Inside | StrokePosition::Outside => style.width,
    };
    let cells = (b.max.x - b.min.x + 2.0 * h + 2.0) * (b.max.y - b.min.y + 2.0 * h + 2.0);
    if !cells.is_finite() || cells > MAX_STROKE_BAND_CELLS {
        return Err(EngineError::with_field(
            ErrorCode::OutOfRange,
            "/stroke/widthPx",
            format!(
                "shape stroke widthPx {} over these path bounds asks for a ~{:.3e}-cell \
                 coverage plane, past this engine's {:.3e}-cell band ceiling — refused \
                 typed rather than aborting the compositor (the §20.2 wire pins no \
                 width range)",
                style.width, cells, MAX_STROKE_BAND_CELLS
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Strict param-object parsing (§40.7 payload rules)
// ---------------------------------------------------------------------

fn vec2_of(v: &Value, ptr: &str) -> Result<Vec2, EngineError> {
    let arr = v
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or_else(|| oob(format!("{ptr}: expected [x, y]")))?;
    let (x, y) = (arr[0].as_f64(), arr[1].as_f64());
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Vec2::new(x, y)),
        _ => Err(oob(format!("{ptr}: coordinates must be finite numbers"))),
    }
}

fn strict_path_from_param(path_obj: &Map<String, Value>) -> Result<VectorPath, EngineError> {
    let fill_rule = match path_obj.get("fillRule").and_then(Value::as_str) {
        Some("nonzero") => FillRule::NonZero,
        Some("evenodd") => FillRule::EvenOdd,
        Some(other) => return Err(oob(format!("/path/fillRule: unknown value {other:?}"))),
        None => return Err(oob("/path/fillRule: required")),
    };
    let subs = path_obj
        .get("subpaths")
        .and_then(Value::as_array)
        .ok_or_else(|| oob("/path/subpaths: required array"))?;
    if subs.is_empty() {
        return Err(oob("/path/subpaths: empty path (minItems 1, §40.7)"));
    }
    let mut path = VectorPath::new(PathId::nil(), fill_rule);
    for (si, sub) in subs.iter().enumerate() {
        let closed = sub
            .get("closed")
            .and_then(Value::as_bool)
            .ok_or_else(|| oob(format!("/path/subpaths/{si}/closed: required boolean")))?;
        let segs = sub
            .get("segments")
            .and_then(Value::as_array)
            .ok_or_else(|| oob(format!("/path/subpaths/{si}/segments: required array")))?;
        let mut segments = Vec::with_capacity(segs.len());
        for (gi, seg) in segs.iter().enumerate() {
            let ptr = format!("/path/subpaths/{si}/segments/{gi}");
            let f = |k: &str| -> Result<Vec2, EngineError> {
                vec2_of(
                    seg.get(k)
                        .ok_or_else(|| oob(format!("{ptr}/{k}: required")))?,
                    &format!("{ptr}/{k}"),
                )
            };
            segments.push(CubicSeg::new(f("p0")?, f("c1")?, f("c2")?, f("p1")?));
        }
        path.subpaths.push(SubPath { segments, closed });
    }
    // STRICT §20.1 validation — no lenient re-weld for params.
    path.validate()
        .map_err(|e| oob(format!("/path: §20.1 invariant violated: {e:?}")))?;
    Ok(path)
}

// ---------------------------------------------------------------------
// vector.edit op application (§20.4 pen-gesture rows)
// ---------------------------------------------------------------------

/// Rebuild a subpath's segments from its anchor list (§20.1 model:
/// segment i = anchors[i] → anchors[i+1], c1 = out, c2 = in).
fn segments_from_anchors(anchors: &[Anchor2], closed: bool) -> Vec<CubicSeg> {
    let n = anchors.len();
    if n < 2 {
        return Vec::new();
    }
    let pairs = if closed { n } else { n - 1 };
    (0..pairs)
        .map(|i| {
            let a = &anchors[i];
            let b = &anchors[(i + 1) % n];
            CubicSeg::new(a.point, a.out_handle, b.in_handle, b.point)
        })
        .collect()
}

/// §20.4 convert-point synthesis: collinear handles at ⅓ of neighbor
/// distances (used by add_anchor kind smooth/symmetric).
fn synthesize_smooth(anchors: &mut [Anchor2], idx: usize, closed: bool, symmetric: bool) {
    let n = anchors.len();
    let prev = if idx > 0 {
        Some(anchors[idx - 1].point)
    } else if closed && n > 1 {
        Some(anchors[n - 1].point)
    } else {
        None
    };
    let next = if idx + 1 < n {
        Some(anchors[idx + 1].point)
    } else if closed && n > 1 {
        Some(anchors[0].point)
    } else {
        None
    };
    let p = anchors[idx].point;
    let dir = match (prev, next) {
        (Some(a), Some(b)) => (b - a).normalize(),
        (Some(a), None) => (p - a).normalize(),
        (None, Some(b)) => (b - p).normalize(),
        (None, None) => None,
    };
    let Some(dir) = dir else { return };
    let din = prev.map_or(0.0, |a| p.distance(a) / 3.0);
    let dout = next.map_or(0.0, |b| p.distance(b) / 3.0);
    let (din, dout) = if symmetric {
        let d = f64::max(din, dout);
        (d, d)
    } else {
        (din, dout)
    };
    anchors[idx].in_handle = p - dir * din;
    anchors[idx].out_handle = p + dir * dout;
    anchors[idx].kind = crate::anchor::classify(
        anchors[idx].point,
        anchors[idx].in_handle,
        anchors[idx].out_handle,
    );
}

fn apply_one_op(
    path: &mut VectorPath,
    op: &Map<String, Value>,
    oi: usize,
) -> Result<(), EngineError> {
    let ptr = format!("/ops/{oi}");
    let name = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| oob(format!("{ptr}/op: required")))?;
    let sub_idx = op.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
    if sub_idx >= path.subpaths.len() {
        return Err(oob(format!(
            "{ptr}/subpath: {sub_idx} out of range ({} subpaths)",
            path.subpaths.len()
        )));
    }
    let closed = path.subpaths[sub_idx].closed;
    match name {
        "close" => {
            let sub = &mut path.subpaths[sub_idx];
            if !sub.closed {
                if let (Some(first), Some(last)) = (sub.segments.first(), sub.segments.last()) {
                    let (start, end) = (first.p0, last.p1);
                    if start != end {
                        sub.segments.push(CubicSeg::line(end, start));
                    }
                }
                sub.closed = true;
            }
            Ok(())
        }
        "open" => {
            path.subpaths[sub_idx].closed = false;
            Ok(())
        }
        "add_anchor" | "delete_anchor" | "move_anchor" | "set_handles" => {
            let mut list = anchors(&path.subpaths[sub_idx]);
            let n = list.len();
            let index =
                op.get("index")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| oob(format!("{ptr}/index: required")))? as usize;
            match name {
                "add_anchor" => {
                    if index > n {
                        return Err(oob(format!("{ptr}/index: {index} > {n} anchors")));
                    }
                    let pos = vec2_of(
                        op.get("position").ok_or_else(|| {
                            oob(format!("{ptr}/position: required for add_anchor"))
                        })?,
                        &format!("{ptr}/position"),
                    )?;
                    list.insert(
                        index,
                        Anchor2 {
                            point: pos,
                            in_handle: pos,
                            out_handle: pos,
                            kind: crate::anchor::AnchorKind::Corner,
                        },
                    );
                    match op.get("kind").and_then(Value::as_str) {
                        Some("smooth") => synthesize_smooth(&mut list, index, closed, false),
                        Some("symmetric") => synthesize_smooth(&mut list, index, closed, true),
                        Some("corner") | None => {} // retracted handles
                        Some(other) => {
                            // Review V-2: strict §40.7 posture — an
                            // unknown kind refuses, never degrades.
                            return Err(oob(format!("{ptr}/kind: unknown anchor kind {other:?}")));
                        }
                    }
                }
                "delete_anchor" => {
                    if index >= n {
                        return Err(oob(format!("{ptr}/index: {index} >= {n} anchors")));
                    }
                    let interior = closed || (index > 0 && index + 1 < n);
                    if interior && n >= 3 {
                        // §20.4: neighbors re-joined by ONE fitted
                        // segment (§20.3 refit, 0.1 px): flatten the two
                        // segments around the anchor, fit with neighbor
                        // tangents, splice.
                        let prev = (index + n - 1) % n;
                        let segs = segments_from_anchors(&list, closed);
                        let seg_in = &segs[if closed { prev } else { index - 1 }];
                        let seg_out = &segs[index % segs.len()];
                        // Review V-1 (+ behavioral follow-up): the
                        // rejoin polyline samples both neighbor
                        // segments UNIFORMLY (17 samples each) rather
                        // than adaptively — straight segments flatten
                        // to bare endpoints, which starves the fit's
                        // sampled-error check and let a single cubic
                        // bow 0.16 px between samples. Uniform density
                        // makes the §20.3 0.1 px contract enforceable
                        // at the fit's own error metric. Deterministic.
                        let mut pts = Vec::with_capacity(33);
                        for i in 0..=16 {
                            pts.push(seg_in.eval(f64::from(i) / 16.0));
                        }
                        for i in 1..=16 {
                            pts.push(seg_out.eval(f64::from(i) / 16.0));
                        }
                        let t0 = (seg_in.c1 - seg_in.p0)
                            .normalize()
                            .or_else(|| (seg_in.p1 - seg_in.p0).normalize())
                            .unwrap_or(Vec2::new(1.0, 0.0));
                        let t1 = (seg_out.c2 - seg_out.p1)
                            .normalize()
                            .or_else(|| (seg_out.p0 - seg_out.p1).normalize())
                            .unwrap_or(Vec2::new(-1.0, 0.0));
                        // Pinned reading of §20.4 "a SINGLE fitted
                        // segment" (V-1 behavioral round): exactly one
                        // least-squares cubic over the neighbors'
                        // sampled outline with neighbor end tangents —
                        // §20.3's fitting MACHINERY, never split.
                        // Across a deleted CORNER the outline
                        // legitimately deviates (removing the corner is
                        // the user's intent); 0.1 px is the machinery's
                        // target where achievable, not a preservation
                        // guarantee across the deletion.
                        let fitted = fit_single(&pts, t0, t1);
                        list.remove(index);
                        let mut rebuilt = segments_from_anchors(&list, closed);
                        let at = if closed {
                            prev.min(rebuilt.len().saturating_sub(1))
                        } else {
                            index - 1
                        };
                        if let (Some(seg), true) = (fitted, at < rebuilt.len()) {
                            rebuilt[at] = seg;
                        }
                        path.subpaths[sub_idx].segments = rebuilt;
                        return finish_subpath(path, sub_idx, &ptr);
                    } else {
                        // End anchor of an open subpath: drop the end
                        // segment (§20.4 Backspace semantics).
                        list.remove(index);
                        if list.len() < 2 {
                            if path.subpaths.len() == 1 {
                                return Err(oob(format!(
                                    "{ptr}: delete_anchor would empty the path (§40.7 minItems 1)"
                                )));
                            }
                            path.subpaths.remove(sub_idx);
                            return Ok(());
                        }
                    }
                }
                "move_anchor" => {
                    if index >= n {
                        return Err(oob(format!("{ptr}/index: {index} >= {n} anchors")));
                    }
                    let pos = vec2_of(
                        op.get("position").ok_or_else(|| {
                            oob(format!("{ptr}/position: required for move_anchor"))
                        })?,
                        &format!("{ptr}/position"),
                    )?;
                    list[index] = list[index].drag_point(pos);
                }
                "set_handles" => {
                    if index >= n {
                        return Err(oob(format!("{ptr}/index: {index} >= {n} anchors")));
                    }
                    let ih = op.get("inHandle");
                    let oh = op.get("outHandle");
                    if ih.is_none() && oh.is_none() {
                        return Err(oob(format!(
                            "{ptr}: set_handles requires inHandle and/or outHandle (§40.7)"
                        )));
                    }
                    if let Some(v) = ih {
                        list[index].in_handle = vec2_of(v, &format!("{ptr}/inHandle"))?;
                    }
                    if let Some(v) = oh {
                        list[index].out_handle = vec2_of(v, &format!("{ptr}/outHandle"))?;
                    }
                    // `kind` accepted; classification derives (§20.1):
                    list[index].kind = crate::anchor::classify(
                        list[index].point,
                        list[index].in_handle,
                        list[index].out_handle,
                    );
                }
                _ => unreachable!(),
            }
            path.subpaths[sub_idx].segments = segments_from_anchors(&list, closed);
            finish_subpath(path, sub_idx, &ptr)
        }
        other => Err(oob(format!("{ptr}/op: unknown op {other:?}"))),
    }
}

/// Post-op subpath hygiene: an open subpath needs ≥ 1 segment; a closed
/// one ≥ 2 anchors. Anything less is an op error (atomicity: the caller
/// discards the working copy).
fn finish_subpath(path: &mut VectorPath, sub_idx: usize, ptr: &str) -> Result<(), EngineError> {
    if path.subpaths[sub_idx].segments.is_empty() {
        return Err(oob(format!(
            "{ptr}: op left subpath {sub_idx} without segments"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// The trait impl
// ---------------------------------------------------------------------

fn num(geom: &Map<String, Value>, k: &str, ptr: &str) -> Result<f64, EngineError> {
    geom.get(k)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| oob(format!("{ptr}/{k}: required finite number")))
}

impl VectorGeometry for VectorGeometryService {
    fn schema(&self) -> &'static str {
        VECTOR_SIDECAR_SCHEMA
    }

    fn validate_sidecar(&self, bytes: &[u8]) -> Result<GeometryResult, EngineError> {
        let path = parse(bytes)?;
        emit(&path)
    }

    fn path_param_sidecar(&self, path: &Map<String, Value>) -> Result<GeometryResult, EngineError> {
        let p = strict_path_from_param(path)?;
        emit(&p)
    }

    fn primitive_sidecar(
        &self,
        kind: &str,
        primitive: &Map<String, Value>,
    ) -> Result<GeometryResult, EngineError> {
        let geom = primitive
            .get("geometry")
            .and_then(Value::as_object)
            .ok_or_else(|| oob("/primitive/geometry: required object"))?;
        let ptr = "/primitive/geometry";
        let sub = match kind {
            "rect" | "rounded_rect" => {
                let (x, y, w, h) = (
                    num(geom, "x", ptr)?,
                    num(geom, "y", ptr)?,
                    num(geom, "w", ptr)?,
                    num(geom, "h", ptr)?,
                );
                let r = if kind == "rounded_rect" {
                    primitive
                        .get("cornerRadius")
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .ok_or_else(|| oob("/primitive/cornerRadius: required ≥ 0"))?
                } else {
                    0.0
                };
                crate::primitives::rectangle(x, y, w, h, [r; 4])
            }
            "ellipse" => {
                let (x, y, w, h) = (
                    num(geom, "x", ptr)?,
                    num(geom, "y", ptr)?,
                    num(geom, "w", ptr)?,
                    num(geom, "h", ptr)?,
                );
                crate::primitives::ellipse(x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0)
            }
            "polygon" => {
                let (x, y, w, h) = (
                    num(geom, "x", ptr)?,
                    num(geom, "y", ptr)?,
                    num(geom, "w", ptr)?,
                    num(geom, "h", ptr)?,
                );
                let sides_raw = primitive
                    .get("sides")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| oob("/primitive/sides: required for polygon (§40.7)"))?;
                // Review V-3: the door refuses out-of-range values —
                // the geometry constructors' clamps are defense in
                // depth, not the contract surface (§20.4: sides 3–100).
                if !(3..=100).contains(&sides_raw) {
                    return Err(oob(format!(
                        "/primitive/sides: {sides_raw} outside 3–100 (§20.4)"
                    )));
                }
                let sides = sides_raw as u32;
                let r = f64::min(w, h) / 2.0;
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                let rotation = geom.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);
                match geom.get("inset").and_then(Value::as_f64) {
                    Some(inset) => {
                        if !inset.is_finite() || !(0.0..=0.99).contains(&inset) {
                            return Err(oob(format!(
                                "{ptr}/inset: {inset} outside 0–99 % (§20.4)"
                            )));
                        }
                        crate::primitives::star(cx, cy, r, sides, inset, rotation)
                    }
                    None => crate::primitives::polygon(cx, cy, r, sides, rotation),
                }
            }
            "line" => {
                let (x, y, w, h) = (
                    num(geom, "x", ptr)?,
                    num(geom, "y", ptr)?,
                    num(geom, "w", ptr)?,
                    num(geom, "h", ptr)?,
                );
                let weight = geom.get("weight").and_then(Value::as_f64).unwrap_or(1.0);
                let subs = crate::primitives::line(
                    Vec2::new(x, y),
                    Vec2::new(x + w, y + h),
                    weight,
                    None,
                    None,
                );
                let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
                path.subpaths = subs;
                if path.subpaths.is_empty() {
                    return Err(oob(format!("{ptr}: degenerate line refused")));
                }
                path.normalize().map_err(path_err)?;
                return emit(&path);
            }
            "custom" => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "vector.shape_add: custom shape assets resolve core-side (§28.1); \
                     the asset-payload door is a follow-up filing",
                ));
            }
            other => return Err(oob(format!("/primitive/kind: unknown kind {other:?}"))),
        };
        let Some(sub) = sub else {
            return Err(oob(format!(
                "{ptr}: degenerate primitive dimensions refused"
            )));
        };
        let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
        path.subpaths.push(sub);
        path.normalize().map_err(path_err)?;
        emit(&path)
    }

    fn path_boolean(
        &self,
        a: &[u8],
        b: &[u8],
        op: VectorBoolOp,
    ) -> Result<GeometryResult, EngineError> {
        let pa = parse(a)?;
        let pb = parse(b)?;
        let bop = match op {
            VectorBoolOp::Union => BoolOp::Union,
            VectorBoolOp::Subtract => BoolOp::Subtract,
            VectorBoolOp::Intersect => BoolOp::Intersect,
            VectorBoolOp::Exclude => BoolOp::Exclude,
        };
        let out = geo_boolean(&pa, &pb, bop, PathId::nil())
            .map_err(|e| oob(format!("vector boolean failed: {e}")))?;
        emit(&out)
    }

    fn rasterize_fill(&self, sidecar: &[u8]) -> Result<CoveragePlaneData, EngineError> {
        let path = parse(sidecar)?;
        let plane = rasterize_path(&path, TOL_BOOLEAN);
        Ok(CoveragePlaneData {
            rect: plane.rect,
            values: plane.v,
        })
    }

    fn stroke_path_coverage(
        &self,
        sidecar: &[u8],
        width_px: f64,
        position: StrokePosition,
    ) -> Result<CoveragePlaneData, EngineError> {
        let path = parse(sidecar)?;
        let plane = crate::stroke::stroke_coverage(&path, width_px, position)
            .map_err(|e| oob(format!("stroke geometry: {e:?}")))?;
        Ok(CoveragePlaneData {
            rect: plane.rect,
            values: plane.v,
        })
    }

    fn stroke_boundary_coverage(
        &self,
        plane: &CoveragePlaneData,
        width_px: f64,
        position: StrokePosition,
    ) -> Result<CoveragePlaneData, EngineError> {
        let cov = crate::raster::CoveragePlane {
            rect: plane.rect,
            v: plane.values.clone(),
        };
        let out = crate::stroke::stroke_boundary_coverage(&cov, width_px, position, PathId::nil())
            .map_err(|e| oob(format!("boundary stroke geometry: {e:?}")))?;
        Ok(CoveragePlaneData {
            rect: out.rect,
            values: out.v,
        })
    }

    fn shape_stroke_coverage(
        &self,
        sidecar: &[u8],
        style: ShapeStrokeStyle,
    ) -> Result<CoveragePlaneData, EngineError> {
        let path = parse(sidecar)?;
        band_plane_budget(&path, style)?;
        let plane = crate::stroke::shape_stroke_coverage(&path, style);
        Ok(CoveragePlaneData {
            rect: plane.rect,
            values: plane.v,
        })
    }

    fn apply_edit_ops(&self, sidecar: &[u8], ops: &[Value]) -> Result<GeometryResult, EngineError> {
        // Atomic: mutate a working copy; ANY failing op discards it.
        let mut path = parse(sidecar)?;
        for (oi, op) in ops.iter().enumerate() {
            let obj = op
                .as_object()
                .ok_or_else(|| oob(format!("/ops/{oi}: expected object")))?;
            apply_one_op(&mut path, obj, oi)?;
        }
        path.normalize().map_err(path_err)?;
        emit(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn svc() -> VectorGeometryService {
        VectorGeometryService
    }

    fn square_sidecar(x0: f64, y0: f64, sz: f64) -> Vec<u8> {
        let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
        let pts = [
            Vec2::new(x0, y0),
            Vec2::new(x0 + sz, y0),
            Vec2::new(x0 + sz, y0 + sz),
            Vec2::new(x0, y0 + sz),
        ];
        p.subpaths.push(SubPath {
            closed: true,
            segments: (0..4)
                .map(|i| CubicSeg::line(pts[i], pts[(i + 1) % 4]))
                .collect(),
        });
        path_to_sidecar_json(&p).unwrap()
    }

    fn as_map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn registration_smoke_on_the_production_core_assembly() {
        // The same wrap the assembly points apply; the double-register
        // and schema asserts fire here if the contract drifts.
        let _assembly = register(appname_core::engine::kinds::register(
            EngineAssembly::default(),
        ));
    }

    #[test]
    fn validate_sidecar_round_trips_and_reports_the_integer_hull() {
        let g = svc()
            .validate_sidecar(&square_sidecar(1.25, 2.5, 10.0))
            .unwrap();
        assert_eq!(g.bounds, IRect::new(1, 2, 12, 13), "floor/ceil hull");
        // Normalized re-emission parses back identically.
        let again = svc().validate_sidecar(&g.sidecar).unwrap();
        assert_eq!(g.sidecar, again.sidecar, "normalization is idempotent");
    }

    #[test]
    fn wrong_schema_stamp_is_the_typed_wrong_schema_error() {
        let bytes = br#"{"schema":"appname.vector.v999","fillRule":"nonzero","subpaths":[]}"#;
        let err = svc().validate_sidecar(bytes).unwrap_err();
        assert_eq!(err.code, ErrorCode::WrongSchema);
    }

    #[test]
    fn param_ingestion_is_strict() {
        // Valid square param object passes.
        let ok = as_map(json!({
            "fillRule": "nonzero",
            "subpaths": [{"closed": true, "segments": [
                {"p0": [0,0], "c1": [1,0], "c2": [2,0], "p1": [3,0]},
                {"p0": [3,0], "c1": [3,1], "c2": [3,2], "p1": [3,3]},
                {"p0": [3,3], "c1": [2,3], "c2": [1,3], "p1": [0,3]},
                {"p0": [0,3], "c1": [0,2], "c2": [0,1], "p1": [0,0]}
            ]}]
        }));
        assert!(svc().path_param_sidecar(&ok).is_ok());
        // Empty path refused (minItems 1).
        let empty = as_map(json!({"fillRule": "nonzero", "subpaths": []}));
        assert_eq!(
            svc().path_param_sidecar(&empty).unwrap_err().code,
            ErrorCode::OutOfRange
        );
        // Discontiguous segments: STRICT (§20.1 bitwise) — a 1e-7 gap a
        // lenient reader would weld is refused at the param door.
        let gap = as_map(json!({
            "fillRule": "nonzero",
            "subpaths": [{"closed": false, "segments": [
                {"p0": [0,0], "c1": [1,0], "c2": [2,0], "p1": [3,0]},
                {"p0": [3.0000001,0], "c1": [4,0], "c2": [5,0], "p1": [6,0]}
            ]}]
        }));
        assert_eq!(
            svc().path_param_sidecar(&gap).unwrap_err().code,
            ErrorCode::OutOfRange
        );
        // Unknown fillRule refused (params are contract-strict).
        let bad = as_map(json!({"fillRule": "winding", "subpaths": [
            {"closed": false, "segments": [{"p0": [0,0], "c1": [1,0], "c2": [2,0], "p1": [3,0]}]}
        ]}));
        assert_eq!(
            svc().path_param_sidecar(&bad).unwrap_err().code,
            ErrorCode::OutOfRange
        );
    }

    #[test]
    fn primitives_map_the_geometry_rect_per_the_pinned_elections() {
        let prim = |v: Value| as_map(v);
        // Ellipse inscribes the rect.
        let e = svc()
            .primitive_sidecar(
                "ellipse",
                &prim(json!({"geometry": {"x": 0, "y": 0, "w": 10, "h": 6}})),
            )
            .unwrap();
        assert_eq!(e.bounds, IRect::new(0, 0, 10, 6));
        // Polygon requires sides; star engages via geometry.inset.
        let p = svc()
            .primitive_sidecar(
                "polygon",
                &prim(json!({"geometry": {"x": 0, "y": 0, "w": 10, "h": 10}, "sides": 6})),
            )
            .unwrap();
        assert!(p.bounds.min.x >= 0 && p.bounds.max.x <= 10);
        let s = svc()
            .primitive_sidecar(
                "polygon",
                &prim(json!({"geometry": {"x": 0, "y": 0, "w": 10, "h": 10, "inset": 0.5}, "sides": 5})),
            )
            .unwrap();
        let star_path = parse(&s.sidecar).unwrap();
        assert_eq!(star_path.subpaths[0].segments.len(), 10, "2n star vertices");
        // rounded_rect requires cornerRadius.
        assert_eq!(
            svc()
                .primitive_sidecar(
                    "rounded_rect",
                    &prim(json!({"geometry": {"x":0,"y":0,"w":8,"h":8}}))
                )
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );
        // Degenerate dims refused; custom is typed-NI.
        assert_eq!(
            svc()
                .primitive_sidecar(
                    "rect",
                    &prim(json!({"geometry": {"x":0,"y":0,"w":0,"h":5}}))
                )
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );
        assert_eq!(
            svc()
                .primitive_sidecar(
                    "custom",
                    &prim(json!({"geometry": {"x":0,"y":0,"w":5,"h":5}}))
                )
                .unwrap_err()
                .code,
            ErrorCode::NotImplemented
        );
    }

    #[test]
    fn boolean_and_raster_ride_the_reviewed_engines() {
        let a = square_sidecar(0.0, 0.0, 10.0);
        let b = square_sidecar(5.0, 5.0, 10.0);
        let u = svc().path_boolean(&a, &b, VectorBoolOp::Union).unwrap();
        assert_eq!(u.bounds, IRect::new(0, 0, 15, 15));
        // A − A = empty result, empty bounds.
        let d = svc().path_boolean(&a, &a, VectorBoolOp::Subtract).unwrap();
        assert_eq!(d.bounds, IRect::EMPTY);
        // Coverage of the 10×10 square sums to its area (interior 1.0).
        let cov = svc().rasterize_fill(&a).unwrap();
        let total: f64 = cov.values.iter().map(|&v| f64::from(v)).sum();
        assert!((total - 100.0).abs() < 0.5, "sum {total}");
    }

    #[test]
    fn edit_ops_apply_atomically_with_typed_errors() {
        let s = square_sidecar(0.0, 0.0, 10.0);
        // move_anchor drags anchor 0 (translate + handles).
        let moved = svc()
            .apply_edit_ops(
                &s,
                &[json!({"op": "move_anchor", "subpath": 0, "index": 0, "position": [-5.0, -5.0]})],
            )
            .unwrap();
        assert_eq!(moved.bounds, IRect::new(-5, -5, 10, 10));
        // set_handles without coords is the contract's semantic error.
        assert_eq!(
            svc()
                .apply_edit_ops(
                    &s,
                    &[json!({"op": "set_handles", "subpath": 0, "index": 1})]
                )
                .unwrap_err()
                .code,
            ErrorCode::OutOfRange
        );
        // delete_anchor on the closed square: 4 → 3 anchors joined by
        // exactly ONE fitted segment (the pinned §20.4 reading: the
        // corner is deliberately GONE — the fit is the least-squares
        // single cubic over the old outline, not a preservation of
        // it).
        let del = svc()
            .apply_edit_ops(
                &s,
                &[json!({"op": "delete_anchor", "subpath": 0, "index": 1})],
            )
            .unwrap();
        let p = parse(&del.sidecar).unwrap();
        let sub = &p.subpaths[0];
        assert!(sub.closed);
        assert_eq!(
            sub.segments.len(),
            3,
            "ONE fitted segment joins the neighbors"
        );
        let alist = crate::anchor::anchors(sub);
        // The deleted anchor (10,0) is GONE — no anchor there.
        assert!(alist.iter().all(|a| a.point != Vec2::new(10.0, 0.0)));
        assert!(alist.iter().any(|a| a.point == Vec2::new(0.0, 0.0)));
        assert!(alist.iter().any(|a| a.point == Vec2::new(10.0, 10.0)));
        // Deterministic.
        let del2 = svc()
            .apply_edit_ops(
                &s,
                &[json!({"op": "delete_anchor", "subpath": 0, "index": 1})],
            )
            .unwrap();
        assert_eq!(del.sidecar, del2.sidecar);
        // Curved-neighbor regression (the review's V-1 repro class):
        // delete on a service-built ellipse must SUCCEED (the old
        // rejoin polyline missed its start point and the whole op was
        // refused typed on any curved subpath).
        let ell = svc()
            .primitive_sidecar(
                "ellipse",
                &as_map(json!({"geometry": {"x": 0, "y": 0, "w": 200, "h": 120}})),
            )
            .unwrap();
        let edel = svc()
            .apply_edit_ops(
                &ell.sidecar,
                &[json!({"op": "delete_anchor", "subpath": 0, "index": 1})],
            )
            .unwrap();
        let ep = parse(&edel.sidecar).unwrap();
        assert_eq!(ep.subpaths[0].segments.len(), 3, "ellipse 4 → 3 segments");
        // Out-of-range subpath/index are typed OutOfRange.
        for bad in [
            json!({"op": "move_anchor", "subpath": 7, "index": 0, "position": [0,0]}),
            json!({"op": "move_anchor", "subpath": 0, "index": 99, "position": [0,0]}),
            json!({"op": "warp_anchor", "subpath": 0, "index": 0}),
        ] {
            assert_eq!(
                svc().apply_edit_ops(&s, &[bad]).unwrap_err().code,
                ErrorCode::OutOfRange
            );
        }
        // Atomicity: a valid op followed by a failing one errors the
        // whole call (the working copy is discarded by contract).
        let two = [
            json!({"op": "move_anchor", "subpath": 0, "index": 0, "position": [1,1]}),
            json!({"op": "move_anchor", "subpath": 0, "index": 99, "position": [0,0]}),
        ];
        assert!(svc().apply_edit_ops(&s, &two).is_err());
    }

    #[test]
    fn every_emission_is_the_validators_byte_exact_fixpoint() {
        // DEFECT-1 (QA re-pin round): the rounded_rect constructor
        // sidecar was NOT validate_sidecar's fixpoint — serde_json's
        // default float parse mis-rounded a kappa-arc control by 1 ulp
        // (fixed by the float_roundtrip feature). This property makes
        // the whole class impossible: for EVERY primitive kind over a
        // param grid, plus boolean and edit-op outputs, the service's
        // emission must re-validate to BYTE-IDENTICAL sidecar bytes.
        let svc = svc();
        let fixpoint = |label: &str, g: &GeometryResult| {
            let v = svc.validate_sidecar(&g.sidecar).unwrap();
            assert_eq!(
                g.sidecar, v.sidecar,
                "{label}: emission is not the validator's fixpoint"
            );
        };
        // The exact QA repro first (96x64, r=8), then the grid.
        let prim = |kind: &str, geom: Value, extra: &[(&str, Value)]| {
            let mut m = Map::new();
            m.insert("geometry".into(), geom);
            for (k, v) in extra {
                m.insert((*k).into(), v.clone());
            }
            svc.primitive_sidecar(kind, &m).unwrap()
        };
        fixpoint(
            "QA repro rounded_rect 96x64 r8",
            &prim(
                "rounded_rect",
                json!({"x": 0, "y": 0, "w": 96, "h": 64}),
                &[("cornerRadius", json!(8.0))],
            ),
        );
        for (w, h) in [(96.0, 64.0), (877.0, 1240.0), (10.0, 6.0), (3.3, 7.7)] {
            let geom = json!({"x": 1.5, "y": 2.25, "w": w, "h": h});
            fixpoint("rect", &prim("rect", geom.clone(), &[]));
            for r in [0.5, 2.0, 8.0, 100.0] {
                fixpoint(
                    "rounded_rect",
                    &prim("rounded_rect", geom.clone(), &[("cornerRadius", json!(r))]),
                );
            }
            fixpoint("ellipse", &prim("ellipse", geom.clone(), &[]));
            fixpoint(
                "polygon",
                &prim("polygon", geom.clone(), &[("sides", json!(7))]),
            );
            fixpoint(
                "star",
                &prim(
                    "polygon",
                    json!({"x": 1.5, "y": 2.25, "w": w, "h": h, "inset": 0.4, "rotation": 0.3}),
                    &[("sides", json!(5))],
                ),
            );
            fixpoint(
                "line",
                &prim(
                    "line",
                    json!({"x": 0, "y": 0, "w": w, "h": h, "weight": 2.5}),
                    &[],
                ),
            );
        }
        // Boolean and edit-op outputs ride the same writer — same law.
        let a = square_sidecar(0.0, 0.0, 10.0);
        let b = square_sidecar(5.0, 5.0, 10.0);
        let u = svc.path_boolean(&a, &b, VectorBoolOp::Union).unwrap();
        fixpoint("boolean union", &u);
        let e = svc
            .apply_edit_ops(
                &a,
                &[json!({"op": "move_anchor", "subpath": 0, "index": 0, "position": [-2.5, 0.75]})],
            )
            .unwrap();
        fixpoint("edit ops", &e);
    }

    #[test]
    fn close_and_open_are_deterministic_and_inverse_in_flag() {
        // Open L: two segments (0,0)→(10,0)→(10,10).
        let mut p = VectorPath::new(PathId::nil(), FillRule::NonZero);
        p.subpaths.push(SubPath {
            closed: false,
            segments: vec![
                CubicSeg::line(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)),
                CubicSeg::line(Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)),
            ],
        });
        let s = path_to_sidecar_json(&p).unwrap();
        let closed = svc()
            .apply_edit_ops(&s, &[json!({"op": "close", "subpath": 0, "index": 0})])
            .unwrap();
        let cp = parse(&closed.sidecar).unwrap();
        assert!(cp.subpaths[0].closed);
        assert_eq!(cp.subpaths[0].segments.len(), 3, "closing segment appended");
        let reopened = svc()
            .apply_edit_ops(
                &closed.sidecar,
                &[json!({"op": "open", "subpath": 0, "index": 0})],
            )
            .unwrap();
        let op_ = parse(&reopened.sidecar).unwrap();
        assert!(!op_.subpaths[0].closed);
        assert_eq!(
            op_.subpaths[0].segments.len(),
            3,
            "open removes no geometry"
        );
    }
    // -----------------------------------------------------------------
    // §20.2 shape-stroke band budget (the ceiling this door owns)
    // -----------------------------------------------------------------

    fn shape_style(width: f64, align: StrokePosition) -> ShapeStrokeStyle {
        use appname_core::engine::vector_ops::{LineCap, LineJoin};
        ShapeStrokeStyle {
            width,
            cap: LineCap::Butt,
            join: LineJoin::Miter(4.0),
            align,
        }
    }

    /// The wire pins NO `widthPx` range (`vector.shape_add` says so in the
    /// field's own description), so an absurd width is a CONTRACT-VALID
    /// document. It must come back as a typed refusal — never an
    /// allocation abort, and never a saturating-cast plane whose `w * h`
    /// wraps to a silently wrong answer. Checked BEFORE any geometry, so
    /// this test costs nothing to run.
    #[test]
    fn an_unbounded_stroke_width_refuses_typed_instead_of_aborting() {
        let sidecar = square_sidecar(1.25, 2.5, 10.0);
        let path = parse(&sidecar).unwrap();
        for w in [9.0e3, 1.0e9, f64::MAX] {
            let e = band_plane_budget(&path, shape_style(w, StrokePosition::Center))
                .expect_err("past the ceiling");
            assert_eq!(e.code.as_str(), "error.schema.out_of_range", "width {w}");
            assert_eq!(e.field.as_deref(), Some("/stroke/widthPx"), "width {w}");
            assert!(e.message.contains("band ceiling"), "{}", e.message);
        }
        // The whole door refuses the same way — the guard is wired, not
        // merely present.
        let e = svc()
            .shape_stroke_coverage(&sidecar, shape_style(1.0e9, StrokePosition::Center))
            .expect_err("the door refuses");
        assert_eq!(e.code.as_str(), "error.schema.out_of_range");
        // And an ordinary width still serves through the same door.
        let plane = svc()
            .shape_stroke_coverage(&sidecar, shape_style(4.0, StrokePosition::Center))
            .expect("an ordinary stroke still serves");
        assert!(!plane.values.is_empty() && !plane.rect.is_empty());
    }

    /// The Inside/Outside arms build the DOUBLE band before gating it, so
    /// the budget must count the doubled reach. A width that fits as
    /// `center` must NOT sneak past as `inside`/`outside` — the ceiling is
    /// 8192 cells square, so `center` clears 6000 px (12 + 6000) and the
    /// doubled arms do not (12 + 12000).
    #[test]
    fn the_budget_counts_the_doubled_band_the_align_arms_actually_build() {
        let path = parse(&square_sidecar(1.25, 2.5, 10.0)).unwrap();
        band_plane_budget(&path, shape_style(6.0e3, StrokePosition::Center))
            .expect("center fits at 6000 px");
        for align in [StrokePosition::Inside, StrokePosition::Outside] {
            band_plane_budget(&path, shape_style(6.0e3, align))
                .expect_err("the doubled band does not fit at 6000 px");
        }
    }

    /// An empty path has no band and no budget question — it must not
    /// refuse (the `control_bounds` `None` arm).
    #[test]
    fn an_empty_path_has_no_band_budget_to_exceed() {
        let empty = VectorPath::new(PathId::nil(), FillRule::NonZero);
        band_plane_budget(&empty, shape_style(1.0e9, StrokePosition::Center))
            .expect("no geometry, no ceiling");
    }
}
