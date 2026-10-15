// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §40.7 vector command entries (verbatim rows), §20.1–§20.4,
//! §40.4 `selection.from_path`; DD ㉕; contracts
//! `contracts/commands/vector.*.yaml` + `selection.from_path.yaml`.
//!
//! The seven `vector.*` kind arms + `selection.from_path` (CP-1.0 door
//! slice, TEXTVEC Door 3/5). Core stays geometry-blind: every geometry
//! operation crosses the [`crate::engine::vector_ops::VectorGeometry`]
//! seam; arms on an assembly with no registered service answer the typed
//! NI mirror (house pattern). Staging per §40.6: stage-5 schema already
//! ran — arms do semantic reads → target/id resolution → lock checks →
//! execution through the recorder-coupled doors.
//!
//! D-48 (bound posture, INTEGRATION-log 2026-08-13): `vector.shape_add`
//! VALUE-CAPTURES its generated sidecar — replay never re-runs
//! constructor trig (the capture table rides the journal).
//!
//! Pinned readings recorded in the lane log (fold-in candidates):
//! `path_delete` refuses `target_in_use` on SHAPE references too (the
//! contract names vector masks; a dangling shape `pathRef` would violate
//! the document.v1 resolve rule); `path_bool` doc-path mode bakes into
//! the FIRST operand with `flatten` governing operand consumption (the
//! §40.7 undo row's reading); `path_bool` shape-mode APPEND is refused
//! typed pending an O-INT escalation (the XOR excludes `pathIds` in shape
//! mode, leaving the appended operand unspecified — not invented).

use std::sync::Arc;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::engine::delta::ShapeDataBlock;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::engine::vector_ops::{sidecar_content_hash, GeometryResult, VectorBoolOp};
use crate::geom::IRect;
use crate::graph::id::{LayerId, PathId};
use crate::graph::node::{LayerNode, ShapeLayerData};
use crate::graph::path_node::{PathNode, PathValueBlock};
use crate::tile::map::TileCoord;
use crate::tile::TILE;

pub struct VectorPathNewKind;
pub struct VectorPathEditKind;
pub struct VectorPathDeleteKind;
pub struct VectorPathBoolKind;
pub struct VectorShapeAddKind;
pub struct VectorShapeSetStyleKind;
pub struct VectorEditKind;
pub struct SelectionFromPathKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(VectorPathNewKind),
        Arc::new(VectorPathEditKind),
        Arc::new(VectorPathDeleteKind),
        Arc::new(VectorPathBoolKind),
        Arc::new(VectorShapeAddKind),
        Arc::new(VectorShapeSetStyleKind),
        Arc::new(VectorEditKind),
        Arc::new(SelectionFromPathKind),
    ]
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// §9.6 creation-id fill: the contract marks creation ids optional
/// ("wrapper fills when omitted"); the HEADLESS door fills
/// deterministically — UUIDv5 over (envelope id, role) — so journal
/// replay and action playback reproduce identical ids with no wrapper
/// present (D-48).
pub(crate) fn creation_id(cx: &KindContext<'_>, field: &str) -> Result<Uuid, EngineError> {
    match cx.params().get(field) {
        Some(v) => {
            let id = v
                .as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::range(field, format!("`{field}` must be a UUID")))?;
            Ok(id)
        }
        None => Ok(Uuid::new_v5(
            &crate::engine::doc_new::APPNAME_NS,
            format!("creation/{}/{}", cx.envelope().id, field).as_bytes(),
        )),
    }
}

fn resolve_path_param(cx: &KindContext<'_>, field: &str) -> Result<PathId, EngineError> {
    let id = cx
        .params()
        .get(field)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(PathId::from_uuid)
        .ok_or_else(|| EngineError::missing(field))?;
    if cx.document().path(id).is_none() {
        return Err(EngineError::bad_target(format!(
            "path {id} does not resolve (§2.7 arenas)"
        )));
    }
    Ok(id)
}

/// Shape-layer target resolution (§40.7 family rule): target.layerId MUST
/// resolve and MUST be a Shape layer.
fn resolve_shape_target(cx: &KindContext<'_>) -> Result<LayerId, EngineError> {
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
    if node.shape_data().is_none() {
        return Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "the target is not a Shape layer (§20.2)",
        ));
    }
    Ok(lid)
}

/// §11.1/§40.1 effective locks: strictest along the ancestry — the UNION
/// of every flag from the node to the root (all flags transitive).
fn effective_locks(cx: &KindContext<'_>, layer: LayerId) -> crate::graph::node::LockFlags {
    cx.document().effective_locks(layer)
}

/// PIXELS/ALL gate for the shape-content kinds (§40.7 `locks:` rows).
fn refuse_shape_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    use crate::graph::node::LockFlags;
    let locks = effective_locks(cx, layer);
    if locks.contains(LockFlags::ALL) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "ALL lock along the ancestry blocks (§11.1 transitive rule)",
        ));
    }
    if locks.contains(LockFlags::PIXELS) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "PIXELS lock blocks shape-content edits (§40.7; §11.1 transitive rule)",
        ));
    }
    Ok(())
}

/// The referencing set for `path_delete`/`path_edit` dirty (§40.7): shape
/// layers by base path or bool_ops operand. (Vector masks join this walk
/// when the vector-mask kind arm lands — MaskKind::Vector is not in the
/// arena yet.)
fn referencing_layers(cx: &KindContext<'_>, id: PathId) -> Vec<LayerId> {
    cx.document().path_referencing_layers(id)
}

fn path_param_object<'a>(
    cx: &'a KindContext<'_>,
    field: &str,
) -> Result<&'a Map<String, Value>, EngineError> {
    cx.params()
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| EngineError::missing(field))
}

// ---------------------------------------------------------------------------
// vector.path_new · R · inverse delta (§40.7 entry verbatim)
// ---------------------------------------------------------------------------

impl KindImpl for VectorPathNewKind {
    fn kind(&self) -> &'static str {
        "vector.path_new"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let path_obj = path_param_object(cx, "path")?.clone();
        let name = cx
            .params()
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let new_id = creation_id(cx, "newPathId")?;
        let svc = cx.require_vector_geometry()?;
        // §20.1 payload validation + normalization (empty path →
        // out_of_range etc. — the service speaks the §40.7 error rows).
        let g = svc.path_param_sidecar(&path_obj)?;
        let hash = sidecar_content_hash(&g.sidecar);
        let pid = cx.insert_path(PathNode::new(
            PathId::from_uuid(new_id),
            name,
            Arc::new(g.sidecar),
            hash,
            g.bounds,
        ))?;
        // §40.7 dirty: no composite D — naked paths dirty no pixels.
        Ok(KindOutcome {
            created_ids: vec![pid.as_uuid()],
        })
    }
}

// ---------------------------------------------------------------------------
// vector.path_edit · M · inverse delta (full-state geometry replacement)
// ---------------------------------------------------------------------------

impl KindImpl for VectorPathEditKind {
    fn kind(&self) -> &'static str {
        "vector.path_edit"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let pid = resolve_path_param(cx, "pathId")?;
        let path_obj = path_param_object(cx, "path")?.clone();
        // §40.7 locks row: none — a path referenced by a locked layer's
        // vector mask is STILL editable (the lock gates the layer's
        // surfaces, not the shared path).
        let svc = cx.require_vector_geometry()?;
        let g = svc.path_param_sidecar(&path_obj)?;
        let hash = sidecar_content_hash(&g.sidecar);
        let before_bounds = cx.document().path(pid).expect("resolved").bounds();
        let name = cx
            .document()
            .path(pid)
            .expect("resolved")
            .name()
            .map(str::to_string);
        cx.set_path_value(
            pid,
            PathValueBlock {
                name,
                sidecar: Arc::new(g.sidecar),
                content_hash: hash,
                bounds: g.bounds,
            },
        )?;
        // §40.7 dirty: no composite D for naked paths; referencing shape
        // layers repaint over bounds before ∪ after (§10.3 vector row:
        // "D over affected bounds — shape layers only").
        let union = before_bounds.union(&g.bounds);
        for lid in referencing_layers(cx, pid) {
            cx.note_layer_dirty(lid, union);
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// vector.path_delete · R · inverse delta
// ---------------------------------------------------------------------------

impl KindImpl for VectorPathDeleteKind {
    fn kind(&self) -> &'static str {
        "vector.path_delete"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let pid = resolve_path_param(cx, "pathId")?;
        let refs = referencing_layers(cx, pid);
        if !refs.is_empty() {
            let names: Vec<String> = refs
                .iter()
                .filter_map(|l| cx.document().layer(*l).map(|n| n.name().to_string()))
                .collect();
            return Err(EngineError::new(
                ErrorCode::TargetInUse,
                format!(
                    "path {pid} is referenced by layer(s) {names:?} (§40.7 \
                     target_in_use; detail lists referencing layers)"
                ),
            ));
        }
        cx.remove_path(pid)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// vector.path_bool · R · inverse delta (§40.7 XOR addressing)
// ---------------------------------------------------------------------------

impl KindImpl for VectorPathBoolKind {
    fn kind(&self) -> &'static str {
        "vector.path_bool"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let op = cx
            .params()
            .get("op")
            .and_then(Value::as_str)
            .and_then(VectorBoolOp::from_wire)
            .ok_or_else(|| EngineError::missing("op"))?;
        let flatten = cx
            .params()
            .get("flatten")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let has_path_ids = cx.params().contains_key("pathIds");
        let has_target = cx
            .envelope()
            .target
            .as_ref()
            .and_then(|t| t.layer_id)
            .is_some();
        // §40.7 validates, verbatim: "exactly one addressing mode —
        // pathIds (len ≥ 2, doc paths) XOR shape-layer target (bool_ops
        // append) → error.schema.missing_field. The XOR spans params and
        // the envelope target block — a semantic check, not
        // params-schema."
        if has_path_ids == has_target {
            return Err(EngineError::with_field(
                ErrorCode::MissingField,
                "pathIds|target.layerId",
                format!(
                    "vector.path_bool addresses EXACTLY ONE mode — params.pathIds XOR a \
                     shape-layer target ({} given; §40.7)",
                    if has_path_ids { "both" } else { "neither" }
                ),
            ));
        }

        if has_path_ids {
            // Doc-path mode. Locks: none (§40.7).
            let raw = cx
                .params()
                .get("pathIds")
                .and_then(Value::as_array)
                .ok_or_else(|| EngineError::missing("pathIds"))?
                .clone();
            let mut pids = Vec::with_capacity(raw.len());
            for v in &raw {
                let id = v
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .map(PathId::from_uuid)
                    .ok_or_else(|| EngineError::range("pathIds", "members must be uuids"))?;
                if cx.document().path(id).is_none() {
                    return Err(EngineError::bad_target(format!(
                        "pathIds member {id} does not resolve"
                    )));
                }
                pids.push(id);
            }
            let svc = cx.require_vector_geometry()?;
            // Left fold (§20.3): result bakes into the FIRST operand
            // (§40.7 undo row: "SetProp geometry (+ RemoveNode of consumed
            // paths when flatten)" — pinned reading: flatten governs
            // operand consumption only).
            let mut acc: GeometryResult = {
                let base = cx.document().path(pids[0]).expect("resolved");
                GeometryResult {
                    sidecar: base.sidecar().as_ref().clone(),
                    bounds: base.bounds(),
                }
            };
            for pid in &pids[1..] {
                let operand = cx.document().path(*pid).expect("resolved");
                acc = svc.path_boolean(&acc.sidecar, operand.sidecar(), op)?;
            }
            let hash = sidecar_content_hash(&acc.sidecar);
            let name = cx
                .document()
                .path(pids[0])
                .expect("resolved")
                .name()
                .map(str::to_string);
            cx.set_path_value(
                pids[0],
                PathValueBlock {
                    name,
                    sidecar: Arc::new(acc.sidecar),
                    content_hash: hash,
                    bounds: acc.bounds,
                },
            )?;
            if flatten {
                // flatten:true consumes the other operands (doc paths
                // removed) — refuse if still referenced (dangling pathRef
                // would violate the document.v1 resolve rule).
                for pid in &pids[1..] {
                    let refs = referencing_layers(cx, *pid);
                    if !refs.is_empty() {
                        return Err(EngineError::new(
                            ErrorCode::TargetInUse,
                            format!(
                                "flatten would consume path {pid}, still referenced by \
                                 {refs:?} (§40.7 target_in_use)"
                            ),
                        ));
                    }
                    cx.remove_path(*pid)?;
                }
            }
            // §40.7 dirty: doc-path mode — no composite D.
            Ok(KindOutcome::default())
        } else {
            // Shape-layer mode. Target must be a Shape (raster →
            // target_kind_mismatch, contract test row); PIXELS/ALL block.
            let lid = resolve_shape_target(cx)?;
            refuse_shape_locked(cx, lid)?;
            if !flatten {
                // ESCALATED (O-INT), not invented: the XOR excludes
                // params.pathIds in shape mode, so the operand of a
                // non-destructive bool_ops APPEND is unspecified by the
                // contract. Typed refusal until the ruling lands.
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "vector.path_bool shape-mode APPEND: the contract's XOR addressing \
                     leaves the appended operand unspecified (pathIds are doc-path mode \
                     only) — escalated to O-INT; §20.3 bool_ops appends land on its \
                     ruling",
                ));
            }
            // flatten:true = "merge shape components" (§20.3): bake the
            // composite (left fold of bool_ops over the base path) into
            // the base path and clear bool_ops. Value-block deltas make
            // this cleanly invertible (stronger than the §10.5 snapshot
            // posture; noted in the lane log).
            let svc = cx.require_vector_geometry()?;
            let (base_pid, ops): (PathId, Vec<(VectorBoolOp, PathId)>) = {
                let sd = cx
                    .document()
                    .layer(lid)
                    .expect("resolved")
                    .shape_data()
                    .expect("shape");
                (sd.path(), sd.bool_ops().to_vec())
            };
            if ops.is_empty() {
                return Ok(KindOutcome::default()); // §40.1 no-op push
            }
            let mut acc: GeometryResult = {
                let base = cx.document().path(base_pid).ok_or_else(|| {
                    EngineError::bad_target(format!("shape base path {base_pid} dangling"))
                })?;
                GeometryResult {
                    sidecar: base.sidecar().as_ref().clone(),
                    bounds: base.bounds(),
                }
            };
            for (bop, pid) in &ops {
                let operand = cx.document().path(*pid).ok_or_else(|| {
                    EngineError::bad_target(format!("bool_ops operand {pid} dangling"))
                })?;
                acc = svc.path_boolean(&acc.sidecar, operand.sidecar(), *bop)?;
            }
            let hash = sidecar_content_hash(&acc.sidecar);
            let before_bounds = cx.document().path(base_pid).expect("resolved").bounds();
            let name = cx
                .document()
                .path(base_pid)
                .expect("resolved")
                .name()
                .map(str::to_string);
            let after_bounds = acc.bounds;
            cx.set_path_value(
                base_pid,
                PathValueBlock {
                    name,
                    sidecar: Arc::new(acc.sidecar),
                    content_hash: hash,
                    bounds: after_bounds,
                },
            )?;
            let (fill, stroke) = {
                let sd = cx
                    .document()
                    .layer(lid)
                    .expect("resolved")
                    .shape_data()
                    .expect("shape");
                (sd.fill().clone(), sd.stroke().clone())
            };
            cx.set_shape_data(
                lid,
                ShapeDataBlock {
                    fill,
                    stroke,
                    bool_ops: Vec::new(),
                },
            )?;
            // §40.7 dirty: shape mode — D = shape bounds.
            cx.note_layer_dirty(lid, before_bounds.union(&after_bounds));
            Ok(KindOutcome::default())
        }
    }
}

// ---------------------------------------------------------------------------
// vector.shape_add · R · inverse delta (D-48 value-captured sidecar)
// ---------------------------------------------------------------------------

impl KindImpl for VectorShapeAddKind {
    fn kind(&self) -> &'static str {
        "vector.shape_add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Stage-5 already enforced: fill+stroke required, path XOR
        // primitive (top-level oneOf), primitive cross-fields (if/then).
        let fill = cx.params().get("fill").cloned().unwrap_or(Value::Null);
        // q-scenario/Q-17 (MAJOR, filed tick 89; operator WORD 14 (4): "refuse a
        // fill that matches no oneOf branch"). Stage 5 enforces that `fill` is
        // PRESENT and stops there, so `fill:{color:…}` with no `type` — which
        // every branch of $defs/fillContent forbids, each requiring `type` —
        // was accepted here and rendered BLACK, discarding the caller's colour
        // with no diagnostic. decode_fill_content is the door the fill-LAYER
        // path already uses; running the same decode here refuses the same
        // shapes it refuses there. The decoded value is dropped: the node
        // stores the wire form, and this call is the validation.
        crate::engine::fill_reg::decode_fill_content(&fill, "fill")?;
        let stroke = cx.params().get("stroke").cloned().unwrap_or(Value::Null);
        // Read the stroke's §40.7 dirty reach off the payload HERE, while
        // it is still in hand — it is moved into the layer node below.
        let stroke_inflate = crate::engine::vector_ops::shape_stroke_dirty_inflate(&stroke);
        let new_layer = creation_id(cx, "newLayerId")?;
        let new_path = Uuid::new_v5(
            &crate::engine::doc_new::APPNAME_NS,
            format!("creation/{}/shapePath", cx.envelope().id).as_bytes(),
        );
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
            if effective_locks(cx, parent).contains(LockFlags::ALL) {
                return Err(EngineError::new(
                    ErrorCode::Locked,
                    "ALL lock along the parent chain blocks (§40.7)",
                ));
            }
        }
        let index = cx
            .params()
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize)
            .unwrap_or_else(|| {
                cx.document()
                    .children_of(parent)
                    .map(|c| c.len())
                    .unwrap_or(0)
            })
            .min(
                cx.document()
                    .children_of(parent)
                    .map(|c| c.len())
                    .unwrap_or(0),
            );

        // Geometry: consume THIS envelope's capture on replay (D-48 —
        // never re-run constructor trig), else generate and CAPTURE.
        let geom: Arc<GeometryResult> = match cx.vector_capture() {
            Some(g) => Arc::clone(g),
            None => {
                let svc = cx.require_vector_geometry()?;
                let g = if let Some(p) = cx.params().get("path").and_then(Value::as_object) {
                    svc.path_param_sidecar(p)?
                } else {
                    let prim = cx
                        .params()
                        .get("primitive")
                        .and_then(Value::as_object)
                        .ok_or_else(|| EngineError::missing("primitive"))?;
                    let kind = prim
                        .get("kind")
                        .and_then(Value::as_str)
                        .ok_or_else(|| EngineError::missing("primitive.kind"))?;
                    if kind == "custom" {
                        // §28.1 shape assets are not in the v1 asset store
                        // (Samples/RasterImage only) — typed, recorded.
                        return Err(EngineError::new(
                            ErrorCode::NotImplemented,
                            "primitive kind \"custom\" needs the §28.1 shape-asset store \
                             (not in the v1 AssetStore) — typed refusal, recorded",
                        ));
                    }
                    svc.primitive_sidecar(kind, prim)?
                };
                let g = Arc::new(g);
                cx.record_vector_capture(Arc::clone(&g));
                g
            }
        };
        let hash = sidecar_content_hash(&geom.sidecar);
        let pid = cx.insert_path(PathNode::new(
            PathId::from_uuid(new_path),
            None,
            Arc::new(geom.sidecar.clone()),
            hash,
            geom.bounds,
        ))?;
        let lid = LayerId::from_uuid(new_layer);
        cx.insert_layer(
            parent,
            index,
            LayerNode::new_shape(
                lid,
                "Shape",
                ShapeLayerData {
                    path: pid,
                    fill,
                    stroke,
                    bool_ops: Vec::new(),
                },
            ),
        )?;
        // §40.7 dirty: D = shape bounds, INFLATED BY THE STROKE.
        //
        // The contract row reads "D = shape bounds", and the primitive
        // hull is not what the compositor paints: since the §20.2
        // SHAPE-LAYER stroke slice a `widthPx > 0` layer inks a band
        // reaching outward from that hull (width/2 for the Center
        // default, the full width for Outside, further still off a
        // miter apex), and every one of those pixels outside D is a
        // pixel the shell is never told to repaint — user-visible stale
        // ink. Before that slice `widthPx > 0` was REFUSED at the
        // compositor, so the exposure was unreachable; serving strokes
        // is what makes it reachable, and this is the other half of
        // serving them. The sibling `shape_set_style` row already
        // inflates on a width CHANGE for exactly this reason.
        //
        // Whether "shape bounds" means the primitive or the painted
        // result is FI-25, a fold-in wording question; the staleness is
        // the same either way, so the bound does not wait on it.
        cx.note_layer_dirty(lid, geom.bounds.inflate(stroke_inflate, stroke_inflate));
        Ok(KindOutcome {
            created_ids: vec![new_layer, new_path],
        })
    }
}

// ---------------------------------------------------------------------------
// vector.shape_set_style · M · inverse delta (SetProp)
// ---------------------------------------------------------------------------

impl KindImpl for VectorShapeSetStyleKind {
    fn kind(&self) -> &'static str {
        "vector.shape_set_style"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = resolve_shape_target(cx)?;
        refuse_shape_locked(cx, lid)?;
        let fill_new = cx.params().get("fill").cloned();
        // q-scenario/Q-17's second door: the finding names "`fill` wherever
        // $defs/fillContent is used", and a partial style update carries a
        // COMPLETE new fill when it carries one at all (§10.3 full-state).
        // Absent stays untouched; present is decoded, so the same untyped
        // fill cannot enter through the edit door either.
        if let Some(f) = &fill_new {
            crate::engine::fill_reg::decode_fill_content(f, "fill")?;
        }
        let stroke_new = cx.params().get("stroke").cloned();
        // Partial per the full-state rule (§10.3): each PRESENT field
        // carries its complete new value; a missing field means
        // "untouched"; empty params = the §40.1 no-op push.
        let (fill_before, stroke_before, bool_ops, base_pid) = {
            let sd = cx
                .document()
                .layer(lid)
                .expect("resolved")
                .shape_data()
                .expect("shape");
            (
                sd.fill().clone(),
                sd.stroke().clone(),
                sd.bool_ops().to_vec(),
                sd.path(),
            )
        };
        let stroke_after = stroke_new.clone().unwrap_or_else(|| stroke_before.clone());
        // §40.7 dirty: D = shape bounds; stroke width changes inflate —
        // use max of before/after widthPx (mechanical read; the payload
        // stays engine-opaque otherwise).
        let width = |v: &Value| -> f64 { v.get("widthPx").and_then(Value::as_f64).unwrap_or(0.0) };
        let inflate = width(&stroke_before).max(width(&stroke_after)).ceil() as i32;
        cx.set_shape_data(
            lid,
            ShapeDataBlock {
                fill: fill_new.unwrap_or(fill_before),
                stroke: stroke_after,
                bool_ops,
            },
        )?;
        let b = cx
            .document()
            .path(base_pid)
            .map(|p| p.bounds())
            .unwrap_or(IRect::EMPTY);
        let dirty = IRect::new(
            b.min.x - inflate,
            b.min.y - inflate,
            b.max.x + inflate,
            b.max.y + inflate,
        );
        cx.note_layer_dirty(lid, dirty);
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// vector.edit · R · inverse delta (incremental anchor ops; §10.5 consumer)
// ---------------------------------------------------------------------------

impl KindImpl for VectorEditKind {
    fn kind(&self) -> &'static str {
        "vector.edit"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let lid = resolve_shape_target(cx)?;
        refuse_shape_locked(cx, lid)?;
        let ops = cx
            .params()
            .get("ops")
            .and_then(Value::as_array)
            .ok_or_else(|| EngineError::missing("ops"))?
            .clone();
        if ops.is_empty() {
            return Ok(KindOutcome::default()); // §40.1 no-op push
        }
        // Field-level validation per the contract's cross-field rows
        // (stage-5 checks array presence only; item rules live here).
        const OPS: [&str; 6] = [
            "add_anchor",
            "delete_anchor",
            "move_anchor",
            "set_handles",
            "close",
            "open",
        ];
        for (i, row) in ops.iter().enumerate() {
            let obj = row
                .as_object()
                .ok_or_else(|| EngineError::range("ops", format!("ops[{i}] must be an object")))?;
            let op = obj
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| EngineError::missing(&format!("ops[{i}].op")))?;
            if !OPS.contains(&op) {
                return Err(EngineError::range(
                    &format!("ops[{i}].op"),
                    format!("unknown anchor op \"{op}\" (§40.7 enum)"),
                ));
            }
            for req in ["subpath", "index"] {
                if obj.get(req).and_then(Value::as_u64).is_none() {
                    return Err(EngineError::missing(&format!("ops[{i}].{req}")));
                }
            }
            match op {
                "add_anchor" | "move_anchor" => {
                    if obj.get("position").and_then(Value::as_array).is_none() {
                        return Err(EngineError::missing(&format!("ops[{i}].position")));
                    }
                }
                "set_handles"
                    if obj.get("inHandle").is_none() && obj.get("outHandle").is_none() =>
                {
                    return Err(EngineError::missing(&format!(
                        "ops[{i}].inHandle|ops[{i}].outHandle"
                    )));
                }
                _ => {}
            }
        }
        let base_pid = cx
            .document()
            .layer(lid)
            .expect("resolved")
            .shape_data()
            .expect("shape")
            .path();
        let svc = cx.require_vector_geometry()?;
        let (before_bounds, sidecar) = {
            let p = cx.document().path(base_pid).ok_or_else(|| {
                EngineError::bad_target(format!("shape base path {base_pid} dangling"))
            })?;
            (p.bounds(), Arc::clone(p.sidecar()))
        };
        // ATOMIC application (§10.2 stage 6): the service applies the whole
        // list or errors with the sidecar untouched.
        let g = svc.apply_edit_ops(&sidecar, &ops)?;
        let hash = sidecar_content_hash(&g.sidecar);
        let name = cx
            .document()
            .path(base_pid)
            .expect("resolved")
            .name()
            .map(str::to_string);
        let after_bounds = g.bounds;
        cx.set_path_value(
            base_pid,
            PathValueBlock {
                name,
                sidecar: Arc::new(g.sidecar),
                content_hash: hash,
                bounds: after_bounds,
            },
        )?;
        // §40.7 dirty: D = shape bounds before ∪ after.
        cx.note_layer_dirty(lid, before_bounds.union(&after_bounds));
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// selection.from_path · R · inverse delta (§40.4; writes selection)
// ---------------------------------------------------------------------------

impl KindImpl for SelectionFromPathKind {
    fn kind(&self) -> &'static str {
        "selection.from_path"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        use crate::engine::mask_ops::{combine, SelectOp};
        let pid = resolve_path_param(cx, "pathId")?;
        let antialias = cx
            .params()
            .get("antialias")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let feather = cx
            .params()
            .get("feather")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let op = cx
            .params()
            .get("op")
            .and_then(Value::as_str)
            .and_then(SelectOp::from_wire)
            .unwrap_or(SelectOp::Replace);
        if feather > 0.0 {
            // The §13 feather pipeline (true Gaussian on the mask) is the
            // O-TOOLS engine — typed refusal, never an approximation.
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "selection.from_path{feather > 0} routes through the §13 mask feather \
                 engine (O-TOOLS) — typed refusal, recorded",
            ));
        }
        let svc = cx.require_vector_geometry()?;
        let sidecar = Arc::clone(cx.document().path(pid).expect("resolved").sidecar());
        // §40.4: analytic coverage — bit-exact with vector-mask
        // rasterization of the same path (the contract's named test).
        let plane = svc.rasterize_fill(&sidecar)?;
        let rect = plane.rect;
        let sample = |x: i32, y: i32| -> f32 {
            if x < rect.min.x || x >= rect.max.x || y < rect.min.y || y >= rect.max.y {
                return 0.0;
            }
            let idx = (y - rect.min.y) as usize * rect.width() as usize + (x - rect.min.x) as usize;
            let v = plane.values[idx];
            if antialias {
                v
            } else {
                // antialias:false = binarize analytic coverage at 0.5.
                if v >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
        };

        // Same Selection/DeltaRecorder doors as the O-TOOLS engines
        // (install_selection_tile_plane; §13.2 combine math shared).
        let existing_coords: Vec<TileCoord> = cx
            .document()
            .selection()
            .mask()
            .tiles()
            .iter()
            .map(|(c, _)| c)
            .collect();
        let t = TILE as i32;
        let mut coords: Vec<TileCoord> = Vec::new();
        {
            let x0 = rect.min.x.div_euclid(t);
            let y0 = rect.min.y.div_euclid(t);
            let x1 = (rect.max.x - 1).div_euclid(t);
            let y1 = (rect.max.y - 1).div_euclid(t);
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    coords.push(TileCoord::new(0, tx, ty));
                }
            }
        }
        // Reviewer MAJOR (door-slice review, RULED fix): §13.2 intersect
        // = min(a, b) EVERYWHERE — outside the path b = 0 must CLEAR the
        // existing selection, so Intersect visits the UNION of the
        // path-cover tiles and the existing-selection tiles (the
        // path-cover-only walk left outside selection untouched and the
        // bounds canvas-wide). Add (a+0 = a) and Subtract (a−0 = a) are
        // identity outside the path — path-cover tiles suffice, verified
        // by the reviewer and pinned below.
        if matches!(op, SelectOp::Intersect) {
            for c in &existing_coords {
                if !coords.contains(c) {
                    coords.push(*c);
                }
            }
        }
        if matches!(op, SelectOp::Replace) {
            // Replace zeroes previously selected space outside the plane.
            for c in &existing_coords {
                if !coords.contains(c) {
                    let zeros = vec![0.0f32; TILE * TILE];
                    cx.install_selection_tile_plane(*c, &zeros)?;
                }
            }
        }
        for c in coords {
            let base_x = c.x * t;
            let base_y = c.y * t;
            let mut values = vec![0.0f32; TILE * TILE];
            let mut any = false;
            for yy in 0..t {
                for xx in 0..t {
                    let (px, py) = (base_x + xx, base_y + yy);
                    let b = sample(px, py);
                    let a = cx
                        .document()
                        .selection()
                        .mask()
                        .read_pixel_f32(crate::geom::IVec2::new(px, py))[0];
                    let v = combine(op, a, b);
                    values[(yy * t + xx) as usize] = v;
                    any = any || v != 0.0 || a != 0.0;
                }
            }
            if any || matches!(op, SelectOp::Replace) {
                cx.install_selection_tile_plane(c, &values)?;
            }
        }
        match op {
            SelectOp::Replace => cx.set_selection_bounds(rect)?,
            SelectOp::Add => cx.grow_selection_bounds(rect)?,
            SelectOp::Subtract | SelectOp::Intersect => cx.recompute_selection_bounds()?,
        }
        cx.sync_selection_bounds();
        // §40.4 dirty: no composite D; overlay scope:"selection".
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::vector_ops::{CoveragePlaneData, VectorGeometry, VECTOR_SIDECAR_SCHEMA};
    use crate::engine::Engine;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// In-crate PROBE geometry service (the production impl is
    /// appname-vector's — TEXTVEC Door 1; this probe exercises the SEAM,
    /// not real geometry): the sidecar format is
    /// `{schema, fillRule, subpaths: [{"rect":[x,y,w,h]}…]}` — enough to
    /// drive validation, booleans-as-rect-algebra, coverage, and edit ops
    /// deterministically. `gen_counter` makes `primitive_sidecar`
    /// DELIBERATELY nondeterministic across calls, so the D-48 capture
    /// pin below proves replay consumes captures instead of re-running
    /// the constructor.
    struct ProbeGeom {
        gen_counter: AtomicU64,
    }

    impl ProbeGeom {
        fn new() -> ProbeGeom {
            ProbeGeom {
                gen_counter: AtomicU64::new(0),
            }
        }

        #[allow(clippy::type_complexity)]
        fn parse(bytes: &[u8]) -> Result<(Vec<[i32; 4]>, String, Option<u64>), EngineError> {
            let v: Value = serde_json::from_slice(bytes)
                .map_err(|e| EngineError::range("path", format!("sidecar not JSON: {e}")))?;
            if v.get("schema").and_then(Value::as_str) != Some(VECTOR_SIDECAR_SCHEMA) {
                return Err(EngineError::new(
                    ErrorCode::WrongSchema,
                    "sidecar schema must be appname.vector.v1",
                ));
            }
            let subs = v
                .get("subpaths")
                .and_then(Value::as_array)
                .ok_or_else(|| EngineError::missing("path.subpaths"))?;
            if subs.is_empty() {
                return Err(EngineError::range("path.subpaths", "empty path (§40.7)"));
            }
            let mut rects = Vec::new();
            for s in subs {
                let r = s
                    .get("rect")
                    .and_then(Value::as_array)
                    .ok_or_else(|| EngineError::range("path.subpaths", "probe wants rects"))?;
                let g: Vec<i32> = r
                    .iter()
                    .filter_map(|x| x.as_i64())
                    .map(|x| x as i32)
                    .collect();
                if g.len() != 4 {
                    return Err(EngineError::range(
                        "path.subpaths",
                        "rect must be [x,y,w,h]",
                    ));
                }
                rects.push([g[0], g[1], g[2], g[3]]);
            }
            let rule = v
                .get("fillRule")
                .and_then(Value::as_str)
                .unwrap_or("nonzero")
                .to_string();
            // §7.7 unknown-field posture (TEXTVEC wave-1 flag 3, re-emit
            // recommendation): `gen` is the probe's "unknown field" —
            // parse carries it so validate re-emits byte-faithfully.
            let gen = v.get("gen").and_then(Value::as_u64);
            Ok((rects, rule, gen))
        }

        fn emit(rects: &[[i32; 4]], rule: &str, gen: Option<u64>) -> GeometryResult {
            let mut body = json!({
                "schema": VECTOR_SIDECAR_SCHEMA,
                "fillRule": rule,
                "subpaths": rects.iter().map(|r| json!({"rect": r})).collect::<Vec<_>>(),
            });
            if let Some(g) = gen {
                body["gen"] = json!(g); // the deliberate nondeterminism
            }
            let bounds = Self::bounds_of(rects);
            GeometryResult {
                sidecar: serde_json::to_vec(&body).expect("probe emits"),
                bounds,
            }
        }

        fn bounds_of(rects: &[[i32; 4]]) -> IRect {
            let mut b = IRect::EMPTY;
            for [x, y, w, h] in rects {
                b = b.union(&IRect::new(*x, *y, x + w, y + h));
            }
            b
        }
    }

    impl VectorGeometry for ProbeGeom {
        fn schema(&self) -> &'static str {
            VECTOR_SIDECAR_SCHEMA
        }
        fn validate_sidecar(&self, bytes: &[u8]) -> Result<GeometryResult, EngineError> {
            let (rects, rule, gen) = Self::parse(bytes)?;
            Ok(Self::emit(&rects, &rule, gen))
        }
        fn path_param_sidecar(
            &self,
            path: &Map<String, Value>,
        ) -> Result<GeometryResult, EngineError> {
            let mut v = Value::Object(path.clone());
            v["schema"] = json!(VECTOR_SIDECAR_SCHEMA);
            let (rects, rule, gen) = Self::parse(&serde_json::to_vec(&v).expect("json"))?;
            Ok(Self::emit(&rects, &rule, gen))
        }
        fn primitive_sidecar(
            &self,
            kind: &str,
            primitive: &Map<String, Value>,
        ) -> Result<GeometryResult, EngineError> {
            let g = primitive
                .get("geometry")
                .and_then(Value::as_object)
                .ok_or_else(|| EngineError::missing("primitive.geometry"))?;
            let get = |k: &str| g.get(k).and_then(Value::as_f64).unwrap_or(0.0) as i32;
            let rect = [get("x"), get("y"), get("w"), get("h")];
            if kind == "polygon" && !primitive.contains_key("sides") {
                return Err(EngineError::missing("primitive.sides"));
            }
            // DELIBERATE nondeterminism: every constructor call yields
            // different bytes — the D-48 capture pin depends on it.
            let gen = self.gen_counter.fetch_add(1, Ordering::SeqCst);
            Ok(Self::emit(&[rect], "nonzero", Some(gen)))
        }
        fn path_boolean(
            &self,
            a: &[u8],
            b: &[u8],
            op: VectorBoolOp,
        ) -> Result<GeometryResult, EngineError> {
            let (ra, rule, _) = Self::parse(a)?;
            let (rb, _, _) = Self::parse(b)?;
            let rects = match op {
                VectorBoolOp::Union | VectorBoolOp::Exclude => {
                    let mut r = ra;
                    r.extend(rb);
                    r
                }
                VectorBoolOp::Subtract => ra,
                VectorBoolOp::Intersect => {
                    let ia = Self::bounds_of(&ra);
                    let ib = Self::bounds_of(&rb);
                    let i = ia.intersect(&ib);
                    vec![[i.min.x, i.min.y, i.width(), i.height()]]
                }
            };
            Ok(Self::emit(&rects, &rule, None))
        }
        fn rasterize_fill(&self, sidecar: &[u8]) -> Result<CoveragePlaneData, EngineError> {
            let (rects, _, _) = Self::parse(sidecar)?;
            let rect = Self::bounds_of(&rects);
            let mut values = vec![0.0f32; (rect.width() * rect.height()).max(0) as usize];
            for [x, y, w, h] in &rects {
                for yy in *y..y + h {
                    for xx in *x..x + w {
                        if xx >= rect.min.x
                            && xx < rect.max.x
                            && yy >= rect.min.y
                            && yy < rect.max.y
                        {
                            let idx = (yy - rect.min.y) as usize * rect.width() as usize
                                + (xx - rect.min.x) as usize;
                            values[idx] = 0.75; // fractional: AA-vs-binarize distinguishable
                        }
                    }
                }
            }
            Ok(CoveragePlaneData { rect, values })
        }
        fn apply_edit_ops(
            &self,
            sidecar: &[u8],
            ops: &[Value],
        ) -> Result<GeometryResult, EngineError> {
            let (mut rects, rule, gen) = Self::parse(sidecar)?;
            for (i, op) in ops.iter().enumerate() {
                let sub = op.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
                if sub >= rects.len() {
                    return Err(EngineError::range(
                        &format!("ops[{i}].subpath"),
                        format!("subpath {sub} out of range 0..{}", rects.len()),
                    ));
                }
                if op.get("op").and_then(Value::as_str) == Some("move_anchor") {
                    let pos: Vec<i32> = op
                        .get("position")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_i64())
                                .map(|v| v as i32)
                                .collect()
                        })
                        .unwrap_or_default();
                    if pos.len() == 2 {
                        rects[sub][0] = pos[0];
                        rects[sub][1] = pos[1];
                    }
                }
            }
            Ok(Self::emit(&rects, &rule, gen))
        }
    }

    fn assembly_with_geom() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
            .with_vector_geometry(Arc::new(ProbeGeom::new()))
    }

    fn ready(assembly: &EngineAssembly) -> Engine {
        let mut e = assembly.build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000000000001",
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
            "id": format!("0b7e0000-0000-4000-8000-0002{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.to_string()});
        }
        v.to_string()
    }

    fn rect_path(x: i32, y: i32, w: i32, h: i32) -> Value {
        json!({"fillRule": "nonzero", "subpaths": [{"rect": [x, y, w, h]}]})
    }

    /// Bare-core mirror: geometry-requiring arms answer typed NI; stage-4
    /// resolution still precedes (dangling ids answer bad_target first).
    #[test]
    fn bare_assembly_answers_typed_ni_and_stage_order_holds() {
        let bare = crate::engine::kinds::register(EngineAssembly::new());
        let mut e = ready(&bare);
        let err = e
            .apply_json(&env(
                1,
                "vector.path_new",
                None,
                json!({"path": rect_path(0, 0, 8, 8)}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(
            err.message.contains("VectorGeometry"),
            "the NI names the seam"
        );
        // Stage 4 first: dangling pathId → bad_target even on bare core.
        let err = e
            .apply_json(&env(
                2,
                "vector.path_edit",
                None,
                json!({"pathId": Uuid::new_v4().to_string(), "path": rect_path(0, 0, 8, 8)}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    /// path_new / path_edit / path_delete round-trip: creation ids
    /// (given + derived), value-block undo/redo byte identity, the
    /// referenced-delete refusal, and the global §9.6 duplicate check.
    #[test]
    fn path_lifecycle_round_trips_with_value_block_deltas() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let pre = e.state_hash();
        let pid = Uuid::new_v4();
        let receipt = e
            .apply_json(&env(
                1,
                "vector.path_new",
                None,
                json!({"path": rect_path(4, 4, 16, 16), "name": "outline",
                       "newPathId": pid.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![pid]);
        assert!(
            receipt.dirty.is_empty(),
            "naked paths dirty no pixels (§40.7)"
        );
        let with_path = e.state_hash();
        assert_ne!(pre, with_path, "paths fold into state_hash when present");
        let stored = e.document().unwrap().path(PathId::from_uuid(pid)).unwrap();
        assert_eq!(stored.bounds(), IRect::new(4, 4, 20, 20));
        assert_eq!(stored.name(), Some("outline"));

        // §9.6 GLOBAL duplicate: a path id colliding with a LAYER id.
        let bg = e.document().unwrap().z_order_walk()[0];
        let err = e
            .apply_json(&env(
                2,
                "vector.path_new",
                None,
                json!({"path": rect_path(0, 0, 4, 4), "newPathId": bg.as_uuid().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);

        // path_edit is a full-state value replacement.
        e.apply_json(&env(
            3,
            "vector.path_edit",
            None,
            json!({"pathId": pid.to_string(), "path": rect_path(10, 10, 30, 30)}),
        ))
        .unwrap();
        assert_eq!(
            e.document()
                .unwrap()
                .path(PathId::from_uuid(pid))
                .unwrap()
                .bounds(),
            IRect::new(10, 10, 40, 40)
        );
        let edited = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), with_path, "SetPathValue undo is byte-exact");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), edited);

        // Deterministic creation-id derivation when newPathId is omitted
        // (§9.6 wrapper-fills rule at the headless door): same envelope id
        // ⇒ same derived id on replay.
        let receipt = e
            .apply_json(&env(
                4,
                "vector.path_new",
                None,
                json!({"path": rect_path(0, 0, 2, 2)}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids.len(), 1);
        let derived = receipt.created_ids[0];
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "derived ids are replay-stable (D-48)"
        );
        assert!(replayed
            .document()
            .unwrap()
            .path(PathId::from_uuid(derived))
            .is_some());

        // path_delete: free path deletes; undo re-inserts byte-exact.
        let before_delete = e.state_hash();
        e.apply_json(&env(
            5,
            "vector.path_delete",
            None,
            json!({"pathId": derived.to_string()}),
        ))
        .unwrap();
        assert!(e
            .document()
            .unwrap()
            .path(PathId::from_uuid(derived))
            .is_none());
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before_delete, "RemovePath undo re-inserts");
    }

    /// vector.path_bool: XOR addressing (§40.7 semantic check), doc-path
    /// fold + flatten consumption, shape-mode append escalation.
    #[test]
    fn path_bool_xor_addressing_and_doc_path_fold() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let (pa, pb) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            1,
            "vector.path_new",
            None,
            json!({"path": rect_path(0, 0, 10, 10), "newPathId": pa.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            2,
            "vector.path_new",
            None,
            json!({"path": rect_path(20, 20, 10, 10), "newPathId": pb.to_string()}),
        ))
        .unwrap();

        // Neither mode → missing_field; both modes → missing_field.
        let err = e
            .apply_json(&env(3, "vector.path_bool", None, json!({"op": "union"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "neither mode");
        let bg = e.document().unwrap().z_order_walk()[0];
        let err = e
            .apply_json(&env(
                4,
                "vector.path_bool",
                Some(bg.as_uuid()),
                json!({"op": "union", "pathIds": [pa.to_string(), pb.to_string()]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField, "both modes (§40.7 XOR)");

        // Shape-layer mode on a RASTER target → target_kind_mismatch.
        let err = e
            .apply_json(&env(
                5,
                "vector.path_bool",
                Some(bg.as_uuid()),
                json!({"op": "union"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);

        // Doc-path fold, flatten:false — result bakes into the FIRST
        // path, operand paths kept (pinned reading; lane log).
        let pre = e.state_hash();
        e.apply_json(&env(
            6,
            "vector.path_bool",
            None,
            json!({"op": "union", "pathIds": [pa.to_string(), pb.to_string()]}),
        ))
        .unwrap();
        assert_eq!(
            e.document()
                .unwrap()
                .path(PathId::from_uuid(pa))
                .unwrap()
                .bounds(),
            IRect::new(0, 0, 30, 30),
            "union baked into the first operand"
        );
        assert!(e.document().unwrap().path(PathId::from_uuid(pb)).is_some());
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "doc-path bake undoes byte-exact");

        // flatten:true consumes the second operand.
        e.apply_json(&env(
            7,
            "vector.path_bool",
            None,
            json!({"op": "union", "flatten": true,
                   "pathIds": [pa.to_string(), pb.to_string()]}),
        ))
        .unwrap();
        assert!(e.document().unwrap().path(PathId::from_uuid(pb)).is_none());
        assert_eq!(e.document().unwrap().path_count(), 1);
    }

    /// q-scenario/Q-17 (MAJOR, tick 89): `vector.shape_add` accepted a `fill`
    /// its own `$defs/fillContent` oneOf forbids — every branch requires
    /// `type` — and rendered the shape BLACK, discarding the caller's colour
    /// with no diagnostic. Stage 5 checked only that `fill` was PRESENT. Both
    /// shape doors now decode it, so the refusal is typed and the black render
    /// is unreachable.
    #[test]
    fn shape_doors_refuse_a_fill_that_matches_no_branch() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let nl = Uuid::new_v4();
        let prim = json!({"kind": "rect", "geometry": {"x": 8, "y": 8, "w": 32, "h": 16}});
        let good = json!({"type": "solid", "color": {"model": "rgba", "comps": [0, 0.5, 1, 1]}});

        // THE FINDING: a solid fill with no `type` at all.
        let err = e
            .apply_json(&env(
                1,
                "vector.shape_add",
                None,
                json!({"primitive": prim, "fill": {"color": {"model": "rgba", "comps": [0, 0.5, 1, 1]}},
                       "stroke": {"widthPx": 0.0}, "newLayerId": nl.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::MissingField,
            "typed refusal, not a black render"
        );
        assert!(
            err.message.contains("fill.type"),
            "names the field: {}",
            err.message
        );
        assert_eq!(
            e.document().unwrap().z_order_walk().len(),
            1,
            "nothing was created"
        );

        // CONTROL: the same envelope with the branch's required key lands.
        e.apply_json(&env(
            2,
            "vector.shape_add",
            None,
            json!({"primitive": prim, "fill": good, "stroke": {"widthPx": 0.0},
                   "newLayerId": nl.to_string()}),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().z_order_walk().len(),
            2,
            "the typed fill still creates a shape"
        );

        // A branch whose OWN required key is missing refuses too (solid without
        // color), so the check is the whole oneOf and not just `type`.
        let err = e
            .apply_json(&env(
                3,
                "vector.shape_add",
                None,
                json!({"primitive": prim, "fill": {"type": "solid"}, "stroke": {"widthPx": 0.0},
                       "newLayerId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        assert!(
            err.message.contains("fill.color"),
            "names the branch's key: {}",
            err.message
        );

        // THE SECOND DOOR: the same untyped fill through the edit path.
        let err = e
            .apply_json(&env(
                4,
                "vector.shape_set_style",
                Some(nl),
                json!({"fill": {"color": {"model": "rgba", "comps": [1, 0, 0, 1]}}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::MissingField,
            "shape_set_style refuses it too"
        );
        // and a style update with no fill at all is still the untouched case.
        e.apply_json(&env(
            5,
            "vector.shape_set_style",
            Some(nl),
            json!({"stroke": {"widthPx": 1.0}}),
        ))
        .unwrap();
    }

    /// vector.shape_add + shape_set_style + vector.edit over the shape:
    /// creation, transitive locks, partial style updates, atomic edit
    /// failure, dirty rects.
    #[test]
    fn shape_layer_lifecycle_locks_and_edits() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let nl = Uuid::new_v4();
        let receipt = e
            .apply_json(&env(
                1,
                "vector.shape_add",
                None,
                json!({"primitive": {"kind": "rect", "geometry": {"x": 8, "y": 8, "w": 32, "h": 16}},
                       "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0, 0.5, 1, 1]}}, "stroke": {"widthPx": 2.0},
                       "newLayerId": nl.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids[0], nl);
        assert_eq!(receipt.created_ids.len(), 2, "layer + its base path");
        let lid = LayerId::from_uuid(nl);
        assert_eq!(
            e.document().unwrap().layer(lid).unwrap().kind().kind_str(),
            "shape"
        );
        // §40.7 dirty: D = shape bounds (8,8..40,24) INFLATED BY THE
        // STROKE. `widthPx` 2.0 with the pinned silent defaults (Center,
        // Miter(4)) reaches `h·limit` = 1·4 = 4 px, plus the one-pixel
        // AA/flatten margin ⇒ 5. The bare hull would leave the band's own
        // ink outside the repainted region.
        assert_eq!(
            receipt.dirty,
            vec![crate::engine::DirtyRect {
                layer: lid,
                rect: IRect::new(3, 3, 45, 29)
            }],
            "§40.7 dirty: D = shape bounds, inflated by the stroke's reach"
        );

        // shape_set_style: partial (fill only) keeps stroke; undo/redo.
        let pre = e.state_hash();
        e.apply_json(&env(
            2,
            "vector.shape_set_style",
            Some(nl),
            json!({"fill": {"type": "none"}}),
        ))
        .unwrap();
        {
            let sd = e
                .document()
                .unwrap()
                .layer(lid)
                .unwrap()
                .shape_data()
                .unwrap()
                .clone();
            assert_eq!(sd.fill(), &json!({"type": "none"}));
            assert_eq!(
                sd.stroke(),
                &json!({"widthPx": 2.0}),
                "absent field untouched"
            );
        }
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "SetShapeData undo is byte-exact");
        e.redo().unwrap();

        // vector.edit: bad op name → out_of_range; mid-list range failure
        // is ATOMIC (geometry untouched); a valid move applies.
        let err = e
            .apply_json(&env(
                3,
                "vector.edit",
                Some(nl),
                json!({"ops": [{"op": "bend_anchor", "subpath": 0, "index": 0}]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let before_atomic = e.state_hash();
        let err = e
            .apply_json(&env(
                4,
                "vector.edit",
                Some(nl),
                json!({"ops": [
                    {"op": "move_anchor", "subpath": 0, "index": 0, "position": [50, 50]},
                    {"op": "move_anchor", "subpath": 9, "index": 0, "position": [1, 1]}
                ]}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange, "subpath 9 out of range");
        assert_eq!(
            e.state_hash(),
            before_atomic,
            "mid-list failure leaves geometry untouched (§10.2 atomicity)"
        );
        e.apply_json(&env(
            5,
            "vector.edit",
            Some(nl),
            json!({"ops": [{"op": "move_anchor", "subpath": 0, "index": 0, "position": [50, 50]}]}),
        ))
        .unwrap();
        let base = e
            .document()
            .unwrap()
            .layer(lid)
            .unwrap()
            .shape_data()
            .unwrap()
            .path();
        assert_eq!(
            e.document().unwrap().path(base).unwrap().bounds(),
            IRect::new(50, 50, 82, 66),
            "move applied via the service"
        );

        // Transitive PIXELS lock (§11.1 letter, ONE place): group the
        // shape, lock the GROUP with pixels; the shape refuses.
        let grp = Uuid::new_v4();
        e.apply_json(&env(
            6,
            "layer.group",
            None,
            json!({"layerIds": [nl.to_string()], "newGroupId": grp.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            10,
            "layer.set_props",
            Some(grp),
            json!({"locks": {"pixels": true}}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                7,
                "vector.shape_set_style",
                Some(nl),
                json!({"fill": {"type": "solid", "color": {"model": "rgba", "comps": [0, 0.5, 1, 1]}}}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::Locked,
            "group PIXELS lock binds the subtree (§11.1 transitive, strictest wins)"
        );
        e.undo().unwrap();

        // Shape-mode bool append: typed escalation refusal (operand
        // unspecified by the contract XOR — O-INT escalation on record).
        let err = e
            .apply_json(&env(
                8,
                "vector.path_bool",
                Some(nl),
                json!({"op": "union"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(err.message.contains("escalated"));
        // …and flatten:true over empty bool_ops is the §40.1 no-op push.
        e.apply_json(&env(
            9,
            "vector.path_bool",
            Some(nl),
            json!({"op": "union", "flatten": true}),
        ))
        .unwrap();
    }

    /// THE D-48 pin (bound posture): shape_add's constructor output is
    /// value-captured — journal replay and package reopen NEVER re-run
    /// constructor trig. The probe's constructor is deliberately
    /// nondeterministic per call, so these equalities hold ONLY if the
    /// capture is consumed.
    #[test]
    fn shape_add_value_capture_survives_replay_and_package_round_trip() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let nl = Uuid::new_v4();
        e.apply_json(&env(
            1,
            "vector.shape_add",
            None,
            json!({"primitive": {"kind": "rect", "geometry": {"x": 0, "y": 0, "w": 20, "h": 20}},
                   "fill": {"type": "solid", "color": {"model": "rgba", "comps": [0, 0.5, 1, 1]}}, "stroke": {}, "newLayerId": nl.to_string()}),
        ))
        .unwrap();
        let original = e.state_hash();

        // In-process replay consumes the capture.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            original,
            "replay byte-identity under a NONDETERMINISTIC constructor — \
             the capture was consumed (D-48)"
        );

        // Package round trip: captures persist beside the journal; the
        // loaded engine's replay ALSO consumes them.
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&asm, &bytes).unwrap();
        assert_eq!(
            loaded.engine.state_hash_persistent(),
            e.state_hash_persistent(),
            "package round trip preserves the shape doc"
        );
        let replayed = loaded
            .engine
            .replay_assembled(loaded.engine.journal())
            .unwrap();
        assert_eq!(
            replayed.state_hash_persistent(),
            e.state_hash_persistent(),
            "post-reopen journal replay consumes the PERSISTED capture (D-48)"
        );
        // Save→load→save byte stability with vector content aboard.
        let again = crate::io::package::save_package_bytes(&loaded.engine).unwrap();
        let third = crate::io::package::load_package_bytes_with(&asm, &again).unwrap();
        assert_eq!(
            third.engine.state_hash_persistent(),
            loaded.engine.state_hash_persistent()
        );
    }

    /// selection.from_path: analytic coverage through the SAME selection
    /// doors as the O-TOOLS engines — replace/add semantics, binarize,
    /// feather refusal, undo byte-identity.
    #[test]
    fn selection_from_path_writes_selection_through_the_shared_doors() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let pid = Uuid::new_v4();
        e.apply_json(&env(
            1,
            "vector.path_new",
            None,
            json!({"path": rect_path(10, 10, 20, 20), "newPathId": pid.to_string()}),
        ))
        .unwrap();
        let pre = e.state_hash();

        // feather > 0 routes through the §13 engine — typed refusal.
        let err = e
            .apply_json(&env(
                2,
                "selection.from_path",
                None,
                json!({"pathId": pid.to_string(), "feather": 4.0}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);

        // Default (antialias:true, replace): fractional coverage lands.
        e.apply_json(&env(
            3,
            "selection.from_path",
            None,
            json!({"pathId": pid.to_string()}),
        ))
        .unwrap();
        let sel = e.document().unwrap().selection();
        assert!(sel.active());
        assert_eq!(sel.bounds(), IRect::new(10, 10, 30, 30));
        assert_eq!(
            sel.mask().read_pixel_f32(crate::geom::IVec2::new(15, 15))[0],
            0.75,
            "analytic fractional coverage installs verbatim"
        );
        assert_eq!(
            sel.mask().read_pixel_f32(crate::geom::IVec2::new(5, 5))[0],
            0.0
        );

        // antialias:false binarizes at 0.5.
        e.apply_json(&env(
            4,
            "selection.from_path",
            None,
            json!({"pathId": pid.to_string(), "antialias": false}),
        ))
        .unwrap();
        assert_eq!(
            e.document()
                .unwrap()
                .selection()
                .mask()
                .read_pixel_f32(crate::geom::IVec2::new(15, 15))[0],
            1.0
        );

        // Undo unwinds both selection writes byte-exact.
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "selection-surface tile deltas undo");
    }

    /// Reviewer MAJOR pin pair + the combine-op battery (review MINOR —
    /// the exact gap that hid the MAJOR): every §13.2 combine op over a
    /// pre-existing selection WIDER than the path. Probe shapes verbatim
    /// from the review: canvas-wide prior (canvas-sized path, replace),
    /// then a 20×20 path at (10,10); the far pixel (400,400) is the
    /// outside-the-cover witness.
    #[test]
    fn from_path_combine_ops_over_wider_prior_selection() {
        let asm = assembly_with_geom();
        let mut e = asm.build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 512, "height": 512, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let (big, small) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(&env(
            1,
            "vector.path_new",
            None,
            json!({"path": rect_path(0, 0, 512, 512), "newPathId": big.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            2,
            "vector.path_new",
            None,
            json!({"path": rect_path(10, 10, 20, 20), "newPathId": small.to_string()}),
        ))
        .unwrap();
        let far = crate::geom::IVec2::new(400, 400);
        let inside = crate::geom::IVec2::new(15, 15);
        let sel_at = |e: &Engine, p: crate::geom::IVec2| -> f32 {
            e.document().unwrap().selection().mask().read_pixel_f32(p)[0]
        };
        let wide_prior = |e: &mut Engine, n: u32| {
            e.apply_json(&env(
                n,
                "selection.from_path",
                None,
                json!({"pathId": big.to_string()}),
            ))
            .unwrap();
            assert_eq!(sel_at(e, far), 0.75, "wide prior covers the far witness");
        };

        // MAJOR probe, verbatim: wide prior ∩ small path — outside the
        // path b = 0 must CLEAR (min(a, 0) = 0), bounds must TIGHTEN.
        wide_prior(&mut e, 3);
        e.apply_json(&env(
            4,
            "selection.from_path",
            None,
            json!({"pathId": small.to_string(), "op": "intersect"}),
        ))
        .unwrap();
        assert_eq!(
            sel_at(&e, far),
            0.0,
            "REVIEW MAJOR FLIPPED: intersect clears selection outside the \
             path cover (§13.2 min(a, b) with b = 0)"
        );
        assert_eq!(sel_at(&e, inside), 0.75, "inside: min(0.75, 0.75)");
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            IRect::new(0, 0, 256, 256),
            "bounds TIGHTEN off the canvas hull (512-wide before the fix) to \
             the surviving tile's hull — recompute_selection_bounds is \
             tile-granular (the shared §2.4 house door; conservative \
             superset of the exact 10..30 rect)"
        );

        // The subtract twin, verbatim: outside stays (max(0, a − 0) = a).
        wide_prior(&mut e, 5);
        e.apply_json(&env(
            6,
            "selection.from_path",
            None,
            json!({"pathId": small.to_string(), "op": "subtract"}),
        ))
        .unwrap();
        assert_eq!(
            sel_at(&e, far),
            0.75,
            "subtract twin: outside the path the prior selection STAYS"
        );
        assert_eq!(sel_at(&e, inside), 0.0, "inside: max(0, 0.75 − 0.75)");

        // Add over the wider prior: saturating union, outside unchanged.
        wide_prior(&mut e, 7);
        e.apply_json(&env(
            8,
            "selection.from_path",
            None,
            json!({"pathId": small.to_string(), "op": "add"}),
        ))
        .unwrap();
        assert_eq!(sel_at(&e, far), 0.75, "add: outside the path unchanged");
        assert_eq!(sel_at(&e, inside), 1.0, "inside: min(1, 0.75 + 0.75)");

        // Undo chain stays byte-exact through the widened intersect walk.
        let end = e.state_hash();
        for _ in 0..6 {
            e.undo().unwrap();
        }
        for _ in 0..6 {
            e.redo().unwrap();
        }
        assert_eq!(e.state_hash(), end, "combine walk undo/redo identity");
    }

    fn env_mk(n: u32, kind: &str, params: Value, merge_key: Option<&str>) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("0b7e0000-0000-4000-8000-0003{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(k) = merge_key {
            v["mergeKey"] = json!(k);
        }
        v.to_string()
    }

    /// §10.5 class-M coalescing to the letter: a same-mergeKey run of
    /// vector.path_edit envelopes merges into ONE undo step and ONE
    /// journal row (the newest full-state command); undo restores the
    /// PRE-RUN state in one step; replay of the merged journal is
    /// byte-identical; key changes, null keys, class-R kinds, and undo
    /// boundaries all break the run.
    #[test]
    fn merge_key_coalesces_path_edit_drags_into_one_step() {
        let asm = assembly_with_geom();
        let mut e = ready(&asm);
        let pid = Uuid::new_v4();
        e.apply_json(&env(
            1,
            "vector.path_new",
            None,
            json!({"path": rect_path(0, 0, 10, 10), "newPathId": pid.to_string()}),
        ))
        .unwrap();
        let pre_run = e.state_hash();
        let journal_before = e.journal().len();
        let history_before = e.history().len();

        // The drag session: three full-state geometry replacements, one
        // producer-minted key ("{kind}:{targetId}#path:{sessionUuid}").
        let key = format!("vector.path_edit:{pid}#path:11111111-2222-4333-8444-555555555555");
        let r1 = e
            .apply_json(&env_mk(
                2,
                "vector.path_edit",
                json!({"pathId": pid.to_string(), "path": rect_path(1, 1, 10, 10)}),
                Some(&key),
            ))
            .unwrap();
        assert!(!r1.coalesced, "first envelope of a session opens the step");
        let r2 = e
            .apply_json(&env_mk(
                3,
                "vector.path_edit",
                json!({"pathId": pid.to_string(), "path": rect_path(2, 2, 10, 10)}),
                Some(&key),
            ))
            .unwrap();
        assert!(r2.coalesced, "same-key consecutive envelope merges (§10.5)");
        let r3 = e
            .apply_json(&env_mk(
                4,
                "vector.path_edit",
                json!({"pathId": pid.to_string(), "path": rect_path(3, 3, 10, 10)}),
                Some(&key),
            ))
            .unwrap();
        assert!(r3.coalesced);

        assert_eq!(
            e.journal().len(),
            journal_before + 1,
            "the run holds ONE journal row (the newest command replaces in place)"
        );
        assert_eq!(
            e.history().len(),
            history_before + 1,
            "the history panel shows one entry per coalesced step (§10.5)"
        );
        assert_eq!(
            e.document()
                .unwrap()
                .path(PathId::from_uuid(pid))
                .unwrap()
                .bounds(),
            IRect::new(3, 3, 13, 13),
            "latest full-state values live"
        );
        let final_hash = e.state_hash();

        // ONE undo unwinds the whole run to the PRE-RUN state (earliest
        // before), one redo restores the latest after.
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            pre_run,
            "merged DeltaSet keeps the earliest before"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), final_hash, "…and the latest after");

        // D-48: replay of the merged journal (one full-state row) is
        // byte-identical.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), final_hash);

        // A NEW session key breaks the run.
        let key2 = format!("vector.path_edit:{pid}#path:99999999-2222-4333-8444-555555555555");
        let r4 = e
            .apply_json(&env_mk(
                5,
                "vector.path_edit",
                json!({"pathId": pid.to_string(), "path": rect_path(4, 4, 10, 10)}),
                Some(&key2),
            ))
            .unwrap();
        assert!(!r4.coalesced, "a fresh sessionUuid breaks the run");

        // An UNDO boundary breaks consecutiveness even under the same key.
        e.undo().unwrap();
        let r5 = e
            .apply_json(&env_mk(
                6,
                "vector.path_edit",
                json!({"pathId": pid.to_string(), "path": rect_path(5, 5, 10, 10)}),
                Some(&key2),
            ))
            .unwrap();
        assert!(!r5.coalesced, "an undone tail is not 'consecutive'");

        // A null mergeKey never merges; a class-R kind with a client key
        // never merges in the journal (producer-side coalescing per §9.6
        // rule 3 — the vector.edit escalation).
        let r6 = e
            .apply_json(&env_mk(
                7,
                "vector.path_edit",
                json!({"pathId": pid.to_string(), "path": rect_path(6, 6, 10, 10)}),
                None,
            ))
            .unwrap();
        assert!(!r6.coalesced);
        let r7 = e
            .apply_json(&env_mk(
                8,
                "vector.path_new",
                json!({"path": rect_path(50, 50, 4, 4)}),
                Some("vector.path_new:whatever:11111111-2222-4333-8444-555555555555"),
            ))
            .unwrap();
        assert!(!r7.coalesced, "class R never merges engine-side");
    }

    /// §10.5 property-drag consumer (op.set_params — the shell-slider
    /// shape): a same-key run coalesces; the merged SetOpValue block keeps
    /// earliest before / latest after.
    #[test]
    fn merge_key_coalesces_op_set_params_slider_runs() {
        // op.set_params lives in kinds_ops; register a probe kernel so
        // adjustment.add validates (the compute half is O-TOOLS').
        use crate::engine::op_impl::{OpImpl, PreviewPolicy};
        struct ProbeInvert;
        impl OpImpl for ProbeInvert {
            fn op_id(&self) -> &'static str {
                "adjustment.invert"
            }
            fn validate_params(&self, _p: &Value) -> Result<(), EngineError> {
                Ok(())
            }
            fn default_params(&self) -> Value {
                json!({})
            }
            fn roi_out_to_in(&self, _p: &Value, out: IRect) -> IRect {
                out
            }
            fn pointwise(&self) -> bool {
                true
            }
            fn preview(&self) -> PreviewPolicy {
                PreviewPolicy::LiveFull
            }
            fn apply_cpu(
                &self,
                _p: &Value,
                _rect: IRect,
                input: &[[f32; 4]],
                output: &mut [[f32; 4]],
            ) -> Result<(), EngineError> {
                output.copy_from_slice(input);
                Ok(())
            }
        }
        let asm = assembly_with_geom().with_op(Arc::new(ProbeInvert));
        let mut e = ready(&asm);
        let bg = e.document().unwrap().z_order_walk()[0];
        let (nl, no) = (Uuid::new_v4(), Uuid::new_v4());
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0b7e0000-0000-4000-8000-000400000001",
                "kind": "adjustment.add",
                "target": {"layerId": bg.as_uuid().to_string()},
                "params": {"opKind": "adjustment.invert", "newLayerId": nl.to_string(),
                            "newOpId": no.to_string(), "params": {}},
            })
            .to_string(),
        )
        .unwrap();
        let pre = e.state_hash();
        let history_before = e.history().len();
        let key = format!("op.set_params:{no}#params:aaaaaaaa-2222-4333-8444-555555555555");
        // Two same-key opacity drags (op.set_params is class M).
        e.apply_json(&env_mk(
            1,
            "op.set_params",
            json!({"opId": no.to_string(), "params": {"amount": 0.25}}),
            Some(&key),
        ))
        .unwrap();
        let r = e
            .apply_json(&env_mk(
                2,
                "op.set_params",
                json!({"opId": no.to_string(), "params": {"amount": 0.5}}),
                Some(&key),
            ))
            .unwrap();
        assert!(r.coalesced, "op.set_params slider run merges");
        assert_eq!(e.history().len(), history_before + 1);
        assert_eq!(
            e.document()
                .unwrap()
                .op(crate::graph::id::OpId::from_uuid(no))
                .unwrap()
                .params(),
            &json!({"amount": 0.5}),
            "latest full-state params live (§10.5 merged command = the newer one)"
        );
        e.undo().unwrap();
        assert_eq!(e.state_hash(), pre, "one undo unwinds the whole drag");
    }
}
