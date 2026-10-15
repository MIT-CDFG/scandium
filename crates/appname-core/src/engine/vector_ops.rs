// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §20.2–§20.3 (vector storage + booleans), §40.7 vector kinds;
//! DD ㉕ (one path model). TEXTVEC Door 1 (reports/O-TEXTVEC-log.md
//! 2026-08-13 handoff-shapes filing): the `VectorGeometry` service trait —
//! core DEFINES the seam and stays geometry-blind; `appname-vector`
//! implements it and registers at assembly construction, mirroring the
//! `OpImpl` pattern (`register(EngineAssembly) -> EngineAssembly`,
//! arbitration-1: no runtime door; replay = assembly + journal).
//!
//! The seam is VERSION-STAMPED: every sidecar crossing it is
//! `appname.vector.v1` (§7.5 worked example) — `appname_vector::wire`
//! emits and checks the stamp; [`EngineAssembly::with_vector_geometry`]
//! asserts the implementation declares exactly this schema.
//!
//! Core-side path payloads are OPAQUE bytes + a content hash: the hash is
//! computed HERE (single truth, [`crate::engine::assets::asset_id_for`]
//! machinery) and feeds the §20.3 evaluation cache and render-DAG dedup.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::engine::error::{EngineError, ErrorCode};
use crate::geom::IRect;

/// The one wire schema this seam speaks (§7.5; TEXTVEC constraint 3).
pub const VECTOR_SIDECAR_SCHEMA: &str = "appname.vector.v1";

/// §20.3 boolean operators (wire spelling per the `vector.path_bool`
/// contract enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VectorBoolOp {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

impl VectorBoolOp {
    pub fn from_wire(s: &str) -> Option<VectorBoolOp> {
        match s {
            "union" => Some(VectorBoolOp::Union),
            "subtract" => Some(VectorBoolOp::Subtract),
            "intersect" => Some(VectorBoolOp::Intersect),
            "exclude" => Some(VectorBoolOp::Exclude),
            _ => None,
        }
    }

    pub fn as_wire(self) -> &'static str {
        match self {
            VectorBoolOp::Union => "union",
            VectorBoolOp::Subtract => "subtract",
            VectorBoolOp::Intersect => "intersect",
            VectorBoolOp::Exclude => "exclude",
        }
    }
}

/// What the service hands back for any geometry-producing call: the
/// NORMALIZED sidecar bytes (strict-writer re-emission, §20.1
/// validate/normalize inside the implementation) and the path's bounds in
/// document pixels (integer hull — feeds §40.7 dirty rects).
#[derive(Debug, Clone)]
pub struct GeometryResult {
    pub sidecar: Vec<u8>,
    pub bounds: IRect,
}

/// §20.2 exact-coverage fill result at the seam: row-major f32 coverage in
/// `[0, 1]` over `rect` — the §13.1 mask convention. Mirrors
/// `appname_vector::raster::CoveragePlane` (the implementation converts);
/// core owns the type so the dependency direction stays crates→core.
#[derive(Debug, Clone)]
pub struct CoveragePlaneData {
    pub rect: IRect,
    /// `rect.width() * rect.height()` values, row-major.
    pub values: Vec<f32>,
}

/// TEXTVEC Door 1 — the vector-geometry service. Core never sees geometry
/// types: sidecar bytes in, sidecar bytes out, every payload
/// `appname.vector.v1`.
///
/// Error contract: implementations answer TYPED `EngineError`s per the
/// §40.7 rows — a structurally invalid payload (empty subpaths, bad
/// fillRule, non-contiguous segments) is `error.schema.out_of_range`
/// against the offending field; a wrong/missing schema stamp is
/// `error.schema.wrong_schema`.
pub trait VectorGeometry: Send + Sync {
    /// The wire schema the implementation speaks — MUST return
    /// [`VECTOR_SIDECAR_SCHEMA`] (asserted at registration).
    fn schema(&self) -> &'static str;

    /// §20.1 validate + normalize: parse `bytes` as an
    /// `appname.vector.v1` sidecar, run the strict validation rules, and
    /// re-emit normalized bytes + bounds.
    fn validate_sidecar(&self, bytes: &[u8]) -> Result<GeometryResult, EngineError>;

    /// Validate a `vector.path_new`/`shape_add`-shaped `path` PARAM object
    /// (the contract's inline `vectorPath` payload) and emit its sidecar.
    fn path_param_sidecar(&self, path: &Map<String, Value>) -> Result<GeometryResult, EngineError>;

    /// §20.4 primitive constructors (`vector.shape_add` `primitive`
    /// param): `kind` is the contract enum (`rect`/`rounded_rect`/
    /// `ellipse`/`polygon`/`line`/`custom`), `primitive` the full param
    /// object. D-48: the CALLER value-captures the result — replay never
    /// re-runs constructor trig.
    fn primitive_sidecar(
        &self,
        kind: &str,
        primitive: &Map<String, Value>,
    ) -> Result<GeometryResult, EngineError>;

    /// §20.3 boolean over two sidecars (operands use their own fill
    /// rules; result normalized NonZero per the pipeline).
    fn path_boolean(
        &self,
        a: &[u8],
        b: &[u8],
        op: VectorBoolOp,
    ) -> Result<GeometryResult, EngineError>;

    /// §20.2 exact-coverage rasterization of the sidecar's fill
    /// (`selection.from_path`, vector masks): analytic, zoom- and
    /// tiling-independent.
    fn rasterize_fill(&self, sidecar: &[u8]) -> Result<CoveragePlaneData, EngineError>;

    /// `vector.edit`'s incremental anchor ops (§20.4 pen-gesture form):
    /// apply the contract's op list (`add_anchor`/`delete_anchor`/
    /// `move_anchor`/`set_handles`/`close`/`open` rows, already
    /// field-validated by the arm) ATOMICALLY over the sidecar — any
    /// failing op errors the whole call with the sidecar untouched
    /// (§10.2 stage 6; out-of-range subpath/index →
    /// `error.schema.out_of_range`).
    fn apply_edit_ops(&self, sidecar: &[u8], ops: &[Value]) -> Result<GeometryResult, EngineError>;

    // --- The §40.5 stroke_path geometry arms (O-TEXTVEC stroke slice;
    // default bodies keep every probe/partial rig compiling — the
    // production service overrides both) ------------------------------

    /// `stroke_path{source: pathId}` coverage (§20.2 stroke
    /// tessellation with the position rule [NORMATIVE]: Center = the
    /// 1×-width band; Inside/Outside = the 2×-width centered band
    /// intersected-with/subtracted-by the source's fill region via the
    /// §20.3 booleans). Command-wire pins (Miter-4/Butt; §22.6 carries
    /// no cap/join knobs) live with the implementation.
    fn stroke_path_coverage(
        &self,
        _sidecar: &[u8],
        _width_px: f64,
        _position: StrokePosition,
    ) -> Result<CoveragePlaneData, EngineError> {
        Err(EngineError::new(
            ErrorCode::NotImplemented,
            "stroke_path_coverage: this VectorGeometry does not serve the \
             stroke arms (probe/partial rig; the production service does)",
        ))
    }

    /// `stroke_path{source: "selection"}` coverage: trace the §13.5
    /// boundary (the 0.5 iso-contour of the mask by marching squares)
    /// and stroke the traced contours under the same position rule.
    fn stroke_boundary_coverage(
        &self,
        _plane: &CoveragePlaneData,
        _width_px: f64,
        _position: StrokePosition,
    ) -> Result<CoveragePlaneData, EngineError> {
        Err(EngineError::new(
            ErrorCode::NotImplemented,
            "stroke_boundary_coverage: this VectorGeometry does not serve the \
             stroke arms (probe/partial rig; the production service does)",
        ))
    }

    /// §20.2 SHAPE-LAYER stroke coverage — the `layer.stroke` model's own
    /// band, distinct from the §40.5 `stroke_path` COMMAND arms above:
    /// the shape wire carries cap/join/miterLimit/align knobs the command
    /// wire does not ([`ShapeStrokeStyle`]). Coverage only (§13.1
    /// convention); the caller applies the stroke's own FillContent
    /// through the fill-math seam, exactly as it does for the fill leg.
    fn shape_stroke_coverage(
        &self,
        _sidecar: &[u8],
        _style: ShapeStrokeStyle,
    ) -> Result<CoveragePlaneData, EngineError> {
        Err(EngineError::new(
            ErrorCode::NotImplemented,
            "shape_stroke_coverage: this VectorGeometry does not serve the \
             stroke arms (probe/partial rig; the production service does)",
        ))
    }
}

/// §40.5/§20.2 stroke position (the command's `position` enum; §22.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokePosition {
    Inside,
    Center,
    Outside,
}

/// §20.2 `LineCap` (the shape wire's `cap` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// §20.2 `LineJoin` — `Miter` carries the wire's `miterLimit` (the spec's
/// `Miter(f32)`; past the limit the join falls back to Bevel, SVG
/// semantics).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineJoin {
    Miter(f64),
    Round,
    Bevel,
}

/// The geometry half of a shape layer's §20.2 `VectorStroke`, resolved off
/// the layer's engine-opaque `stroke` payload by
/// [`ShapeStrokeStyle::from_wire`]. Paint is NOT here — it rides the
/// fill-math seam like the fill leg's ([`stroke_paint`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeStrokeStyle {
    /// `widthPx` — the FULL band width (the band is ±width/2 for Center).
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub align: StrokePosition,
}

/// The §20.2 wire defaults this door PINS where the payload is silent.
///
/// The contract declares NO defaults for `vectorStroke` (`defaults: null`
/// on both `vector.shape_add` and `vector.shape_set_style`; §7.3 leaves
/// `layer.stroke` an open object), so a silent field needs a recorded
/// election rather than an invented spec line:
///
/// * `cap` → **Butt**, `join` → **Miter**, `miterLimit` → **4** — the
///   §20.2 Rust model's own declaration order (`LineCap { Butt, … }`,
///   `LineJoin { Miter(f32), … }`) and the SVG semantics §20.2 rule 2
///   names for the miter fallback. Photoshop's shape-stroke defaults
///   agree.
/// * `align` → **Center** — pinned to the ONE default the frozen spec
///   does state for a stroke position: `stroke_path`'s
///   `position: {default: center}` (§40.5/§22.6). Electing anything else
///   would make the two stroke doors disagree on silence.
pub const STROKE_DEFAULT_MITER_LIMIT: f64 = 4.0;

fn oob(field: &str, msg: String) -> EngineError {
    EngineError::with_field(ErrorCode::OutOfRange, field, msg)
}

fn ni(msg: String) -> EngineError {
    EngineError::new(ErrorCode::NotImplemented, msg)
}

impl ShapeStrokeStyle {
    /// Resolve a shape layer's `stroke` payload into the served geometry
    /// style.
    ///
    /// * `Ok(None)` — NO VISIBLE STROKE (absent/non-object payload,
    ///   `widthPx` absent or ≤ 0, or a `fill: {type: none}` stroke paint).
    ///   The fill-only page is the whole answer and its pixels are
    ///   untouched by this door.
    /// * `Ok(Some(style))` — served.
    /// * `Err` — TYPED refusal (QA-F8: refuse, never omit) for the §20.2
    ///   arms this slice does not compute:
    ///   - a non-empty `dashes` pattern (§20.2 rule 4, arc-length dash
    ///     walking);
    ///   - a `profile` width profile (§20.2 rule 6, Catmull-Rom sampled
    ///     multipliers);
    ///   - a non-solid stroke paint (gradient/pattern — the §22
    ///     FillContent binding, the SAME posture the fill leg holds);
    ///   and `error.schema.out_of_range` for a payload that is not the
    ///   contract's `vectorStroke` at all (bad enum spelling, non-finite
    ///   or negative width) — the loader door can deliver one even though
    ///   the command door's stage-5 validation cannot.
    pub fn from_wire(stroke: &Value) -> Result<Option<ShapeStrokeStyle>, EngineError> {
        let Some(obj) = stroke.as_object() else {
            return Ok(None); // null / absent / non-object ⇒ no stroke
        };
        let width = match obj.get("widthPx") {
            None | Some(Value::Null) => 0.0,
            Some(v) => v
                .as_f64()
                .filter(|w| w.is_finite() && *w >= 0.0)
                .ok_or_else(|| {
                    oob(
                        "/stroke/widthPx",
                        format!(
                            "shape stroke widthPx must be a finite number ≥ 0 (§20.2); got {v}"
                        ),
                    )
                })?,
        };
        if width <= 0.0 {
            return Ok(None); // §20.3: zero width is NO visible stroke
        }
        // Paint first: a `none` stroke fill draws nothing at any width,
        // and a gradient/pattern stroke refuses BEFORE geometry work.
        if stroke_paint(stroke)?.is_none() {
            return Ok(None);
        }
        if obj
            .get("dashes")
            .and_then(Value::as_array)
            .is_some_and(|d| d.iter().any(|v| v.as_f64().is_some_and(|n| n > 0.0)))
        {
            return Err(ni(
                "shape stroke `dashes`: the §20.2 rule-4 arc-length dash walk (SVG \
                 odd-count concatenation, phase = dashOffset mod L, per-run caps, the \
                 zero-length round-cap dot) is not computed by this band — refused \
                 typed, never omitted"
                    .into(),
            ));
        }
        if obj
            .get("profile")
            .and_then(Value::as_array)
            .is_some_and(|p| !p.is_empty())
        {
            return Err(ni(
                "shape stroke `profile`: the §20.2 rule-6 WidthProfile (Catmull-Rom \
                 a = −0.5 over the (t, multiplier) control points, §18.2 weights) is \
                 not sampled by this band — refused typed, never omitted"
                    .into(),
            ));
        }
        let cap = match obj.get("cap").and_then(Value::as_str) {
            None => LineCap::Butt,
            Some("butt") => LineCap::Butt,
            Some("round") => LineCap::Round,
            Some("square") => LineCap::Square,
            Some(other) => {
                return Err(oob(
                    "/stroke/cap",
                    format!("§20.2 LineCap is butt|round|square; got \"{other}\""),
                ))
            }
        };
        let miter_limit = match obj.get("miterLimit") {
            None | Some(Value::Null) => STROKE_DEFAULT_MITER_LIMIT,
            Some(v) => v.as_f64().filter(|l| l.is_finite()).ok_or_else(|| {
                oob(
                    "/stroke/miterLimit",
                    format!("§20.2 miterLimit must be a finite number; got {v}"),
                )
            })?,
        };
        let join = match obj.get("join").and_then(Value::as_str) {
            None => LineJoin::Miter(miter_limit),
            Some("miter") => LineJoin::Miter(miter_limit),
            Some("round") => LineJoin::Round,
            Some("bevel") => LineJoin::Bevel,
            Some(other) => {
                return Err(oob(
                    "/stroke/join",
                    format!("§20.2 LineJoin is miter|round|bevel; got \"{other}\""),
                ))
            }
        };
        let align = match obj.get("align").and_then(Value::as_str) {
            None => StrokePosition::Center,
            Some("inside") => StrokePosition::Inside,
            Some("center") => StrokePosition::Center,
            Some("outside") => StrokePosition::Outside,
            Some(other) => {
                return Err(oob(
                    "/stroke/align",
                    format!("§20.2 StrokeAlign is inside|center|outside; got \"{other}\""),
                ))
            }
        };
        Ok(Some(ShapeStrokeStyle {
            width,
            cap,
            join,
            align,
        }))
    }

    /// The maximum distance, in document pixels, that the PAINTED §20.2
    /// stroke band can extend OUTWARD from the source path.
    ///
    /// This is a CLOSED-FORM BOUND, derived piece-by-piece off the very
    /// geometry `appname_vector::stroke::shape_stroke_coverage` builds —
    /// it must cover that rasterizer, so every term below names the piece
    /// it answers:
    ///
    /// * **half-width `h`.** The coverage builds its band at
    ///   `h = width/2` for `Center` and at `h = width` for
    ///   `Inside`/`Outside` (the 2×-width centered band, later gated by
    ///   the fill). Segment quads reach exactly `h`.
    /// * **`Inside` reaches nothing.** That arm gates the band by the
    ///   fill coverage (`min(band, f)`), so it paints only where the fill
    ///   already does — inside the primitive's own hull. Zero.
    /// * **joins.** Round and bevel joins put their outermost vertex at
    ///   `h`. A MITER join runs out to its apex, which the coverage
    ///   accepts while `|apex − v| ≤ limit·h` and falls back to a bevel
    ///   past it — so the miter arm reaches `h · max(1, limit)`.
    /// * **caps.** Butt adds nothing (the chord IS the quad's edge),
    ///   round adds a disc of radius `h`, and SQUARE puts its corner at
    ///   `h` along the tangent AND `h` along the normal: `h·√2`.
    ///
    /// DELIBERATELY CONSERVATIVE at the miter term: `limit·h` is the
    /// worst case over all turn angles, and a shape with no sharp vertex
    /// (an ellipse, an axis-aligned rect) reaches far less. Tightening it
    /// needs the path's actual turn angles, which this door must not
    /// re-derive — D-48 value-captures shape geometry precisely so replay
    /// never re-runs constructor trig, and this bound is pure arithmetic
    /// over the captured payload. A dirty rect that over-covers repaints
    /// more than it must; one that under-covers ships stale pixels.
    pub fn outward_reach(&self) -> f64 {
        if !self.width.is_finite() || self.width <= 0.0 {
            return 0.0;
        }
        let h = match self.align {
            // Gated by the fill: paints only where the primitive already
            // does, so it reaches nothing outside the hull.
            StrokePosition::Inside => return 0.0,
            StrokePosition::Center => self.width / 2.0,
            StrokePosition::Outside => self.width,
        };
        let cap = match self.cap {
            LineCap::Butt | LineCap::Round => 1.0,
            LineCap::Square => std::f64::consts::SQRT_2,
        };
        let join = match self.join {
            LineJoin::Round | LineJoin::Bevel => 1.0,
            // A limit below 1 can never admit an apex (the apex is never
            // closer than `h`), so those payloads bevel — reach `h`.
            LineJoin::Miter(limit) if limit.is_finite() => limit.max(1.0),
            LineJoin::Miter(_) => 1.0,
        };
        h * cap.max(join)
    }
}

/// One whole pixel of slack over the analytic reach: the coverage
/// rasterizer takes `floor`/`ceil` of its vertex bounds and the §20.2
/// flattener works to a tolerance, so the painted plane can round a pixel
/// past the exact analytic edge.
const STROKE_DIRTY_AA_MARGIN: f64 = 1.0;

/// Ceiling on the inflation, in document pixels. `miterLimit` carries NO
/// range on the wire (`vector.shape_add.yaml` says so outright of the
/// stroke's numbers), so a pathological limit would otherwise overflow
/// the `i32` rect math downstream. 2^20 px is orders of magnitude past
/// any v1 canvas dimension — a D clamped here already covers the whole
/// document and then some.
const STROKE_DIRTY_MAX_INFLATE: f64 = (1i32 << 20) as f64;

/// The §40.7 dirty-rect inflation, in whole document pixels, owed by a
/// shape layer's engine-opaque `stroke` payload.
///
/// `D = shape bounds` alone describes the PRIMITIVE, not the painted
/// result: a §20.2 stroke band paints outside that hull, and every such
/// pixel the shell is not told to repaint is a stale pixel the user sees
/// until something else forces a redraw. Callers union this outward reach
/// into their bounds.
///
/// **The zero arm.** A payload [`ShapeStrokeStyle::from_wire`] REFUSES
/// (a `dashes` pattern, a width `profile`, a non-solid stroke paint)
/// inflates nothing — not as a shrug, but because such a payload paints
/// NO pixels at all: the composite door refuses the whole render on it
/// (QA-F8 — refuse, never omit), so no pixel moves and there is nothing
/// to repaint. That rationale is a standing CHECK, not a comment: the
/// conformance battery exhibits both refusals at the composite door, and
/// reds the day a later slice serves one of them without revisiting this
/// bound.
pub fn shape_stroke_dirty_inflate(stroke: &Value) -> i32 {
    let reach = match ShapeStrokeStyle::from_wire(stroke) {
        Ok(Some(style)) => style.outward_reach(),
        // Ok(None) = no visible stroke (absent payload, widthPx ≤ 0,
        // `fill: {type: none}`) — the fill-only page, bounds exactly.
        Ok(None) | Err(_) => 0.0,
    };
    if reach <= 0.0 {
        return 0;
    }
    (reach + STROKE_DIRTY_AA_MARGIN)
        .min(STROKE_DIRTY_MAX_INFLATE)
        .ceil() as i32
}

/// The stroke's PAINT as a §22 `FillContent` value for the fill-math seam.
///
/// Precedence [PINNED]: `fill` is the general form and wins; `color` is
/// the contract's own "solid stroke fill shorthand" (§40.7 script example)
/// and is lifted to `{type: solid, color}`. Neither present ⇒ `None` is
/// handed to the seam, i.e. the fill leg's own absent-payload posture
/// (opaque black).
///
/// * `Ok(None)` — nothing to paint (`fill: {type: none}`).
/// * `Err` — a gradient/pattern stroke fill (the §22 binding the fill leg
///   also awaits).
pub fn stroke_paint(stroke: &Value) -> Result<Option<Value>, EngineError> {
    let Some(obj) = stroke.as_object() else {
        return Ok(None);
    };
    let content: Value = match (obj.get("fill"), obj.get("color")) {
        (Some(f), _) if !f.is_null() => f.clone(),
        (_, Some(c)) if !c.is_null() => {
            serde_json::json!({"type": "solid", "color": c.clone()})
        }
        // Absent paint: the fill leg's own "absent = opaque black" posture
        // (appname-text's fill_to_linear_rgba(None)) — represented as an
        // explicit solid with no color so the seam takes that same arm.
        _ => serde_json::json!({"type": "solid"}),
    };
    match content.get("type").and_then(Value::as_str) {
        Some("none") => Ok(None),
        None | Some("solid") => Ok(Some(content)),
        Some(other) => Err(ni(format!(
            "shape stroke fill type \"{other}\" awaits the §22 FillContent binding \
             (the SAME posture the shape FILL leg holds) — refused typed"
        ))),
    }
}

/// Content hash of a sidecar: UUIDv5 over the BLAKE3 hex of the bytes —
/// the same machinery as content-addressed assets (§7.4), so vector hashes
/// live in the same identity space the render DAG dedups on.
pub fn sidecar_content_hash(bytes: &[u8]) -> Uuid {
    crate::engine::assets::asset_id_for(bytes)
}

/// The typed refusal every geometry-requiring arm answers when no
/// [`VectorGeometry`] is registered on the assembly (bare-core mirror —
/// the house pattern for unregistered-service kinds).
pub fn geometry_unregistered(kind: &str) -> EngineError {
    EngineError::new(
        ErrorCode::NotImplemented,
        format!(
            "{kind}: no VectorGeometry service is registered on this assembly \
             (appname-vector registers it at construction; bare-core mirror)"
        ),
    )
}

/// §20.3 evaluation cache, core-side: composite-path results keyed by
/// `(base content hash, [(op, operand content hash)])` — the spec's
/// "(operand content hashes, op list)" key. Editing any source path
/// changes its content hash, so stale entries are simply never hit again
/// (lazy re-evaluation falls out of the keying; the map is bounded below).
/// Cache key: (base content hash, [(op, operand content hash)]).
type EvalKey = (Uuid, Vec<(VectorBoolOp, Uuid)>);

#[derive(Default)]
pub struct VectorEvalCache {
    entries: HashMap<EvalKey, Arc<GeometryResult>>,
    hits: u64,
    misses: u64,
}

/// Bound: composite results are small (sidecar JSON), but the cache never
/// grows unbounded across long sessions.
const EVAL_CACHE_CAP: usize = 256;

impl VectorEvalCache {
    pub fn new() -> VectorEvalCache {
        VectorEvalCache::default()
    }

    /// Left fold of `ops` over `base` via the service (§20.3: composite
    /// path = fold of bool_ops over the layer's base path), memoized on
    /// the full key. Operand tuples carry `(op, operand sidecar bytes,
    /// operand content hash)`.
    pub fn evaluate_composite(
        &mut self,
        svc: &dyn VectorGeometry,
        base_sidecar: &[u8],
        base_hash: Uuid,
        ops: &[(VectorBoolOp, Arc<Vec<u8>>, Uuid)],
    ) -> Result<Arc<GeometryResult>, EngineError> {
        let key = (
            base_hash,
            ops.iter().map(|(op, _, h)| (*op, *h)).collect::<Vec<_>>(),
        );
        if let Some(hit) = self.entries.get(&key) {
            self.hits += 1;
            return Ok(Arc::clone(hit));
        }
        self.misses += 1;
        let mut acc = svc.validate_sidecar(base_sidecar)?;
        for (op, operand, _) in ops {
            acc = svc.path_boolean(&acc.sidecar, operand, *op)?;
        }
        let result = Arc::new(acc);
        if self.entries.len() >= EVAL_CACHE_CAP {
            self.entries.clear(); // simple full-flush bound; derived state only
        }
        self.entries.insert(key, Arc::clone(&result));
        Ok(result)
    }

    /// (hits, misses) — test/observability surface.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn served(v: Value) -> ShapeStrokeStyle {
        ShapeStrokeStyle::from_wire(&v)
            .expect("served")
            .expect("visible")
    }

    fn refused(v: Value) -> (String, String) {
        let e = ShapeStrokeStyle::from_wire(&v).expect_err("refused");
        (e.code.as_str().to_string(), e.message.clone())
    }

    #[test]
    fn no_visible_stroke_covers_every_shape_the_contract_actually_ships() {
        // The contract's OWN shape_add validMinimal default, the absent
        // payload, an explicit null, a non-object, and a `none` stroke
        // fill — every one is "no stroke", so the fill-only page stays
        // exactly what it was before this leg landed.
        for v in [
            json!({"widthPx": 0}),
            json!({}),
            Value::Null,
            json!(7),
            json!({"widthPx": 4, "fill": {"type": "none"}}),
            json!({"widthPx": 0, "dashes": [4, 2], "profile": [[0, 1]]}),
        ] {
            assert_eq!(
                ShapeStrokeStyle::from_wire(&v).expect("not a refusal"),
                None,
                "no visible stroke: {v}"
            );
        }
    }

    #[test]
    fn the_silent_wire_takes_the_recorded_defaults() {
        // §20.2's Rust declaration order (Butt, Miter) + the ONE default
        // the frozen spec states for a stroke position (stroke_path's
        // `position: center`). A drift here is a behavior change, not a
        // refactor.
        let s = served(json!({"widthPx": 3}));
        assert_eq!(s.width, 3.0);
        assert_eq!(s.cap, LineCap::Butt);
        assert_eq!(s.join, LineJoin::Miter(4.0));
        assert_eq!(s.align, StrokePosition::Center);
    }

    #[test]
    fn every_enum_spelling_the_contract_lists_resolves() {
        for (w, want) in [
            ("butt", LineCap::Butt),
            ("round", LineCap::Round),
            ("square", LineCap::Square),
        ] {
            assert_eq!(served(json!({"widthPx": 1, "cap": w})).cap, want);
        }
        for (w, want) in [
            ("miter", LineJoin::Miter(4.0)),
            ("round", LineJoin::Round),
            ("bevel", LineJoin::Bevel),
        ] {
            assert_eq!(served(json!({"widthPx": 1, "join": w})).join, want);
        }
        for (w, want) in [
            ("inside", StrokePosition::Inside),
            ("center", StrokePosition::Center),
            ("outside", StrokePosition::Outside),
        ] {
            assert_eq!(served(json!({"widthPx": 1, "align": w})).align, want);
        }
        assert_eq!(
            served(json!({"widthPx": 1, "join": "miter", "miterLimit": 1.5})).join,
            LineJoin::Miter(1.5)
        );
    }

    #[test]
    fn the_unserved_20_2_arms_refuse_typed_and_name_their_rule() {
        let (code, msg) = refused(json!({"widthPx": 2, "dashes": [4, 2]}));
        assert_eq!(code, "error.command.not_implemented");
        assert!(
            msg.contains("dashes") && msg.contains("§20.2 rule-4"),
            "{msg}"
        );
        let (code, msg) = refused(json!({"widthPx": 2, "profile": [[0, 1], [1, 0.5]]}));
        assert_eq!(code, "error.command.not_implemented");
        assert!(
            msg.contains("profile") && msg.contains("§20.2 rule-6"),
            "{msg}"
        );
        let (code, msg) = refused(
            json!({"widthPx": 2, "fill": {"type": "gradient", "gradient": {"stops": [], "kind": "linear"}}}),
        );
        assert_eq!(code, "error.command.not_implemented");
        assert!(msg.contains("§22"), "{msg}");
        // An ALL-ZERO dash array is not a dash pattern — it draws the
        // solid stroke, so it serves rather than refusing.
        assert!(ShapeStrokeStyle::from_wire(&json!({"widthPx": 2, "dashes": [0, 0]})).is_ok());
        assert!(ShapeStrokeStyle::from_wire(&json!({"widthPx": 2, "profile": []})).is_ok());
    }

    #[test]
    fn a_payload_that_is_not_a_vector_stroke_is_out_of_range_not_a_wall() {
        for (v, field) in [
            (json!({"widthPx": "wide"}), "/stroke/widthPx"),
            (json!({"widthPx": -1}), "/stroke/widthPx"),
            (json!({"widthPx": 1, "cap": "flat"}), "/stroke/cap"),
            (json!({"widthPx": 1, "join": "arc"}), "/stroke/join"),
            (json!({"widthPx": 1, "align": "middle"}), "/stroke/align"),
            (
                json!({"widthPx": 1, "miterLimit": "four"}),
                "/stroke/miterLimit",
            ),
        ] {
            let e = ShapeStrokeStyle::from_wire(&v).expect_err("rejected");
            assert_eq!(e.code.as_str(), "error.schema.out_of_range", "{v}");
            assert_eq!(e.field.as_deref(), Some(field), "{v}");
        }
    }

    #[test]
    fn stroke_paint_precedence_is_fill_then_the_color_shorthand() {
        let c = json!({"model": "rgba", "comps": [1, 0, 0, 1]});
        // `color` alone lifts to a solid FillContent (the §40.7 script
        // example's shorthand).
        assert_eq!(
            stroke_paint(&json!({"widthPx": 1, "color": c})).unwrap(),
            Some(json!({"type": "solid", "color": {"model": "rgba", "comps": [1, 0, 0, 1]}}))
        );
        // `fill` is the general form and wins outright.
        let both = json!({"widthPx": 1,
                          "color": {"model": "rgba", "comps": [1, 0, 0, 1]},
                          "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0, 1, 0, 1]}}});
        assert_eq!(
            stroke_paint(&both).unwrap(),
            Some(json!({"type": "solid", "color": {"model": "rgba", "comps": [0, 1, 0, 1]}}))
        );
        // Neither: the fill leg's own absent-payload posture (the seam
        // reads {type:solid} with no color EXACTLY as it reads None —
        // opaque black).
        assert_eq!(
            stroke_paint(&json!({"widthPx": 1})).unwrap(),
            Some(json!({"type": "solid"}))
        );
        // `none` paints nothing at any width.
        assert_eq!(
            stroke_paint(&json!({"widthPx": 9, "fill": {"type": "none"}})).unwrap(),
            None
        );
    }

    #[test]
    fn a_bare_vector_geometry_answers_not_implemented_on_the_shape_arm() {
        struct Bare;
        impl VectorGeometry for Bare {
            fn schema(&self) -> &'static str {
                VECTOR_SIDECAR_SCHEMA
            }
            fn validate_sidecar(&self, _: &[u8]) -> Result<GeometryResult, EngineError> {
                unimplemented!()
            }
            fn path_param_sidecar(
                &self,
                _: &Map<String, Value>,
            ) -> Result<GeometryResult, EngineError> {
                unimplemented!()
            }
            fn primitive_sidecar(
                &self,
                _: &str,
                _: &Map<String, Value>,
            ) -> Result<GeometryResult, EngineError> {
                unimplemented!()
            }
            fn path_boolean(
                &self,
                _: &[u8],
                _: &[u8],
                _: VectorBoolOp,
            ) -> Result<GeometryResult, EngineError> {
                unimplemented!()
            }
            fn rasterize_fill(&self, _: &[u8]) -> Result<CoveragePlaneData, EngineError> {
                unimplemented!()
            }
            fn apply_edit_ops(&self, _: &[u8], _: &[Value]) -> Result<GeometryResult, EngineError> {
                unimplemented!()
            }
        }
        let style = ShapeStrokeStyle {
            width: 2.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter(4.0),
            align: StrokePosition::Center,
        };
        let e = Bare.shape_stroke_coverage(b"{}", style).expect_err("wall");
        assert_eq!(e.code.as_str(), "error.command.not_implemented");
    }

    /// The §40.7 dirty inflation, arm by arm — the UNIT half of the
    /// `vector.shape_add` staleness fix (the behavioral half rides the
    /// conformance battery, which measures against real composited ink).
    #[test]
    fn the_dirty_inflate_answers_every_stroke_arm() {
        let px = shape_stroke_dirty_inflate;
        // NO VISIBLE STROKE ⇒ no inflation. These are the arms that must
        // NOT move: the contract's own `validMinimal` carries
        // `stroke: {widthPx: 0}`, and every fill-only fixture in the
        // build would shift if this grew.
        assert_eq!(px(&Value::Null), 0, "absent payload");
        assert_eq!(px(&json!({})), 0, "no widthPx");
        assert_eq!(px(&json!({"widthPx": 0})), 0, "§20.3 zero width");
        assert_eq!(px(&json!({"widthPx": 12, "fill": {"type": "none"}})), 0);
        // REFUSED arms: nothing paints (the composite door refuses the
        // whole render), so nothing repaints.
        assert_eq!(px(&json!({"widthPx": 20, "dashes": [4, 2]})), 0);
        assert_eq!(px(&json!({"widthPx": 20, "profile": [[0, 1], [1, 2]]})), 0);
        // SERVED. Center: h = w/2, and the silent join default is
        // Miter(4) ⇒ 10·4 + 1.
        assert_eq!(px(&json!({"widthPx": 20})), 41);
        // The join arms at the same width: round/bevel reach exactly h.
        assert_eq!(px(&json!({"widthPx": 20, "join": "round"})), 11);
        assert_eq!(px(&json!({"widthPx": 20, "join": "bevel"})), 11);
        // A miterLimit BELOW 1 can never admit an apex, so it bevels.
        assert_eq!(px(&json!({"widthPx": 20, "miterLimit": 0.5})), 11);
        assert_eq!(px(&json!({"widthPx": 20, "miterLimit": 2})), 21);
        // Caps matter only where a subpath is open, but the bound covers
        // them: square puts its corner at h·√2 ⇒ ceil(14.14 + 1).
        assert_eq!(
            px(&json!({"widthPx": 20, "join": "round", "cap": "square"})),
            16
        );
        // ALIGN. Inside is gated by the fill — it reaches nothing.
        assert_eq!(px(&json!({"widthPx": 20, "align": "inside"})), 0);
        assert_eq!(
            px(&json!({"widthPx": 20, "align": "center", "join": "round"})),
            11
        );
        // Outside builds its band at the FULL width, not half.
        assert_eq!(
            px(&json!({"widthPx": 20, "align": "outside", "join": "round"})),
            21
        );
        // OVERFLOW GUARD: `miterLimit` carries no range on the wire, so
        // exhibit that a pathological one clamps instead of running the
        // i32 rect math off a cliff.
        let wild = px(&json!({"widthPx": 1e6, "miterLimit": 1e9}));
        assert_eq!(wild, (1i32 << 20), "clamped, not overflowed");
        assert!(wild > 0, "and still positive");
    }
}
