// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! TEXTVEC Doors 6 + 10: the production
//! [`appname_core::engine::text_ops::TextLayout`] implementation over
//! this crate's landed stack (providers/fontdb/shape/layout/compose/
//! tofu/style/raster) — the text-kinds wiring slice.
//!
//! Spec: ES §23.1–§23.5, §40.7 text rows, Part 44; DD ㉒. Pattern:
//! Door 1's `op_bridge` shape — `register(EngineAssembly)` at
//! construction only, ONE `PRODUCTION_CHAIN` row in `appname-assembly`,
//! core text-blind behind the version-stamped `appname.text.v1` seam.
//! D-48 posture (O-INT-approved): text re-renders PARAMETRICALLY —
//! every call re-derives layout as a pure function of (sidecar, fonts);
//! `outline_geometry` output is value-captured ARM-side through Door 1.
//!
//! SIDECAR WIRE (service-owned; §23.1 `text/{layerId}.json`): the
//! contract `$defs` shapes verbatim — `layout` is text.edit's tagged
//! `textLayout` object (`{kind, origin|rect|pathId}`), `runs[]` are
//! flat `styledRun` objects (`text` + `font`/`size`/`fill`/`tracking`/
//! `kerning`/`baselineShift`/`features`/`variations`/`language`), and
//! `paragraph` is `paragraphStyle`. The FIRST run's `fill` rides the
//! run object itself, which is exactly where the Door-9 arm's pinned
//! mechanical read expects it. §7.7 posture: unknown fields (top level
//! and per run) are preserved verbatim through re-emission; key order
//! is `serde_json`'s BTreeMap order (canonical); floats round-trip
//! exactly (`float_roundtrip`, the DEFECT-1 election) — so emission is
//! the validator's byte-exact fixpoint (property-tested).
//!
//! THREADING (why the service stores BYTES, not engine objects): the
//! trait is `Send + Sync`, but FreeType `Face` and the HarfBuzz wrapper
//! are not `Send` — `Inner` holds only face bytes + the stamp behind a
//! `Mutex`, and every call builds its engine state fresh. Correct and
//! deterministic; the per-call construction cost is a recorded C-6 perf
//! item. No `unsafe` anywhere — this crate stays `forbid(unsafe_code)`.
//!
//! Pinned elections (fold-in register; mirrored in the lane log):
//! * EM AT THE SEAM: `em_px = size_pt` (the §19.1 formula at PPI 72).
//!   The seam does not carry the document PPI — SCHEMA-GAP flag rides
//!   the lane log (the arm should thread geometry.resolution_ppi);
//!   px ≡ pt until it does.
//! * `on_path` ANSWERS TYPED `error.command.not_implemented`: the
//!   sidecar carries `pathId` only; the path GEOMETRY lives in the
//!   document arena and the seam gives the service no way to resolve
//!   it. `layout_on_path` is landed and waiting — the binding needs an
//!   arm-threaded path payload (follow-up filing).
//! * RUN NORMALIZATION (§23.1 minimal runs): zero-length runs refuse
//!   (`out_of_range`); adjacent runs equal in every non-`text` field
//!   merge (first run's unknown fields win — §7.7).
//! * EMPTY LAYERS ARE LEGAL: `text.add` defaults `text` to `""` (the
//!   create-then-type flow), so `runs: []` is a valid sidecar; layout
//!   is empty, bounds `IRect::EMPTY`, `content_len` 0.
//! * WHOLE-TEXT PARAGRAPH: v1 stores ONE ParagraphStyle (§23.1); the
//!   full text lays out as one §23.3 paragraph — embedded newlines are
//!   mandatory breaks, spaceBefore applies at the block's top edge.
//! * MANDATORY-BREAK CHARACTERS RENDER NOTHING (§23.1; the layout
//!   module's cluster rule): glyphs whose cluster base is `\n`/`\r`/
//!   U+2028/U+2029 (the layout's exact break set) are dropped after
//!   shaping — they occupy no pen advance, no ink, no caret width, and
//!   no outline (review MAJOR-1: unmapped breaks otherwise resolve to
//!   tofu and render hex boxes into bounds/dirty/carets/to_shape).
//! * STRICT WIRE VALUES (the trait's error contract: "bad enum →
//!   out_of_range"): known style/paragraph fields with a wrong TYPE or
//!   an unknown ENUM value refuse typed against the wire field name;
//!   only UNKNOWN fields ride the §7.7 preserve-verbatim posture
//!   (review MAJOR-3). Run-merge identity compares numbers by VALUE
//!   (100 ≡ 100.0 — §23.1 minimality is not spelling-sensitive).
//! * MANUAL KERNING (C-6 slice 1; REWORKED at review MAJOR-1):
//!   `Kerning::Manual(k)` adds k ‰-em px on the LEFT glyph's advance at
//!   each INTRA-ITEM pair boundary — never at the run's trailing edge
//!   (the gap machinery attributes boundary gaps to the earlier
//!   cluster, so the earlier tracking-fold bled a 2-scalar pair range
//!   into the NEXT run's gap: a pair kerned TWO gaps). Advance-based
//!   application makes widths, breaking, justification, bounds,
//!   carets, and hit-testing inherit the kern coherently. Cross-ITEM
//!   boundaries inside a kerned range (script/direction/face changes)
//!   are not kerned — v1 pin, FLAGGED. Join suppression uses the same
//!   Joining_Type discriminator as tracking.
//! * OPTICAL KERNING (C-6 slice 2, the pinned deterministic
//!   algorithm): within an Optical run, each pen-order glyph pair's
//!   ink gap (left glyph's right side bearing + right glyph's left
//!   side bearing, from the SAME coverage rasters compose uses —
//!   hint-rule-matched: autohint-light at em ≤ 32 px, unhinted above,
//!   so measured ink is always rendered ink) normalizes toward
//!   OPTICAL_TARGET_EM, the correction clamped
//!   to ±OPTICAL_CLAMP_EM, applied to the LEFT glyph's advance (so
//!   cluster widths, breaking, justification, bounds, carets, and
//!   hit-testing all inherit it coherently). Inkless glyphs (spaces)
//!   and zero-advance marks never kern; cursive-join boundaries are
//!   suppressed by the SAME Joining_Type discriminator as tracking.
//!   Purely shape-derived, no font kern data consulted (§23.4
//!   "derived from glyph shapes"); constants are fold-in pins.
//! * ALIGNMENT WORDS: contract `left|right` map to `Align::Start|End`
//!   (start/end-relative under the base direction, §23.3 mirroring);
//!   point text uses the literal `PointAlign` reading (justify→Left).
//! * BASE DIRECTION: `direction: auto` resolves by UAX #9 P2/P3 over
//!   the full text (unicode-bidi paragraph level); `ltr`/`rtl` force.
//! * TOFU METRICS: ascent 0.8·em, descent 0.2·em, gap 0 (the §23.5
//!   hex-box face carries no tables; deterministic pin).
//! * FILL v1: PER-RUN compositing (the [`TextPageData`] contract:
//!   coverage composites "per-run through each run's `FillContent`") —
//!   each run's coverage premultiplies its own fill and runs composite
//!   src-over in run order (§4.4; overlap between runs is rare —
//!   antialiased seams only). Solid `gray|grayA|rgb|rgba` via the sRGB
//!   EOTF into linear; absent fill = opaque black; `none` = clear;
//!   gradient/pattern/cmyk/lab compose as opaque black pending the §22
//!   FillContent binding and the §5 CMM (flagged).
//! * BOUNDS = the composed page's nonzero-coverage integer hull (the
//!   strictest honest reading of §40.7 "laid-out bounds").
//! * CARET/HIT v1: carets cluster-snap (§23.2); the line-end caret
//!   sits at the visual end matching the base direction; at a bidi
//!   level-parity boundary a half-height secondary caret is emitted at
//!   the previous cluster's trailing edge; the caret AFTER a trailing
//!   mandatory break synthesizes the next line's start (leading edge,
//!   one §23.3 leading below the last baseline — create-then-type).
//!   Refinement is C-6.
//! * FACE METADATA: enumeration lists document-embedded faces as
//!   family "Embedded", style "Regular", `embedded-<uuid>` (name-table
//!   parsing is C-6); `embeddable` per §23.5 fsType (restricted 0x0002
//!   or bitmap-only 0x0200 → false; no OS/2 = installable). No system
//!   or bundled faces exist in v1 (§3.10: none vendored; the system
//!   scan is a shell-side arm).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use unicode_bidi::BidiInfo;
use uuid::Uuid;

use appname_core::engine::error::{EngineError, ErrorCode};
use appname_core::engine::kind_impl::EngineAssembly;
use appname_core::engine::text_ops::{
    CaretRect, FontFaceInfo, LaidMetrics, TextLayout, TextLayoutResult, TextPageData,
    TEXT_SIDECAR_SCHEMA,
};
use appname_core::geom::IRect;
use appname_core::PathId;
use appname_vector::geom::Vec2;
use appname_vector::path::{CubicSeg, FillRule, SubPath, VectorPath};
use appname_vector::wire::path_to_sidecar_json;

use crate::cache::RasterCache;
use crate::compose::{compose_page, CoveragePage, ItemRaster};
use crate::fontdb::{DbProbe, FaceInfo, FaceSource, FontDb, ResolveCtx, TOFU_FACE};
use crate::layout::{
    layout_paragraph, layout_point_text, Align, JustifyLast, LayoutBox, ParagraphLayout,
    ParagraphStyle, PointAlign, StyledItem,
};
use crate::pipeline::{
    itemize, FaceId, GlyphRasterizer, GraphemeSegmenter, LineBreakProvider, RunSpan, ShapedGlyph,
    Shaper, StyleId,
};
use crate::providers::{
    is_cursive_join_boundary, IcuGraphemeSegmenter, IcuLineBreakProvider, IcuScriptClassifier,
    UnicodeBidiProvider,
};
use crate::raster::{FaceMetricsPx, FreeTypeRasterizer, OutlineCmd};
use crate::shape::HarfBuzzShaper;
use crate::style::{CharStyle, DecorLine, Kerning};
use crate::tofu::tofu_shape;

/// Tofu vertical-metrics pin (module-header election).
const TOFU_ASCENT_EM: f64 = 0.8;
const TOFU_DESCENT_EM: f64 = 0.2;
/// Optical-kerning pins (module-header election): pair ink gaps
/// normalize toward the target, corrections clamped (em fractions).
const OPTICAL_TARGET_EM: f64 = 0.16;
const OPTICAL_CLAMP_EM: f64 = 0.05;
/// Caret em when a layer has no runs to style one with (§23.4 default).
const DEFAULT_SIZE_PT: f64 = 12.0;
/// The contract `styledRunStyle` field set — the edit-inheritance
/// walker iterates exactly these (anything else on a run is an unknown
/// field, preserved verbatim per §7.7, never inherited). `script` (a
/// `styledRun` field) is DELIBERATELY excluded: it is
/// itemization-derived (§23.2), so inheriting it across an edit could
/// pin a stale script onto new text.
const STYLE_FIELDS: [&str; 9] = [
    "font",
    "size",
    "fill",
    "tracking",
    "kerning",
    "baselineShift",
    "features",
    "variations",
    "language",
];

// ---------------------------------------------------------------------
// Service + registration
// ---------------------------------------------------------------------

/// The Doors 6 + 10 service. See the module header for the design.
#[derive(Default)]
pub struct TextLayoutService {
    inner: Mutex<Inner>,
}

/// Host-door registration facts for one face (§44.2 source class +
/// host-derived FontRef metadata).
#[derive(Clone)]
struct ProvidedMeta {
    source: FaceSource,
    family: String,
    style: String,
    postscript_name: String,
}

/// One stored face keyed by its content-addressed asset id. MEMBERSHIP
/// IS A SET, not a single source (review MAJOR-2): the same bytes can
/// legitimately arrive through BOTH doors (a document embedding a font
/// the host also provisions), and resolution must depend on the
/// membership SET — never on arrival order, or the same sidecar
/// renders differently per host (§23.2 pixel-identity, D-48).
#[derive(Clone)]
struct StoredFace {
    asset: Uuid,
    bytes: Vec<u8>,
    /// §44.2 step-2 membership (document truth, the loader door):
    /// `Some(seq)` carries the EMBED-DOOR registration sequence — the
    /// asset-index order step 2 probes in. Sequenced at membership
    /// GRANT time so the step-2 order can never depend on when the
    /// HOST door happened to insert the face (re-review NEW-1: vec
    /// insertion order hoisted a pre-provided dual face ahead of the
    /// document's own embed order — same sidecar, different ink per
    /// host).
    embedded: Option<u64>,
    /// Host-door membership (User/System/Bundled) + metadata.
    provided: Option<ProvidedMeta>,
}

impl StoredFace {
    /// The FontDb registration source. An embedded face registers
    /// DocEmbedded even when also provided: step 2 outranks steps 4/5
    /// for every cluster it covers, so dual membership loses nothing —
    /// and step-2 membership stays document-truth on every host.
    fn effective_source(&self) -> FaceSource {
        if self.embedded.is_some() {
            FaceSource::DocEmbedded
        } else {
            self.provided
                .as_ref()
                .map_or(FaceSource::DocEmbedded, |p| p.source)
        }
    }
    /// Enumeration metadata: host-derived when present (richer), else
    /// the DocEmbedded fallbacks (name-table parsing is C-6).
    fn family(&self) -> String {
        self.provided
            .as_ref()
            .map_or_else(|| "Embedded".into(), |p| p.family.clone())
    }
    fn style_name(&self) -> String {
        self.provided
            .as_ref()
            .map_or_else(|| "Regular".into(), |p| p.style.clone())
    }
    fn ps_name(&self) -> String {
        self.provided.as_ref().map_or_else(
            || format!("embedded-{}", self.asset),
            |p| p.postscript_name.clone(),
        )
    }
}

#[derive(Default)]
struct Inner {
    /// Monotonic embed-door sequence (see `StoredFace::embedded`).
    embed_counter: u64,
    /// Faces in registration order — §44.2 step 2's asset-index order
    /// for the DocEmbedded subset (`ResolveCtx::embedded`), and the
    /// step-5 enumeration order discipline for the rest.
    faces: Vec<StoredFace>,
    /// Font-environment generation; bumps on every face-set change.
    stamp: u64,
}

/// Register THE text-layout service (construction-only; the one
/// `PRODUCTION_CHAIN` row in `appname-assembly` wraps exactly this).
pub fn register(assembly: EngineAssembly) -> EngineAssembly {
    register_with_handle(assembly).0
}

/// Registration variant that also hands back the CONCRETE service —
/// the request-10 door glue's hook: `Arc<TextLayoutService>` coerces to
/// `Arc<dyn TextLayout>` for the assembly while the concrete handle
/// keeps [`TextLayoutService::register_provided_face`] reachable
/// WITHOUT any core trait change (filed with the §44.1 shape reading).
pub fn register_with_handle(assembly: EngineAssembly) -> (EngineAssembly, Arc<TextLayoutService>) {
    let svc = Arc::new(TextLayoutService::default());
    (assembly.with_text_layout(svc.clone()), svc)
}

fn oob(field: &str, msg: impl Into<String>) -> EngineError {
    EngineError::range(field, msg)
}

// ---------------------------------------------------------------------
// Sidecar parse / normalize / emit
// ---------------------------------------------------------------------

fn parse_sidecar(bytes: &[u8]) -> Result<Map<String, Value>, EngineError> {
    let v: Value =
        serde_json::from_slice(bytes).map_err(|e| oob("/", format!("sidecar not JSON: {e}")))?;
    let Value::Object(obj) = v else {
        return Err(oob("/", "sidecar must be a JSON object"));
    };
    if obj.get("schema").and_then(Value::as_str) != Some(TEXT_SIDECAR_SCHEMA) {
        return Err(EngineError::new(
            ErrorCode::WrongSchema,
            format!("text sidecar schema must be {TEXT_SIDECAR_SCHEMA}"),
        ));
    }
    Ok(obj)
}

/// A run minus its `text` — the §23.1 merge identity (style fields AND
/// unknown fields; runs merge only when nothing but text differs).
fn run_identity(run: &Value) -> Map<String, Value> {
    let mut m = run.as_object().cloned().unwrap_or_default();
    m.remove("text");
    m
}

/// Structural equality with numbers compared by VALUE (`100` ≡ `100.0`)
/// so the §23.1 minimal-runs invariant is not JSON-spelling-sensitive
/// (review MINOR-5). Integer pairs compare exactly; mixed int/float
/// pairs compare as f64 (values beyond 2^53 mix-compare lossily — no
/// wire field carries such magnitudes).
fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(i), Some(j)) => i == j,
            _ => match (x.as_u64(), y.as_u64()) {
                (Some(i), Some(j)) => i == j,
                _ => x.as_f64() == y.as_f64(),
            },
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| value_eq(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| value_eq(v, w)))
        }
        _ => a == b,
    }
}

fn identity_eq(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, v)| b.get(k).is_some_and(|w| value_eq(v, w)))
}

/// The layout module's exact mandatory-break set (measure_clusters) —
/// these characters render NOTHING (module-header pin, review MAJOR-1).
fn is_mandatory_break_char(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Document-coordinate bound on layout anchors (re-review NEW-1): 2^24
/// px — beyond any §2.3 canvas while keeping every downstream i32 page
/// computation (compose_page's hull + em margins) far from overflow.
/// Finite-but-huge coordinates refuse typed instead of panicking.
const DOC_COORD_MAX: f64 = 16_777_216.0;

fn coord_ok(v: f64) -> bool {
    v.is_finite() && v.abs() <= DOC_COORD_MAX
}

/// C-6 Optical kerning (module-header pin): normalize each pen-order
/// pair's ink gap toward OPTICAL_TARGET_EM, clamped, on the LEFT
/// glyph's advance. Ink extents come from the SAME coverage rasters
/// compose uses — hint-rule-matched (autohint-light at em ≤ 32 px,
/// unhinted above; spot-check R3-1) — so what the eye sees is what
/// the algorithm measures. Skips: inkless glyphs (spaces —
/// word gaps are tracking/justification territory), zero-advance
/// marks, and cursive-join boundaries (the tracking discriminator:
/// UNSAFE_TO_BREAK gated by Joining_Type).
/// The shared pair-kern skip rule (Manual + Optical, module pins):
/// zero-advance glyphs on EITHER side never kern (a base|mark pair
/// would drift the mark off its anchor — re-review NEW-3), a pair
/// whose clusters straddle a mandatory-break character never kerns
/// (break glyphs are DROPPED after shaping, so buffer-adjacent glyphs
/// can span a line break — re-review NEW-2), and cursive-join
/// boundaries are suppressed by the SAME Joining_Type discriminator
/// as tracking (splitting a join would break shaping).
fn pair_kern_skips(l: &ShapedGlyph, r: &ShapedGlyph, text: &str) -> bool {
    if l.advance == 0.0 || r.advance == 0.0 {
        return true;
    }
    let (lo, hi) = (l.cluster.min(r.cluster), l.cluster.max(r.cluster));
    if text
        .get(lo..hi)
        .is_some_and(|s| s.chars().any(is_mandatory_break_char))
    {
        return true;
    }
    r.unsafe_to_break
        && r.cluster <= text.len()
        && is_cursive_join_boundary(&text[..r.cluster], &text[r.cluster..])
}

/// C-6 Manual kerning (module-header pin, review MAJOR-1 shape): the
/// constant per-pair value on the LEFT glyph's advance at each
/// intra-item boundary — a kerned 2-scalar pair range kerns exactly
/// its ONE internal boundary, never the trailing run edge.
fn apply_manual_kerns(glyphs: &mut [ShapedGlyph], text: &str, kern_px: f64) {
    for i in 0..glyphs.len().saturating_sub(1) {
        let (l, r) = (glyphs[i].clone(), glyphs[i + 1].clone());
        if pair_kern_skips(&l, &r, text) {
            continue;
        }
        glyphs[i].advance += kern_px;
    }
}

/// COST POSTURE (re-review NEW-5, explicit): this pass rasterizes
/// BOTH glyphs of every candidate pair UNCACHED on every layout call
/// (`RasterCache` serves compose only), on top of the recorded
/// per-call engine-construction cost — Optical-kerned runs pay
/// 2×pairs rasterizations per trait call. A (face, glyph, em)-keyed
/// bearing cache is the C-6 perf item's second entry.
fn apply_optical_kerns(
    glyphs: &mut [ShapedGlyph],
    text: &str,
    face: FaceId,
    em: f64,
    rasterizer: &FreeTypeRasterizer,
) {
    for i in 0..glyphs.len().saturating_sub(1) {
        let (l, r) = (glyphs[i].clone(), glyphs[i + 1].clone());
        if pair_kern_skips(&l, &r, text) {
            continue;
        }
        let lc = rasterizer.raster(face, l.glyph, em);
        let rc = rasterizer.raster(face, r.glyph, em);
        if lc.width == 0 || rc.width == 0 {
            continue; // inkless side: no optical pair
        }
        let rsb = l.advance - f64::from(lc.left + lc.width as i32);
        let lsb = f64::from(rc.left);
        let gap = rsb + lsb;
        let kern =
            (OPTICAL_TARGET_EM * em - gap).clamp(-OPTICAL_CLAMP_EM * em, OPTICAL_CLAMP_EM * em);
        glyphs[i].advance += kern;
    }
}

/// The visual LEADING edge of a glyph-bearing line — where a fresh
/// line's caret starts: minimum glyph x under LTR, maximum advance
/// edge under RTL (the reference line carries alignment/indent truth
/// for the caret paths that synthesize positions; re-review NEW-2).
fn leading_edge_of(line: &crate::layout::LaidLine, rtl: bool) -> Option<f64> {
    if line.glyphs.is_empty() {
        return None;
    }
    Some(if rtl {
        line.glyphs
            .iter()
            .map(|g| g.x + g.advance)
            .fold(f64::NEG_INFINITY, f64::max)
    } else {
        line.glyphs
            .iter()
            .map(|g| g.x)
            .fold(f64::INFINITY, f64::min)
    })
}

/// §23.1 run-model invariants + layout-object structure: zero-length
/// runs refuse; adjacent identical-identity runs merge; the layout
/// object must be a known tagged shape with its payload present.
fn normalize(obj: &mut Map<String, Value>) -> Result<(), EngineError> {
    let layout = obj
        .get("layout")
        .and_then(Value::as_object)
        .ok_or_else(|| oob("/layout", "required tagged object {kind, …}"))?;
    let kind = layout
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| oob("/layout/kind", "required enum point|paragraph|on_path"))?;
    match kind {
        "point" => {
            let ok = layout
                .get("origin")
                .and_then(Value::as_array)
                .is_some_and(|a| {
                    a.len() == 2 && a.iter().all(|v| v.as_f64().is_some_and(coord_ok))
                });
            if !ok {
                return Err(oob(
                    "/layout/origin",
                    "point layout requires origin [x, y] within document \
                     coordinates (|v| ≤ 2^24)",
                ));
            }
        }
        "paragraph" => {
            let r = layout
                .get("rect")
                .and_then(Value::as_object)
                .ok_or_else(|| oob("/layout/rect", "paragraph layout requires rect {x,y,w,h}"))?;
            for k in ["x", "y", "w", "h"] {
                if !r.get(k).and_then(Value::as_f64).is_some_and(coord_ok) {
                    return Err(oob(
                        &format!("/layout/rect/{k}"),
                        "required number within document coordinates (|v| ≤ 2^24)",
                    ));
                }
            }
            for k in ["w", "h"] {
                if r.get(k).and_then(Value::as_f64).is_some_and(|v| v < 0.0) {
                    return Err(oob(&format!("/layout/rect/{k}"), "must be non-negative"));
                }
            }
        }
        "on_path" => {
            let ok = layout
                .get("pathId")
                .and_then(Value::as_str)
                .is_some_and(|s| Uuid::parse_str(s).is_ok());
            if !ok {
                return Err(oob(
                    "/layout/pathId",
                    "on_path layout requires a UUID pathId",
                ));
            }
        }
        other => {
            return Err(oob(
                "/layout/kind",
                format!("unknown layout kind {other:?}"),
            ));
        }
    }
    let runs = obj
        .get("runs")
        .and_then(Value::as_array)
        .ok_or_else(|| oob("/runs", "required array (may be empty — an empty layer)"))?
        .clone();
    let mut out: Vec<Value> = Vec::with_capacity(runs.len());
    for (i, r) in runs.iter().enumerate() {
        if !r.is_object() {
            return Err(oob(&format!("/runs/{i}"), "run must be an object"));
        }
        let text = r
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| oob(&format!("/runs/{i}/text"), "required string"))?;
        if text.is_empty() {
            return Err(oob(
                &format!("/runs/{i}/text"),
                "zero-length runs are forbidden (§23.1)",
            ));
        }
        match out.last_mut() {
            Some(prev) if identity_eq(&run_identity(prev), &run_identity(r)) => {
                let joined = format!(
                    "{}{}",
                    prev.get("text").and_then(Value::as_str).unwrap_or(""),
                    text
                );
                prev.as_object_mut()
                    .expect("run object")
                    .insert("text".into(), Value::String(joined));
            }
            _ => out.push(r.clone()),
        }
    }
    obj.insert("runs".into(), Value::Array(out));
    Ok(())
}

/// Canonical emission: BTreeMap key order + exact float round-trip
/// (`float_roundtrip`) make this the validator's byte-exact fixpoint.
fn emit(obj: &Map<String, Value>) -> Vec<u8> {
    serde_json::to_vec_pretty(&Value::Object(obj.clone())).expect("text sidecar serializes")
}

fn scalar_len_of(obj: &Map<String, Value>) -> u64 {
    obj.get("runs")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .map(|t| t.chars().count() as u64)
                .sum()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Wire style → engine style
// ---------------------------------------------------------------------

fn tag4(s: &str) -> Option<[u8; 4]> {
    let b = s.as_bytes();
    (b.len() == 4 && b.iter().all(u8::is_ascii_alphanumeric)).then(|| [b[0], b[1], b[2], b[3]])
}

/// A KNOWN field present with the wrong type or an out-of-range value
/// refuses typed against the WIRE field name (the trait's error
/// contract; review MAJOR-3 + MINOR-8 — internal names never leak).
fn num_field(
    m: &Map<String, Value>,
    key: &str,
    ptr: &str,
    lo: f64,
    hi: f64,
) -> Result<Option<f64>, EngineError> {
    match m.get(key) {
        None => Ok(None),
        Some(v) => {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| oob(&format!("{ptr}/{key}"), "must be a finite number"))?;
            if !(lo..=hi).contains(&n) {
                return Err(oob(
                    &format!("{ptr}/{key}"),
                    format!("{n} outside the §23.4 range {lo}–{hi}"),
                ));
            }
            Ok(Some(n))
        }
    }
}

/// §7.3 FillContent wire validation (known-field strictness; the §22
/// binding composes solid arms, everything else is flagged inert).
fn validate_fill_wire(f: &Value, ptr: &str) -> Result<(), EngineError> {
    let m = f
        .as_object()
        .ok_or_else(|| oob(ptr, "fill must be a FillContent object"))?;
    match m.get("type").and_then(Value::as_str) {
        Some("none") => Ok(()),
        Some("gradient") => m
            .get("gradient")
            .and_then(Value::as_object)
            .map(|_| ())
            .ok_or_else(|| oob(&format!("{ptr}/gradient"), "required object")),
        Some("pattern") => m
            .get("patternRef")
            .and_then(Value::as_str)
            .map(|_| ())
            .ok_or_else(|| oob(&format!("{ptr}/patternRef"), "required string")),
        Some("solid") => {
            let c = m
                .get("color")
                .and_then(Value::as_object)
                .ok_or_else(|| oob(&format!("{ptr}/color"), "required color object"))?;
            match c.get("model").and_then(Value::as_str) {
                Some("gray" | "grayA" | "rgb" | "rgba" | "cmyk" | "cmykA" | "lab" | "labA") => {}
                _ => {
                    return Err(oob(
                        &format!("{ptr}/color/model"),
                        "unknown color model (§7.3 enum)",
                    ));
                }
            }
            let comps_ok = c.get("comps").and_then(Value::as_array).is_some_and(|a| {
                a.len() == 4 && a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))
            });
            if !comps_ok {
                return Err(oob(
                    &format!("{ptr}/color/comps"),
                    "must be 4 finite numbers (§7.3)",
                ));
            }
            Ok(())
        }
        _ => Err(oob(
            &format!("{ptr}/type"),
            "unknown fill type (§7.3 FillContent enum)",
        )),
    }
}

/// One flat `styledRun` object → the §23.4 typed model. Known-field
/// values validate strictly here (wrong type / bad enum / live range →
/// `out_of_range` against the wire field, per the trait contract);
/// unknown fields pass through untouched (§7.7).
fn char_style_of(run: &Value, ptr: &str) -> Result<CharStyle, EngineError> {
    let mut s = CharStyle::default();
    let Some(m) = run.as_object() else {
        return Ok(s);
    };
    if let Some(v) = num_field(m, "size", ptr, 0.1, 1296.0)? {
        s.size_pt = v;
    }
    if let Some(v) = num_field(m, "tracking", ptr, -1000.0, 10_000.0)? {
        s.tracking_permille = v;
    }
    if let Some(v) = num_field(m, "baselineShift", ptr, -s.size_pt, s.size_pt)? {
        s.baseline_shift_pt = v;
    }
    match m.get("kerning") {
        None => {}
        Some(Value::String(k)) if k == "metric" => s.kerning = Kerning::Metric,
        Some(Value::String(k)) if k == "optical" => s.kerning = Kerning::Optical,
        Some(v) if v.as_f64().is_some() => {
            let n = v.as_f64().expect("checked");
            if !n.is_finite() || !(-1000.0..=1000.0).contains(&n) {
                return Err(oob(
                    &format!("{ptr}/kerning"),
                    format!("{n} outside the §23.4 Manual range ±1000 ‰"),
                ));
            }
            s.kerning = Kerning::Manual(n);
        }
        Some(_) => {
            return Err(oob(
                &format!("{ptr}/kerning"),
                "kerning must be \"metric\", \"optical\", or a ‰-em number",
            ));
        }
    }
    match m.get("language") {
        None => {}
        Some(Value::String(v)) => s.language = Some(v.clone()),
        Some(_) => return Err(oob(&format!("{ptr}/language"), "must be a BCP 47 string")),
    }
    match m.get("font") {
        None | Some(Value::String(_)) | Some(Value::Object(_)) => {}
        Some(_) => {
            return Err(oob(
                &format!("{ptr}/font"),
                "FontRef must be a family string or an object (§23.5)",
            ));
        }
    }
    if let Some(v) = m.get("fill") {
        validate_fill_wire(v, &format!("{ptr}/fill"))?;
    }
    match m.get("features") {
        None => {}
        Some(Value::Object(f)) => {
            for (k, v) in f {
                let tag = tag4(k)
                    .ok_or_else(|| oob(&format!("{ptr}/features"), format!("bad tag {k:?}")))?;
                let on = v
                    .as_bool()
                    .ok_or_else(|| oob(&format!("{ptr}/features/{k}"), "must be a boolean"))?;
                s.features.push((tag, u32::from(on)));
            }
        }
        Some(_) => return Err(oob(&format!("{ptr}/features"), "must be a tag→bool object")),
    }
    match m.get("variations") {
        None => {}
        Some(Value::Object(f)) => {
            for (k, v) in f {
                let tag = tag4(k)
                    .ok_or_else(|| oob(&format!("{ptr}/variations"), format!("bad tag {k:?}")))?;
                let n = v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| {
                    oob(&format!("{ptr}/variations/{k}"), "must be a finite number")
                })?;
                // §44.3 axis coords are 32-bit: a value that cannot
                // survive the f32 narrowing refuses instead of turning
                // into ∞ downstream (re-review NEW-5).
                if n.abs() > f64::from(f32::MAX) {
                    return Err(oob(
                        &format!("{ptr}/variations/{k}"),
                        "outside the 32-bit axis-coordinate range (§44.3)",
                    ));
                }
                s.variations.push((tag, n as f32));
            }
        }
        Some(_) => {
            return Err(oob(
                &format!("{ptr}/variations"),
                "must be a tag→number object",
            ));
        }
    }
    // `font` does not resolve in v1 — no installed faces to match
    // (§3.10); the §44.2 chain covers embedded + tofu. Name matching
    // lands with the system scan (C-6). The typed-model belt check
    // cannot fire after the wire checks above (defensive).
    debug_assert!(s.validate().is_ok());
    Ok(s)
}

/// `paragraphStyle` wire → the §23.4 typed model — strict on known
/// fields (wrong type / unknown enum / range → typed refusal, the
/// trait's error contract; review MAJOR-3), unknown fields preserved
/// upstream (§7.7).
fn paragraph_style_of(obj: &Map<String, Value>, text: &str) -> Result<ParagraphStyle, EngineError> {
    let mut p = ParagraphStyle::default();
    let m = match obj.get("paragraph") {
        None => None,
        Some(Value::Object(m)) => Some(m),
        Some(_) => return Err(oob("/paragraph", "must be a paragraphStyle object")),
    };
    let Some(m) = m else {
        p.rtl_base = !text.is_empty() && BidiInfo::new(text, None).paragraphs[0].level.is_rtl();
        return Ok(p);
    };
    let enum_of = |key: &str| -> Result<Option<&str>, EngineError> {
        match m.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(oob(&format!("/paragraph/{key}"), "must be an enum string")),
        }
    };
    p.align = match enum_of("alignment")? {
        None | Some("left") => Align::Start,
        Some("center") => Align::Center,
        Some("right") => Align::End,
        Some("justify") => Align::Justify,
        Some(other) => {
            return Err(oob(
                "/paragraph/alignment",
                format!("unknown alignment {other:?} (left|center|right|justify)"),
            ));
        }
    };
    p.justify_last = match enum_of("justifyLastLine")? {
        None | Some("left") => JustifyLast::Start,
        Some("center") => JustifyLast::Center,
        Some("right") => JustifyLast::End,
        Some("full") => JustifyLast::Full,
        Some(other) => {
            return Err(oob(
                "/paragraph/justifyLastLine",
                format!("unknown value {other:?} (left|center|right|full)"),
            ));
        }
    };
    if let Some(v) = num_field(m, "autoLeadingPct", "/paragraph", 20.0, 500.0)? {
        p.auto_leading_pct = v / 100.0;
    }
    if let Some(v) = num_field(m, "indentLeft", "/paragraph", -1296.0, 1296.0)? {
        p.indent_left = v;
    }
    if let Some(v) = num_field(m, "indentRight", "/paragraph", -1296.0, 1296.0)? {
        p.indent_right = v;
    }
    if let Some(v) = num_field(m, "indentFirstLine", "/paragraph", -1296.0, 1296.0)? {
        p.indent_first = v;
    }
    if let Some(v) = num_field(m, "spaceBefore", "/paragraph", 0.0, 1296.0)? {
        p.space_before = v;
    }
    if let Some(v) = num_field(m, "spaceAfter", "/paragraph", 0.0, 1296.0)? {
        p.space_after = v;
    }
    if let Some(h) = m.get("hyphenation") {
        if !h.is_boolean() {
            return Err(oob(
                "/paragraph/hyphenation",
                "must be a boolean (reserved)",
            ));
        }
    }
    p.rtl_base = match enum_of("direction")? {
        Some("rtl") => true,
        Some("ltr") => false,
        // auto (default): UAX #9 P2/P3 over the text.
        None | Some("auto") => {
            !text.is_empty() && BidiInfo::new(text, None).paragraphs[0].level.is_rtl()
        }
        Some(other) => {
            return Err(oob(
                "/paragraph/direction",
                format!("unknown direction {other:?} (auto|ltr|rtl)"),
            ));
        }
    };
    Ok(p)
}

/// sRGB EOTF (§4.4: compositing is premultiplied LINEAR).
fn srgb_to_linear(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// THE fill-conversion surface (EXPORTED — O-INT composite-leg ruling,
/// the C4/kernel-seam precedent: one math library, conditioning never
/// re-derived consumer-side). §7.3 `FillContent` JSON → STRAIGHT
/// (non-premultiplied) linear working-space RGBA via the sRGB EOTF.
/// Arms (the shared v1 posture, pinned in the module header): absent
/// fill = opaque black; `none` = clear; solid gray/grayA/rgb/rgba
/// convert; solid cmyk/lab await the §5 CMM and gradient/pattern the
/// §22 binding — all four compose as opaque black, FLAGGED. The core
/// tile-installation legs consume this + [`premultiply_coverage`] so
/// text and shape fills share every constant.
///
/// TOTAL-FUNCTION POSTURE (review MINOR-4; wording per re-review
/// NEW-4): this surface NEVER refuses. An unknown `type`, non-object,
/// or unknown color model falls to the opaque-black arm like the
/// flagged cmyk/gradient classes; a NON-NUMERIC COMP reads 0.0 IN ITS
/// POSITION (never shifting later comps across channels), so e.g. a
/// non-numeric ALPHA comp yields a CLEAR fill, not black. VALIDATION
/// IS NOT THIS FUNCTION'S JOB: in-engine, fills are strictly
/// validated at the sidecar gate (`validate_fill_wire`) before any
/// composition; a consumer that wants malformed fills REFUSED must
/// gate the same way and not rely on output pixels to signal errors.
pub fn fill_to_linear_rgba(fill: Option<&Value>) -> [f32; 4] {
    page_fill_of(fill)
}

/// The §4.4 premultiply step over one coverage sample: straight linear
/// RGBA × coverage → premultiplied linear RGBA (the exported second
/// half of the fill math; see [`fill_to_linear_rgba`]). Coverage is
/// CLAMPED to [0, 1] (review NOTE-5: the §13.1/§20.2 plane convention
/// is the domain; out-of-range samples never mint alpha > 1).
pub fn premultiply_coverage(rgba: [f32; 4], coverage: f32) -> [f32; 4] {
    let a = rgba[3] * coverage.clamp(0.0, 1.0);
    [rgba[0] * a, rgba[1] * a, rgba[2] * a, a]
}

/// FillContent → straight linear RGBA (module-header fill pins).
fn page_fill_of(fill: Option<&Value>) -> [f32; 4] {
    let black = [0.0, 0.0, 0.0, 1.0];
    let Some(f) = fill else { return black };
    match f.get("type").and_then(Value::as_str) {
        Some("none") => [0.0, 0.0, 0.0, 0.0],
        Some("solid") => {
            // POSITION-PRESERVING: a non-numeric comp reads 0.0 in
            // place — filter_map would SHIFT later comps into earlier
            // channels (fix-round catch on the total-function posture).
            let comps: Vec<f64> = f
                .get("color")
                .and_then(|c| c.get("comps"))
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
                .unwrap_or_default();
            let c = |i: usize| comps.get(i).copied().unwrap_or(0.0);
            let (r, g, b, a) = match f
                .get("color")
                .and_then(|c| c.get("model"))
                .and_then(Value::as_str)
            {
                Some("gray") => (c(0), c(0), c(0), 1.0),
                Some("grayA") => (c(0), c(0), c(0), c(1)),
                Some("rgb") => (c(0), c(1), c(2), 1.0),
                Some("rgba") => (c(0), c(1), c(2), c(3)),
                // cmyk/lab need the §5 CMM — opaque black pending (flag).
                _ => return black,
            };
            [
                srgb_to_linear(r) as f32,
                srgb_to_linear(g) as f32,
                srgb_to_linear(b) as f32,
                a.clamp(0.0, 1.0) as f32,
            ]
        }
        // gradient/pattern await the §22 FillContent binding (flag).
        _ => black,
    }
}

// ---------------------------------------------------------------------
// The per-call engine build
// ---------------------------------------------------------------------

struct Laid {
    text: String,
    /// Byte span of each run in `text` (index-aligned with `styles`).
    run_spans: Vec<Range<usize>>,
    styles: Vec<CharStyle>,
    /// Per-run straight linear RGBA fills (index-aligned with
    /// `run_spans`) — the per-run compositing contract.
    fills: Vec<[f32; 4]>,
    items: Vec<StyledItem>,
    /// UAX #29 grapheme boundaries of `text`.
    bounds: Vec<usize>,
    layout: ParagraphLayout,
    rasterizer: FreeTypeRasterizer,
    /// The layout anchor (point origin, or the paragraph rect's
    /// top-left) — the empty-layer caret sits here.
    anchor: (f64, f64),
}

impl Laid {
    fn style_of_byte(&self, byte: usize) -> CharStyle {
        self.run_spans
            .iter()
            .position(|r| r.contains(&byte))
            .and_then(|i| self.styles.get(i))
            .cloned()
            .unwrap_or_default()
    }

    fn item_of_byte(&self, byte: usize) -> Option<&StyledItem> {
        self.items.iter().find(|si| si.item.range.contains(&byte))
    }

    /// Grapheme boundary AFTER `byte` (trailing edge of its cluster).
    fn next_boundary(&self, byte: usize) -> usize {
        self.bounds
            .iter()
            .copied()
            .find(|&b| b > byte)
            .unwrap_or(self.text.len())
    }

    fn scalar_of_byte(&self, byte: usize) -> u64 {
        self.text[..byte.min(self.text.len())].chars().count() as u64
    }
}

fn build_and_layout(
    embedded: &[StoredFace],
    obj: &Map<String, Value>,
) -> Result<Laid, EngineError> {
    let layout_obj = obj
        .get("layout")
        .and_then(Value::as_object)
        .expect("normalized");
    let kind = layout_obj
        .get("kind")
        .and_then(Value::as_str)
        .expect("normalized");
    if kind == "on_path" {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "text on_path layout: the seam carries pathId only — binding needs the \
             arm-threaded path payload (module-header pin; follow-up filing)",
        ));
    }

    let runs = obj
        .get("runs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    let mut run_spans = Vec::with_capacity(runs.len());
    let mut styles = Vec::with_capacity(runs.len());
    let mut fills = Vec::with_capacity(runs.len());
    for (i, r) in runs.iter().enumerate() {
        let t = r.get("text").and_then(Value::as_str).unwrap_or("");
        let start = text.len();
        text.push_str(t);
        run_spans.push(start..text.len());
        styles.push(char_style_of(r, &format!("/runs/{i}"))?);
        fills.push(page_fill_of(r.get("fill")));
    }

    let mut rasterizer =
        FreeTypeRasterizer::new().map_err(|e| oob("/", format!("freetype init: {e}")))?;
    let mut db = FontDb::new();
    // §44.2 step 2 consults ONLY the DocEmbedded subset; provided
    // User/System/Bundled faces enter the index for the resolver's
    // step-3/4/5 arms by their source tag.
    let mut embedded_ids = Vec::new();
    for (i, f) in embedded.iter().enumerate() {
        let id = FaceId(i as u32 + 1);
        let info = FaceInfo {
            family: f.family(),
            style: f.style_name(),
            postscript_name: f.ps_name(),
            source: f.effective_source(),
            cjk_variant: None,
        };
        // Both halves were validated at registration; a failure here is
        // defensive. RASTERIZER FIRST (review NOTE-6): a face must
        // never sit in the db as a resolve candidate without raster
        // backing — the whole face drops out of the chain instead.
        if rasterizer.add_face(id, f.bytes.clone()).is_ok()
            && db.register(id, &f.bytes, info).is_ok()
        {
            if let Some(seq) = f.embedded {
                embedded_ids.push((seq, id));
            }
        }
    }
    // §44.2 step-2 probe order = the EMBED door's sequence, never the
    // faces-vec insertion order (re-review NEW-1).
    embedded_ids.sort_unstable_by_key(|&(seq, _)| seq);
    let face_ids: Vec<FaceId> = embedded_ids.into_iter().map(|(_, id)| id).collect();

    let anchor = match kind {
        "point" => {
            let a = layout_obj
                .get("origin")
                .and_then(Value::as_array)
                .expect("normalized");
            (
                a[0].as_f64().expect("normalized"),
                a[1].as_f64().expect("normalized"),
            )
        }
        _ => {
            let r = layout_obj
                .get("rect")
                .and_then(Value::as_object)
                .expect("normalized");
            let g = |k: &str| r.get(k).and_then(Value::as_f64).expect("normalized");
            (g("x"), g("y"))
        }
    };

    let para = paragraph_style_of(obj, &text)?;

    if text.is_empty() {
        return Ok(Laid {
            text,
            run_spans,
            styles,
            fills,
            items: Vec::new(),
            bounds: vec![0],
            layout: ParagraphLayout {
                lines: Vec::new(),
                overflowed: false,
                first_hidden: None,
                bottom: anchor.1,
            },
            rasterizer,
            anchor,
        });
    }

    let scripts = IcuScriptClassifier::new();
    let ctx = ResolveCtx {
        embedded: face_ids,
        run_language: styles.first().and_then(|s| s.language.clone()),
        ..Default::default()
    };
    let probe = DbProbe {
        db: &db,
        ctx: &ctx,
        scripts: &scripts,
    };
    let spans: Vec<RunSpan<'_>> = run_spans
        .iter()
        .enumerate()
        .map(|(i, r)| RunSpan {
            text: &text[r.clone()],
            style: StyleId(i as u32),
        })
        .collect();
    let segmenter = IcuGraphemeSegmenter::new();
    let items = itemize(&spans, &scripts, &UnicodeBidiProvider, &segmenter, &probe);

    // One shaper per distinct em (fixed-em shaper design); every
    // embedded face registers into each.
    let mut shapers: HashMap<u64, HarfBuzzShaper> = HashMap::new();
    let styled: Vec<StyledItem> = items
        .into_iter()
        .map(|item| {
            let style = &styles[item.style.0 as usize];
            let em = style.em_px(72.0); // PPI-72 pin (module header)
            let shaper = shapers.entry(em.to_bits()).or_insert_with(|| {
                let mut sh = HarfBuzzShaper::new(em);
                for (i, f) in embedded.iter().enumerate() {
                    let bytes = &f.bytes;
                    let _ = sh.add_face(FaceId(i as u32 + 1), bytes);
                }
                sh
            });
            shaper.set_style(item.style, style);
            let (mut glyphs, metrics) = if item.face == TOFU_FACE {
                (
                    tofu_shape(&item, &text, em),
                    FaceMetricsPx {
                        ascent: TOFU_ASCENT_EM * em,
                        descent: TOFU_DESCENT_EM * em,
                        line_gap: 0.0,
                    },
                )
            } else {
                (
                    shaper.shape(&item, &text),
                    rasterizer.metrics(item.face, em),
                )
            };
            // Mandatory-break characters render NOTHING (module-header
            // pin, review MAJOR-1): drop their glyphs so they carry no
            // advance, no ink, no caret width, and no outline — the
            // cluster keeps its mandatory_after break role in layout.
            glyphs.retain(|g| {
                !text[g.cluster..]
                    .chars()
                    .next()
                    .is_some_and(is_mandatory_break_char)
            });
            // Manual/Optical kerning (module-header pins): both apply
            // on the LEFT glyph's advance at intra-item pair
            // boundaries — never the run's trailing edge (review
            // MAJOR-1); tracking stays the gap machinery's alone.
            match style.kerning {
                Kerning::Manual(k) => {
                    apply_manual_kerns(&mut glyphs, &text, k / 1000.0 * em);
                }
                Kerning::Optical if item.face != TOFU_FACE => {
                    apply_optical_kerns(&mut glyphs, &text, item.face, em, &rasterizer);
                }
                _ => {}
            }
            StyledItem {
                item,
                glyphs,
                em_px: em,
                metrics,
                tracking_px: style.tracking_px(em),
            }
        })
        .collect();

    let bounds = segmenter.cluster_boundaries(&text);
    let layout = match kind {
        "point" => {
            let align = match para.align {
                Align::Center => PointAlign::Center,
                Align::End => PointAlign::Right,
                _ => PointAlign::Left,
            };
            layout_point_text(&text, &styled, &bounds, &para, anchor, align)
        }
        _ => {
            let r = layout_obj
                .get("rect")
                .and_then(Value::as_object)
                .expect("normalized");
            let g = |k: &str| r.get(k).and_then(Value::as_f64).expect("normalized");
            let box_ = LayoutBox {
                left: g("x"),
                top: g("y"),
                right: g("x") + g("w"),
                bottom: g("y") + g("h"),
            };
            let opps = IcuLineBreakProvider::new().opportunities(&text);
            // spaceBefore stacks at the block's top edge (whole-text-
            // paragraph pin).
            layout_paragraph(
                &text,
                &styled,
                &bounds,
                &opps,
                &para,
                &box_,
                box_.top + para.space_before,
            )
        }
    };

    Ok(Laid {
        text,
        run_spans,
        styles,
        fills,
        items: styled,
        bounds,
        layout,
        rasterizer,
        anchor,
    })
}

/// Zero-geometry decoration (v1 wire carries no underline/strikethrough
/// fields, so decoration never draws — the closures are inert).
fn no_decor(_f: FaceId, _e: f64) -> DecorLine {
    DecorLine {
        position: 0.0,
        thickness: 0.0,
    }
}

/// Compose ONE layout (the full one, or a per-run filtered subset)
/// with the style facts resolved by cluster byte.
fn compose_layout(laid: &Laid, layout: &ParagraphLayout) -> CoveragePage {
    let run_spans = laid.run_spans.clone();
    let styles = laid.styles.clone();
    let style_at = move |byte: usize| -> CharStyle {
        run_spans
            .iter()
            .position(|r| r.contains(&byte))
            .and_then(|i| styles.get(i))
            .cloned()
            .unwrap_or_default()
    };
    let style_em = style_at.clone();
    let mut cache = RasterCache::new(8 << 20);
    compose_page(
        layout,
        &laid.rasterizer,
        &mut cache,
        move |l, gi| style_em(l.glyphs[gi].cluster).em_px(72.0),
        move |l, gi| {
            let s = style_at(l.glyphs[gi].cluster);
            ItemRaster {
                em_scale: 1.0,
                // pt ≡ px at the PPI-72 pin; positive-up per §23.4.
                baseline_shift: s.baseline_shift_pt,
                faux_italic: false,
                underline: false,
                strikethrough: false,
            }
        },
        no_decor,
        no_decor,
    )
}

/// The same layout restricted to one run's glyphs (per-run fill
/// compositing; line boxes/baselines unchanged).
fn layout_for_span(layout: &ParagraphLayout, span: &Range<usize>) -> ParagraphLayout {
    ParagraphLayout {
        lines: layout
            .lines
            .iter()
            .map(|l| crate::layout::LaidLine {
                range: l.range.clone(),
                baseline: l.baseline,
                width: l.width,
                glyphs: l
                    .glyphs
                    .iter()
                    .filter(|g| span.contains(&g.cluster))
                    .cloned()
                    .collect(),
            })
            .collect(),
        overflowed: layout.overflowed,
        first_hidden: layout.first_hidden,
        bottom: layout.bottom,
    }
}

fn compose_of(laid: &Laid) -> (CoveragePage, LaidMetrics) {
    let page = compose_layout(laid, &laid.layout);
    let bounds = ink_hull(&page);
    let metrics = LaidMetrics {
        bounds,
        line_count: laid.layout.lines.len() as u32,
        overflowed: laid.layout.overflowed,
    };
    (page, metrics)
}

/// Nonzero-coverage integer hull (the §40.7 laid-out-bounds pin).
fn ink_hull(page: &CoveragePage) -> IRect {
    if page.width == 0 || page.height == 0 {
        return IRect::EMPTY;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let w = page.width as i32;
    for (idx, &v) in page.data.iter().enumerate() {
        if v > 0.0 {
            let x = page.left + (idx as i32 % w);
            let y = page.top + (idx as i32 / w);
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + 1);
            max_y = max_y.max(y + 1);
        }
    }
    if min_x == i32::MAX {
        IRect::EMPTY
    } else {
        IRect::new(min_x, min_y, max_x, max_y)
    }
}

// ---------------------------------------------------------------------
// Outline conversion (Door 9's engine half)
// ---------------------------------------------------------------------

/// FT outline commands (px, y-up, glyph-origin-relative) → §20.1
/// subpaths in doc space at pen (dx, dy). Quadratics elevate exactly
/// (c1 = p0 + ⅔(q−p0), c2 = p1 + ⅔(q−p1)); contiguity is bitwise by
/// p0-chaining; `normalize` downstream removes degenerates and drops
/// emptied contours.
fn cmds_to_subpaths(cmds: &[OutlineCmd], dx: f64, dy: f64) -> Vec<SubPath> {
    let at = |p: [f64; 2]| Vec2::new(dx + p[0], dy - p[1]);
    let mut subs = Vec::new();
    let mut segs: Vec<CubicSeg> = Vec::new();
    let mut cur = Vec2::new(0.0, 0.0);
    let mut start = cur;
    for cmd in cmds {
        match *cmd {
            OutlineCmd::MoveTo(p) => {
                cur = at(p);
                start = cur;
            }
            OutlineCmd::LineTo(p) => {
                let p1 = at(p);
                segs.push(CubicSeg::line(cur, p1));
                cur = p1;
            }
            OutlineCmd::QuadTo(q, p) => {
                let (q, p1) = (at(q), at(p));
                let c1 = Vec2::new(
                    cur.x + 2.0 / 3.0 * (q.x - cur.x),
                    cur.y + 2.0 / 3.0 * (q.y - cur.y),
                );
                let c2 = Vec2::new(
                    p1.x + 2.0 / 3.0 * (q.x - p1.x),
                    p1.y + 2.0 / 3.0 * (q.y - p1.y),
                );
                segs.push(CubicSeg::new(cur, c1, c2, p1));
                cur = p1;
            }
            OutlineCmd::CubicTo(c1, c2, p) => {
                let p1 = at(p);
                segs.push(CubicSeg::new(cur, at(c1), at(c2), p1));
                cur = p1;
            }
            OutlineCmd::Close => {
                if !segs.is_empty() {
                    if cur != start {
                        segs.push(CubicSeg::line(cur, start));
                    }
                    subs.push(SubPath {
                        segments: std::mem::take(&mut segs),
                        closed: true,
                    });
                }
                cur = start;
            }
        }
    }
    // Defensive flush: `glyph_outline_px` closes every contour, but a
    // future caller ending mid-contour must not silently lose ink —
    // emit the tail as an OPEN subpath (review NOTE-10).
    if !segs.is_empty() {
        subs.push(SubPath {
            segments: segs,
            closed: false,
        });
    }
    subs
}

// ---------------------------------------------------------------------
// The trait implementation
// ---------------------------------------------------------------------

impl TextLayoutService {
    fn normalized(&self, bytes: &[u8]) -> Result<Map<String, Value>, EngineError> {
        let mut obj = parse_sidecar(bytes)?;
        normalize(&mut obj)?;
        Ok(obj)
    }

    /// Lock the state, RECOVERING from poisoning (review MINOR-9): the
    /// state is plain bytes + a counter, valid under any interleaving —
    /// one panicked call must not convert every later call into a
    /// panic on a seam that promises typed errors.
    fn state(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn laid_of(&self, obj: &Map<String, Value>) -> Result<Laid, EngineError> {
        let embedded = self.state().faces.clone();
        build_and_layout(&embedded, obj)
    }

    fn result_of(&self, obj: Map<String, Value>) -> Result<TextLayoutResult, EngineError> {
        let laid = self.laid_of(&obj)?;
        let (_page, metrics) = compose_of(&laid);
        Ok(TextLayoutResult {
            sidecar: emit(&obj),
            bounds: metrics.bounds,
        })
    }

    fn check_range(obj: &Map<String, Value>, range: (u64, u64)) -> Result<(), EngineError> {
        let len = scalar_len_of(obj);
        if range.0 > range.1 || range.1 > len {
            return Err(oob(
                "/range",
                format!(
                    "family rule 0 ≤ {} ≤ {} ≤ {len} violated (Unicode scalar values)",
                    range.0, range.1
                ),
            ));
        }
        Ok(())
    }
}

impl TextLayout for TextLayoutService {
    fn schema(&self) -> &'static str {
        TEXT_SIDECAR_SCHEMA
    }

    fn validate_sidecar(&self, bytes: &[u8]) -> Result<TextLayoutResult, EngineError> {
        let obj = self.normalized(bytes)?;
        self.result_of(obj)
    }

    fn sidecar_from_add_params(
        &self,
        params: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError> {
        let kind = params
            .get("layout")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("layout"))?;
        let mut layout = Map::new();
        layout.insert("kind".into(), Value::String(kind.into()));
        for k in ["origin", "rect", "pathId"] {
            if let Some(v) = params.get(k) {
                layout.insert(k.into(), v.clone());
            }
        }
        let mut obj = Map::new();
        obj.insert("schema".into(), Value::String(TEXT_SIDECAR_SCHEMA.into()));
        obj.insert("layout".into(), Value::Object(layout));
        let text = params.get("text").and_then(Value::as_str).unwrap_or("");
        let runs = if text.is_empty() {
            Vec::new() // empty layer (create-then-type pin)
        } else {
            // The flat styledRun: text + the style object's fields.
            let mut run = Map::new();
            run.insert("text".into(), Value::String(text.into()));
            if let Some(style) = params.get("style").and_then(Value::as_object) {
                for (k, v) in style {
                    run.insert(k.clone(), v.clone());
                }
            }
            vec![Value::Object(run)]
        };
        obj.insert("runs".into(), Value::Array(runs));
        if let Some(p) = params.get("paragraph") {
            obj.insert("paragraph".into(), p.clone());
        }
        normalize(&mut obj)?;
        self.result_of(obj)
    }

    fn apply_edit(
        &self,
        current: &[u8],
        params: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError> {
        let mut obj = self.normalized(current)?;
        // §10.3 full-state runs replacement; absent style FIELDS default
        // from the layer's current first run (trait contract note).
        let inherit: Map<String, Value> = obj
            .get("runs")
            .and_then(Value::as_array)
            .and_then(|r| r.first())
            .map(run_identity)
            .unwrap_or_default();
        let new_runs = params
            .get("runs")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("runs"))?;
        let filled: Vec<Value> = new_runs
            .iter()
            .map(|r| {
                let mut m = r.as_object().cloned().unwrap_or_default();
                for k in STYLE_FIELDS {
                    if !m.contains_key(k) {
                        if let Some(v) = inherit.get(k) {
                            m.insert(k.into(), v.clone());
                        }
                    }
                }
                Value::Object(m)
            })
            .collect();
        obj.insert("runs".into(), Value::Array(filled));
        // Complete-value replacements (full-state rule).
        if let Some(l) = params.get("layout") {
            obj.insert("layout".into(), l.clone());
        }
        if let Some(p) = params.get("paragraph") {
            obj.insert("paragraph".into(), p.clone());
        }
        normalize(&mut obj)?;
        self.result_of(obj)
    }

    fn apply_style_range(
        &self,
        current: &[u8],
        range: (u64, u64),
        style: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError> {
        let mut obj = self.normalized(current)?;
        Self::check_range(&obj, range)?;
        let runs = obj.get("runs").and_then(Value::as_array).cloned().unwrap();
        let mut out: Vec<Value> = Vec::new();
        let mut pos: u64 = 0;
        for r in &runs {
            let text = r.get("text").and_then(Value::as_str).unwrap_or("");
            let n = text.chars().count() as u64;
            let (rs, re) = (pos, pos + n);
            pos = re;
            let (os, oe) = (range.0.max(rs), range.1.min(re));
            if os >= oe {
                out.push(r.clone());
                continue;
            }
            let byte_at = |scalars: u64| {
                text.char_indices()
                    .nth(scalars as usize)
                    .map_or(text.len(), |(b, _)| b)
            };
            let (bs, be) = (byte_at(os - rs), byte_at(oe - rs));
            let mut push = |t: &str, patched: bool| {
                if t.is_empty() {
                    return;
                }
                let mut m = r.as_object().cloned().unwrap_or_default();
                m.insert("text".into(), Value::String(t.to_owned()));
                if patched {
                    // Each present field carries its complete new value
                    // (§10.3 patch semantics), flat on the run.
                    for (k, v) in style {
                        m.insert(k.clone(), v.clone());
                    }
                }
                out.push(Value::Object(m));
            };
            push(&text[..bs], false);
            push(&text[bs..be], true);
            push(&text[be..], false);
        }
        obj.insert("runs".into(), Value::Array(out));
        normalize(&mut obj)?; // re-merge (§23.1 minimal-run invariant)
        self.result_of(obj)
    }

    fn apply_paragraph_range(
        &self,
        current: &[u8],
        range: (u64, u64),
        paragraph: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError> {
        let mut obj = self.normalized(current)?;
        Self::check_range(&obj, range)?;
        // v1: ONE ParagraphStyle per layer (§23.1) — any intersecting
        // range patches it; each present field carries its new value.
        let mut pm = obj
            .get("paragraph")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (k, v) in paragraph {
            pm.insert(k.clone(), v.clone());
        }
        obj.insert("paragraph".into(), Value::Object(pm));
        self.result_of(obj)
    }

    fn content_len(&self, sidecar: &[u8]) -> Result<u64, EngineError> {
        Ok(scalar_len_of(&self.normalized(sidecar)?))
    }

    fn layout_and_compose(
        &self,
        sidecar: &[u8],
    ) -> Result<(TextPageData, LaidMetrics), EngineError> {
        let obj = self.normalized(sidecar)?;
        let laid = self.laid_of(&obj)?;
        // The full-coverage page fixes the union rect + metrics; each
        // run then composes its OWN coverage and composites src-over in
        // run order through its OWN FillContent (the TextPageData
        // contract; review MAJOR-2).
        let (page, metrics) = compose_of(&laid);
        let rect = IRect::new(
            page.left,
            page.top,
            page.left + page.width as i32,
            page.top + page.height as i32,
        );
        let mut pixels = vec![[0.0f32; 4]; page.data.len()];
        for (ri, span) in laid.run_spans.iter().enumerate() {
            let [r, g, b, a] = laid.fills[ri];
            let sub = layout_for_span(&laid.layout, span);
            if sub.lines.iter().all(|l| l.glyphs.is_empty()) {
                continue;
            }
            let run_page = compose_layout(&laid, &sub);
            let rw = run_page.width as i32;
            for (idx, &cov) in run_page.data.iter().enumerate() {
                if cov <= 0.0 {
                    continue;
                }
                let x = run_page.left + (idx as i32 % rw) - page.left;
                let y = run_page.top + (idx as i32 / rw) - page.top;
                if x < 0 || y < 0 || x >= page.width as i32 || y >= page.height as i32 {
                    continue;
                }
                let dst = &mut pixels[(y * page.width as i32 + x) as usize];
                // src-over, premultiplied (§4.4): out = src + dst·(1−αs)
                // — through the ONE exported premultiply (single-source
                // with the core tile-installation legs).
                let src = premultiply_coverage([r, g, b, a], cov);
                let sa = src[3];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = s + *d * (1.0 - sa);
                }
            }
        }
        Ok((TextPageData { rect, pixels }, metrics))
    }

    fn outline_geometry(&self, sidecar: &[u8]) -> Result<Vec<u8>, EngineError> {
        let obj = self.normalized(sidecar)?;
        let mut laid = self.laid_of(&obj)?;
        let mut path = VectorPath::new(PathId::from_uuid(Uuid::nil()), FillRule::NonZero);
        let lines = laid.layout.lines.clone();
        for line in &lines {
            for g in &line.glyphs {
                if g.face == TOFU_FACE {
                    continue; // the §23.5 hex box is raster-only
                }
                let em = laid.style_of_byte(g.cluster).em_px(72.0);
                let cmds = laid.rasterizer.glyph_outline_px(g.face, g.glyph, em);
                path.subpaths.extend(cmds_to_subpaths(&cmds, g.x, g.y));
            }
        }
        if path.subpaths.is_empty() {
            return Err(oob(
                "/",
                "no outline geometry: the layer is empty or fully tofu (§23.5 — \
                 tofu carries no outlines)",
            ));
        }
        path.normalize()
            .map_err(|e| oob("/", format!("outline normalize: {e:?}")))?;
        path_to_sidecar_json(&path).map_err(|e| oob("/", format!("outline emit: {e:?}")))
    }

    fn caret_geometry(
        &self,
        sidecar: &[u8],
        scalar_index: u64,
    ) -> Result<Vec<CaretRect>, EngineError> {
        let obj = self.normalized(sidecar)?;
        let laid = self.laid_of(&obj)?;
        let len = laid.text.chars().count() as u64;
        if scalar_index > len {
            return Err(oob(
                "/index",
                format!("caret index {scalar_index} exceeds content length {len}"),
            ));
        }
        if laid.layout.lines.is_empty() {
            let em = laid
                .styles
                .first()
                .map_or(DEFAULT_SIZE_PT, |s| s.em_px(72.0));
            return Ok(vec![CaretRect {
                x: laid.anchor.0,
                top: laid.anchor.1 - TOFU_ASCENT_EM * em,
                bottom: laid.anchor.1 + TOFU_DESCENT_EM * em,
                primary: true,
            }]);
        }
        let pstyle = paragraph_style_of(&obj, &laid.text)?;
        // Cluster-snap (§23.2), then locate the caret's line.
        let raw_byte = laid
            .text
            .char_indices()
            .nth(scalar_index as usize)
            .map_or(laid.text.len(), |(b, _)| b);
        let byte = laid
            .bounds
            .iter()
            .copied()
            .rfind(|&b| b <= raw_byte)
            .unwrap_or(0);
        // The caret AFTER a trailing mandatory break starts the NEXT
        // line, which does not exist yet — synthesize it one §23.3
        // leading below the last baseline at the leading edge
        // (module-header pin; the create-then-type flow).
        if byte == laid.text.len()
            && laid
                .text
                .chars()
                .next_back()
                .is_some_and(is_mandatory_break_char)
        {
            let last = laid.layout.lines.last().expect("nonempty lines");
            let (asc, desc, gap, em) = last
                .glyphs
                .last()
                .and_then(|g| laid.item_of_byte(g.cluster))
                .or_else(|| laid.items.first())
                .map_or(
                    (
                        DEFAULT_SIZE_PT * TOFU_ASCENT_EM,
                        DEFAULT_SIZE_PT * TOFU_DESCENT_EM,
                        0.0,
                        DEFAULT_SIZE_PT,
                    ),
                    |si| {
                        (
                            si.metrics.ascent,
                            si.metrics.descent,
                            si.metrics.line_gap,
                            si.em_px,
                        )
                    },
                );
            // The §23.3 auto-leading rule (line_metrics' formula).
            let leading = match pstyle.leading {
                crate::layout::Leading::Fixed(l) => l,
                crate::layout::Leading::Auto => {
                    (pstyle.auto_leading_pct * em).max(asc + desc + gap)
                }
            };
            // Fresh-line x = the nearest glyph-bearing line's LEADING
            // edge (re-review NEW-2/NEW-3: the reference line carries
            // the alignment/indent truth in BOTH directions — under
            // RTL that is the visual right). An all-breaks layer falls
            // back to the anchor (+ leading indent, LTR paragraph).
            let is_paragraph = obj
                .get("layout")
                .and_then(|l| l.get("kind"))
                .and_then(Value::as_str)
                == Some("paragraph");
            let x = laid
                .layout
                .lines
                .iter()
                .rev()
                .find_map(|l| leading_edge_of(l, pstyle.rtl_base))
                .unwrap_or(
                    laid.anchor.0
                        + if is_paragraph && !pstyle.rtl_base {
                            pstyle.indent_left
                        } else {
                            0.0
                        },
                );
            let baseline = last.baseline + leading;
            return Ok(vec![CaretRect {
                x,
                top: baseline - asc,
                bottom: baseline + desc,
                primary: true,
            }]);
        }
        let li = laid
            .layout
            .lines
            .iter()
            .position(|l| l.range.contains(&byte))
            .or_else(|| laid.layout.lines.iter().rposition(|l| byte >= l.range.end))
            .unwrap_or(0);
        let line = &laid.layout.lines[li];
        let (asc, desc) = laid
            .item_of_byte(byte.min(laid.text.len().saturating_sub(1)))
            .or_else(|| laid.items.first())
            .map_or((DEFAULT_SIZE_PT * 0.8, DEFAULT_SIZE_PT * 0.2), |si| {
                (si.metrics.ascent, si.metrics.descent)
            });
        let leading_edge = line.glyphs.iter().find(|g| g.cluster == byte).map(|g| g.x);
        let x = leading_edge.unwrap_or_else(|| {
            // A glyphless line (between two mandatory breaks — break
            // glyphs render nothing) carets at the nearest glyph-
            // bearing line's leading edge (re-review NEW-2/NEW-3: that
            // line carries the alignment/indent truth, both
            // directions); an all-breaks layer falls back to the
            // anchor.
            if line.glyphs.is_empty() {
                return laid.layout.lines[..li]
                    .iter()
                    .rev()
                    .chain(laid.layout.lines[li + 1..].iter())
                    .find_map(|l| leading_edge_of(l, pstyle.rtl_base))
                    .unwrap_or(laid.anchor.0);
            }
            // Line end: the visual end matching the base direction (pin).
            let rtl = pstyle.rtl_base;
            line.glyphs
                .iter()
                .map(|g| if rtl { g.x } else { g.x + g.advance })
                .fold(
                    if rtl {
                        f64::INFINITY
                    } else {
                        f64::NEG_INFINITY
                    },
                    |m, v| if rtl { m.min(v) } else { m.max(v) },
                )
        });
        let mut carets = vec![CaretRect {
            x,
            top: line.baseline - asc,
            bottom: line.baseline + desc,
            primary: true,
        }];
        // Bidi split: level parity changes across the boundary (§23.2 —
        // primary full-height, secondary half-height; v1 pin).
        if byte > 0 && byte < laid.text.len() {
            let prev = laid.item_of_byte(byte - 1).map(|si| si.item.bidi_level % 2);
            let here = laid.item_of_byte(byte).map(|si| si.item.bidi_level % 2);
            if let (Some(a), Some(b)) = (prev, here) {
                if a != b {
                    let prev_cluster = laid
                        .bounds
                        .iter()
                        .copied()
                        .rfind(|&bnd| bnd < byte)
                        .unwrap_or(0);
                    if let Some(g) = line.glyphs.iter().find(|g| g.cluster == prev_cluster) {
                        carets.push(CaretRect {
                            x: g.x + g.advance,
                            top: line.baseline - asc / 2.0,
                            bottom: line.baseline,
                            primary: false,
                        });
                    }
                }
            }
        }
        Ok(carets)
    }

    fn hit_test(&self, sidecar: &[u8], point: [f64; 2]) -> Result<u64, EngineError> {
        if !point[0].is_finite() || !point[1].is_finite() {
            return Err(oob("/point", "hit point must be finite"));
        }
        let obj = self.normalized(sidecar)?;
        let laid = self.laid_of(&obj)?;
        let Some(line) = laid.layout.lines.iter().min_by(|a, b| {
            (a.baseline - point[1])
                .abs()
                .partial_cmp(&(b.baseline - point[1]).abs())
                .expect("finite baselines vs finite point")
        }) else {
            return Ok(0);
        };
        let mut best = (f64::INFINITY, 0u64);
        for g in &line.glyphs {
            let leading = (g.x, laid.scalar_of_byte(g.cluster));
            let trailing = (
                g.x + g.advance,
                laid.scalar_of_byte(laid.next_boundary(g.cluster)),
            );
            for (edge_x, scalar) in [leading, trailing] {
                let d = (edge_x - point[0]).abs();
                if d < best.0 {
                    best = (d, scalar);
                }
            }
        }
        Ok(best.1)
    }

    fn register_embedded_face(&self, asset: Uuid, bytes: &[u8]) -> Result<(), EngineError> {
        validate_face_bytes(bytes)?;
        let mut inner = self.state();
        if let Some(idx) = inner.faces.iter().position(|f| f.asset == asset) {
            if inner.faces[idx].bytes != bytes {
                return Err(oob(
                    "/font",
                    format!("asset {asset} already registered with different bytes"),
                ));
            }
            // Membership GROWS order-independently (review MAJOR-2),
            // sequenced at GRANT time (re-review NEW-1): a host-
            // provided face the document also embeds joins step 2 at
            // the EMBED door's order; already-embedded is a pure no-op.
            if inner.faces[idx].embedded.is_none() {
                let seq = inner.embed_counter;
                inner.embed_counter += 1;
                inner.faces[idx].embedded = Some(seq);
                inner.stamp += 1;
            }
            return Ok(());
        }
        let seq = inner.embed_counter;
        inner.embed_counter += 1;
        inner.faces.push(StoredFace {
            asset,
            bytes: bytes.to_vec(),
            embedded: Some(seq),
            provided: None,
        });
        inner.stamp += 1;
        Ok(())
    }

    fn enumerate_faces(&self) -> Vec<FontFaceInfo> {
        let inner = self.state();
        inner
            .faces
            .iter()
            .map(|f| FontFaceInfo {
                family: f.family(),
                style: f.style_name(),
                postscript_name: f.ps_name(),
                embeddable: embeddable_of(&f.bytes),
            })
            .collect()
    }

    fn fonts_stamp(&self) -> u64 {
        self.state().stamp
    }
}

impl TextLayoutService {
    /// Request-10 engine half (the §44.1 shape reading, ruled): a
    /// HOST-PROVISIONED face — the door glue hands over the bytes, the
    /// §44.2 source class, and host-derived FontRef metadata; the
    /// engine stays platform-blind and layout stays a pure function of
    /// (sidecar, provided fonts) — the D-48 spine. `DocEmbedded` is
    /// refused here (that class belongs to the loader's trait door).
    /// Membership is a SET (review MAJOR-2): providing bytes the
    /// document already embeds ADDS host membership + metadata
    /// order-independently; re-providing identically is a pure no-op
    /// (first host metadata wins WITHIN this door — pin). A PostScript
    /// name colliding with a DIFFERENT provided face refuses typed
    /// (review MINOR-3: silent §23.5 shadowing could hand tofu to text
    /// while enumeration lists the covering face as available — a host
    /// bug surfaces instead).
    pub fn register_provided_face(
        &self,
        asset: Uuid,
        bytes: &[u8],
        source: FaceSource,
        family: &str,
        style: &str,
        postscript_name: &str,
    ) -> Result<(), EngineError> {
        if source == FaceSource::DocEmbedded {
            return Err(oob(
                "/font",
                "DocEmbedded faces register through the package loader door \
                 (register_embedded_face), never as provided faces",
            ));
        }
        validate_face_bytes(bytes)?;
        let mut inner = self.state();
        if inner.faces.iter().any(|f| {
            f.asset != asset
                && f.provided
                    .as_ref()
                    .is_some_and(|p| p.postscript_name == postscript_name)
        }) {
            return Err(oob(
                "/font",
                format!(
                    "PostScript name {postscript_name:?} already provided by a \
                     different face (§23.5 dedup would silently shadow one)"
                ),
            ));
        }
        let meta = ProvidedMeta {
            source,
            family: family.to_owned(),
            style: style.to_owned(),
            postscript_name: postscript_name.to_owned(),
        };
        if let Some(f) = inner.faces.iter_mut().find(|f| f.asset == asset) {
            if f.bytes != bytes {
                return Err(oob(
                    "/font",
                    format!("asset {asset} already registered with different bytes"),
                ));
            }
            if f.provided.is_none() {
                f.provided = Some(meta);
                inner.stamp += 1;
            }
            return Ok(());
        }
        inner.faces.push(StoredFace {
            asset,
            bytes: bytes.to_vec(),
            embedded: None,
            provided: Some(meta),
        });
        inner.stamp += 1;
        Ok(())
    }
}

/// Byte validation shared by both doors: BOTH consumers must accept
/// (HarfBuzz cmap probe + FreeType face) — a refusal is typed, never a
/// downstream panic.
fn validate_face_bytes(bytes: &[u8]) -> Result<(), EngineError> {
    let shape_ok = appname_shape_sys::ShapeFont::from_bytes(bytes, 0, 1000).is_ok();
    let ft_ok = FreeTypeRasterizer::new()
        .map(|mut r| r.add_face(FaceId(0), bytes.to_vec()).is_ok())
        .unwrap_or(false);
    if !shape_ok || !ft_ok {
        return Err(oob(
            "/font",
            "unusable font bytes (not a face both HarfBuzz and FreeType accept)",
        ));
    }
    Ok(())
}

/// §23.5 fsType verdict: restricted (0x0002) or bitmap-embedding-only
/// (0x0200) refuse; anything else — including no OS/2 — is embeddable.
fn embeddable_of(bytes: &[u8]) -> bool {
    let Ok(mut r) = FreeTypeRasterizer::new() else {
        return false;
    };
    if r.add_face(FaceId(0), bytes.to_vec()).is_err() {
        return false;
    }
    match r.os2_fs_type(FaceId(0)) {
        Some(fs) => (fs & 0x0002) == 0 && (fs & 0x0200) == 0,
        None => true,
    }
}
