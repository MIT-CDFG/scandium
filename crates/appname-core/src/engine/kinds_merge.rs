// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `layer.merge_down` / `layer.merge_visible` /
//! `image.flatten`; ES §4.4 (source-over), §11.2 (effective mask
//! coverage), §12.4 (bake/flatten semantics), §10.4 (flatten = class C,
//! snapshot inverse). CP-0.5 wave-4 kind tail, merge half.
//!
//! v1 scope (typed refusals name their rows): NORMAL blend only (the
//! §4.5 kernel table is the render-side shared math seam); no
//! clip_to_below participants; raster masks bake through the §11.2 exact
//! formula with feather 0 (feathered bake = the O-TOOLS gaussian seam);
//! GROUP participants refuse typed — §4.7 isolated-group semantics live
//! with the compositor row. The math here intentionally mirrors the
//! stroke-family storage-space compositing (standing interim divergence)
//! so merged pixels equal what the render normal-path produces.

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::geom::{IRect, IVec2};
use crate::graph::id::LayerId;
use crate::graph::node::{BlendMode, LayerNode, LockFlags};
use crate::tile::map::TileCoord;
use crate::tile::{TILE, TILE_I32};

pub struct LayerMergeDownKind;
pub struct LayerMergeVisibleKind;
pub struct ImageFlattenKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(LayerMergeDownKind),
        Arc::new(LayerMergeVisibleKind),
        Arc::new(ImageFlattenKind),
    ]
}

/// A node participates in a v1 core merge only inside the implemented
/// compositing subset; anything else answers typed with its row.
fn check_mergeable(cx: &KindContext<'_>, id: LayerId) -> Result<(), EngineError> {
    let n = cx.document().layer(id).expect("caller resolved");
    if n.is_group() {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "group participants need the §4.7 isolation semantics (compositor row)",
        ));
    }
    if n.pixels().is_none() {
        return Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "participants must be rasterizable — parametric kinds bake per §12.4 first",
        ));
    }
    if n.blend() != BlendMode::Normal {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            format!(
                "blend {:?} needs the §4.5 kernel table (render-side shared math)",
                n.blend()
            ),
        ));
    }
    if n.clip_to_below() {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "clip_to_below participants arrive with the compositor clipping row",
        ));
    }
    for m in n.masks() {
        let props = cx.document().mask(*m).expect("validated tree").props();
        if props.feather > 0.0 {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "feathered mask bake needs the O-TOOLS gaussian seam (§11.2 step 2)",
            ));
        }
    }
    Ok(())
}

/// §11.2 effective coverage at `p` (raster masks, feather 0; exact order
/// sample → density → invert; disabled ⇒ 1).
fn mask_coverage(cx: &KindContext<'_>, id: LayerId, p: IVec2) -> f32 {
    let doc = cx.document();
    let n = doc.layer(id).expect("resolved");
    let mut c = 1.0f32;
    for m in n.masks() {
        let mask = doc.mask(*m).expect("validated tree");
        let props = mask.props();
        if !props.enabled {
            continue;
        }
        let mut v = mask.coverage().read_pixel_f32(p)[0].clamp(0.0, 1.0);
        v = 1.0 - props.density * (1.0 - v);
        if props.inverted {
            v = 1.0 - v;
        }
        c *= v;
    }
    c
}

/// The §4.4 union of content each source contributes.
fn sources_bounds(cx: &KindContext<'_>, sources: &[LayerId]) -> IRect {
    sources.iter().fold(IRect::EMPTY, |acc, id| {
        let b = cx
            .document()
            .layer(*id)
            .and_then(|n| n.pixels())
            .map(|p| p.bounds())
            .unwrap_or(IRect::EMPTY);
        acc.union(&b)
    })
}

/// Bake the receiver's OWN §11.2 coverage into its alpha and detach its
/// masks (merge semantics: the receiving layer's mask applies to its
/// pre-merge pixels, never to the incoming content).
fn bake_own_mask(cx: &mut KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    let masks: Vec<_> = cx
        .document()
        .layer(layer)
        .expect("resolved")
        .masks()
        .to_vec();
    if masks.is_empty() {
        return Ok(());
    }
    let snap = cx
        .document()
        .layer(layer)
        .expect("resolved")
        .pixels()
        .expect("raster")
        .clone();
    let content = snap.bounds();
    if !content.is_empty() {
        let coords: Vec<(i32, i32)> = content.tiles(TILE_I32).collect();
        for (tx, ty) in coords {
            let coord = TileCoord::new(0, tx, ty);
            let rect = coord.pixel_rect();
            if rect.intersect(&content).is_empty() {
                continue;
            }
            let mut plane = vec![[0.0f32; 4]; TILE * TILE];
            let mut changed = false;
            let mut i = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    let v = snap.read_pixel_f32(p);
                    let c = mask_coverage(cx, layer, p);
                    let out = [v[0] * c, v[1] * c, v[2] * c, v[3] * c];
                    if out != v {
                        changed = true;
                    }
                    plane[i] = out;
                    i += 1;
                }
            }
            if changed {
                cx.install_tile_plane(layer, coord, &plane)?;
            }
        }
        // PIXEL-EXACT shrink (Q-1 determinism class): the coverage multiply
        // can zero content up to an edge inside a still-occupied tile (mask
        // edge off the tile grid; soft coverage quantizing to zero). The
        // tile-granular shrink kept the proud extent, forking `region_hash`
        // (which folds bounds) against the loader's pixel-exact recompute and
        // killing D-48 replay-as-recovery undo across save→reopen.
        cx.recompute_layer_bounds_pixel_exact(layer)?;
    }
    for m in masks {
        cx.detach_mask(layer, m)?;
    }
    Ok(())
}

/// Composite `sources` (bottom→top, §4.4 source-over, premult storage
/// space) onto `receiver` in place through the recorder doors.
fn merge_onto(
    cx: &mut KindContext<'_>,
    receiver: LayerId,
    sources: &[LayerId],
) -> Result<(), EngineError> {
    let region = sources_bounds(cx, sources);
    if region.is_empty() {
        return Ok(()); // §40.1 empty push
    }
    // COW snapshots so reads stay pre-merge while installs land.
    let snaps: Vec<(LayerId, crate::buffer::Buffer, f32)> = sources
        .iter()
        .map(|id| {
            let n = cx.document().layer(*id).expect("resolved");
            (
                *id,
                n.pixels().expect("raster").clone(),
                n.opacity() * n.fill_opacity(),
            )
        })
        .collect();
    let dst_snap = cx
        .document()
        .layer(receiver)
        .expect("resolved")
        .pixels()
        .expect("raster")
        .clone();
    let visible: Vec<bool> = sources
        .iter()
        .map(|id| cx.document().layer(*id).expect("resolved").visible())
        .collect();
    let coords: Vec<(i32, i32)> = region.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        let span = rect.intersect(&region);
        if span.is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        let mut changed = false;
        let mut i = 0;
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                let p = IVec2::new(x, y);
                let mut out = dst_snap.read_pixel_f32(p);
                if span.contains(p) {
                    for (k, (id, snap, alpha_scale)) in snaps.iter().enumerate() {
                        if !visible[k] {
                            continue;
                        }
                        let src = snap.read_pixel_f32(p);
                        // ONE storage-space source-over, shared verbatim
                        // with the §45.2/§13.3 sampling composite
                        // (`engine::sampling`) so "sample all layers" and
                        // "flatten, then sample" cannot drift apart. The
                        // extraction is bit-identical: same expression,
                        // same `alpha_scale`-then-`coverage` multiply
                        // order (f32 multiplication is not associative),
                        // same `a <= 0` skip.
                        out = crate::engine::sampling::source_over_storage(
                            out,
                            src,
                            *alpha_scale,
                            mask_coverage(cx, *id, p),
                        );
                    }
                    if out != dst_snap.read_pixel_f32(p) {
                        changed = true;
                    }
                }
                plane[i] = out;
                i += 1;
            }
        }
        if changed {
            cx.install_tile_plane(receiver, coord, &plane)?;
        }
    }
    // PIXEL-EXACT grow scoped to the written region (Q-1 determinism class):
    // the old eager grow unioned the SOURCES' bounds rect wholesale, but a
    // source's tight rect can carry corners/fringes whose composite over the
    // receiver stores nothing (sub-quantum source alpha; transparent rect
    // corners over a transparent receiver) — the receiver extent then runs
    // proud of the persisted plane and `region_hash` forks live-vs-reloaded,
    // killing D-48 replay-as-recovery undo across save→reopen. Source-over
    // never REDUCES receiver alpha, so the scoped grow is exact; the
    // receiver's own pre-merge bounds are already in `bounds` (the dst snap
    // is this buffer), so only `region` needs the scan.
    cx.grow_layer_bounds_pixel_exact(receiver, region)?;
    Ok(())
}

impl KindImpl for LayerMergeDownKind {
    fn kind(&self) -> &'static str {
        "layer.merge_down"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let upper = cx
            .envelope()
            .target
            .as_ref()
            .and_then(|t| t.layer_id)
            .map(LayerId::from_uuid)
            .ok_or_else(|| EngineError::bad_target("layer.merge_down requires target.layerId"))?;
        if cx.document().layer(upper).is_none() || upper == cx.document().root() {
            return Err(EngineError::bad_target(format!(
                "target.layerId {upper} does not resolve"
            )));
        }
        let parent = cx.document().parent_of(upper).expect("non-root");
        let siblings = cx.document().children_of(parent).expect("group").to_vec();
        let at = siblings.iter().position(|s| *s == upper).expect("listed");
        // First VISIBLE sibling below (§40.7).
        let lower = siblings[..at]
            .iter()
            .rev()
            .find(|s| {
                cx.document()
                    .layer(**s)
                    .map(|n| n.visible())
                    .unwrap_or(false)
            })
            .copied()
            .ok_or_else(|| EngineError::bad_target("nothing below to merge into (§40.7)"))?;
        check_mergeable(cx, upper)?;
        check_mergeable(cx, lower)?;
        // Locks: PIXELS/ALL on the receiver; ALL on the target (deleted).
        let lower_locks = cx.document().layer(lower).expect("resolved").locks();
        if lower_locks.contains(LockFlags::PIXELS) {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "PIXELS/ALL on the receiving layer blocks merge_down (§40.7)",
            ));
        }
        if cx
            .document()
            .layer(upper)
            .expect("resolved")
            .locks()
            .contains(LockFlags::ALL)
        {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "ALL on the target blocks merge_down (it is deleted, §40.7)",
            ));
        }
        bake_own_mask(cx, lower)?;
        merge_onto(cx, lower, &[upper])?;
        cx.remove_subtree(upper)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for LayerMergeVisibleKind {
    fn kind(&self) -> &'static str {
        "layer.merge_visible"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let into_new = cx
            .params()
            .get("intoNew")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let root = cx.document().root();
        let stack = cx.document().children_of(root).expect("root").to_vec();
        let visibles: Vec<LayerId> = stack
            .iter()
            .filter(|id| {
                cx.document()
                    .layer(**id)
                    .map(|n| n.visible())
                    .unwrap_or(false)
            })
            .copied()
            .collect();
        if visibles.is_empty() {
            return Err(EngineError::bad_target(
                "no visible layers to merge (§40.7)",
            ));
        }
        for v in &visibles {
            check_mergeable(cx, *v)?;
        }
        if into_new {
            // "Stamp Visible": composite into a NEW layer above the topmost
            // visible; sources only read — no locks consulted (§40.7).
            let new_id = cx
                .params()
                .get("newLayerId")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or_else(|| EngineError::missing("newLayerId"))?;
            let lid = LayerId::from_uuid(new_id);
            if cx.document().layer(lid).is_some() {
                return Err(EngineError::with_field(
                    ErrorCode::DuplicateId,
                    "newLayerId",
                    format!("newLayerId {new_id} already exists (§9.6)"),
                ));
            }
            let top_pos = stack
                .iter()
                .position(|s| *s == *visibles.last().expect("non-empty"))
                .expect("listed");
            let working = cx.document().color().working;
            let depth = cx.document().color().depth;
            let node = LayerNode::new_raster(
                lid,
                "Merged",
                crate::buffer::Buffer::new(
                    depth,
                    crate::tile::model::ColorModelKind::Rgba,
                    working,
                ),
            );
            let id = cx.insert_layer(root, top_pos + 1, node)?;
            merge_onto(cx, id, &visibles)?;
            Ok(KindOutcome {
                created_ids: vec![id.as_uuid()],
            })
        } else {
            let receiver = visibles[0];
            let upper: Vec<LayerId> = visibles[1..].to_vec();
            let locks = cx.document().layer(receiver).expect("resolved").locks();
            if locks.contains(LockFlags::PIXELS) {
                return Err(EngineError::new(
                    ErrorCode::Locked,
                    "PIXELS/ALL on the receiving (lowest visible) layer blocks (§40.7)",
                ));
            }
            for u in &upper {
                if cx
                    .document()
                    .layer(*u)
                    .expect("resolved")
                    .locks()
                    .contains(LockFlags::ALL)
                {
                    return Err(EngineError::new(
                        ErrorCode::Locked,
                        "ALL on a merged-away node blocks (§40.7)",
                    ));
                }
            }
            bake_own_mask(cx, receiver)?;
            merge_onto(cx, receiver, &upper)?;
            for u in upper {
                cx.remove_subtree(u)?;
            }
            Ok(KindOutcome::default())
        }
    }
}

impl KindImpl for ImageFlattenKind {
    fn kind(&self) -> &'static str {
        "image.flatten"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Class C: the engine holds the snapshot inverse; per-node locks
        // die with the nodes — none checked (§40.7).
        let _discard_hidden = cx
            .params()
            .get("discardHidden")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let root = cx.document().root();
        let stack = cx.document().children_of(root).expect("root").to_vec();
        if stack.is_empty() {
            return Ok(KindOutcome::default()); // flat/empty doc: no-op push
        }
        let visibles: Vec<LayerId> = stack
            .iter()
            .filter(|id| {
                cx.document()
                    .layer(**id)
                    .map(|n| n.visible())
                    .unwrap_or(false)
            })
            .copied()
            .collect();
        if stack.len() == 1 && visibles.len() == 1 && cx.document().mask_count() == 0 {
            return Ok(KindOutcome::default()); // already flat: no-op push (§40.7)
        }
        if visibles.is_empty() {
            return Err(EngineError::bad_target(
                "no visible layers to flatten (§40.7)",
            ));
        }
        for v in &visibles {
            check_mergeable(cx, *v)?;
        }
        // Receiver = the lowest visible node; composite the rest in, drop
        // everything else, bake the receiver's own mask, reset its
        // presentation props (§12.4 background-ize).
        let receiver = visibles[0];
        bake_own_mask(cx, receiver)?;
        merge_onto(cx, receiver, &visibles[1..])?;
        for id in stack {
            if id != receiver {
                cx.remove_subtree(id)?;
            }
        }
        let mut props = cx.document().props_block(receiver).expect("resolved");
        props.opacity = 1.0;
        props.fill_opacity = 1.0;
        cx.set_layer_props(receiver, props)?;
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;
    use serde_json::json;

    fn assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
    }

    fn ready() -> Engine {
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "3e60e000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 128, "height": 128, "background": "transparent"},
            })
            .to_string(),
        )
        .unwrap();
        // Journaled empty stack: drop the doc.new background so the merge
        // fixtures own the whole z-order.
        let bg = e.document().unwrap().z_order_walk()[0];
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "3e60e000-0000-4000-8000-000000000002",
                "kind": "layer.delete",
                "target": {"layerId": bg.as_uuid().to_string()},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, layer: Option<LayerId>, params: serde_json::Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("3e60e000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.as_uuid().to_string()});
        }
        v.to_string()
    }

    /// Journaled colored block via edit.paste.
    fn block(e: &mut Engine, n: u32, rect: IRect, rgba: [u8; 4]) -> LayerId {
        let (w, h) = (rect.width() as u32, rect.height() as u32);
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&rgba.repeat((w * h) as usize)).unwrap();
        }
        let asset = e.add_raster_asset(png).unwrap();
        let id = Uuid::new_v4();
        e.apply_json(&env(
            n,
            "edit.paste",
            None,
            json!({"assetId": asset.to_string(), "at": [rect.min.x, rect.min.y],
                    "newLayerId": id.to_string()}),
        ))
        .unwrap();
        LayerId::from_uuid(id)
    }

    fn px(e: &Engine, l: LayerId, x: i32, y: i32) -> [f32; 4] {
        e.document()
            .unwrap()
            .layer(l)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    fn children(e: &Engine) -> Vec<LayerId> {
        let d = e.document().unwrap();
        d.children_of(d.root()).unwrap().to_vec()
    }

    #[test]
    fn merge_down_composites_and_round_trips() {
        let mut e = ready();
        let lower = block(&mut e, 1, IRect::new(0, 0, 64, 64), [255, 0, 0, 255]);
        let upper = block(&mut e, 2, IRect::new(32, 32, 96, 96), [0, 0, 255, 255]);
        // Upper at 50% opacity via layer.set_props.
        e.apply_json(&env(
            3,
            "layer.set_props",
            Some(upper),
            json!({"opacity": 0.5}),
        ))
        .unwrap();
        let before = e.state_hash();
        e.apply_json(&env(4, "layer.merge_down", Some(upper), json!({})))
            .unwrap();
        assert_eq!(children(&e), vec![lower], "upper removed; lower keeps id");
        // Overlap (40,40): 50% blue over red.
        let p = px(&e, lower, 40, 40);
        assert!(
            (p[2] - 0.5).abs() < 0.01 && (p[0] - 0.5).abs() < 0.01,
            "50/50 blend: {p:?}"
        );
        // Upper-only region lands in the receiver.
        let p = px(&e, lower, 90, 90);
        assert!(
            (p[2] - 0.5).abs() < 0.01 && p[0] == 0.0,
            "upper content carried: {p:?}"
        );
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            before,
            "tile deltas + RemoveSubtree restore"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after);
    }

    #[test]
    fn merge_down_rejects_typed() {
        let mut e = ready();
        let bottom = block(&mut e, 10, IRect::new(0, 0, 16, 16), [255, 0, 0, 255]);
        // Nothing below the bottom layer.
        let err = e
            .apply_json(&env(11, "layer.merge_down", Some(bottom), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        let upper = block(&mut e, 12, IRect::new(0, 0, 16, 16), [0, 255, 0, 255]);
        // Non-normal blend on a participant is typed.
        e.apply_json(&env(
            13,
            "layer.set_props",
            Some(upper),
            json!({"blend": "multiply"}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(14, "layer.merge_down", Some(upper), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        e.apply_json(&env(
            15,
            "layer.set_props",
            Some(upper),
            json!({"blend": "normal"}),
        ))
        .unwrap();
        // PIXELS on the receiver blocks.
        e.apply_json(&env(
            16,
            "layer.set_props",
            Some(bottom),
            json!({"locks": {"pixels": true}}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(17, "layer.merge_down", Some(upper), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    #[test]
    fn merge_visible_stamp_and_collapse() {
        let mut e = ready();
        let a = block(&mut e, 20, IRect::new(0, 0, 32, 32), [255, 0, 0, 255]);
        let b = block(&mut e, 21, IRect::new(16, 16, 48, 48), [0, 255, 0, 255]);
        let hidden = block(&mut e, 22, IRect::new(60, 60, 90, 90), [0, 0, 255, 255]);
        e.apply_json(&env(
            23,
            "layer.set_props",
            Some(hidden),
            json!({"visible": false}),
        ))
        .unwrap();
        // Stamp Visible: new layer, sources remain.
        let stamp = Uuid::new_v4();
        let receipt = e
            .apply_json(&env(
                24,
                "layer.merge_visible",
                None,
                json!({"intoNew": true, "newLayerId": stamp.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![stamp]);
        assert_eq!(children(&e).len(), 4, "sources remain + stamp");
        let stamp = LayerId::from_uuid(stamp);
        let p = px(&e, stamp, 20, 20);
        assert!(p[1] > 0.9, "green over red at the overlap: {p:?}");
        assert_eq!(px(&e, stamp, 70, 70)[3], 0.0, "hidden layer excluded");
        // Collapse (intoNew:false): receiver = lowest visible (a).
        let before = e.state_hash();
        e.apply_json(&env(25, "layer.merge_visible", None, json!({})))
            .unwrap();
        let kids = children(&e);
        assert!(kids.contains(&a) && kids.contains(&hidden) && !kids.contains(&b));
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn flatten_is_class_c_snapshot_with_mask_bake() {
        let mut e = ready();
        let base = block(&mut e, 30, IRect::new(0, 0, 64, 64), [255, 0, 0, 255]);
        let top = block(&mut e, 31, IRect::new(32, 0, 96, 64), [0, 0, 255, 255]);
        let hidden = block(&mut e, 32, IRect::new(100, 100, 120, 120), [0, 255, 0, 255]);
        e.apply_json(&env(
            33,
            "layer.set_props",
            Some(hidden),
            json!({"visible": false}),
        ))
        .unwrap();
        // Half-coverage mask on top (fill 0.5 over its extent).
        let m = Uuid::new_v4();
        e.apply_json(&env(
            34,
            "mask.add",
            Some(top),
            json!({"kind": "raster", "newMaskId": m.to_string()}),
        ))
        .unwrap();
        e.apply_json(&env(
            35,
            "mask.edit",
            Some(top), // target block built below
            json!({"edit": {"op": "fill", "value": 0.5}}),
        ))
        .unwrap_err(); // needs maskId in target — build properly:
        let mask_env = {
            let mut v: serde_json::Value = serde_json::from_str(&env(
                36,
                "mask.edit",
                None,
                json!({"edit": {"op": "fill", "value": 0.5}}),
            ))
            .unwrap();
            v["target"] = json!({"layerId": top.as_uuid().to_string(), "maskId": m.to_string()});
            v.to_string()
        };
        e.apply_json(&mask_env).unwrap();
        let before = e.state_hash();
        e.apply_json(&env(37, "image.flatten", None, json!({})))
            .unwrap();
        assert_eq!(children(&e), vec![base], "single background remains");
        assert_eq!(e.document().unwrap().mask_count(), 0, "masks baked away");
        // Overlap (40,32): top at mask 0.5 over red.
        let p = px(&e, base, 40, 32);
        assert!(
            (p[2] - 0.5).abs() < 0.01 && (p[0] - 0.5).abs() < 0.01,
            "masked blend: {p:?}"
        );
        assert_eq!(px(&e, base, 110, 110)[3], 0.0, "hidden content discarded");
        // §10.4 snap inverse: O(1) undo/redo swaps.
        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "snapshot restores everything");
        assert_eq!(e.document().unwrap().mask_count(), 1);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        // Already-flat: no-op push.
        let l = e.journal().len();
        e.apply_json(&env(38, "image.flatten", None, json!({})))
            .unwrap();
        assert_eq!((e.journal().len(), e.state_hash()), (l + 1, after));
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "class-C replay determinism"
        );
    }
}
