// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Composite region readback — one truth: the render compositor.
//!
//! Spec: ES §4.4 (source-over in linear premultiplied f32), §34.3 (the CPU
//! compositor as the permanent parity oracle), §8.2 (sRGB straight-alpha
//! 8-bit delivery convention), §6.4 (unpremultiply → transfer-encode →
//! quantize delivery math); plan §2.2 row "C ABI v1" readback note.
//!
//! `appname_doc_read_region` and `appname_doc_export` share ONE compositing
//! path: `appname_render::composite_document_with` over the ENGINE'S §12.1
//! op registry (blend modes, layer/fill opacity, groups, adjustment layers,
//! enabled smart-filter chains — the §34.3 parity-gated compositor; the
//! same `engine.assembly().ops()` the CLI's export dispatch passes),
//! followed by the §6.4 delivery conversion
//! `appname_io::linear_plane_to_srgb8` — the exact functions the export
//! door's general path calls (`export.rs`). A readback is therefore
//! pixel-identical to the export of the same region by shared code, not by
//! re-derivation; the conformance suite pins the equality on a
//! non-normal-blend + partial-opacity document. (The CP-0.3 interim oracle
//! that mirrored `export_png_bytes`'s source-over-only math retired here —
//! it ignored blend mode and layer opacity. The consolidated round closed a
//! second truth gap the same way: the composite calls carried a BARE op
//! registry, so adjustment-carrying documents refused through the ABI while
//! the CLI served them. Do not grow compositing logic in this module: it
//! routes, converts, and validates geometry only.)
//!
//! Per-layer content readback ([`layer_region_rgba8`], ABI 1.4 — O-UI
//! ABI-growth request 7) serves a SINGLE layer's self-contained content
//! under the same delivery convention: raster content directly (compositing
//! a lone source onto transparency is the §4.4 identity — no blend math to
//! re-derive), smart-object content as the §12.1-EVALUATED chain result
//! through core's exported `ChainEvaluator` (never the raw proxy while a
//! chain is enabled; §12.4 identity when none is), and GROUP content (the
//! return-leg growth under the same symbol — the recorded render handoff
//! landed) through `appname_render::composite_subtree_with` over the
//! engine's op registry: the group's children composited AS-IF-ISOLATED
//! onto transparency (§4.6 step 4b), the group's own contribution props
//! never applied, the root group ≡ the whole-document composite by the
//! door's identity contract. The per-node refusal matrix mirrors the
//! compositor's flatten arms and the export door's posture — refuse typed,
//! never approximate.

use appname_core::tile::model::ColorModelKind;
use appname_core::{Engine, EngineError, ErrorCode, IRect, IVec2};

/// Readback size cap: 1 << 26 px (a 256 MiB RGBA8 block). Larger requests
/// are `error.api.invalid_arg` — tile/viewport readers never come close, and
/// the cap keeps a hostile `w*h` from wedging the process.
pub(crate) const MAX_READBACK_PIXELS: u64 = 1 << 26;

/// Validate readback geometry shared by every region reader: non-zero
/// extent, the pixel cap, and i32 coordinate range for `[x, x+w) x [y, y+h)`.
fn readback_rect(x: i32, y: i32, w: u32, h: u32) -> Result<IRect, EngineError> {
    if w == 0 || h == 0 {
        return Err(EngineError::with_field(
            ErrorCode::InvalidArg,
            "rect",
            "readback rect must have non-zero width and height",
        ));
    }
    let px = u64::from(w) * u64::from(h);
    if px > MAX_READBACK_PIXELS {
        return Err(EngineError::with_field(
            ErrorCode::InvalidArg,
            "rect",
            format!(
                "readback rect covers {px} px; the cap is {MAX_READBACK_PIXELS} px — read in tiles"
            ),
        ));
    }
    let max_x = i64::from(x) + i64::from(w);
    let max_y = i64::from(y) + i64::from(h);
    if max_x > i64::from(i32::MAX) || max_y > i64::from(i32::MAX) {
        return Err(EngineError::with_field(
            ErrorCode::InvalidArg,
            "rect",
            "readback rect exceeds the i32 document coordinate range",
        ));
    }
    Ok(IRect::new(x, y, max_x as i32, max_y as i32))
}

/// Composite `w x h` document pixels starting at `(x, y)` into tightly
/// packed RGBA8 (stride = `w * 4`), sRGB-encoded, straight alpha. Pixels
/// outside every layer (or outside the canvas) come back `0,0,0,0`.
/// Blend modes, layer/fill opacity, and groups composite through the §34.3
/// parity-gated render compositor — the same path (and the same delivery
/// math) as `appname_doc_export`.
pub(crate) fn composite_region_rgba8(
    engine: &Engine,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> Result<Vec<u8>, EngineError> {
    let doc = engine
        .document()
        .ok_or_else(|| EngineError::bad_target("an uninitialized document has no composite"))?;
    if doc.color().mode != ColorModelKind::Rgba {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "readback of non-rgba document modes needs the §5/§6 conversion pipeline (CP-0.5)",
        ));
    }
    let roi = readback_rect(x, y, w, h)?;

    // The one compositing truth (§34.3): readback composites through
    // render's `composite_document_with` over the engine's FULL assembly
    // (§12.1 ops + the Door-6 text / Door-1 vector / fill-math seams), the
    // SAME call the export door (export.rs) and the CLI export dispatch now
    // make — so every shipped surface serves every layer kind (raster,
    // group, smart, adjustment, AND parametric shape/text) uniformly.
    // Readback is pixel-identical to export by construction: one door, one
    // math path (the composite-legs slice collapsed the old three-way
    // disagreement where readback/export refused shape/text).
    let composite =
        appname_render::composite_document_with(doc, engine.assembly(), roi).map_err(|e| {
            EngineError::new(
                ErrorCode::NotImplemented,
                format!("render composite refused: {e}"),
            )
        })?;
    let plane = composite.region_rgba_f32(roi);
    // §6.4 delivery: unpremultiply, sRGB-encode, quantize — the export
    // pipeline's own conversion, shared, not mirrored.
    appname_io::linear_plane_to_srgb8(w, h, &plane).map_err(|e| {
        // Unreachable in practice (the plane length is w*h*4 by
        // construction); keep it typed rather than a panic path.
        EngineError::new(ErrorCode::InvalidArg, format!("delivery conversion: {e}"))
    })
}

/// Materialize `w x h` document-space pixels of one linear premultiplied
/// f32 source as a flat delivery plane and convert per §6.4 (the shared
/// export/readback conversion). `read` answers document-space coordinates;
/// pixels outside the source read transparent (§2.4 absent-tile rule).
fn deliver_rgba8(
    w: u32,
    h: u32,
    roi: IRect,
    read: impl Fn(IVec2) -> [f32; 4],
) -> Result<Vec<u8>, EngineError> {
    let mut plane = Vec::with_capacity(w as usize * h as usize * 4);
    for y in roi.min.y..roi.max.y {
        for x in roi.min.x..roi.max.x {
            plane.extend_from_slice(&read(IVec2::new(x, y)));
        }
    }
    appname_io::linear_plane_to_srgb8(w, h, &plane).map_err(|e| {
        // Unreachable in practice (plane length is w*h*4 by construction);
        // typed rather than a panic path.
        EngineError::new(ErrorCode::InvalidArg, format!("delivery conversion: {e}"))
    })
}

/// Read `w x h` document pixels of ONE layer's SELF-CONTAINED content
/// starting at `(x, y)` as tightly packed RGBA8 (stride = `w * 4`),
/// sRGB-encoded, straight alpha — the read_region delivery convention
/// (ABI 1.4, O-UI ABI-growth request 7: layer thumbnails, faithful Cut
/// pairing, move-tool per-layer echo).
///
/// CONTENT, not contribution: the node's own compositing props — blend
/// mode, opacity, fillOpacity — describe its interaction with the BACKDROP
/// and are deliberately NOT applied (a thumbnail does not fade with layer
/// opacity; the Cut carrier is the content; hosts echoing a drag apply
/// §4.4 themselves from info's reported props). Lane election, recorded in
/// the log.
///
/// Service matrix (the render/core truth, refusals mirroring the
/// compositor's flatten arms — typed, never approximate):
/// * raster layer — the buffer's pixels (a lone source over transparency
///   is the §4.4 source-over identity; no compositing math re-derived);
/// * smart object — the §12.1-EVALUATED result: the value-captured proxy
///   through the layer's ENABLED smart-filter chain via core's exported
///   `ChainEvaluator` over the engine's op registry (§12.4 structural
///   identity serves the proxy directly when no entry is enabled);
/// * adjustment layer — `error.command.target_kind_mismatch`: its §17.1
///   result exists only over its backdrop (read the document composite);
/// * group (incl. the root group) — SERVES, the as-if-isolated election:
///   the group's children composited onto TRANSPARENCY with full §4.4–§4.7
///   semantics WITHIN the subtree (`composite_subtree_with` over the
///   engine's op registry — the recorded render handoff, landed; the
///   capability arrived additively under this same symbol, exactly as
///   filed). The group's own §4.7 election is irrelevant to its CONTENT
///   (a pass-through group's isolated content is a definition of content,
///   not a decomposition of the document composite); root-as-target is the
///   whole-document composite by the door's identity contract. Adjustment
///   layers inside the target scope refuse `error.command.not_implemented`
///   with the isolating scope NAMED (the door's §11.4 boundary: their
///   §17.1 result over the door's transparency backdrop is not their
///   document content — deliberately stricter than the full compositor's
///   root-scope serve);
/// * anything the compositor refuses on the node (non-identity transform,
///   clipToBelow, attached masks, non-RGBA sources, non-identity
///   smart-object placement, §12.1 chain refusals) refuses here with the
///   same typed posture.
pub(crate) fn layer_region_rgba8(
    engine: &Engine,
    layer: uuid::Uuid,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> Result<Vec<u8>, EngineError> {
    use appname_core::engine::op_eval::{ChainEvaluator, OpPlane};
    use appname_core::graph::id::LayerId;

    let doc = engine
        .document()
        .ok_or_else(|| EngineError::bad_target("an uninitialized document has no layers"))?;
    if doc.color().mode != ColorModelKind::Rgba {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "readback of non-rgba document modes needs the §5/§6 conversion pipeline (CP-0.5)",
        ));
    }
    let roi = readback_rect(x, y, w, h)?;
    let lid = LayerId::from_uuid(layer);
    let node = doc
        .layer(lid)
        .ok_or_else(|| EngineError::bad_target(format!("layer {layer} does not resolve")))?;
    // Node-level refusal matrix, mirroring the compositor's flatten arms
    // (typed refusal, never silently ignored state).
    if !node.transform().is_identity() {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "per-layer readback of a non-identity layer transform composites through the \
             §18 resample engines — refused typed, never omitted (mirrors the compositor)",
        ));
    }
    if node.clip_to_below() {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "clipping (clipToBelow) has no composite path at this checkpoint — refused \
             typed (mirrors the compositor)",
        ));
    }
    if !node.masks().is_empty() {
        return Err(EngineError::new(
            ErrorCode::NotImplemented,
            "mask application is the render row — a masked layer's readback arrives with \
             it; refused typed (mirrors the compositor)",
        ));
    }

    if node.is_group() {
        // THE GROUP ARM (return-leg growth, same symbol — the recorded
        // render handoff landed as `composite_subtree_with`): the group's
        // children composited AS-IF-ISOLATED onto transparency (§4.6 step
        // 4b) over the engine's §12.1 op registry, the group's own
        // contribution props never applied (the door's own election — one
        // truth, not re-derived here). Root-as-target is the whole-document
        // composite by the door's identity contract, so the former
        // root-refusal branch retired with this arm. Refusals (adjustment
        // in the target scope with the isolating scope named, node-level
        // constructs, §12.1 chain refusals) surface through the cabi's
        // standing typed arm, exactly like the whole-document composite.
        let composite = appname_render::composite_subtree_with(doc, engine.assembly(), lid, roi)
            .map_err(|e| {
                EngineError::new(
                    ErrorCode::NotImplemented,
                    format!("render subtree composite refused: {e}"),
                )
            })?;
        let plane = composite.region_rgba_f32(roi);
        return appname_io::linear_plane_to_srgb8(w, h, &plane).map_err(|e| {
            // Unreachable in practice (the plane length is w*h*4 by
            // construction); typed rather than a panic path.
            EngineError::new(ErrorCode::InvalidArg, format!("delivery conversion: {e}"))
        });
    }
    if node.adjustment_op().is_some() {
        return Err(EngineError::new(
            ErrorCode::TargetKindMismatch,
            "an adjustment layer has no self-contained pixels — its §17.1 result exists \
             only over its backdrop; read the document composite (appname_doc_read_region)",
        ));
    }
    if let Some(buf) = node.pixels() {
        if buf.model() != ColorModelKind::Rgba {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "layer buffer model {:?} needs the §5/§6 pipeline (CP-0.5)",
                    buf.model()
                ),
            ));
        }
        // §4.4: a lone source composited onto transparency is the source.
        return deliver_rgba8(w, h, roi, |p| buf.read_pixel_f32(p));
    }
    if let Some(so) = node.smart_object() {
        let proxy = so.cached_composite();
        if proxy.model() != ColorModelKind::Rgba {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "smart proxy model {:?} needs the §5/§6 pipeline (CP-0.5)",
                    proxy.model()
                ),
            ));
        }
        if !so.place_transform().is_identity() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "non-identity smart-object placement composites through the tools \
                 resample engines (§18.2) — refused typed (mirrors the compositor)",
            ));
        }
        let chain = so.smart_filters();
        let enabled = chain
            .iter()
            .any(|o| doc.op(*o).map(|n| n.enabled()).unwrap_or(false));
        if !enabled {
            // §12.4 structural identity: the proxy IS the layer's pixels.
            return deliver_rgba8(w, h, roi, |p| proxy.read_pixel_f32(p));
        }
        // §12.1 evaluation through core's exported evaluator over the
        // engine's op registry — the EVALUATED result, never the raw
        // proxy (refusal matrix inherited verbatim from the evaluator:
        // unregistered enabled kind, non-Normal fade, feathered op mask).
        let ev = ChainEvaluator::new(engine.assembly().ops(), doc);
        let in_rect = ev.chain_input_rect(chain, roi)?;
        let mut plane = OpPlane::transparent(in_rect);
        let iw = in_rect.width() as usize;
        for (i, px) in plane.px.iter_mut().enumerate() {
            let p = IVec2::new(
                in_rect.min.x + (i % iw) as i32,
                in_rect.min.y + (i / iw) as i32,
            );
            *px = proxy.read_pixel_f32(p);
        }
        let out = ev.eval_chain(chain, plane, roi)?;
        return deliver_rgba8(w, h, roi, |p| out.get(p));
    }
    // §2.6 #[non_exhaustive] tail: shape/text/… land with their own
    // composite paths (the compositor's own posture for unknown kinds).
    Err(EngineError::new(
        ErrorCode::NotImplemented,
        format!(
            "layer kind \"{}\" has no composite path at this checkpoint — refused typed \
             (mirrors the compositor)",
            node.kind().kind_str()
        ),
    ))
}

/// Read `w x h` document pixels of the SELECTION coverage mask starting at
/// `(x, y)` as tightly packed Gray8 (stride = `w` bytes): 0 = unselected,
/// 255 = fully selected, linear coverage quantization (the §13.1 f32 mask
/// delivered under the §8.2 8-bit storage-optimization rule). Pixels
/// outside the mask read 0. Shares `composite_region_rgba8`'s geometry
/// discipline: zero-extent and > [`MAX_READBACK_PIXELS`] rects reject
/// typed, a pre-init handle is `error.command.bad_target`. An INACTIVE
/// selection reads back all-zero (the honest "nothing selected" answer —
/// hosts branch on info's `selection.active` first).
pub(crate) fn selection_region_gray8(
    engine: &Engine,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
) -> Result<Vec<u8>, EngineError> {
    let doc = engine
        .document()
        .ok_or_else(|| EngineError::bad_target("an uninitialized document has no selection"))?;
    readback_rect(x, y, w, h)?;
    let mask = doc.selection().mask();
    let mut data = Vec::with_capacity(w as usize * h as usize);
    for row in 0..i64::from(h) {
        for col in 0..i64::from(w) {
            let p = IVec2::new((i64::from(x) + col) as i32, (i64::from(y) + row) as i32);
            // Gray/f32 coverage in component 0 (ES §13.1; absent tiles read
            // zero per §2.4).
            let v = mask.read_pixel_f32(p)[0];
            data.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use appname_core::io::srgb::srgb_decode;
    use serde_json::json;

    fn engine_with(background: &str, w: u32, h: u32) -> Engine {
        let mut e = Engine::new();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c1000000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": w, "height": h, "background": background}
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    #[test]
    fn white_background_reads_back_solid_white() {
        let e = engine_with("white", 8, 4);
        let data = composite_region_rgba8(&e, 0, 0, 8, 4).unwrap();
        assert_eq!(data.len(), 8 * 4 * 4);
        assert!(data.chunks(4).all(|p| p == [255, 255, 255, 255]));
    }

    #[test]
    fn out_of_canvas_pixels_are_transparent_zero() {
        let e = engine_with("black", 4, 4);
        let data = composite_region_rgba8(&e, -2, -2, 4, 4).unwrap();
        // Top-left quadrant off-canvas, bottom-right on the black background.
        assert_eq!(&data[0..4], [0, 0, 0, 0]);
        let last = &data[(4 * 4 - 1) * 4..];
        assert_eq!(last, [0, 0, 0, 255]);
    }

    #[test]
    fn full_canvas_readback_matches_png_export_bytes() {
        // THE oracle relationship: readback == decode(export_png). Compared
        // in the linear domain through appname-core's own decoder (exact for
        // the opaque background; both paths share the u8 sRGB round-trip).
        let e = engine_with("white", 6, 3);
        let png = appname_core::export_png_bytes(&e).unwrap();
        let (decoded, pw, ph) =
            appname_core::decode_png_to_buffer(&png, appname_core::ProfileId::nil()).unwrap();
        assert_eq!((pw, ph), (6, 3));
        let mine = composite_region_rgba8(&e, 0, 0, 6, 3).unwrap();
        for row in 0..3 {
            for col in 0..6 {
                let i = (row * 6 + col) * 4;
                let a = mine[i + 3] as f32 / 255.0;
                let lin = [
                    srgb_decode(mine[i] as f32 / 255.0) * a,
                    srgb_decode(mine[i + 1] as f32 / 255.0) * a,
                    srgb_decode(mine[i + 2] as f32 / 255.0) * a,
                    a,
                ];
                let want = decoded.read_pixel_f32(IVec2::new(col as i32, row as i32));
                assert_eq!(lin, want, "pixel ({col},{row})");
            }
        }
    }

    #[test]
    fn cropped_export_and_readback_agree_over_the_subregion() {
        use appname_core::{IRect, IVec2, ProfileId};
        // §19.1 one-compositing-truth cross-door check on the SHIPPED cabi
        // surface: a non-destructively cropped doc must export (core
        // reference door) and read back (render compositor door) to the
        // SAME crop sub-region — offset + size honoured, byte-identical.
        // Production assembly (carries doc.crop + brush.stroke); 24×16 white
        // doc with a hard-round red dab at (8,8) so the crop straddles the
        // red/white boundary (a uniform doc could not observe the offset).
        let mut e = appname_assembly::production_assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c2000000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 24, "height": 16, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        let layer = e.document().unwrap().z_order_walk()[0];
        let samples = appname_tools::brush::encode_samples(&[appname_tools::brush::InputSample {
            x: 8.0,
            y: 8.0,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            velocity: 0.0,
            t: 0.0,
        }])
        .unwrap();
        let asset = e.add_samples_asset(samples);
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c2000000-0000-4000-8000-000000000002",
                "kind": "brush.stroke",
                "target": {"layerId": layer.as_uuid().to_string()},
                "params": {
                    "brushPreset": appname_tools::kind_impls::preset_hard_round_16_id().to_string(),
                    "color": {"model": "rgb", "comps": [1, 0, 0, 0]},
                },
                "samples": asset.to_string(),
            })
            .to_string(),
        )
        .unwrap();

        // Render readback of the target sub-region BEFORE the crop.
        let crop = IRect::new(4, 4, 20, 12); // {x:4,y:4,w:16,h:8}
        let before = composite_region_rgba8(&e, 4, 4, 16, 8).unwrap();

        // Non-destructive crop to that sub-region.
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c2000000-0000-4000-8000-000000000003",
                "kind": "doc.crop",
                "params": {"rect": {"x": 4, "y": 4, "w": 16, "h": 8}},
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            e.document().unwrap().geometry().crop_rect,
            Some(crop),
            "non-destructive crop set the window"
        );

        // Door A — core reference, auto-framed on image_rect: export PNG.
        let png = appname_core::export_png_bytes(&e).unwrap();
        let (decoded, pw, ph) = appname_core::decode_png_to_buffer(&png, ProfileId::nil()).unwrap();
        assert_eq!((pw, ph), (16, 8), "core export framed on the crop sub-rect");

        // Door B — render compositor, explicit ROI at the crop origin: the
        // SAME sub-region, and UNCHANGED by the non-destructive crop.
        let after = composite_region_rgba8(&e, 4, 4, 16, 8).unwrap();
        assert_eq!(after, before, "non-destructive: composited pixels unmoved");

        // One compositing truth: the two doors agree byte-for-byte over the
        // cropped sub-region (linear domain — the existing oracle relation).
        for row in 0..8usize {
            for col in 0..16usize {
                let i = (row * 16 + col) * 4;
                let a = after[i + 3] as f32 / 255.0;
                let lin = [
                    srgb_decode(after[i] as f32 / 255.0) * a,
                    srgb_decode(after[i + 1] as f32 / 255.0) * a,
                    srgb_decode(after[i + 2] as f32 / 255.0) * a,
                    a,
                ];
                let want = decoded.read_pixel_f32(IVec2::new(col as i32, row as i32));
                assert_eq!(lin, want, "cropped pixel ({col},{row}) agrees across doors");
            }
        }
        // Offset sanity: local (0,0) == world (4,4) is inside the red dab;
        // local (15,7) == world (19,11) is outside it (white).
        let red = decoded.read_pixel_f32(IVec2::new(0, 0));
        assert!(
            red[0] > 0.8 && red[1] < 0.3,
            "crop origin is the dab: {red:?}"
        );
        let white = decoded.read_pixel_f32(IVec2::new(15, 7));
        assert!(white[1] > 0.8, "far corner is white: {white:?}");
    }

    #[test]
    fn shell_and_bad_rects_are_typed() {
        let shell = Engine::new();
        let err = composite_region_rgba8(&shell, 0, 0, 1, 1).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);

        let e = engine_with("white", 4, 4);
        let err = composite_region_rgba8(&e, 0, 0, 0, 4).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArg);
        let err = composite_region_rgba8(&e, 0, 0, 1 << 15, 1 << 15).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArg);
        assert!(err.message.contains("cap"));
    }
}
