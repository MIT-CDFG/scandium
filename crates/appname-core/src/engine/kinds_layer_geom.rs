// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `layer.align` / `layer.distribute` / `layer.transform`;
//! ES §40.7, §11.1 (POSITION blocks moves), §18.2/§18.3 (resample +
//! exactness), §19.1 (integer offsets). CP-0.5 wave-4 kind tail, geometry
//! half.
//!
//! v1 semantics + divergences (lane log): the compositor refuses
//! non-identity node transforms until its transform row lands, so
//! align/distribute BAKE their integer translations into pixels (tile
//! deltas — visually correct and render-compatible today) instead of the
//! contract's SetProp-transform note; the transform prop stays identity.
//! Recorded divergence, revisited with the compositor row.
//!
//! `layer.transform` LIFTED to `appname-tools` (lane/transform-183, the
//! brokered swap — fill-lift precedent): its general (scale/rotate/affine)
//! arm resamples through the §18.2 kernel, which lives tools-side
//! (`resample`/`xform`), so the whole kind moves home beside
//! `transform.free`. Core keeps only the integer-only geometry kinds
//! (`layer.align`/`layer.distribute`); they share [`translate_layer_pixels`]
//! and never touch a resample kernel. Registration: this module's
//! [`impls`] no longer lists `LayerTransformKind`; the assembly points
//! adopt the tools copy through `assembly()` (dup-registration guard: it is
//! registered exactly once, tools-side).

use std::sync::Arc;

use serde_json::Value;

use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::geom::{IRect, IVec2};
use crate::graph::id::LayerId;
use crate::graph::node::LockFlags;
use crate::tile::map::TileCoord;
use crate::tile::{TILE, TILE_I32};
use uuid::Uuid;

pub struct LayerAlignKind;
pub struct LayerDistributeKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    // `layer.transform` LIFTED to appname-tools (module doc): the assembly
    // points register the tools copy. Core keeps the integer-only geometry
    // kinds here.
    vec![Arc::new(LayerAlignKind), Arc::new(LayerDistributeKind)]
}

fn position_locked(cx: &KindContext<'_>, layer: LayerId) -> bool {
    // Door-slice lock consolidation: the one-place effective set (§11.1;
    // POSITION alone suffices — ALL contains it).
    cx.document()
        .effective_locks(layer)
        .contains(LockFlags::POSITION)
}

/// Bake an integer translation into a raster layer's pixels (exact remap:
/// dest(p) = src(p − d); vacated pixels clear; bounds translate exactly).
fn translate_layer_pixels(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    d: IVec2,
) -> Result<(), EngineError> {
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

/// A layer's operative rect for align math (content bounds; §2.5).
fn layer_rect(cx: &KindContext<'_>, layer: LayerId) -> Result<IRect, EngineError> {
    let n = cx
        .document()
        .layer(layer)
        .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?;
    let b = n.pixels().map(|p| p.bounds()).unwrap_or(IRect::EMPTY);
    if b.is_empty() {
        return Err(EngineError::bad_target(format!(
            "layer {layer} has no content to align (empty bounds)"
        )));
    }
    Ok(b)
}

fn parse_layer_ids(cx: &KindContext<'_>, min: usize) -> Result<Vec<LayerId>, EngineError> {
    let ids = cx
        .params()
        .get("layerIds")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("layerIds"))?;
    if ids.len() < min {
        return Err(EngineError::range(
            "layerIds",
            format!("at least {min} members required"),
        ));
    }
    let mut out = Vec::with_capacity(ids.len());
    for v in ids {
        let id = v
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| EngineError::range("layerIds", "members must be uuids"))?;
        let lid = LayerId::from_uuid(id);
        if cx.document().layer(lid).is_none() || lid == cx.document().root() {
            return Err(EngineError::bad_target(format!(
                "layer {id} does not resolve"
            )));
        }
        if position_locked(cx, lid) {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "POSITION or ALL on a moved layer blocks (§11.1 names align/distribute)",
            ));
        }
        out.push(lid);
    }
    Ok(out)
}

impl KindImpl for LayerAlignKind {
    fn kind(&self) -> &'static str {
        "layer.align"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layers = parse_layer_ids(cx, 1)?;
        let mode = cx
            .params()
            .get("mode")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("mode"))?
            .to_string();
        let to = cx
            .params()
            .get("to")
            .and_then(Value::as_str)
            .unwrap_or("layers")
            .to_string();
        let rects: Vec<IRect> = layers
            .iter()
            .map(|l| layer_rect(cx, *l))
            .collect::<Result<_, _>>()?;
        let frame = match to.as_str() {
            "layers" => {
                if layers.len() == 1 {
                    // §40.7: single layer to itself is the documented no-op
                    // push.
                    return Ok(KindOutcome::default());
                }
                rects.iter().fold(IRect::EMPTY, |a, r| a.union(r))
            }
            "selection" => {
                let sel = cx.document().selection();
                if !sel.active() {
                    return Err(EngineError::new(
                        ErrorCode::SelectionRequired,
                        "to:'selection' requires an active selection (§40.7)",
                    ));
                }
                sel.bounds()
            }
            "canvas" => cx.document().geometry().canvas_rect(),
            other => {
                return Err(EngineError::range(
                    "to",
                    format!("unknown frame \"{other}\""),
                ))
            }
        };
        for (l, r) in layers.iter().zip(rects.iter()) {
            let d = match mode.as_str() {
                "left" => IVec2::new(frame.min.x - r.min.x, 0),
                "right" => IVec2::new(frame.max.x - r.max.x, 0),
                "h_center" => IVec2::new(
                    (frame.min.x + frame.max.x - (r.min.x + r.max.x)).div_euclid(2),
                    0,
                ),
                "top" => IVec2::new(0, frame.min.y - r.min.y),
                "bottom" => IVec2::new(0, frame.max.y - r.max.y),
                "v_center" => IVec2::new(
                    0,
                    (frame.min.y + frame.max.y - (r.min.y + r.max.y)).div_euclid(2),
                ),
                other => {
                    return Err(EngineError::range(
                        "mode",
                        format!("unknown mode \"{other}\""),
                    ))
                }
            };
            translate_layer_pixels(cx, *l, d)?;
        }
        Ok(KindOutcome::default())
    }
}

impl KindImpl for LayerDistributeKind {
    fn kind(&self) -> &'static str {
        "layer.distribute"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layers = parse_layer_ids(cx, 3)?;
        let mode = cx
            .params()
            .get("mode")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("mode"))?
            .to_string();
        let horizontal = matches!(mode.as_str(), "left" | "h_center" | "right" | "h_gaps");
        let rects: Vec<IRect> = layers
            .iter()
            .map(|l| layer_rect(cx, *l))
            .collect::<Result<_, _>>()?;
        // Order by the mode's reference coordinate.
        let key = |r: &IRect| -> i32 {
            match mode.as_str() {
                "left" | "top" => {
                    if horizontal {
                        r.min.x
                    } else {
                        r.min.y
                    }
                }
                "right" | "bottom" => {
                    if horizontal {
                        r.max.x
                    } else {
                        r.max.y
                    }
                }
                _ => {
                    if horizontal {
                        r.min.x + r.width() / 2
                    } else {
                        r.min.y + r.height() / 2
                    }
                }
            }
        };
        let mut order: Vec<usize> = (0..layers.len()).collect();
        order.sort_by_key(|i| key(&rects[*i]));
        let first = &rects[order[0]];
        let last = &rects[*order.last().unwrap()];
        let n = order.len();
        match mode.as_str() {
            "h_gaps" | "v_gaps" => {
                // Equalize gaps across the span (§40.7; max−min gap ≤ 1 px).
                let span = if horizontal {
                    last.max.x - first.min.x
                } else {
                    last.max.y - first.min.y
                };
                let content: i32 = order
                    .iter()
                    .map(|i| {
                        if horizontal {
                            rects[*i].width()
                        } else {
                            rects[*i].height()
                        }
                    })
                    .sum();
                let total_gap = span - content;
                let gaps = (n - 1) as i32;
                let mut cursor = if horizontal { first.min.x } else { first.min.y };
                for (k, idx) in order.iter().enumerate() {
                    let r = &rects[*idx];
                    let want = cursor;
                    let cur = if horizontal { r.min.x } else { r.min.y };
                    let d = want - cur;
                    let dv = if horizontal {
                        IVec2::new(d, 0)
                    } else {
                        IVec2::new(0, d)
                    };
                    translate_layer_pixels(cx, layers[*idx], dv)?;
                    let size = if horizontal { r.width() } else { r.height() };
                    // Integer-fair gap: distribute the remainder 1px-wise.
                    let gap = if gaps > 0 {
                        total_gap / gaps + i32::from((k as i32) < total_gap.rem_euclid(gaps.max(1)))
                    } else {
                        0
                    };
                    cursor = want + size + gap;
                }
            }
            "left" | "h_center" | "right" | "top" | "v_center" | "bottom" => {
                // Equal spacing of the reference coordinate between first
                // and last (endpoints stay).
                let a = key(first);
                let b = key(last);
                for (k, idx) in order.iter().enumerate().skip(1).take(n - 2) {
                    let want = a + ((b - a) * k as i32) / (n as i32 - 1);
                    let cur = key(&rects[*idx]);
                    let d = want - cur;
                    let dv = if horizontal {
                        IVec2::new(d, 0)
                    } else {
                        IVec2::new(0, d)
                    };
                    translate_layer_pixels(cx, layers[*idx], dv)?;
                }
            }
            other => {
                return Err(EngineError::range(
                    "mode",
                    format!("unknown mode \"{other}\""),
                ))
            }
        }
        Ok(KindOutcome::default())
    }
}

// `layer.transform` (LayerTransformKind) LIFTED to appname-tools
// (module doc): its general arm resamples through the tools-side §18.2
// kernel. `translate_layer_pixels` above stays in core for
// align/distribute; the tools copy ports the same exact integer path.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;
    use serde_json::json;

    fn assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
            .with_kind(Arc::new(crate::engine::kinds::FillKind))
    }

    fn ready() -> Engine {
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "9e0a0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 256, "height": 256, "background": "transparent"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, layer: Option<LayerId>, params: serde_json::Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("9e0a0000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.as_uuid().to_string()});
        }
        v.to_string()
    }

    /// A raster layer whose content is an opaque block at `rect`, built
    /// FULLY JOURNALED (edit.paste over an ingested asset) so replay
    /// assertions hold.
    fn block_layer(e: &mut Engine, n: u32, rect: crate::geom::IRect) -> LayerId {
        let (w, h) = (rect.width() as u32, rect.height() as u32);
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&[128u8, 128, 128, 255].repeat((w * h) as usize))
                .unwrap();
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

    fn bounds(e: &Engine, l: LayerId) -> IRect {
        e.document()
            .unwrap()
            .layer(l)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds()
    }

    // `layer.transform` moved to appname-tools (module doc); its behavior
    // suite (integer-translate exact + scale/rotate resample + typed
    // refusals + replay) now lives in
    // `appname_tools::kind_impls::transform_tests`.

    #[test]
    fn align_modes_and_frames() {
        let mut e = ready();
        let a = block_layer(&mut e, 10, IRect::new(0, 0, 20, 20));
        let b = block_layer(&mut e, 11, IRect::new(100, 100, 140, 130));
        // to:layers left: both flush to min.x == 0.
        e.apply_json(&env(
            12,
            "layer.align",
            None,
            json!({"layerIds": [a.as_uuid().to_string(), b.as_uuid().to_string()], "mode": "left"}),
        ))
        .unwrap();
        assert_eq!(bounds(&e, b).min.x, 0, "b flushed left to the union frame");
        assert_eq!(bounds(&e, a).min.x, 0);
        // to:canvas bottom.
        e.apply_json(&env(
            13,
            "layer.align",
            None,
            json!({"layerIds": [a.as_uuid().to_string()], "mode": "bottom", "to": "canvas"}),
        ))
        .unwrap();
        assert_eq!(bounds(&e, a).max.y, 256, "flushed to canvas bottom");
        // to:selection requires one.
        let err = e
            .apply_json(&env(
                14,
                "layer.align",
                None,
                json!({"layerIds": [a.as_uuid().to_string()], "mode": "left", "to": "selection"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        // single-layer to:layers = documented no-op push.
        let h = e.state_hash();
        e.apply_json(&env(
            15,
            "layer.align",
            None,
            json!({"layerIds": [a.as_uuid().to_string()], "mode": "left"}),
        ))
        .unwrap();
        assert_eq!(e.state_hash(), h, "no-op still journals");
        // POSITION lock refuses.
        e.doc_mut_for_test().layer_mut(a).unwrap().locks = LockFlags::POSITION;
        let err = e
            .apply_json(&env(16, "layer.align", None,
                json!({"layerIds": [a.as_uuid().to_string(), b.as_uuid().to_string()], "mode": "left"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    #[test]
    fn distribute_equalizes_gaps_within_one_pixel() {
        let mut e = ready();
        let a = block_layer(&mut e, 20, IRect::new(0, 0, 10, 10));
        let b = block_layer(&mut e, 21, IRect::new(13, 0, 33, 10));
        let c = block_layer(&mut e, 22, IRect::new(90, 0, 105, 10));
        e.apply_json(&env(23, "layer.distribute", None,
            json!({"layerIds": [a.as_uuid().to_string(), b.as_uuid().to_string(), c.as_uuid().to_string()],
                    "mode": "h_gaps"})))
            .unwrap();
        let (ra, rb, rc) = (bounds(&e, a), bounds(&e, b), bounds(&e, c));
        assert_eq!(ra.min.x, 0, "first endpoint stays");
        assert_eq!(rc.max.x, 105, "last endpoint stays");
        let g1 = rb.min.x - ra.max.x;
        let g2 = rc.min.x - rb.max.x;
        assert!(
            (g1 - g2).abs() <= 1,
            "§40.7 invariant: max gap − min gap ≤ 1 px ({g1} vs {g2})"
        );
        // minItems 3.
        let err = e
            .apply_json(&env(24, "layer.distribute", None,
                json!({"layerIds": [a.as_uuid().to_string(), b.as_uuid().to_string()], "mode": "h_gaps"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "journaled fixture replays"
        );
    }
}
