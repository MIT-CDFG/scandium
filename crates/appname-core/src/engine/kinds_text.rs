// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §40.7 text command entries (verbatim rows + the family rules),
//! §23.1–§23.5, §10.3 (class M full-state rule), §10.5 (typing/property
//! sessions); contracts `contracts/commands/text.*.yaml`.
//!
//! The five `text.*` kind arms (CP-1.0 door slice 2, TEXTVEC Doors 8–9).
//! Core stays text-blind: every text operation crosses the
//! [`crate::engine::text_ops::TextLayout`] seam; arms on an assembly with
//! no registered service answer the typed NI mirror (house pattern), with
//! stage-4 target resolution PRECEDING the service gate — dangling ids
//! answer `bad_target` first (the pinned vector-arm order).
//!
//! D-48 (the approved posture): the mutating kinds are PARAMETRIC —
//! sidecars re-render at composite time as pure functions of
//! (style, content, fonts); nothing captures. The ONE exception is
//! `text.to_shape` (Door 9): a one-time BAKE whose output geometry is
//! VALUE-CAPTURED through the existing vector capture table, exactly like
//! `vector.shape_add` — replay never re-runs outline extraction, so a
//! font-environment change between record and replay cannot fork the
//! document (§40.7: conversion uses the CURRENT fallback-resolved glyphs).
//!
//! Pinned readings recorded in the lane log (fold-in candidates):
//! `text.to_shape`'s resulting shape carries the FIRST run's `fill`
//! payload (§40.7 names "SetProp kind + geometry payloads" but is silent
//! on style; one ShapeLayerData holds one fill — multi-fill text bakes
//! under this reading) and a null stroke; the fill is read MECHANICALLY
//! from the sidecar JSON (a pure function of document state — replay-
//! stable without capture, core interprets nothing).

use std::sync::Arc;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::engine::delta::ShapeContentBlock;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::engine::text_ops::text_content_hash;
use crate::engine::vector_ops::{sidecar_content_hash, GeometryResult};
use crate::graph::id::{LayerId, PathId};
use crate::graph::node::{LayerNode, TextLayerData};
use crate::graph::path_node::PathNode;

pub struct TextAddKind;
pub struct TextEditKind;
pub struct TextSetStyleKind;
pub struct TextSetParagraphKind;
pub struct TextToShapeKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(TextAddKind),
        Arc::new(TextEditKind),
        Arc::new(TextSetStyleKind),
        Arc::new(TextSetParagraphKind),
        Arc::new(TextToShapeKind),
    ]
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Text-target resolution (§40.7 family rule): `target.layerId` MUST
/// resolve (stage 4 → `bad_target`) and MUST be a Text node
/// (→ `target_kind_mismatch`) — every kind except `text.add`.
fn resolve_text_target(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
    let lid = cx
        .envelope()
        .target
        .as_ref()
        .and_then(|t| t.layer_id)
        .map(LayerId::from_uuid)
        .ok_or_else(|| EngineError::bad_target("this kind requires target.layerId"))?;
    let Some(node) = cx.document().layer(lid) else {
        return Err(EngineError::bad_target(format!(
            "target.layerId {lid} does not resolve"
        )));
    };
    if node.text_data().is_none() {
        return Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "the target is not a Text layer (§40.7 family rule)",
        ));
    }
    Ok(lid)
}

/// PIXELS/ALL gate for the content-mutating text kinds (§40.7 family rule:
/// "content-mutating text kinds are PIXELS/ALL-gated"; §11.1 transitive).
fn refuse_text_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    use crate::graph::node::LockFlags;
    let locks = cx.document().effective_locks(layer);
    if locks.contains(LockFlags::ALL) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "ALL lock along the ancestry blocks (§11.1 transitive rule)",
        ));
    }
    if locks.contains(LockFlags::PIXELS) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "PIXELS lock blocks content-mutating text kinds (§40.7 family rule)",
        ));
    }
    Ok(())
}

/// The `[start, end]` range param (stage-5 typed: 2 non-negative
/// integers). The arm enforces `start ≤ end` (`range: [5, 2]` is the
/// contract's out-of-range case); the SERVICE enforces `end ≤ len` (it
/// owns the scalar count — family rule, both answer
/// `error.schema.out_of_range`).
fn range_param(cx: &KindContext<'_>) -> Result<(u64, u64), EngineError> {
    let arr = cx
        .params()
        .get("range")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("range"))?;
    let get = |i: usize| arr.get(i).and_then(Value::as_u64);
    let (Some(start), Some(end)) = (get(0), get(1)) else {
        return Err(EngineError::range(
            "range",
            "`range` must be [start, end] in Unicode scalar values",
        ));
    };
    if start > end {
        return Err(EngineError::range(
            "range",
            format!("range start {start} exceeds end {end} (family rule 0 ≤ start ≤ end ≤ len)"),
        ));
    }
    Ok((start, end))
}

fn object_param<'a>(
    cx: &'a KindContext<'_>,
    field: &str,
) -> Result<&'a Map<String, Value>, EngineError> {
    cx.params()
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| EngineError::missing(field))
}

/// Install a service result as the layer's new text payload through the
/// SetTextData value door and emit the §40.7 family dirty
/// ("D = laid-out bounds before ∪ after").
fn commit_text_result(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    r: crate::engine::text_ops::TextLayoutResult,
) -> Result<(), EngineError> {
    let before_bounds = cx
        .document()
        .layer(layer)
        .expect("resolved")
        .text_data()
        .expect("text target")
        .bounds();
    let hash = text_content_hash(&r.sidecar);
    cx.set_text_data(
        layer,
        TextLayerData::new(Arc::new(r.sidecar), hash, r.bounds),
    )?;
    cx.note_layer_dirty(layer, before_bounds.union(&r.bounds));
    Ok(())
}

// ---------------------------------------------------------------------------
// text.add · R · inverse delta (§40.7 entry verbatim)
// ---------------------------------------------------------------------------

impl KindImpl for TextAddKind {
    fn kind(&self) -> &'static str {
        "text.add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Stage-5 already enforced: layout enum + cross-fields (point →
        // origin, paragraph → rect, on_path → pathId — the contract's
        // allOf), param types/ranges. Semantic checks follow.
        let layout = cx
            .params()
            .get("layout")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("layout"))?;
        if layout == "on_path" {
            // §40.7: on_path pathId must resolve (stage 4; the contract's
            // `on-path-target-unresolved` case answers bad_target).
            let pid = cx
                .params()
                .get("pathId")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(PathId::from_uuid)
                .ok_or_else(|| EngineError::missing("pathId"))?;
            if cx.document().path(pid).is_none() {
                return Err(EngineError::bad_target(format!(
                    "on_path pathId {pid} does not resolve (§2.7 arenas)"
                )));
            }
        }
        let new_layer = crate::engine::kinds_vector::creation_id(cx, "newLayerId")?;
        let parent = match cx.params().get("parent").and_then(Value::as_str) {
            Some(s) => {
                let id = Uuid::parse_str(s)
                    .map_err(|_| EngineError::range("parent", "`parent` must be a UUID"))?;
                let lid = LayerId::from_uuid(id);
                if cx.document().layer(lid).is_none() {
                    return Err(EngineError::bad_target(format!(
                        "parent {lid} does not resolve"
                    )));
                }
                if !cx.document().layer(lid).expect("resolved").is_group() {
                    return Err(EngineError::new(
                        ErrorCode::TargetKindMismatch,
                        "`parent` must be a group (§2.6)",
                    ));
                }
                lid
            }
            None => cx.document().root(),
        };
        // §40.7 locks: ALL on the parent chain blocks.
        {
            use crate::graph::node::LockFlags;
            if cx
                .document()
                .effective_locks(parent)
                .contains(LockFlags::ALL)
            {
                return Err(EngineError::new(
                    ErrorCode::Locked,
                    "ALL lock along the parent chain blocks (§40.7)",
                ));
            }
        }
        let child_count = cx
            .document()
            .children_of(parent)
            .map(|c| c.len())
            .unwrap_or(0);
        let index = cx
            .params()
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize)
            .unwrap_or(child_count)
            .min(child_count);

        // Door 6: the sidecar derives from the params (missing fonts fall
        // back inside the service — NEVER an error, §23.5/§40.7).
        let svc = cx.require_text_layout()?;
        let r = svc.sidecar_from_add_params(cx.params())?;
        let hash = text_content_hash(&r.sidecar);
        let lid = LayerId::from_uuid(new_layer);
        let bounds = r.bounds;
        cx.insert_layer(
            parent,
            index,
            LayerNode::new_text(
                lid,
                "Text",
                TextLayerData::new(Arc::new(r.sidecar), hash, bounds),
            ),
        )?;
        // §40.7 dirty: D = laid-out bounds (add has no before) · T.
        cx.note_layer_dirty(lid, bounds);
        Ok(KindOutcome {
            created_ids: vec![new_layer],
        })
    }
}

// ---------------------------------------------------------------------------
// text.edit · M · inverse delta (full-state runs replacement, §10.3)
// ---------------------------------------------------------------------------

impl KindImpl for TextEditKind {
    fn kind(&self) -> &'static str {
        "text.edit"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = resolve_text_target(cx)?;
        refuse_text_locked(cx, lid)?;
        let svc = cx.require_text_layout()?;
        let current = Arc::clone(
            cx.document()
                .layer(lid)
                .expect("resolved")
                .text_data()
                .expect("text target")
                .sidecar(),
        );
        // Full-state replacement (§10.3): the service re-normalizes runs
        // (§23.1 minimality) and answers the stage-6 live checks
        // (contiguity/zero-length → out_of_range).
        let r = svc.apply_edit(&current, cx.params())?;
        commit_text_result(cx, lid, r)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// text.set_style · M · inverse delta (character patch over a range)
// ---------------------------------------------------------------------------

impl KindImpl for TextSetStyleKind {
    fn kind(&self) -> &'static str {
        "text.set_style"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = resolve_text_target(cx)?;
        refuse_text_locked(cx, lid)?;
        let range = range_param(cx)?;
        let style = object_param(cx, "style")?.clone();
        let svc = cx.require_text_layout()?;
        let current = Arc::clone(
            cx.document()
                .layer(lid)
                .expect("resolved")
                .text_data()
                .expect("text target")
                .sidecar(),
        );
        // §23.1: split/re-merge runs inside the service, string untouched;
        // range beyond content length answers out_of_range (family rule).
        let r = svc.apply_style_range(&current, range, &style)?;
        commit_text_result(cx, lid, r)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// text.set_paragraph · M · inverse delta (paragraphs intersecting a range)
// ---------------------------------------------------------------------------

impl KindImpl for TextSetParagraphKind {
    fn kind(&self) -> &'static str {
        "text.set_paragraph"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = resolve_text_target(cx)?;
        refuse_text_locked(cx, lid)?;
        let range = range_param(cx)?;
        let paragraph = object_param(cx, "paragraph")?.clone();
        let svc = cx.require_text_layout()?;
        let current = Arc::clone(
            cx.document()
                .layer(lid)
                .expect("resolved")
                .text_data()
                .expect("text target")
                .sidecar(),
        );
        let r = svc.apply_paragraph_range(&current, range, &paragraph)?;
        commit_text_result(cx, lid, r)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// text.to_shape · R · inverse delta (Door 9 — the one-time bake)
// ---------------------------------------------------------------------------

impl KindImpl for TextToShapeKind {
    fn kind(&self) -> &'static str {
        "text.to_shape"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = resolve_text_target(cx)?;
        refuse_text_locked(cx, lid)?;
        let td = cx
            .document()
            .layer(lid)
            .expect("resolved")
            .text_data()
            .expect("text target")
            .clone();

        // D-48: consume THIS envelope's capture on replay (the bake never
        // re-runs — a changed font environment cannot fork the document),
        // else extract outlines (Door 6) and validate/normalize through
        // Door 1 (the service-to-service seam: to_shape asks TextLayout
        // for outline geometry, then the VECTOR door owns it), and CAPTURE.
        let geom: Arc<GeometryResult> = match cx.vector_capture() {
            Some(g) => Arc::clone(g),
            None => {
                let text_svc = cx.require_text_layout()?;
                let vec_svc = cx.require_vector_geometry()?;
                let outline = text_svc.outline_geometry(td.sidecar())?;
                let g = Arc::new(vec_svc.validate_sidecar(&outline)?);
                cx.record_vector_capture(Arc::clone(&g));
                g
            }
        };

        // Pinned reading (lane log): the shape carries the FIRST run's
        // `fill` payload verbatim (mechanical sidecar read — a pure
        // function of document state, replay-stable) and a null stroke.
        let fill = serde_json::from_slice::<Value>(td.sidecar())
            .ok()
            .and_then(|v| {
                v.get("runs")
                    .and_then(Value::as_array)
                    .and_then(|runs| runs.first().cloned())
                    .and_then(|run| run.get("fill").cloned())
            })
            .unwrap_or(Value::Null);

        // Deterministic path id (the shape_add "wrapper fills" mechanism —
        // no creation params on this kind, §40.7 row: no params).
        let new_path = Uuid::new_v5(
            &crate::engine::doc_new::APPNAME_NS,
            format!("creation/{}/toShapePath", cx.envelope().id).as_bytes(),
        );
        let hash = sidecar_content_hash(&geom.sidecar);
        let pid = cx.insert_path(PathNode::new(
            PathId::from_uuid(new_path),
            None,
            Arc::new(geom.sidecar.clone()),
            hash,
            geom.bounds,
        ))?;
        cx.convert_text_to_shape(
            lid,
            ShapeContentBlock {
                path: pid,
                fill,
                stroke: Value::Null,
                bool_ops: Vec::new(),
            },
        )?;
        // §40.7 dirty: D = layer bounds (before text ∪ after shape) · T
        // topic:"tree" (kind change).
        cx.note_layer_dirty(lid, td.bounds().union(&geom.bounds));
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::text_ops::{
        CaretRect, FontFaceInfo, LaidMetrics, TextLayout, TextLayoutResult, TextPageData,
        TEXT_SIDECAR_SCHEMA,
    };
    use crate::engine::vector_ops::{
        CoveragePlaneData, VectorBoolOp, VectorGeometry, VECTOR_SIDECAR_SCHEMA,
    };
    use crate::engine::Engine;
    use crate::geom::IRect;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    /// In-crate PROBE text service (the production impl is appname-text's
    /// — TEXTVEC Door 6; this probe exercises the SEAM, not real
    /// shaping): sidecar = `{schema, layoutKind, origin|box|pathRef,
    /// runs:[{text, fill?, style?}], paragraph}`; bounds = a deterministic
    /// function of anchor + scalar count (8 px/char, 16 px/line).
    /// `outline_gen` makes `outline_geometry` DELIBERATELY nondeterministic
    /// across calls, so the Door-9 capture pin below proves replay consumes
    /// the bake instead of re-running outline extraction.
    struct TextProbe {
        outline_gen: AtomicU64,
        faces: Mutex<Vec<uuid::Uuid>>,
        stamp: AtomicU64,
    }

    impl TextProbe {
        fn new() -> TextProbe {
            TextProbe {
                outline_gen: AtomicU64::new(0),
                faces: Mutex::new(Vec::new()),
                stamp: AtomicU64::new(1),
            }
        }

        fn parse(bytes: &[u8]) -> Result<Value, EngineError> {
            let v: Value = serde_json::from_slice(bytes)
                .map_err(|e| EngineError::range("text", format!("sidecar not JSON: {e}")))?;
            if v.get("schema").and_then(Value::as_str) != Some(TEXT_SIDECAR_SCHEMA) {
                return Err(EngineError::new(
                    ErrorCode::WrongSchema,
                    "sidecar schema must be appname.text.v1",
                ));
            }
            if let Some(runs) = v.get("runs").and_then(Value::as_array) {
                for r in runs {
                    if r.get("text").and_then(Value::as_str).map(str::is_empty) != Some(false) {
                        return Err(EngineError::range(
                            "runs",
                            "zero-length runs are forbidden (§23.1)",
                        ));
                    }
                }
            }
            Ok(v)
        }

        fn scalar_len(v: &Value) -> u64 {
            v.get("runs")
                .and_then(Value::as_array)
                .map(|rs| {
                    rs.iter()
                        .filter_map(|r| r.get("text").and_then(Value::as_str))
                        .map(|t| t.chars().count() as u64)
                        .sum()
                })
                .unwrap_or(0)
        }

        fn bounds_of(v: &Value) -> IRect {
            let anchor = v
                .get("origin")
                .or_else(|| v.get("box"))
                .and_then(Value::as_array)
                .map(|a| {
                    let g = |i: usize| a.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
                    (g(0), g(1))
                })
                .unwrap_or((0, 0));
            let n = Self::scalar_len(v).max(1) as i32;
            let lines = 1 + v
                .get("runs")
                .and_then(Value::as_array)
                .map(|rs| {
                    rs.iter()
                        .filter_map(|r| r.get("text").and_then(Value::as_str))
                        .map(|t| t.matches('\n').count() as i32)
                        .sum()
                })
                .unwrap_or(0);
            IRect::new(anchor.0, anchor.1, anchor.0 + 8 * n, anchor.1 + 16 * lines)
        }

        fn emit(v: &Value) -> TextLayoutResult {
            TextLayoutResult {
                sidecar: serde_json::to_vec(v).expect("probe emits"),
                bounds: Self::bounds_of(v),
            }
        }

        fn check_range(v: &Value, range: (u64, u64)) -> Result<(), EngineError> {
            let len = Self::scalar_len(v);
            if range.1 > len {
                return Err(EngineError::range(
                    "range",
                    format!(
                        "range end {} exceeds content length {len} (family rule)",
                        range.1
                    ),
                ));
            }
            Ok(())
        }
    }

    impl TextLayout for TextProbe {
        fn schema(&self) -> &'static str {
            TEXT_SIDECAR_SCHEMA
        }
        fn validate_sidecar(&self, bytes: &[u8]) -> Result<TextLayoutResult, EngineError> {
            let v = Self::parse(bytes)?;
            Ok(Self::emit(&v))
        }
        fn sidecar_from_add_params(
            &self,
            params: &Map<String, Value>,
        ) -> Result<TextLayoutResult, EngineError> {
            let layout = params
                .get("layout")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::missing("layout"))?;
            let mut v = json!({"schema": TEXT_SIDECAR_SCHEMA, "layoutKind": layout});
            if let Some(o) = params.get("origin") {
                v["origin"] = o.clone();
            }
            if let Some(r) = params.get("rect").and_then(Value::as_object) {
                let g = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(0);
                v["box"] = json!([g("x"), g("y"), g("w"), g("h")]);
            }
            if let Some(p) = params.get("pathId") {
                v["pathRef"] = p.clone();
            }
            let text = params.get("text").and_then(Value::as_str).unwrap_or("");
            let mut runs = Vec::new();
            if !text.is_empty() {
                let mut run = json!({"text": text});
                if let Some(style) = params.get("style") {
                    run["style"] = style.clone();
                    if let Some(f) = style.get("fill") {
                        run["fill"] = f.clone();
                    }
                }
                runs.push(run);
            }
            v["runs"] = Value::Array(runs);
            if let Some(p) = params.get("paragraph") {
                v["paragraph"] = p.clone();
            }
            Ok(Self::emit(&v))
        }
        fn apply_edit(
            &self,
            current: &[u8],
            params: &Map<String, Value>,
        ) -> Result<TextLayoutResult, EngineError> {
            let mut v = Self::parse(current)?;
            let runs = params
                .get("runs")
                .and_then(Value::as_array)
                .ok_or_else(|| EngineError::missing("runs"))?;
            for r in runs {
                if r.get("text").and_then(Value::as_str).map(str::is_empty) != Some(false) {
                    return Err(EngineError::range(
                        "runs",
                        "zero-length runs are forbidden (§23.1 stage-6 live check)",
                    ));
                }
            }
            v["runs"] = Value::Array(runs.clone());
            if let Some(layout) = params.get("layout").and_then(Value::as_object) {
                if let Some(k) = layout.get("kind") {
                    v["layoutKind"] = k.clone();
                }
                if let Some(o) = layout.get("origin") {
                    v["origin"] = o.clone();
                }
                if let Some(r) = layout.get("rect").and_then(Value::as_object) {
                    let g = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(0);
                    v["box"] = json!([g("x"), g("y"), g("w"), g("h")]);
                }
            }
            if let Some(p) = params.get("paragraph") {
                v["paragraph"] = p.clone();
            }
            Ok(Self::emit(&v))
        }
        fn apply_style_range(
            &self,
            current: &[u8],
            range: (u64, u64),
            style: &Map<String, Value>,
        ) -> Result<TextLayoutResult, EngineError> {
            let mut v = Self::parse(current)?;
            Self::check_range(&v, range)?;
            if let Some(runs) = v.get_mut("runs").and_then(Value::as_array_mut) {
                for r in runs {
                    let s = r
                        .as_object_mut()
                        .expect("run object")
                        .entry("style")
                        .or_insert_with(|| json!({}));
                    for (k, val) in style {
                        s[k] = val.clone();
                    }
                }
            }
            Ok(Self::emit(&v))
        }
        fn apply_paragraph_range(
            &self,
            current: &[u8],
            range: (u64, u64),
            paragraph: &Map<String, Value>,
        ) -> Result<TextLayoutResult, EngineError> {
            let mut v = Self::parse(current)?;
            Self::check_range(&v, range)?;
            let p = v
                .as_object_mut()
                .expect("sidecar object")
                .entry("paragraph")
                .or_insert_with(|| json!({}));
            for (k, val) in paragraph {
                p[k] = val.clone();
            }
            Ok(Self::emit(&v))
        }
        fn content_len(&self, sidecar: &[u8]) -> Result<u64, EngineError> {
            Ok(Self::scalar_len(&Self::parse(sidecar)?))
        }
        fn layout_and_compose(
            &self,
            sidecar: &[u8],
        ) -> Result<(TextPageData, LaidMetrics), EngineError> {
            let v = Self::parse(sidecar)?;
            let rect = Self::bounds_of(&v);
            let pixels =
                vec![[0.5, 0.0, 0.0, 0.5]; (rect.width().max(0) * rect.height().max(0)) as usize];
            Ok((
                TextPageData { rect, pixels },
                LaidMetrics {
                    bounds: rect,
                    line_count: 1,
                    overflowed: false,
                },
            ))
        }
        fn outline_geometry(&self, sidecar: &[u8]) -> Result<Vec<u8>, EngineError> {
            let v = Self::parse(sidecar)?;
            let b = Self::bounds_of(&v);
            // DELIBERATE nondeterminism: the Door-9 capture pin depends on
            // every extraction call yielding different bytes.
            let gen = self.outline_gen.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::to_vec(&json!({
                "schema": VECTOR_SIDECAR_SCHEMA,
                "fillRule": "nonzero",
                "subpaths": [{"rect": [b.min.x, b.min.y, b.width(), b.height()]}],
                "gen": gen,
            }))
            .expect("probe emits"))
        }
        fn caret_geometry(
            &self,
            sidecar: &[u8],
            scalar_index: u64,
        ) -> Result<Vec<CaretRect>, EngineError> {
            let v = Self::parse(sidecar)?;
            Self::check_range(&v, (scalar_index, scalar_index))?;
            let b = Self::bounds_of(&v);
            Ok(vec![CaretRect {
                x: f64::from(b.min.x) + 8.0 * scalar_index as f64,
                top: f64::from(b.min.y),
                bottom: f64::from(b.max.y),
                primary: true,
            }])
        }
        fn hit_test(&self, sidecar: &[u8], point: [f64; 2]) -> Result<u64, EngineError> {
            let v = Self::parse(sidecar)?;
            let b = Self::bounds_of(&v);
            let raw = ((point[0] - f64::from(b.min.x)) / 8.0).round().max(0.0) as u64;
            Ok(raw.min(Self::scalar_len(&v)))
        }
        fn register_embedded_face(
            &self,
            asset: uuid::Uuid,
            bytes: &[u8],
        ) -> Result<(), EngineError> {
            if bytes.is_empty() {
                return Err(EngineError::range(
                    "font",
                    "empty face blob (probe refusal)",
                ));
            }
            let mut faces = self.faces.lock().unwrap();
            if !faces.contains(&asset) {
                faces.push(asset);
                self.stamp.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
        fn enumerate_faces(&self) -> Vec<FontFaceInfo> {
            let mut out = vec![FontFaceInfo {
                family: "Probe Sans".into(),
                style: "Regular".into(),
                postscript_name: "ProbeSans-Regular".into(),
                embeddable: true,
            }];
            let mut ids: Vec<uuid::Uuid> = self.faces.lock().unwrap().clone();
            ids.sort();
            for id in ids {
                out.push(FontFaceInfo {
                    family: "Embedded".into(),
                    style: "Regular".into(),
                    postscript_name: format!("Embedded-{id}"),
                    embeddable: true,
                });
            }
            out
        }
        fn fonts_stamp(&self) -> u64 {
            self.stamp.load(Ordering::SeqCst)
        }
    }

    /// Minimal Door-1 probe for the to_shape seam: validates/normalizes
    /// the rect-model sidecar (PRESERVING the probe's `gen` field — §7.7
    /// unknown-field posture, so the capture byte-comparison is honest);
    /// every other geometry call answers typed (unused by these tests).
    struct OutlineSink;

    impl OutlineSink {
        fn parse(bytes: &[u8]) -> Result<Value, EngineError> {
            let v: Value = serde_json::from_slice(bytes)
                .map_err(|e| EngineError::range("path", format!("sidecar not JSON: {e}")))?;
            if v.get("schema").and_then(Value::as_str) != Some(VECTOR_SIDECAR_SCHEMA) {
                return Err(EngineError::new(
                    ErrorCode::WrongSchema,
                    "sidecar schema must be appname.vector.v1",
                ));
            }
            Ok(v)
        }
        fn bounds_of(v: &Value) -> IRect {
            let mut b = IRect::EMPTY;
            if let Some(subs) = v.get("subpaths").and_then(Value::as_array) {
                for s in subs {
                    if let Some(r) = s.get("rect").and_then(Value::as_array) {
                        let g = |i: usize| r.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
                        b = b.union(&IRect::new(g(0), g(1), g(0) + g(2), g(1) + g(3)));
                    }
                }
            }
            b
        }
        fn refuse() -> EngineError {
            EngineError::new(
                ErrorCode::NotImplemented,
                "OutlineSink serves to_shape only",
            )
        }
    }

    impl VectorGeometry for OutlineSink {
        fn schema(&self) -> &'static str {
            VECTOR_SIDECAR_SCHEMA
        }
        fn validate_sidecar(&self, bytes: &[u8]) -> Result<GeometryResult, EngineError> {
            let v = Self::parse(bytes)?;
            Ok(GeometryResult {
                sidecar: serde_json::to_vec(&v).expect("emit"),
                bounds: Self::bounds_of(&v),
            })
        }
        fn path_param_sidecar(
            &self,
            _: &Map<String, Value>,
        ) -> Result<GeometryResult, EngineError> {
            Err(Self::refuse())
        }
        fn primitive_sidecar(
            &self,
            _: &str,
            _: &Map<String, Value>,
        ) -> Result<GeometryResult, EngineError> {
            Err(Self::refuse())
        }
        fn path_boolean(
            &self,
            _: &[u8],
            _: &[u8],
            _: VectorBoolOp,
        ) -> Result<GeometryResult, EngineError> {
            Err(Self::refuse())
        }
        fn rasterize_fill(&self, _: &[u8]) -> Result<CoveragePlaneData, EngineError> {
            Err(Self::refuse())
        }
        fn apply_edit_ops(&self, _: &[u8], _: &[Value]) -> Result<GeometryResult, EngineError> {
            Err(Self::refuse())
        }
    }

    fn text_assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
            .with_text_layout(Arc::new(TextProbe::new()))
    }

    fn full_assembly() -> EngineAssembly {
        text_assembly().with_vector_geometry(Arc::new(OutlineSink))
    }

    fn ready(assembly: &EngineAssembly) -> Engine {
        let mut e = assembly.build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "7e070000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 128, "height": 128, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, layer: Option<Uuid>, params: Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("7e070000-0000-4000-8000-0002{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.to_string()});
        }
        v.to_string()
    }

    fn env_mk(n: u32, kind: &str, layer: Uuid, params: Value, key: &str) -> String {
        let mut v: Value = serde_json::from_str(&env(n, kind, Some(layer), params)).unwrap();
        v["mergeKey"] = json!(key);
        v.to_string()
    }

    fn add_text(e: &mut Engine, n: u32, text: &str) -> Uuid {
        let id = Uuid::new_v4();
        e.apply_json(&env(
            n,
            "text.add",
            None,
            json!({"layout": "point", "origin": [10, 20], "text": text,
                   "style": {"size": 24, "fill": {"type": "solid"}},
                   "newLayerId": id.to_string()}),
        ))
        .unwrap();
        id
    }

    fn sidecar_json(e: &Engine, id: Uuid) -> Value {
        let d = e.document().unwrap();
        let td = d
            .layer(LayerId::from_uuid(id))
            .unwrap()
            .text_data()
            .unwrap();
        serde_json::from_slice(td.sidecar()).unwrap()
    }

    /// Bare-core mirror: text-requiring arms answer typed NI naming the
    /// seam; stage-4 resolution still precedes (dangling targets answer
    /// bad_target, wrong-kind targets target_kind_mismatch, even bare).
    #[test]
    fn bare_assembly_answers_typed_ni_and_stage_order_holds() {
        let bare = crate::engine::kinds::register(EngineAssembly::new());
        let mut e = ready(&bare);
        let err = e
            .apply_json(&env(
                1,
                "text.add",
                None,
                json!({"layout": "point", "origin": [0, 0]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(err.message.contains("TextLayout"), "the NI names the seam");
        // Stage 4 first: dangling target → bad_target even on bare core.
        let err = e
            .apply_json(&env(
                2,
                "text.edit",
                Some(Uuid::new_v4()),
                json!({"runs": [{"text": "x"}]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // Wrong-kind target → target_kind_mismatch before the NI gate.
        let bg = e.document().unwrap().z_order_walk()[0].as_uuid();
        let err = e
            .apply_json(&env(
                3,
                "text.edit",
                Some(bg),
                json!({"runs": [{"text": "x"}]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        // to_shape with a text service but NO vector service: the Door-9
        // half-registered assembly answers the VECTOR NI (service-to-
        // service seam; target resolves first).
        let mut e = ready(&text_assembly());
        let id = add_text(&mut e, 4, "Hi");
        let err = e
            .apply_json(&env(5, "text.to_shape", Some(id), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("VectorGeometry"),
            "to_shape names the missing Door-1 seam"
        );
    }

    /// text.add: parametric creation — zero tiles, laid-out bounds dirty,
    /// derived-and-given creation ids, §9.6 global duplicate check, state
    /// hash folds text present-only, undo/redo byte identity.
    #[test]
    fn text_add_creates_parametric_layer_with_deterministic_ids() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let pre = e.state_hash();

        // Given id.
        let id = Uuid::new_v4();
        let receipt = e
            .apply_json(&env(
                1,
                "text.add",
                None,
                json!({"layout": "point", "origin": [10, 20], "text": "Appname",
                       "newLayerId": id.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![id]);
        // 7 chars × 8 px @ (10, 20), one 16-px line.
        let want = IRect::new(10, 20, 66, 36);
        assert_eq!(
            receipt.dirty,
            vec![crate::engine::DirtyRect {
                layer: LayerId::from_uuid(id),
                rect: want,
            }]
        );
        let d = e.document().unwrap();
        let node = d.layer(LayerId::from_uuid(id)).unwrap();
        assert_eq!(node.kind().kind_str(), "text");
        assert!(node.pixels().is_none(), "parametric — zero tiles (§23.1)");
        assert_eq!(node.text_data().unwrap().bounds(), want);
        let after = e.state_hash();
        assert_ne!(pre, after, "text folds into state_hash when present");

        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "AddNode inverse");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);

        // Omitted newLayerId fills DETERMINISTICALLY from the envelope id
        // (the shape_add mechanism; replay-stable).
        let receipt = e
            .apply_json(&env(
                2,
                "text.add",
                None,
                json!({"layout": "paragraph", "rect": {"x": 0, "y": 0, "w": 64, "h": 32}}),
            ))
            .unwrap();
        let derived = Uuid::new_v5(
            &crate::engine::doc_new::APPNAME_NS,
            "creation/7e070000-0000-4000-8000-000200000002/newLayerId"
                .to_string()
                .as_bytes(),
        );
        assert_eq!(receipt.created_ids, vec![derived]);

        // §9.6 GLOBAL duplicate: a text id colliding with the bg layer.
        let bg = e.document().unwrap().z_order_walk()[0].as_uuid();
        let err = e
            .apply_json(&env(
                3,
                "text.add",
                None,
                json!({"layout": "point", "origin": [0, 0],
                       "newLayerId": bg.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);

        // Replay determinism over the whole journal (derived ids re-derive).
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// text.add semantic refusals: on_path dangling pathId (bad_target),
    /// non-group parent (target_kind_mismatch), ALL-locked parent chain.
    #[test]
    fn text_add_semantic_refusals() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let err = e
            .apply_json(&env(
                1,
                "text.add",
                None,
                json!({"layout": "on_path", "pathId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::BadTarget,
            "on_path pathId resolves (stage 4)"
        );

        let bg = e.document().unwrap().z_order_walk()[0].as_uuid();
        let err = e
            .apply_json(&env(
                2,
                "text.add",
                None,
                json!({"layout": "point", "origin": [0, 0], "parent": bg.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::TargetKindMismatch,
            "parent must be a group"
        );

        // ALL-locked group parent blocks (§11.1 transitive).
        let gid = Uuid::new_v4();
        e.apply_json(&env(
            3,
            "layer.group",
            None,
            json!({"layerIds": [bg.to_string()], "newGroupId": gid.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            4,
            "layer.set_props",
            Some(gid),
            json!({"locks": {"all": true}}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                5,
                "text.add",
                None,
                json!({"layout": "point", "origin": [0, 0], "parent": gid.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    /// text.edit: full-state runs replacement through the SetTextData
    /// value door — bounds re-derive, dirty = before ∪ after, undo/redo
    /// byte identity, stage-6 live checks, PIXELS lock gate.
    #[test]
    fn text_edit_full_state_value_round_trip() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let id = add_text(&mut e, 1, "Hi");
        let pre = e.state_hash();

        let receipt = e
            .apply_json(&env(
                2,
                "text.edit",
                Some(id),
                json!({"runs": [{"text": "Appname 1.0"}],
                       "paragraph": {"alignment": "center"}}),
            ))
            .unwrap();
        // before (2 ch) ∪ after (11 ch) @ (10, 20).
        assert_eq!(
            receipt.dirty,
            vec![crate::engine::DirtyRect {
                layer: LayerId::from_uuid(id),
                rect: IRect::new(10, 20, 98, 36),
            }]
        );
        let v = sidecar_json(&e, id);
        assert_eq!(
            v["runs"][0]["text"], "Appname 1.0",
            "full-state replacement"
        );
        assert_eq!(v["paragraph"]["alignment"], "center");
        let after = e.state_hash();

        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "SetTextData value inverse");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);

        // Stage-6 live check: a zero-length run answers out_of_range.
        let err = e
            .apply_json(&env(
                3,
                "text.edit",
                Some(id),
                json!({"runs": [{"text": ""}]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);

        // PIXELS lock blocks content-mutating text kinds (family rule),
        // transitively through a group (§11.1).
        let gid = Uuid::new_v4();
        e.apply_json(&env(
            4,
            "layer.group",
            None,
            json!({"layerIds": [id.to_string()], "newGroupId": gid.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            5,
            "layer.set_props",
            Some(gid),
            json!({"locks": {"pixels": true}}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                6,
                "text.edit",
                Some(id),
                json!({"runs": [{"text": "blocked"}]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);

        // Replay determinism.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// text.set_style / text.set_paragraph: range gates (arm: start ≤ end;
    /// service: end ≤ len — both out_of_range) and patch application.
    #[test]
    fn style_and_paragraph_range_battery() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let id = add_text(&mut e, 1, "Appname"); // len 7

        let err = e
            .apply_json(&env(
                2,
                "text.set_style",
                Some(id),
                json!({"range": [5, 2], "style": {"size": 32}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "start > end (contract case)"
        );

        let err = e
            .apply_json(&env(
                3,
                "text.set_style",
                Some(id),
                json!({"range": [0, 8], "style": {"size": 32}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::OutOfRange,
            "end > len (family live rule)"
        );

        let pre = e.state_hash();
        e.apply_json(&env(
            4,
            "text.set_style",
            Some(id),
            json!({"range": [0, 5], "style": {"size": 32}}),
        ))
        .unwrap();
        assert_eq!(sidecar_json(&e, id)["runs"][0]["style"]["size"], 32);
        assert_ne!(e.state_hash(), pre);
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre);
        e.redo().unwrap();

        e.apply_json(&env(
            5,
            "text.set_paragraph",
            Some(id),
            json!({"range": [0, 0], "paragraph": {"alignment": "center"}}),
        ))
        .unwrap();
        assert_eq!(sidecar_json(&e, id)["paragraph"]["alignment"], "center");

        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// §10.5 class-M coalescing consumes the SetTextData merge arm: a
    /// same-mergeKey typing run holds ONE journal row and ONE history
    /// step; one undo restores the pre-run state; merged-journal replay is
    /// byte-identical.
    #[test]
    fn text_edit_merge_key_coalesces_typing_sessions() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let id = add_text(&mut e, 1, "A");
        let pre_run = e.state_hash();
        let journal_before = e.journal().len();
        let history_before = e.history().len();

        let key = format!("text.edit:{id}#runs:11111111-2222-4333-8444-555555555555");
        let r1 = e
            .apply_json(&env_mk(
                2,
                "text.edit",
                id,
                json!({"runs": [{"text": "Ap"}]}),
                &key,
            ))
            .unwrap();
        assert!(!r1.coalesced);
        let r2 = e
            .apply_json(&env_mk(
                3,
                "text.edit",
                id,
                json!({"runs": [{"text": "App"}]}),
                &key,
            ))
            .unwrap();
        assert!(r2.coalesced, "same-key consecutive typing merges (§10.5)");
        let r3 = e
            .apply_json(&env_mk(
                4,
                "text.edit",
                id,
                json!({"runs": [{"text": "Appname"}]}),
                &key,
            ))
            .unwrap();
        assert!(r3.coalesced);

        assert_eq!(
            e.journal().len(),
            journal_before + 1,
            "one row, newest wins"
        );
        assert_eq!(e.history().len(), history_before + 1, "one undo step");
        assert_eq!(sidecar_json(&e, id)["runs"][0]["text"], "Appname");
        let final_hash = e.state_hash();

        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre_run, "earliest before in one step");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), final_hash, "latest after");

        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), final_hash);
    }

    /// Door 9 — text.to_shape: the one-time bake VALUE-CAPTURES its
    /// output geometry (probe outline extraction is deliberately
    /// nondeterministic — replay byte-identity holds ONLY because the
    /// capture is consumed); undo restores the TEXT node with editability
    /// intact; the shape carries the first run's fill (pinned reading).
    #[test]
    fn to_shape_bakes_captured_geometry_and_stays_invertible() {
        let asm = full_assembly();
        let mut e = ready(&asm);
        let id = Uuid::new_v4();
        e.apply_json(&env(
            1,
            "text.add",
            None,
            json!({"layout": "point", "origin": [10, 20], "text": "Hi",
                   "style": {"fill": {"type": "solid", "color": {"model": "rgb"}}},
                   "newLayerId": id.to_string()}),
        ))
        .unwrap();
        let pre = e.state_hash();

        e.apply_json(&env(2, "text.to_shape", Some(id), json!({})))
            .unwrap();
        let d = e.document().unwrap();
        let node = d.layer(LayerId::from_uuid(id)).unwrap();
        assert_eq!(
            node.kind().kind_str(),
            "shape",
            "kind swapped in place (§40.7)"
        );
        let sd = node.shape_data().unwrap();
        assert_eq!(
            sd.fill()["type"],
            "solid",
            "first run's fill carried (pinned reading)"
        );
        let pid = sd.path();
        assert!(d.path(pid).is_some(), "outline path landed in the arena");
        assert_eq!(
            d.path(pid).unwrap().bounds(),
            IRect::new(10, 20, 26, 36),
            "outline bounds = the laid-out text bounds (probe model)"
        );
        let baked = e.state_hash();

        // D-48 capture pin: replay consumes the captured geometry — with
        // the nondeterministic probe, byte-identity holds ONLY then.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), baked);

        // Undo restores TEXT; redo re-bakes from the delta (no re-run).
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "SetContentKind + AddPath inverses");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), baked, "redo re-bakes from the delta");

        // Editability after undo: a live text.edit lands (this truncates
        // the undone bake from the journal — §10.2 tail truncation).
        e.undo().unwrap();
        e.apply_json(&env(
            3,
            "text.edit",
            Some(id),
            json!({"runs": [{"text": "still editable"}]}),
        ))
        .unwrap();

        // PIXELS/ALL gate (§40.7): a locked text layer refuses the bake.
        let id2 = add_text(&mut e, 4, "locked");
        e.apply_json(&env(
            5,
            "layer.set_props",
            Some(id2),
            json!({"locks": {"pixels": true}}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(6, "text.to_shape", Some(id2), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    /// Door 7 persistence: text/{layerId}.json + textRef + textBounds;
    /// save→load→save byte-stable WITH the service; a text-blind (bare)
    /// assembly carries the layer OPAQUELY — same state hash, same bytes.
    #[test]
    fn package_round_trip_carries_text_with_and_without_the_service() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let id = add_text(&mut e, 1, "Appname");
        let hash = e.state_hash();

        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let names_have = |b: &[u8], n: &str| -> bool {
            let mut za = zip::ZipArchive::new(std::io::Cursor::new(b.to_vec())).unwrap();
            (0..za.len()).any(|i| za.by_index(i).unwrap().name() == n)
        };
        assert!(
            names_have(&bytes, &format!("text/{id}.json")),
            "the textRef convention entry exists"
        );

        // Service-carrying load: Door-6 validation pass re-normalizes.
        let loaded = crate::io::package::load_package_bytes_with(&asm, &bytes).unwrap();
        assert_eq!(loaded.engine.state_hash(), hash);
        let again = crate::io::package::save_package_bytes(&loaded.engine).unwrap();
        assert_eq!(bytes, again, "save→load→save byte-stable (service)");

        // Text-blind load: opaque carriage (§7.7 lenient) — recorded
        // textBounds + verbatim sidecar keep the fold identical.
        let bare = crate::engine::kinds::register(EngineAssembly::new());
        let opaque = crate::io::package::load_package_bytes_with(&bare, &bytes).unwrap();
        assert_eq!(
            opaque.engine.state_hash(),
            hash,
            "opaque carriage holds the fold"
        );
        let again = crate::io::package::save_package_bytes(&opaque.engine).unwrap();
        assert_eq!(bytes, again, "…and re-emits byte-identically");
        // Mutating arms on the blind assembly answer the typed NI mirror.
        let mut blind = opaque.engine;
        let err = blind
            .apply_json(&env(
                2,
                "text.edit",
                Some(id),
                json!({"runs": [{"text": "x"}]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    /// Door 10 — the FontDb bridge: ingestion registers into the service,
    /// enumeration + stamp reflect it, a refused face answers typed, and a
    /// package round-trip re-registers embedded faces at load (§44.2
    /// step 2).
    #[test]
    fn font_bridge_registers_enumerates_and_survives_reopen() {
        let asm = text_assembly();
        let mut e = ready(&asm);
        let svc_faces = |e: &Engine| e.text_faces().unwrap();
        assert_eq!(svc_faces(&e).len(), 1, "the probe's base face");

        let err = e.add_font_asset(Vec::new()).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "service refusal is typed");

        let face = b"probe-face-bytes".to_vec();
        let id = e.add_font_asset(face.clone()).unwrap();
        assert_eq!(id, crate::engine::assets::asset_id_for(&face));
        assert_eq!(svc_faces(&e).len(), 2, "registered face enumerates");
        assert!(svc_faces(&e)
            .iter()
            .any(|f| f.postscript_name == format!("Embedded-{id}")));

        // Round-trip: the face blob rides assets/{id}.font and re-registers
        // into a FRESH service at load.
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let fresh = text_assembly();
        let loaded = crate::io::package::load_package_bytes_with(&fresh, &bytes).unwrap();
        assert!(
            loaded
                .engine
                .text_faces()
                .unwrap()
                .iter()
                .any(|f| f.postscript_name == format!("Embedded-{id}")),
            "§44.2 step 2: the document is its own first fallback resource"
        );
    }
}
