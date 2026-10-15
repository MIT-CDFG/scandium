// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §23.1–§23.5 (text layers, shaping pipeline, layout, fonts),
//! §40.7 text command entries, Part 44 (font edge contracts); DD ㉒.
//! TEXTVEC Doors 6 + 10 (reports/O-TEXTVEC-log.md 2026-08-17 "THE
//! TEXT-KINDS FILING", O-INT-approved): the `TextLayout` service trait —
//! core DEFINES the seam and stays text-blind; `appname-text` implements
//! it and registers at assembly construction, mirroring the
//! [`super::vector_ops::VectorGeometry`] Door-1 pattern
//! (`register(EngineAssembly) -> EngineAssembly`, no runtime door).
//!
//! The seam is VERSION-STAMPED: every sidecar crossing it is
//! `appname.text.v1` (§7.5 worked example; §23.1 "the layer serializes to
//! the `text/{layerId}.json` sidecar") —
//! [`super::kind_impl::EngineAssembly::with_text_layout`] asserts the
//! implementation declares exactly this schema.
//!
//! D-48 POSTURE (O-INT-APPROVED, deliberately UNLIKE `shape_add`'s
//! value capture): text RE-RENDERS PARAMETRICALLY — layout re-derives at
//! render as a pure function of (style, content, fonts). Determinism is
//! carried by the pinned vendored stack (HarfBuzz/ICU/FreeType, §23.2
//! [NORMATIVE] "pinned, vendored versions"; the §34.1 golden corpus is the
//! proof), exactly §23.1's model; value capture is neither needed nor
//! wanted — the §44.1 font-missing fallback MUST re-resolve on open. The
//! Door-10 bridge makes fonts part of the input: a font change re-lays-out,
//! correct. The ONE exception is `text.to_shape` (Door 9): conversion is a
//! one-time BAKE, so its OUTPUT geometry is value-captured through the
//! existing vector capture table, exactly like `shape_add`.
//!
//! Core-side text payloads are OPAQUE bytes + a content hash (the
//! [`super::vector_ops::sidecar_content_hash`] identity space, §7.4).

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::engine::error::{EngineError, ErrorCode};
use crate::geom::IRect;

/// The one wire schema this seam speaks (§7.5 worked example; §23.1).
pub const TEXT_SIDECAR_SCHEMA: &str = "appname.text.v1";

/// What the service hands back for any sidecar-producing call: the
/// NORMALIZED sidecar bytes (strict-writer re-emission — §23.1 run-model
/// invariants enforced inside the implementation: minimal runs, no
/// zero-length runs, adjacent equal styles merged) and the LAID-OUT bounds
/// in document pixels (integer ink hull over the §23.3 layout — feeds the
/// §40.7 family dirty rule "D = laid-out bounds before ∪ after").
#[derive(Debug, Clone)]
pub struct TextLayoutResult {
    pub sidecar: Vec<u8>,
    pub bounds: IRect,
}

/// §23.2/§23.3 laid-out metrics at the seam (Door 6's compose pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaidMetrics {
    /// Laid-out ink bounds in document pixels (integer hull).
    pub bounds: IRect,
    /// Lines laid (post §23.3 breaking).
    pub line_count: u32,
    /// §23.3 paragraph-box overflow ("clips after the last fully fitting
    /// line"; the shell's ⊞ badge state).
    pub overflowed: bool,
}

/// Door 6 compose output: one premultiplied linear working-space RGBA page
/// over `rect`, row-major — §23.2 rasterization ("composites through that
/// coverage, premultiplied linear per §4.4") already applied per-run
/// through each run's `FillContent`. Mirrors
/// [`super::vector_ops::CoveragePlaneData`]'s shape; core owns the type so
/// the dependency direction stays crates→core.
#[derive(Debug, Clone)]
pub struct TextPageData {
    pub rect: IRect,
    /// `rect.width() * rect.height()` premultiplied RGBA values, row-major.
    pub pixels: Vec<[f32; 4]>,
}

/// One caret rectangle (§23.2 "Carets and selection"): at a bidi boundary
/// the caret splits — the PRIMARY draws full-height on the base-direction
/// side, the secondary half-height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretRect {
    pub x: f64,
    pub top: f64,
    pub bottom: f64,
    pub primary: bool,
}

/// Door 10 — one enumerable face (§23.5 `FontRef` fields; the shell/engine
/// font enumeration surface: family pickers, the §44.1 substitution
/// dialog's FontDb listing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceInfo {
    pub family: String,
    pub style: String,
    pub postscript_name: String,
    /// §23.5 `fsType` embedding policy verdict (the save-side subset-embed
    /// gate is O-IO's arm; this surfaces the bit for UI/policy).
    pub embeddable: bool,
}

/// TEXTVEC Door 6 — the text-layout service. Core never sees text
/// structure: sidecar bytes in, sidecar bytes out, every payload
/// `appname.text.v1`. Door 10 (the FontDb bridge, §44.2 step 2) rides the
/// same service because fonts are layout INPUT — one seam, one stamp.
///
/// Error contract (§40.7 family rows): a structurally invalid payload
/// (zero-length run, non-contiguous runs, bad enum) is
/// `error.schema.out_of_range` against the offending field; a wrong or
/// missing schema stamp is `error.schema.wrong_schema`; a range beyond the
/// content length (family rule `0 ≤ start ≤ end ≤ len`, Unicode scalar
/// values) is `error.schema.out_of_range`. MISSING FONTS NEVER FAIL A
/// CALL — the §44.2 fallback chain applies inside the implementation and
/// layout proceeds (§23.5/§40.7 "missing fonts NEVER fail a command").
pub trait TextLayout: Send + Sync {
    /// The wire schema the implementation speaks — MUST return
    /// [`TEXT_SIDECAR_SCHEMA`] (asserted at registration).
    fn schema(&self) -> &'static str;

    /// §23.1 validate + normalize: parse `bytes` as an `appname.text.v1`
    /// sidecar, enforce the run-model invariants, lay out (§23.3), and
    /// re-emit normalized bytes + laid-out bounds. §7.7 unknown-field
    /// posture: reserved/unknown fields are PRESERVED verbatim in the
    /// re-emission (old readers ignore, new writers preserve — §23.4).
    fn validate_sidecar(&self, bytes: &[u8]) -> Result<TextLayoutResult, EngineError>;

    /// Build the layer sidecar from `text.add`'s stage-5-validated params
    /// (`layout`/`origin`/`rect`/`pathId`/`text`/`style`/`paragraph` per
    /// the contract; §23.1 defaults fill absent style/paragraph fields).
    fn sidecar_from_add_params(
        &self,
        params: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError>;

    /// `text.edit`'s FULL-STATE replacement (§10.3 full-state rule): new
    /// `runs` (+ optional complete `layout`/`paragraph`) over the current
    /// sidecar — absent run style fields default from the layer's current
    /// style (contract note); run contiguity/minimality re-normalized
    /// (§23.1). Stage-6 live checks (zero-length run reaching here,
    /// contiguity breach) answer `error.schema.out_of_range`.
    fn apply_edit(
        &self,
        current: &[u8],
        params: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError>;

    /// `text.set_style`: apply a character-style patch over
    /// `[range.0, range.1)` in Unicode scalar values (§23.4), splitting and
    /// re-merging runs as needed, never touching the string (§23.1).
    fn apply_style_range(
        &self,
        current: &[u8],
        range: (u64, u64),
        style: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError>;

    /// `text.set_paragraph`: apply a paragraph-style patch over the
    /// paragraphs INTERSECTING the range (§10.3; v1 stores ONE
    /// ParagraphStyle per layer — §23.1 — so any intersecting range
    /// patches the layer's style; the wording is forward-looking).
    fn apply_paragraph_range(
        &self,
        current: &[u8],
        range: (u64, u64),
        paragraph: &Map<String, Value>,
    ) -> Result<TextLayoutResult, EngineError>;

    /// Content length in Unicode scalar values (the family-rule range
    /// bound).
    fn content_len(&self, sidecar: &[u8]) -> Result<u64, EngineError>;

    /// Door 6's compose pair: itemize → shape → break → layout → compose
    /// (§23.2 pipeline) over the landed stack. The render/export rows and
    /// the CLI reference arm consume this; core never interprets the page.
    fn layout_and_compose(
        &self,
        sidecar: &[u8],
    ) -> Result<(TextPageData, LaidMetrics), EngineError>;

    /// Door 9's engine half: the CURRENT fallback-resolved glyph outlines
    /// (§23.5 — substituted fonts bake their substitutes, documented
    /// §40.7) as an `appname.vector.v1` sidecar (FreeType outlines →
    /// §20.1 paths). The `text.to_shape` arm feeds this through Door 1
    /// (`VectorGeometry::validate_sidecar`) and value-captures the result.
    fn outline_geometry(&self, sidecar: &[u8]) -> Result<Vec<u8>, EngineError>;

    /// §23.2 cluster-map consumer: caret geometry at a scalar index
    /// (1 entry, or 2 at a bidi boundary — primary + secondary).
    fn caret_geometry(
        &self,
        sidecar: &[u8],
        scalar_index: u64,
    ) -> Result<Vec<CaretRect>, EngineError>;

    /// §23.2 cluster-map consumer: document-pixel point → nearest caret
    /// scalar index (UAX #29 cluster-snapped).
    fn hit_test(&self, sidecar: &[u8], point: [f64; 2]) -> Result<u64, EngineError>;

    // --- Door 10 — the FontDb bridge (§44.2 step 2; §23.5) ---------------

    /// Register a document-embedded face (a package `assets/` font blob)
    /// into the implementation's FontDb under its content-addressed asset
    /// id. Idempotent per (asset, bytes) — the loader and asset-ingestion
    /// doors call this for every font asset; §44.2 step 2 makes the
    /// document its own first fallback resource.
    fn register_embedded_face(&self, asset: Uuid, bytes: &[u8]) -> Result<(), EngineError>;

    /// Enumerate the FontDb (system + bundled + registered embedded faces)
    /// deterministically (§44.2 step-5 order discipline): the shell/engine
    /// font enumeration surface.
    fn enumerate_faces(&self) -> Vec<FontFaceInfo>;

    /// Font-environment generation stamp: bumps whenever the face set
    /// changes (install/uninstall watch, embedded registration). Layout is
    /// a pure function of (sidecar, fonts) — consumers key caches on this
    /// (§44.1 step 4: installing a missing font re-resolves).
    fn fonts_stamp(&self) -> u64;
}

/// Content hash of a text sidecar: same machinery as vector sidecars and
/// content-addressed assets (§7.4 identity space).
pub fn text_content_hash(bytes: &[u8]) -> Uuid {
    crate::engine::assets::asset_id_for(bytes)
}

/// D-48 page-capture framing (the composite-leg slice): the rasterize
/// arm value-captures the Door-6 page through the ONE capture table
/// (`GeometryResult { sidecar: bytes, bounds }` — bytes are opaque to the
/// table and its `journal/captures/` persistence), so replay NEVER
/// re-runs layout (§23.2 layout is pure in (sidecar, FONTS); the font
/// environment may differ between record and replay — the to_shape
/// outline-capture rationale verbatim). The framing is versioned inside
/// the bytes: magic + rect (i32 LE ×4) + premultiplied RGBA f32 LE
/// pixels, deterministic by construction. POSTURE RECORDED: the capture
/// table's persistence meta stamps `appname.vector.capture.v1`
/// unconditionally — the magic below is the discriminator; a dedicated
/// meta schema is a fold-in-free cleanup candidate for the capture
/// table's owner slice.
pub const TEXT_PAGE_CAPTURE_MAGIC: &[u8] = b"appname.textpage.capture.v1\0";

/// Encode a Door-6 page into capture bytes (deterministic framing).
pub fn encode_page_capture(page: &TextPageData) -> Vec<u8> {
    let mut out = Vec::with_capacity(TEXT_PAGE_CAPTURE_MAGIC.len() + 16 + page.pixels.len() * 16);
    out.extend_from_slice(TEXT_PAGE_CAPTURE_MAGIC);
    for v in [
        page.rect.min.x,
        page.rect.min.y,
        page.rect.max.x,
        page.rect.max.y,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for px in &page.pixels {
        for c in px {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

/// Decode capture bytes back into the page (replay consumption; a
/// malformed capture is a typed refusal — never a panic).
pub fn decode_page_capture(bytes: &[u8]) -> Result<TextPageData, EngineError> {
    let bad = |what: &str| {
        EngineError::bad_target(format!(
            "text-page capture malformed: {what} (D-48 framing)"
        ))
    };
    let rest = bytes
        .strip_prefix(TEXT_PAGE_CAPTURE_MAGIC)
        .ok_or_else(|| bad("magic missing"))?;
    if rest.len() < 16 {
        return Err(bad("rect truncated"));
    }
    let mut r = [0i32; 4];
    for (i, v) in r.iter_mut().enumerate() {
        *v = i32::from_le_bytes(rest[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
    }
    let rect = IRect::new(r[0], r[1], r[2], r[3]);
    let n = (rect.width().max(0) as usize) * (rect.height().max(0) as usize);
    let body = &rest[16..];
    if body.len() != n * 16 {
        return Err(bad("pixel plane length mismatch"));
    }
    let mut pixels = Vec::with_capacity(n);
    for i in 0..n {
        let mut px = [0.0f32; 4];
        for (c, slot) in px.iter_mut().enumerate() {
            let o = i * 16 + c * 4;
            *slot = f32::from_le_bytes(body[o..o + 4].try_into().expect("4 bytes"));
        }
        pixels.push(px);
    }
    Ok(TextPageData { rect, pixels })
}

/// The typed refusal every text-requiring arm answers when no
/// [`TextLayout`] is registered on the assembly (bare-core mirror — the
/// house pattern, exactly [`super::vector_ops::geometry_unregistered`]).
pub fn text_unregistered(kind: &str) -> EngineError {
    EngineError::new(
        ErrorCode::NotImplemented,
        format!(
            "{kind}: no TextLayout service is registered on this assembly \
             (appname-text registers it at construction; bare-core mirror)"
        ),
    )
}
