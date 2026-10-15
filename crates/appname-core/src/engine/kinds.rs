// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `fill.yaml`, `edit.cut.yaml`, `edit.paste.yaml`,
//! `edit.paste_into.yaml` (params, lock tables, selection behavior); ES
//! §10.3 (fill + clipboard families), §22.6 (fill semantics), §40.5
//! (family rules), §13.1 (selection coverage), §11.1 (locks), §9.6 rule 2
//! (paste data enters as an ASSET first — never inline); plan §2.3.
//! CP-0.5 wave-1 kinds: `fill`, `edit.cut`, `edit.paste`,
//! `edit.paste_into` (the ratified MAJOR-2 reconciliation — the CLI
//! `import` verb submits `edit.paste` over an ingested raster asset, so
//! imports are journaled and replay-exact).
//!
//! These impls need nothing outside core, so they live IN core and register
//! at the assembly points through [`register`] (CLI now; cabi at O-SCRIPT's
//! next slice). They mutate only through the [`KindContext`] doors, exactly
//! like the externally-assembled stroke kinds — being in-crate buys no
//! side door (D-48).
//!
//! v1 scope (typed refusals, stroke-kind precedent):
//! - `fill` implements `content` = `color` / `foreground` / `background`
//!   with `normal` blend; `gradient` (§22.4), `pattern` (§22.5),
//!   `content_aware` (§21.4/§45.3), the flood variant (`seed`), and
//!   non-normal blends answer `error.command.not_implemented` until their
//!   rows.
//! - Mask-surface targets (`target.maskId`) answer typed until the mask.*
//!   kinds row wires mask-coverage editing end-to-end (stroke precedent).
//!
//! Divergences recorded in the lane log: compositing runs in the surface's
//! STORAGE encoding with §22.1 colors decoded to linear (the standing
//! stroke-family interim; linear unification lands with the §5 color wave);
//! `foreground`/`background` resolve to the headless default swatches
//! (black / white — no tool state exists; the doc.new `background` reading);
//! fill's no-selection "whole layer" reads as the CANVAS rect (the layer's
//! operative §2.4 extent — a raster surface is otherwise unbounded);
//! paste's absent `at` centers on the CANVAS (no viewport exists headless)
//! and the journal keeps the SUBMITTED envelope — resolution is a pure
//! function of document geometry, so replay stays exact; §40.5's
//! store-the-resolved-position discipline is flagged for the fold-in list;
//! pasted layers insert at the TOP of the root stack (no active-layer
//! state) under the fixed name "Pasted" (the contract schema admits no
//! name param).

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::buffer::Buffer;
use crate::engine::assets::AssetKind;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{EngineAssembly, KindContext, KindImpl, KindOutcome};
use crate::geom::{IRect, IVec2};
use crate::graph::id::{LayerId, MaskId};
use crate::graph::mask_node::MaskNode;
use crate::graph::node::{LayerNode, LockFlags};
use crate::io::srgb::srgb_decode;
use crate::mask::Selection;
use crate::tile::channel::ChannelKind;
use crate::tile::map::TileCoord;
use crate::tile::model::ColorModelKind;
use crate::tile::{TILE, TILE_I32};

/// Register the core-implemented kinds onto an assembly (the CLI/cabi
/// assembly points call this over the stroke assembly).
pub fn register(assembly: EngineAssembly) -> EngineAssembly {
    // FillKind's OWNERSHIP moved to appname-tools at the brokered swap
    // (wave-3 fill lift: flood variant needs the §22.6 shared wand
    // predicate, which lives tools-side); the assembly points wrap this
    // register() in `with_tools_fill(...)`.
    let assembly = assembly
        .with_kind(Arc::new(EditCutKind))
        .with_kind(Arc::new(EditPasteKind))
        .with_kind(Arc::new(EditPasteIntoKind))
        .with_kind(Arc::new(MaskAddKind))
        .with_kind(Arc::new(MaskEditKind))
        .with_kind(Arc::new(MaskDeleteKind))
        .with_kind(Arc::new(MaskSetPropsKind))
        .with_kind(Arc::new(MaskApplyKind))
        .with_kind(Arc::new(MaskToSelectionKind))
        .with_kind(Arc::new(DocAssignProfileKind))
        .with_kind(Arc::new(DocConvertProfileKind))
        .with_kind(Arc::new(DocDepthConvertKind));
    // Wave-4 registration commit: the kind tail joins the canonical
    // assembly (one corpus re-entry, rev 10).
    register_wave4_tail(assembly)
}

/// The wave-4 tail (layer.* family, geometry, merge/flatten, guides/grid).
/// Called by [`register`]; kept separate for the gate-green commit-by-
/// commit landing history.
fn register_wave4_tail(assembly: EngineAssembly) -> EngineAssembly {
    let assembly = crate::engine::kinds_layer::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    let assembly = crate::engine::kinds_layer_geom::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    let assembly = crate::engine::kinds_merge::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    let assembly = crate::engine::kinds_guides::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-0.7 wave 1: the OpNode family + smart objects + rasterize (the
    // history daggers are engine-built-ins, not assembled impls).
    let assembly = crate::engine::kinds_ops::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-1.0 door slice: the seven vector kinds + selection.from_path
    // (geometry-requiring arms answer typed NI until appname-vector
    // registers the VectorGeometry service at the assembly points).
    let assembly = crate::engine::kinds_vector::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-1.0 door slice 2: the five text kinds (TEXTVEC Doors 8–9;
    // text-requiring arms answer typed NI until appname-text registers
    // the TextLayout service at the composition point).
    let assembly = crate::engine::kinds_text::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-1.0 door slice 3, group 2: selection.save/load + the channel.*
    // family (doors C2–C3).
    let assembly = crate::engine::kinds_channels::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-1.0 door slice 3, group 3: the slice family + meta.set (O-IO
    // door asks 2–3).
    let assembly = crate::engine::kinds_slices_meta::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-1.0 door slice 3, group 5: doc.mode_convert (door C5).
    let assembly = crate::engine::kinds_mode::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k));
    // CP-1.0 door slice 3, group 7: channel.apply_image/calculations
    // (door C4, the addendum shape; §4.5-kernel modes need the
    // with_blend_kernels seam — assembly-injected).
    crate::engine::kinds_apply_image::impls()
        .into_iter()
        .fold(assembly, |a, k| a.with_kind(k))
}

pub struct FillKind;
pub struct EditCutKind;
pub struct EditPasteKind;
pub struct EditPasteIntoKind;
pub struct MaskAddKind;
pub struct MaskEditKind;
pub struct MaskDeleteKind;
pub struct MaskSetPropsKind;
pub struct MaskApplyKind;
pub struct MaskToSelectionKind;
pub struct DocAssignProfileKind;
pub struct DocConvertProfileKind;
pub struct DocDepthConvertKind;

// ---------------------------------------------------------------------------
// Shared stage-6 helpers.
// ---------------------------------------------------------------------------

/// Which wire error a resolving-but-not-raster target maps to: `fill`'s
/// contract stays inside the §40.1 baseline (`bad_target`); `edit.cut`
/// carries the §40.5 `target_kind_mismatch` addition.
enum NonRasterError {
    BadTarget,
    TargetKindMismatch,
}

/// Resolve `target.layerId` to a raster layer (stage 6, §40.5). Mask and
/// channel surfaces answer typed until their rows (stroke precedent).
fn resolve_raster_target(
    cx: &KindContext<'_>,
    non_raster: NonRasterError,
) -> Result<LayerId, EngineError> {
    let target = cx.envelope().target.as_ref();
    let (layer, mask, channel) = match target {
        Some(t) => (t.layer_id, t.mask_id, t.channel_id),
        None => (None, None, None),
    };
    if mask.is_some() || channel.is_some() {
        return Err(EngineError::bad_target(
            "mask/channel surface editing arrives with the mask.*/channel kinds rows (CP-0.5); \
             this kind targets target.layerId today",
        ));
    }
    let Some(layer) = layer else {
        return Err(EngineError::bad_target(
            "this kind requires target.layerId (§40.5)",
        ));
    };
    let id = LayerId::from_uuid(layer);
    let node = cx.document().layer(id).ok_or_else(|| {
        EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
    })?;
    if node.pixels().is_none() {
        return Err(match non_raster {
            NonRasterError::BadTarget => {
                EngineError::bad_target(format!("target.layerId {layer} is not a raster surface"))
            }
            NonRasterError::TargetKindMismatch => EngineError::new(
                ErrorCode::TargetKindMismatch,
                format!(
                    "target.layerId {layer} is not raster-editable (§40.5; §12.4 offers rasterize)"
                ),
            ),
        });
    }
    Ok(id)
}

/// Door-slice lock consolidation: EFFECTIVE locks (§11.1/§40.1 —
/// "strictest along the ancestry", ALL flags transitive) via the ONE
/// place, [`crate::graph::document::Document::effective_locks`]. The
/// former own-node read under-enforced the letter: a group's PIXELS lock
/// now binds its subtree.
fn locks_of(cx: &KindContext<'_>, layer: LayerId) -> LockFlags {
    cx.document().effective_locks(layer)
}

fn require_pixels_unlocked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    if locks_of(cx, layer).contains(LockFlags::PIXELS) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the target layer's pixels are locked (§11.1; strictest along the ancestry)",
        ));
    }
    Ok(())
}

fn param_f32(cx: &KindContext<'_>, name: &str, default: f32) -> f32 {
    cx.params()
        .get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .unwrap_or(default)
}

/// §22.1 color object → straight working-space linear RGB (the stroke
/// family's boundary, mirrored; a shared home comes with the §5 color wave).
fn decode_color_object(color: &serde_json::Map<String, Value>) -> Result<[f32; 3], EngineError> {
    if color.get("profileId").is_some() {
        return Err(EngineError::with_field(
            ErrorCode::ProfileSpaceMismatch,
            "color.profileId",
            "explicit color profiles cannot be verified before the CMM (§5.5); omit profileId",
        ));
    }
    let comps = color
        .get("comps")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::missing("color.comps"))?;
    if comps.len() != 4 || !comps.iter().all(|c| c.is_number()) {
        return Err(EngineError::range(
            "color.comps",
            "`color.comps` must be 4 numbers (§22.1)",
        ));
    }
    let c = |i: usize| comps[i].as_f64().unwrap_or(0.0) as f32;
    match color.get("model").and_then(Value::as_str) {
        Some("rgb") | Some("rgba") => Ok([
            srgb_decode(c(0).clamp(0.0, 1.0)),
            srgb_decode(c(1).clamp(0.0, 1.0)),
            srgb_decode(c(2).clamp(0.0, 1.0)),
        ]),
        Some("gray") | Some("grayA") => {
            let v = srgb_decode(c(0).clamp(0.0, 1.0));
            Ok([v, v, v])
        }
        Some(other) => Err(EngineError::new(
            ErrorCode::NotImplemented,
            format!("color model \"{other}\" needs the §5/§6 conversion pipeline"),
        )),
        None => Err(EngineError::missing("color.model")),
    }
}

/// Apply a per-pixel edit over `region` on a raster layer through the
/// recorder-coupled doors. For each touched tile: read the backdrop plane
/// and per-pixel selection coverage (inactive selection ⇒ 1.0), map pixels
/// inside the region, and install the plane back ONLY when something
/// changed. Returns the union rect of changed pixels (empty ⇒ the §40.1
/// empty-DeltaSet push).
fn edit_region(
    cx: &mut KindContext<'_>,
    layer: LayerId,
    region: IRect,
    f: &mut dyn FnMut([f32; 4], f32) -> [f32; 4],
) -> Result<IRect, EngineError> {
    let mut changed_rect = IRect::EMPTY;
    if region.is_empty() {
        return Ok(changed_rect);
    }
    let coords: Vec<(i32, i32)> = region.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        let span = rect.intersect(&region);
        if span.is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        let mut tile_changed = IRect::EMPTY;
        {
            let doc = cx.document();
            let buf = doc
                .layer(layer)
                .and_then(|n| n.pixels())
                .expect("resolved raster target");
            let sel = doc.selection();
            let sel_active = sel.active();
            let mut i = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    let dst = buf.read_pixel_f32(p);
                    let out = if span.contains(p) {
                        let cov = if sel_active {
                            sel.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let v = f(dst, cov);
                        if v != dst {
                            tile_changed = tile_changed.union(&IRect::new(x, y, x + 1, y + 1));
                        }
                        v
                    } else {
                        dst
                    };
                    plane[i] = out;
                    i += 1;
                }
            }
        }
        if !tile_changed.is_empty() {
            cx.install_tile_plane(layer, coord, &plane)?;
            changed_rect = changed_rect.union(&tile_changed);
        }
    }
    Ok(changed_rect)
}

// ---------------------------------------------------------------------------
// fill
// ---------------------------------------------------------------------------

impl KindImpl for FillKind {
    fn kind(&self) -> &'static str {
        "fill"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_raster_target(cx, NonRasterError::BadTarget)?;
        require_pixels_unlocked(cx, layer)?;
        // §22.6/§11.1: the TRANSPARENCY lock constrains; preserveTransparency
        // applies the SAME clamp even unlocked (dst α caps the fill).
        let constrain = locks_of(cx, layer).contains(LockFlags::TRANSPARENCY)
            || cx
                .params()
                .get("preserveTransparency")
                .and_then(Value::as_bool)
                == Some(true);

        if cx.params().get("seed").is_some() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "the flood-fill variant (seed) arrives with the selections/wand row (§22.6 \
                 shared-predicate rule)",
            ));
        }
        match cx.params().get("blend").and_then(Value::as_str) {
            None | Some("normal") => {}
            Some(other) => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!("blend mode {other:?} needs the §4.5 kernel table; normal is built in"),
                ))
            }
        }
        let content = cx
            .params()
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("content"))?;
        let color = match content {
            "color" => {
                let obj = cx
                    .params()
                    .get("color")
                    .and_then(Value::as_object)
                    .ok_or_else(|| EngineError::missing("color"))?; // §40.5 cross-field
                decode_color_object(obj)?
            }
            // Headless default swatches (lane-log divergence): foreground
            // black, background white — no tool state exists yet.
            "foreground" => [0.0, 0.0, 0.0],
            "background" => [1.0, 1.0, 1.0],
            other => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    format!(
                        "fill content \"{other}\" arrives with its own row (gradient §22.4, \
                         pattern §22.5, content_aware §21.4/§45.3)"
                    ),
                ))
            }
        };
        let opacity = param_f32(cx, "opacity", 1.0).clamp(0.0, 1.0);

        // Region: selection gates coverage (operative bounds ∩ canvas,
        // §22.6); no selection ⇒ whole layer = the canvas rect (module-doc
        // reading).
        let canvas = cx.document().geometry().canvas_rect();
        let sel = cx.document().selection();
        let selection_active = sel.active();
        let region = if selection_active {
            sel.bounds().intersect(&canvas)
        } else {
            canvas
        };

        // The pixel-exact re-tighten below (aud-replay F1) is only NEEDED when a
        // written pixel can store transparent while `changed` (a pre-quantization
        // f32 delta) still counted it — a sub-quantum premultiplied result. That
        // requires fractional coverage (an active/feathered selection), a
        // sub-quantum source alpha (partial `opacity`), or the transparency
        // `constrain` (scales src α by dst α). This reference rejects the flood
        // variant, so those three are the whole trigger. A no-selection,
        // full-opacity, unconstrained whole-layer fill writes α = 1.0 uniformly:
        // every touched pixel stores opaque and none turns transparent, so the
        // eager grow is already pixel-exact and the recompute is a provable no-op
        // (aud-perf r2 FINDING 1). Mirrors the production `FillKind` guard
        // (there also `|| flood.is_some()`), so the twins stay behavior-identical.
        let needs_pixel_exact = selection_active || constrain || opacity < 1.0;

        let changed = edit_region(cx, layer, region, &mut |dst, cov| {
            let mut src_a = opacity * cov;
            if constrain {
                src_a *= dst[3].clamp(0.0, 1.0); // source-atop (§22.6/§11.1)
            }
            if src_a <= 0.0 {
                return dst;
            }
            let ia = 1.0 - src_a;
            [
                color[0] * src_a + dst[0] * ia,
                color[1] * src_a + dst[1] * ia,
                color[2] * src_a + dst[2] * ia,
                src_a + dst[3] * ia,
            ]
        })?;
        if !changed.is_empty() {
            // §2.4 eager growth by the written bounds (paint-path rule).
            cx.grow_layer_bounds(layer, changed)?;
            // PIXEL-EXACT re-tighten — aud-replay F1. `changed` counts
            // pixels by their pre-quantization f32 delta, so a fill through
            // a FEATHERED selection over-grows by the sub-quantum fringe
            // (stores transparent), forking live-vs-reloaded `region_hash`
            // and killing replay-as-recovery undo. Matches the loader's
            // `recompute_bounds_pixel_exact`. Kept bitwise-identical to the
            // production `FillKind` (appname-tools) so the lift-equivalence
            // reference stays honest on feathered fills too.
            //
            // GUARD (aud-perf r2 FINDING 1): skip the redundant second pixel
            // scan only for the provably no-op case (see `needs_pixel_exact`
            // above); keep it for every sub-quantum-capable fill.
            if needs_pixel_exact {
                cx.recompute_layer_bounds_pixel_exact(layer)?;
            }
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// edit.cut
// ---------------------------------------------------------------------------

impl KindImpl for EditCutKind {
    fn kind(&self) -> &'static str {
        "edit.cut"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let layer = resolve_raster_target(cx, NonRasterError::TargetKindMismatch)?;
        require_pixels_unlocked(cx, layer)?;
        // Contract lock table: TRANSPARENCY BLOCKS — the §40.1 rule (a
        // command whose only possible effect is an alpha change fails
        // outright).
        if locks_of(cx, layer).contains(LockFlags::TRANSPARENCY) {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "TRANSPARENCY lock blocks edit.cut outright (§40.5: clearing decreases alpha)",
            ));
        }
        // Region: selection ∩ surface bounds; no selection = layer bounds
        // (the whole content clears, §40.5).
        let doc = cx.document();
        let content = doc
            .layer(layer)
            .and_then(|n| n.pixels())
            .expect("resolved raster")
            .bounds();
        let sel = doc.selection();
        let region = if sel.active() {
            sel.bounds().intersect(&content)
        } else {
            content
        };

        let changed = edit_region(cx, layer, region, &mut |dst, cov| {
            if cov <= 0.0 {
                return dst;
            }
            // §13.1: fractional coverage scales removed alpha; premultiplied
            // storage scales all components uniformly.
            let keep = 1.0 - cov;
            [dst[0] * keep, dst[1] * keep, dst[2] * keep, dst[3] * keep]
        })?;
        if !changed.is_empty() {
            // §2.4 shrink, PIXEL-EXACT (Q-1 determinism class): a cut through
            // a FEATHERED selection pulls the true content edge inside a
            // still-occupied tile (near-full coverage quantizes pixels to
            // exactly zero), and the tile-granular lazy shrink cannot tighten
            // below that tile — the live extent stays proud of the persisted
            // plane, `region_hash` (which folds bounds) forks live-vs-
            // reloaded, and the D-48 replay-as-recovery gate kills undo/redo
            // across save→reopen. The pixel-exact door matches the loader's
            // canonicalization.
            cx.recompute_layer_bounds_pixel_exact(layer)?;
        }
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// edit.paste / edit.paste_into
// ---------------------------------------------------------------------------

/// Stage-4-analog asset resolution (§40.5): `params.assetId` must name an
/// INGESTED raster asset (`Engine::add_raster_asset` / cabi asset ingress).
fn resolve_raster_asset(cx: &KindContext<'_>) -> Result<Uuid, EngineError> {
    let id = creation_uuid(cx, "assetId")?;
    match cx.asset_kind(id) {
        Some(AssetKind::RasterImage) => Ok(id),
        Some(AssetKind::Samples) => Err(EngineError::bad_target(format!(
            "asset {id} is a samples recording, not an ingested raster asset (§10.3/§9.6 rule 2)"
        ))),
        Some(AssetKind::FontFace) => Err(EngineError::bad_target(format!(
            "asset {id} is an embedded font face, not an ingested raster asset (§23.5)"
        ))),
        Some(AssetKind::MaskPlane) => Err(EngineError::bad_target(format!(
            "asset {id} is a mask coverage plane, not an ingested raster asset (D-ML-1)"
        ))),
        None => Err(EngineError::bad_target(format!(
            "params.assetId {id} does not resolve to an ingested raster asset (§9.2; \
             `appname asset add-image` / appname_asset_create)"
        ))),
    }
}

fn creation_uuid(cx: &KindContext<'_>, name: &str) -> Result<Uuid, EngineError> {
    cx.params()
        .get(name)
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| EngineError::missing(name))
}

/// `at` = [x, y] doc px of the pasted top-left; absent = centered on the
/// CANVAS (headless divergence, module doc — resolution is a pure function
/// of document geometry, so replay stays exact).
fn paste_origin(cx: &KindContext<'_>, w: u32, h: u32) -> Result<IVec2, EngineError> {
    match cx.params().get("at").and_then(Value::as_array) {
        Some(items) if items.len() == 2 && items.iter().all(Value::is_number) => {
            let x = items[0].as_f64().unwrap_or(0.0);
            let y = items[1].as_f64().unwrap_or(0.0);
            Ok(IVec2::new(x.round() as i32, y.round() as i32))
        }
        Some(_) => Err(EngineError::range("at", "`at` must be [x, y] numbers")),
        None => {
            let canvas = cx.document().geometry().canvas_rect();
            Ok(IVec2::new(
                canvas.min.x + (canvas.width() - w as i32) / 2,
                canvas.min.y + (canvas.height() - h as i32) / 2,
            ))
        }
    }
}

/// Decode + shared paste half: collision gate (§9.6 creation UUIDs are
/// fresh → `error.schema.duplicate_id`), parent ALL-lock gate (§40.5),
/// empty-node insertion through the recording door, decoded content
/// installed tile-wise through the recorder (so undo/redo and dirty rects
/// ride the standard §10.1 machinery), eager §2.4 bounds growth.
fn insert_pasted_layer(cx: &mut KindContext<'_>, asset: Uuid) -> Result<LayerId, EngineError> {
    let new_layer = creation_uuid(cx, "newLayerId")?;
    let id = LayerId::from_uuid(new_layer);
    if cx.document().layer(id).is_some() {
        return Err(EngineError::with_field(
            ErrorCode::DuplicateId,
            "newLayerId",
            format!("newLayerId {new_layer} already exists (§9.6: creation UUIDs are fresh)"),
        ));
    }
    let bytes = cx
        .asset_bytes(asset)
        .expect("resolved raster asset has bytes")
        .to_vec();
    let working = cx.document().color().working;
    let (decoded, w, h) =
        crate::io::png_io::decode_png_to_buffer(&bytes, working).map_err(|e| {
            EngineError::bad_target(format!("ingested raster asset failed decode: {e}"))
        })?;
    let origin = paste_origin(cx, w, h)?;

    // §40.5: ALL lock on the insertion parent blocks; the new node itself
    // has no locks. Headless divergence (module doc): the parent is the
    // ROOT (no active-layer state), top of the stack.
    let root = cx.document().root();
    if cx
        .document()
        .layer(root)
        .map(|n| n.locks().contains(LockFlags::ALL))
        .unwrap_or(false)
    {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the insertion parent is fully locked (§40.5: ALL blocks on the parent)",
        ));
    }
    let top = cx
        .document()
        .layer(root)
        .map(|n| n.children().len())
        .unwrap_or(0);
    let empty = Buffer::new(ChannelKind::F32, ColorModelKind::Rgba, working);
    let node = LayerNode::new_raster(id, "Pasted", empty);
    cx.insert_layer(root, top, node)?;

    // Install the decoded content through the recorder-coupled doors.
    let dest = IRect::new(origin.x, origin.y, origin.x + w as i32, origin.y + h as i32);
    if !dest.is_empty() {
        let coords: Vec<(i32, i32)> = dest.tiles(TILE_I32).collect();
        for (tx, ty) in coords {
            let coord = TileCoord::new(0, tx, ty);
            let rect = coord.pixel_rect();
            if rect.intersect(&dest).is_empty() {
                continue;
            }
            let mut plane = vec![[0.0f32; 4]; TILE * TILE];
            let mut i = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    if dest.contains(p) {
                        plane[i] =
                            decoded.read_pixel_f32(IVec2::new(p.x - origin.x, p.y - origin.y));
                    }
                    i += 1;
                }
            }
            cx.install_tile_plane(id, coord, &plane)?;
        }
        cx.grow_layer_bounds(id, dest)?;
        // PIXEL-EXACT re-tighten (O-QA W18 finding 2 fix): a pasted
        // raster can carry a TRANSPARENT BORDER — the shape a PSD
        // mask-rasterized-into-alpha layer lands as (io's
        // import_into_engine drives edit.paste). Without this the layer
        // bounds stay the full paste rect while the package loader
        // recomputes pixel-exact on reload, so region_hash (folds bounds)
        // forks live-vs-reloaded — the same D-48 class as the transform
        // bake (finding 1). The scan is over the just-installed tiles.
        cx.recompute_layer_bounds_pixel_exact(id)?;
    }
    Ok(id)
}

impl KindImpl for EditPasteKind {
    fn kind(&self) -> &'static str {
        "edit.paste"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let asset = resolve_raster_asset(cx)?;
        // §40.5: paste never clips to the selection (that is paste_into).
        let id = insert_pasted_layer(cx, asset)?;
        Ok(KindOutcome {
            created_ids: vec![id.as_uuid()],
        })
    }
}

impl KindImpl for EditPasteIntoKind {
    fn kind(&self) -> &'static str {
        "edit.paste_into"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Stage-4-analog asset resolution precedes the live-state selection
        // gate (pinned-order discipline; corpus overlay provenance).
        let asset = resolve_raster_asset(cx)?;
        let new_mask = creation_uuid(cx, "newMaskId")?;
        if cx.document().mask(MaskId::from_uuid(new_mask)).is_some() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "newMaskId",
                format!("newMaskId {new_mask} already exists (§9.6)"),
            ));
        }
        // §40.5: the selection is REQUIRED — consumed as the raster mask's
        // coverage; the selection itself stays unchanged.
        if !cx.document().selection().active() {
            return Err(EngineError::new(
                ErrorCode::SelectionRequired,
                "edit.paste_into requires an active selection (§40.5: it becomes the mask)",
            ));
        }
        let id = insert_pasted_layer(cx, asset)?;

        // Mask coverage = the selection mask, bit-exactly (Arc-shared tile
        // clones — zero pixel copies; the selection is NOT modified).
        let mut coverage = Selection::new_mask_buffer();
        {
            let sel = cx.document().selection();
            let tiles: Vec<_> = sel
                .mask()
                .tiles()
                .iter()
                .map(|(c, arc)| (c, Arc::clone(arc)))
                .collect();
            for (c, arc) in tiles {
                coverage.install_tile(c, arc);
            }
            coverage.set_bounds_recorded(sel.bounds());
        }
        let mask = MaskNode::new_raster(MaskId::from_uuid(new_mask), coverage);
        let mask_id = cx.attach_mask(id, mask)?;
        Ok(KindOutcome {
            created_ids: vec![id.as_uuid(), mask_id.as_uuid()],
        })
    }
}

// ---------------------------------------------------------------------------
// mask.* family (§40.3; contracts mask.add / mask.edit / mask.delete /
// mask.set_props / mask.apply / mask.to_selection on the raster mask
// doc-model). O-TOOLS' half of the split (vector rasterization, wand,
// feather/EDT refinements) plugs in through the same coverage seam.
// ---------------------------------------------------------------------------

use crate::engine::delta::MaskProps;
use crate::engine::mask_ops::{combine, SelectOp};

/// Stage-4 mask-target resolution: `target.layerId` (owner) +
/// `target.maskId`, both resolving, mask attached to that layer.
fn resolve_mask_target(cx: &KindContext<'_>) -> Result<(LayerId, MaskId), EngineError> {
    let target = cx.envelope().target.as_ref();
    let (layer, mask) = match target {
        Some(t) => (t.layer_id, t.mask_id),
        None => (None, None),
    };
    let (Some(layer), Some(mask)) = (layer, mask) else {
        return Err(EngineError::bad_target(
            "this kind requires target.layerId (owner) + target.maskId (§40.3)",
        ));
    };
    let (layer, mask) = (LayerId::from_uuid(layer), MaskId::from_uuid(mask));
    let node = cx.document().layer(layer).ok_or_else(|| {
        EngineError::bad_target(format!("target.layerId {layer} does not resolve"))
    })?;
    if cx.document().mask(mask).is_none() || !node.masks().contains(&mask) {
        return Err(EngineError::bad_target(format!(
            "target.maskId {mask} does not resolve on layer {layer}"
        )));
    }
    Ok((layer, mask))
}

fn require_not_all_locked(cx: &KindContext<'_>, layer: LayerId) -> Result<(), EngineError> {
    if locks_of(cx, layer).contains(LockFlags::ALL) {
        return Err(EngineError::new(
            ErrorCode::Locked,
            "the owning layer is fully locked (§11.1 ALL)",
        ));
    }
    Ok(())
}

/// Single-channel coverage edit over `region` on a coverage surface
/// (mask coverage, a §14.1 named channel, or the selection mask): reads
/// component 0, applies `f` (optionally gated through sel(p), §13.1),
/// installs changed tiles, and returns `(changed_rect, nonzero_rect)` —
/// the §13.1 tight-bounds inputs. `pub(crate)`: `channel.edit` (door
/// slice 3) shares the door (§40.4 "validates as mask.edit").
pub(crate) fn coverage_edit(
    cx: &mut KindContext<'_>,
    surface: Uuid,
    region: IRect,
    gate_selection: bool,
    f: &mut dyn FnMut(IVec2, f32) -> f32,
) -> Result<(IRect, IRect), EngineError> {
    let mut changed_rect = IRect::EMPTY;
    let mut nonzero_rect = IRect::EMPTY;
    if region.is_empty() {
        return Ok((changed_rect, nonzero_rect));
    }
    let coords: Vec<(i32, i32)> = region.tiles(TILE_I32).collect();
    for (tx, ty) in coords {
        let coord = TileCoord::new(0, tx, ty);
        let rect = coord.pixel_rect();
        let span = rect.intersect(&region);
        if span.is_empty() {
            continue;
        }
        let mut plane = vec![[0.0f32; 4]; TILE * TILE];
        let mut tile_changed = IRect::EMPTY;
        let mut tile_nonzero = IRect::EMPTY;
        {
            let doc = cx.document();
            let sel_surface = doc.selection_surface();
            let editing_selection = surface == sel_surface;
            let buf: &crate::buffer::Buffer = if editing_selection {
                doc.selection().mask()
            } else if let Some(m) = doc.mask(MaskId::from_uuid(surface)) {
                m.coverage()
            } else if let Some(c) = doc.channel(crate::graph::id::ChannelId::from_uuid(surface)) {
                // §14.1 named channels joined the door (slice 3).
                c.buffer()
            } else {
                return Err(EngineError::bad_target(format!(
                    "surface {surface} is not a coverage surface"
                )));
            };
            let sel = doc.selection();
            let gate = gate_selection && !editing_selection && sel.active();
            let mut i = 0;
            for y in rect.min.y..rect.max.y {
                for x in rect.min.x..rect.max.x {
                    let p = IVec2::new(x, y);
                    let v = buf.read_pixel_f32(p)[0];
                    let out = if span.contains(p) {
                        let mut nv = f(p, v).clamp(0.0, 1.0);
                        if gate {
                            // §13.1 single-consumer rule: surface writes
                            // blend through sel(p).
                            let s = sel.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0);
                            nv = v + (nv - v) * s;
                        }
                        if nv != v {
                            tile_changed = tile_changed.union(&IRect::new(x, y, x + 1, y + 1));
                        }
                        nv
                    } else {
                        v
                    };
                    if out > 0.0 {
                        tile_nonzero = tile_nonzero.union(&IRect::new(x, y, x + 1, y + 1));
                    }
                    plane[i] = [out, 0.0, 0.0, 0.0];
                    i += 1;
                }
            }
        }
        if !tile_changed.is_empty() {
            cx.install_surface_plane(surface, coord, &plane)?;
            changed_rect = changed_rect.union(&tile_changed);
        }
        nonzero_rect = nonzero_rect.union(&tile_nonzero);
    }
    Ok((changed_rect, nonzero_rect))
}

impl KindImpl for MaskAddKind {
    fn kind(&self) -> &'static str {
        "mask.add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // Target: layerId only — the mask does not exist yet (§40.3).
        let target = cx.envelope().target.as_ref();
        let Some(layer) = target.and_then(|t| t.layer_id) else {
            return Err(EngineError::bad_target(
                "mask.add requires target.layerId (owner)",
            ));
        };
        let layer = LayerId::from_uuid(layer);
        if cx.document().layer(layer).is_none() {
            return Err(EngineError::bad_target(format!(
                "target.layerId {layer} does not resolve"
            )));
        }
        require_not_all_locked(cx, layer)?;

        match cx.params().get("kind").and_then(Value::as_str) {
            Some("raster") => {}
            Some("vector") => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "vector masks arrive with the path arena (§20; O-VECTOR row)",
                ))
            }
            _ => return Err(EngineError::missing("kind")),
        }
        // §11.2 slot rule: at most one raster mask per layer.
        if !cx
            .document()
            .layer(layer)
            .expect("resolved")
            .masks()
            .is_empty()
        {
            return Err(EngineError::new(
                ErrorCode::TargetInUse,
                "the layer's raster-mask slot is already occupied (§11.2)",
            ));
        }
        let new_mask = creation_uuid(cx, "newMaskId")?;
        if cx.document().mask(MaskId::from_uuid(new_mask)).is_some() {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "newMaskId",
                format!("newMaskId {new_mask} already exists (§9.6)"),
            ));
        }
        let from = cx
            .params()
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or("reveal_all");
        let canvas = cx.document().geometry().canvas_rect();
        let mut coverage = Selection::new_mask_buffer();
        match from {
            "reveal_all" => coverage.fill_rect(canvas, [1.0, 0.0, 0.0, 0.0]),
            "hide_all" => {} // absent tiles read 0 (§11.2)
            "selection" | "selection_inverted" => {
                let sel = cx.document().selection();
                if !sel.active() {
                    return Err(EngineError::new(
                        ErrorCode::SelectionRequired,
                        "mask.add from:'selection' requires an active selection (§40.3)",
                    ));
                }
                if from == "selection" {
                    let tiles: Vec<_> = sel
                        .mask()
                        .tiles()
                        .iter()
                        .map(|(c, arc)| (c, Arc::clone(arc)))
                        .collect();
                    for (c, arc) in tiles {
                        coverage.install_tile(c, arc);
                    }
                    coverage.set_bounds_recorded(sel.bounds());
                } else {
                    // 1 − sel(p) over the canvas (outside-selection reveals).
                    for y in canvas.min.y..canvas.max.y {
                        for x in canvas.min.x..canvas.max.x {
                            let p = IVec2::new(x, y);
                            let v = 1.0 - sel.mask().read_pixel_f32(p)[0].clamp(0.0, 1.0);
                            if v > 0.0 {
                                coverage.write_pixel_f32(p, [v, 0.0, 0.0, 0.0]);
                            }
                        }
                    }
                }
            }
            "transparency" => {
                let node = cx.document().layer(layer).expect("resolved");
                let Some(px) = node.pixels() else {
                    return Err(EngineError::bad_target(
                        "mask.add from:'transparency' needs a raster owner (alpha source)",
                    ));
                };
                let b = px.bounds();
                for y in b.min.y..b.max.y {
                    for x in b.min.x..b.max.x {
                        let p = IVec2::new(x, y);
                        let a = px.read_pixel_f32(p)[3].clamp(0.0, 1.0);
                        if a > 0.0 {
                            coverage.write_pixel_f32(p, [a, 0.0, 0.0, 0.0]);
                        }
                    }
                }
            }
            "path" => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "from:'path' arrives with the path arena (§20; O-VECTOR row)",
                ))
            }
            other => {
                return Err(EngineError::range(
                    "from",
                    format!("unknown source \"{other}\""),
                ))
            }
        }
        coverage.recompute_bounds_pixel_exact();
        let mask = MaskNode::new_raster(MaskId::from_uuid(new_mask), coverage);
        let id = cx.attach_mask(layer, mask)?;
        Ok(KindOutcome {
            created_ids: vec![id.as_uuid()],
        })
    }
}

impl KindImpl for MaskEditKind {
    fn kind(&self) -> &'static str {
        "mask.edit"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (layer, mask) = resolve_mask_target(cx)?;
        // §40.3: PIXELS or ALL on the owner block mask-surface writes.
        require_pixels_unlocked(cx, layer)?;
        let edit = cx
            .params()
            .get("edit")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing("edit"))?;
        let op = edit
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("edit.op"))?;
        let canvas = cx.document().geometry().canvas_rect();
        let surface = mask.as_uuid();
        let nonzero =
            match op {
                "fill" => {
                    let v = edit
                        .get("value")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| EngineError::missing("edit.value"))?
                        .clamp(0.0, 1.0) as f32;
                    coverage_edit(cx, surface, canvas, true, &mut |_p, _old| v)?.1
                }
                "invert" => coverage_edit(cx, surface, canvas, true, &mut |_p, old| 1.0 - old)?.1,
                "levels" => {
                    let p = edit.get("params").and_then(Value::as_object);
                    let getf = |k: &str, d: f64| {
                        p.and_then(|m| m.get(k))
                            .and_then(Value::as_f64)
                            .unwrap_or(d) as f32
                    };
                    let (in_b, in_w) = (getf("inBlack", 0.0), getf("inWhite", 1.0));
                    let (out_b, out_w) = (getf("outBlack", 0.0), getf("outWhite", 1.0));
                    let gamma = getf("gamma", 1.0);
                    if in_w - in_b < 1.0 / 255.0 {
                        return Err(EngineError::range(
                            "edit.params.inWhite",
                            "inWhite - inBlack must be >= 1/255 (§40.3 cross-field)",
                        ));
                    }
                    coverage_edit(cx, surface, canvas, true, &mut |_p, old| {
                        let t = ((old - in_b) / (in_w - in_b)).clamp(0.0, 1.0);
                        out_b + t.powf(1.0 / gamma) * (out_w - out_b)
                    })?
                    .1
                }
                "gaussian_blur" => return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "mask gaussian_blur arrives with the O-TOOLS feather/refinement seam (§13.2)",
                )),
                other => {
                    return Err(EngineError::range(
                        "edit.op",
                        format!("unknown op \"{other}\""),
                    ))
                }
            };
        // Exact §13.1-style bounds from the in-pass nonzero tracking (the
        // §2.4 recompute door would trip its eager-growth guard on a
        // previously empty coverage buffer).
        cx.set_surface_bounds(surface, nonzero)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for MaskDeleteKind {
    fn kind(&self) -> &'static str {
        "mask.delete"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (layer, mask) = resolve_mask_target(cx)?;
        // §40.3: ALL blocks; PIXELS does not (structural removal).
        require_not_all_locked(cx, layer)?;
        cx.detach_mask(layer, mask)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for MaskSetPropsKind {
    fn kind(&self) -> &'static str {
        "mask.set_props"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (layer, mask) = resolve_mask_target(cx)?;
        require_not_all_locked(cx, layer)?;
        let cur = cx.document().mask(mask).expect("resolved").props();
        let g = |k: &str| cx.params().get(k);
        let after = MaskProps {
            enabled: g("enabled").and_then(Value::as_bool).unwrap_or(cur.enabled),
            inverted: g("inverted")
                .and_then(Value::as_bool)
                .unwrap_or(cur.inverted),
            density: g("density")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .unwrap_or(cur.density),
            feather: g("feather")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .unwrap_or(cur.feather),
            linked: g("linked").and_then(Value::as_bool).unwrap_or(cur.linked),
        };
        if !(0.0..=1.0).contains(&after.density) {
            return Err(EngineError::range("density", "density outside 0..=1"));
        }
        if !(0.0..=1000.0).contains(&after.feather) {
            return Err(EngineError::range("feather", "feather outside 0..=1000 px"));
        }
        if after != cur {
            cx.set_mask_props(mask, after)?;
        }
        Ok(KindOutcome::default())
    }
}

impl KindImpl for MaskApplyKind {
    fn kind(&self) -> &'static str {
        "mask.apply"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let (layer, mask) = resolve_mask_target(cx)?;
        // Owner MUST be raster (§12.4 rasterize path otherwise).
        if cx
            .document()
            .layer(layer)
            .expect("resolved")
            .pixels()
            .is_none()
        {
            return Err(EngineError::new(
                ErrorCode::TargetKindMismatch,
                "mask.apply needs a Raster owner — parametric layers rasterize first (§12.4)",
            ));
        }
        // §40.3 locks: PIXELS/ALL block; TRANSPARENCY blocks too (baking
        // rewrites alpha both directions).
        require_pixels_unlocked(cx, layer)?;
        if locks_of(cx, layer).contains(LockFlags::TRANSPARENCY) {
            return Err(EngineError::new(
                ErrorCode::Locked,
                "TRANSPARENCY lock blocks mask.apply (§40.3: baking rewrites alpha)",
            ));
        }
        let props = cx.document().mask(mask).expect("resolved").props();
        if props.feather > 0.0 {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "baking a feathered mask needs the O-TOOLS gaussian seam (§11.2 step 2)",
            ));
        }
        // Bake c(p) into alpha over the layer's content (§11.2 exact order:
        // sample → density → invert; disabled ⇒ c ≡ 1).
        let content = cx
            .document()
            .layer(layer)
            .expect("resolved")
            .pixels()
            .expect("raster")
            .bounds();
        if !content.is_empty() && props.enabled {
            let coords: Vec<(i32, i32)> = content.tiles(TILE_I32).collect();
            for (tx, ty) in coords {
                let coord = TileCoord::new(0, tx, ty);
                let rect = coord.pixel_rect();
                let span = rect.intersect(&content);
                if span.is_empty() {
                    continue;
                }
                let mut plane = vec![[0.0f32; 4]; TILE * TILE];
                let mut changed = false;
                {
                    let doc = cx.document();
                    let px = doc
                        .layer(layer)
                        .expect("resolved")
                        .pixels()
                        .expect("raster");
                    let cov = doc.mask(mask).expect("resolved").coverage();
                    let mut i = 0;
                    for y in rect.min.y..rect.max.y {
                        for x in rect.min.x..rect.max.x {
                            let p = IVec2::new(x, y);
                            let dst = px.read_pixel_f32(p);
                            let out = if span.contains(p) {
                                let mut v = cov.read_pixel_f32(p)[0].clamp(0.0, 1.0);
                                v = 1.0 - props.density * (1.0 - v); // §11.2 step 3
                                if props.inverted {
                                    v = 1.0 - v; // §11.2 step 4
                                }
                                if v < 1.0 {
                                    changed = true;
                                }
                                [dst[0] * v, dst[1] * v, dst[2] * v, dst[3] * v]
                            } else {
                                dst
                            };
                            plane[i] = out;
                            i += 1;
                        }
                    }
                }
                if changed {
                    cx.install_tile_plane(layer, coord, &plane)?;
                }
            }
            // PIXEL-EXACT shrink (Q-1 determinism class): the §11.2 bake
            // multiplies by mask coverage, which can zero content up to an
            // edge INSIDE a still-occupied tile (any mask edge off the tile
            // grid — a from-selection mask, a soft edge quantizing to zero).
            // The tile-granular shrink kept the old proud extent there, the
            // loader recomputes pixel-exact on reload, and the forked
            // `region_hash` killed D-48 replay-as-recovery undo across
            // save→reopen.
            cx.recompute_layer_bounds_pixel_exact(layer)?;
        }
        // Then remove the mask node (undo re-attaches it, tiles pinned).
        cx.detach_mask(layer, mask)?;
        Ok(KindOutcome::default())
    }
}

impl KindImpl for MaskToSelectionKind {
    fn kind(&self) -> &'static str {
        "mask.to_selection"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // §40.3/§40.4: selection changes are never lock-gated.
        let (_layer, mask) = resolve_mask_target(cx)?;
        let op = cx
            .params()
            .get("op")
            .and_then(Value::as_str)
            .map(|s| SelectOp::from_wire(s).ok_or_else(|| EngineError::range("op", "§13.2 op")))
            .transpose()?
            .unwrap_or(SelectOp::Replace);
        let doc = cx.document();
        let canvas = doc.geometry().canvas_rect();
        // Region must cover everything the combine can change: the old
        // selection plus the mask's coverage, clamped to the canvas.
        let cov_bounds = doc.mask(mask).expect("resolved").coverage().bounds();
        let region = doc
            .selection()
            .bounds()
            .union(&cov_bounds)
            .intersect(&canvas);
        let sel_surface = doc.selection_surface();
        let mask_uuid = mask.as_uuid();
        // Read the mask's coverage per pixel inside the closure via a
        // pre-borrowed snapshot: coverage tiles are COW Arcs, cheap clone.
        let coverage: Vec<(
            crate::tile::map::TileCoord,
            Arc<crate::tile::storage::TileStorage>,
        )> = doc
            .mask(mask)
            .expect("resolved")
            .coverage()
            .tiles()
            .iter()
            .map(|(c, a)| (c, Arc::clone(a)))
            .collect();
        let mut snap = Selection::new_mask_buffer();
        for (c, a) in coverage {
            snap.install_tile(c, a);
        }
        let _ = mask_uuid;
        let (_changed, nonzero) = coverage_edit(cx, sel_surface, region, false, &mut |p, old| {
            combine(op, old, snap.read_pixel_f32(p)[0].clamp(0.0, 1.0))
        })?;
        cx.set_surface_bounds(sel_surface, nonzero)?;
        cx.sync_selection_bounds();
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// doc.* color family (§40.2; contracts doc.assign_profile /
// doc.convert_profile / doc.depth_convert consuming appname-color per the
// O-COLOR CORE-SIDE NEEDS list — each item re-checked as wired; see
// engine::color_glue for the value-capture reading).
// ---------------------------------------------------------------------------

use crate::engine::color_glue;
use crate::graph::document::DocProfileSource;
use crate::graph::id::ProfileId;

/// Resolve a `profile` param uuid against the document store (stage 4:
/// unresolvable → bad_target) and run the §5.5 space + §5.2 class gates.
fn resolve_profile_param(
    cx: &mut KindContext<'_>,
    id: Uuid,
) -> Result<(ProfileId, appname_color::ProfileUid), EngineError> {
    let pid = ProfileId(id);
    let Some(entry) = cx.document().profile(pid).cloned() else {
        return Err(EngineError::bad_target(format!(
            "profile {id} does not resolve in the document store (§5.1; ingest via              Engine::add_icc_profile)"
        )));
    };
    // §5.5: destination space must match the document mode (v1: Rgba).
    if let DocProfileSource::Builtin(space) = &entry.source {
        if space.is_gray() {
            return Err(EngineError::new(
                ErrorCode::ProfileSpaceMismatch,
                "gray working space does not match the Rgba document mode (§5.5)",
            ));
        }
    }
    // Embedded ICC re-validates through the CMM on resolution (§5.2 —
    // class/space run at ingestion AND here, cheap on the dedup hit).
    let uid = color_glue::resolve_uid(cx.cms(), &entry)?;
    Ok((pid, uid))
}

impl KindImpl for DocAssignProfileKind {
    fn kind(&self) -> &'static str {
        "doc.assign_profile"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        // §5.3 assign: reinterprets bytes, never converts — zero tile
        // deltas; target block ignored (doc.* family rule); never lock-gated.
        let param = cx
            .params()
            .get("profile")
            .ok_or_else(|| EngineError::missing("profile"))?;
        let pid = match param {
            Value::Null => color_glue::untagged_profile_id(),
            Value::String(s) => {
                let id = Uuid::parse_str(s).map_err(|_| {
                    EngineError::range("profile", "`profile` must be a uuid or null")
                })?;
                resolve_profile_param(cx, id)?.0
            }
            _ => {
                return Err(EngineError::range(
                    "profile",
                    "`profile` must be a uuid or null",
                ))
            }
        };
        cx.set_working_profile(pid);
        Ok(KindOutcome::default())
    }
}

impl KindImpl for DocConvertProfileKind {
    fn kind(&self) -> &'static str {
        "doc.convert_profile"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        if cx.params().get("flatten").and_then(Value::as_bool) == Some(true) {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "flatten-on-convert needs the compositor flatten row (§40.2)",
            ));
        }
        let id = cx
            .params()
            .get("profile")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| EngineError::missing("profile"))?;
        let (pid, dst_uid) = resolve_profile_param(cx, id)?;
        let intent = cx
            .params()
            .get("intent")
            .and_then(Value::as_str)
            .map(|s| {
                appname_color::Intent::from_wire(s)
                    .ok_or_else(|| EngineError::range("intent", "§5.3 intent literal"))
            })
            .transpose()?
            .unwrap_or(appname_color::Intent::RelativeColorimetric);
        let bpc = cx
            .params()
            .get("blackPointCompensation")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        // `dither` rides to the §6.3 quantization hook; interim rounding is
        // NoDither (BN64 pends OQ-COLOR-3 — divergence noted, lane log).
        let _dither = cx
            .params()
            .get("dither")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let working = cx.document().color().working;
        let src_entry = cx.document().profile(working).cloned().ok_or_else(|| {
            EngineError::bad_target("the working profile does not resolve in the store (§5.1)")
        })?;
        let src_uid = color_glue::resolve_uid(cx.cms(), &src_entry)?;
        let fmt = appname_color::PixelFormat {
            model: appname_color::ColorModel::Rgba,
            depth: appname_color::Depth::F32,
        };
        let key = appname_color::TransformKey {
            src: src_uid,
            dst: dst_uid,
            proof: None,
            intent,
            bpc,
            fmt_in: fmt,
            fmt_out: fmt,
        };
        let transform = cx
            .cms()
            .transform(key)
            .map_err(color_glue::map_color_error)?;

        if !transform.is_identity() {
            // Per-tile straight-alpha f32 marshalling (O-COLOR need 4: the
            // marshaller is core's tile layer): unpremul → transform →
            // re-premul; a == 0 stays exactly transparent (§6.4).
            let layers: Vec<LayerId> = cx
                .document()
                .z_order_walk()
                .into_iter()
                .filter(|id| {
                    cx.document()
                        .layer(*id)
                        .map(|n| n.pixels().is_some())
                        .unwrap_or(false)
                })
                .collect();
            for layer in layers {
                let content = cx
                    .document()
                    .layer(layer)
                    .expect("walked")
                    .pixels()
                    .expect("raster")
                    .bounds();
                if content.is_empty() {
                    continue;
                }
                let coords: Vec<(i32, i32)> = content.tiles(TILE_I32).collect();
                for (tx, ty) in coords {
                    let coord = TileCoord::new(0, tx, ty);
                    let rect = coord.pixel_rect();
                    if rect.intersect(&content).is_empty() {
                        continue;
                    }
                    let mut straight = vec![0.0f32; TILE * TILE * 4];
                    let mut alphas = vec![0.0f32; TILE * TILE];
                    {
                        let buf = cx
                            .document()
                            .layer(layer)
                            .expect("walked")
                            .pixels()
                            .expect("raster");
                        let mut i = 0;
                        for y in rect.min.y..rect.max.y {
                            for x in rect.min.x..rect.max.x {
                                let v = buf.read_pixel_f32(IVec2::new(x, y));
                                let a = v[3];
                                alphas[i] = a;
                                let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
                                straight[i * 4] = v[0] * inv;
                                straight[i * 4 + 1] = v[1] * inv;
                                straight[i * 4 + 2] = v[2] * inv;
                                straight[i * 4 + 3] = a;
                                i += 1;
                            }
                        }
                    }
                    let mut out = vec![0.0f32; TILE * TILE * 4];
                    transform
                        .apply_f32(&straight, &mut out)
                        .map_err(color_glue::map_color_error)?;
                    let mut plane = vec![[0.0f32; 4]; TILE * TILE];
                    for (i, px) in plane.iter_mut().enumerate() {
                        let a = alphas[i];
                        if a > 0.0 {
                            *px = [
                                (out[i * 4] * a).clamp(0.0, a),
                                (out[i * 4 + 1] * a).clamp(0.0, a),
                                (out[i * 4 + 2] * a).clamp(0.0, a),
                                a,
                            ];
                        }
                    }
                    cx.install_tile_plane(layer, coord, &plane)?;
                }
            }
        }
        // Retag (the SetProp half of the §40.2 inverse).
        cx.set_working_profile(pid);
        Ok(KindOutcome::default())
    }
}

impl KindImpl for DocDepthConvertKind {
    fn kind(&self) -> &'static str {
        "doc.depth_convert"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let to_wire = cx
            .params()
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("to"))?;
        let to = match to_wire {
            "u8" => crate::tile::channel::ChannelKind::U8,
            "u16" => crate::tile::channel::ChannelKind::U16,
            "f16" => {
                return Err(color_glue::map_color_error(
                    appname_color::ColorError::DepthPaused(appname_color::Depth::F16),
                ))
            }
            "f32" => crate::tile::channel::ChannelKind::F32,
            other => {
                return Err(EngineError::range(
                    "to",
                    format!("unknown depth \"{other}\""),
                ))
            }
        };
        if !to.is_storage_backed() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!("depth {to_wire} is not storage-backed in this build (feature-gated)"),
            ));
        }
        // `dither` accepted; interim quantization is round-to-nearest
        // (NoDither — BN64 pends OQ-COLOR-3; divergence noted).
        let _dither = cx
            .params()
            .get("dither")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if cx.document().color().depth == to {
            // §40.1: converting to the current depth is a no-op push.
            return Ok(KindOutcome::default());
        }
        cx.checkpoint_convert_depth(to)?;
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use serde_json::json;

    fn assembly() -> EngineAssembly {
        // Tests keep core's FillKind registered explicitly: production
        // ownership moved tools-side at the brokered swap, but this impl
        // remains the lift's equivalence reference and its battery keeps
        // running against it (in-crate only door).
        register(EngineAssembly::new()).with_kind(Arc::new(FillKind))
    }

    fn doc_new(engine: &mut Engine, background: &str) -> LayerId {
        let text = json!({
            "schema": "appname.command.v1",
            "id": "e1000000-0000-4000-8000-000000000001",
            "kind": "doc.new",
            "params": {"width": 128, "height": 128, "background": background},
        })
        .to_string();
        engine.apply_json(&text).unwrap();
        engine.document().unwrap().z_order_walk()[0]
    }

    fn ready(background: &str) -> (Engine, LayerId) {
        let mut e = assembly().build();
        let layer = doc_new(&mut e, background);
        (e, layer)
    }

    fn env(n: u32, kind: &str, layer: Option<LayerId>, params: Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("e1000000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if let Some(l) = layer {
            v["target"] = json!({"layerId": l.as_uuid().to_string()});
        }
        v.to_string()
    }

    fn fill_color(n: u32, layer: LayerId, comps: [f64; 3], extra: Value) -> String {
        let mut params = json!({
            "content": "color",
            "color": {"model": "rgb", "comps": [comps[0], comps[1], comps[2], 0.0]},
        });
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                params[k] = v.clone();
            }
        }
        env(n, "fill", Some(layer), params)
    }

    fn px(e: &Engine, layer: LayerId, x: i32, y: i32) -> [f32; 4] {
        e.document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    /// Set a uniform-coverage selection through the test door (selection.set
    /// is a later row; tests establish the live state directly).
    fn select_rect(e: &mut Engine, rect: IRect, coverage: f32) {
        let doc = e.doc_mut_for_test();
        let sel = doc.selection_mut();
        *sel = Selection::new(); // replace semantics: drop stale coverage
        for y in rect.min.y..rect.max.y {
            for x in rect.min.x..rect.max.x {
                sel.mask_mut()
                    .write_pixel_f32(IVec2::new(x, y), [coverage, 0.0, 0.0, 0.0]);
            }
        }
        sel.set_bounds(rect);
    }

    fn lock(e: &mut Engine, layer: LayerId, flags: LockFlags) {
        e.doc_mut_for_test().layer_mut(layer).unwrap().locks = flags;
    }

    fn add_group(e: &mut Engine) -> LayerId {
        let id = LayerId::from_uuid(Uuid::new_v4());
        let doc = e.doc_mut_for_test();
        let root = doc.root();
        let n = doc.layer(root).map(|r| r.children().len()).unwrap();
        doc.insert_child(root, n, LayerNode::new_group(id, "G", false))
            .unwrap();
        id
    }

    // ------------------------------------------------------------------
    // fill
    // ------------------------------------------------------------------

    #[test]
    fn fill_color_whole_layer_undoes_redoes_replays() {
        let (mut e, layer) = ready("white");
        let before = e.state_hash();
        let receipt = e
            .apply_json(&fill_color(1, layer, [1.0, 0.0, 0.0], json!({})))
            .unwrap();
        assert!(!receipt.dirty.is_empty());
        let after = e.state_hash();
        assert_ne!(before, after);
        // Full-opacity red covers the whole canvas.
        assert_eq!(px(&e, layer, 0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&e, layer, 127, 127), [1.0, 0.0, 0.0, 1.0]);

        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            before,
            "COW-delta undo restores byte identity"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "replay through the same assembly"
        );
    }

    #[test]
    fn whole_layer_fill_eager_bounds_already_pixel_exact() {
        // aud-perf r2 FINDING 1 — SOUNDNESS PIN for the perf guard.
        //
        // For a NO-selection / NO-flood whole-layer fill the guarded skip of
        // `recompute_layer_bounds_pixel_exact` must be a provable NO-OP: the
        // eager (`grow_layer_bounds`) extent has to ALREADY equal what the
        // pixel-exact rescan would produce. This proves the skip is sound
        // (it drops zero real bounds tightening), not merely faster. If this
        // ever fails, the guard is unsound and must be reverted — a
        // whole-layer fill would then be leaving a stale extent that forks
        // live-vs-reloaded `region_hash` (the exact D-48 failure the recompute
        // was added to fix). It fails closed, on the value, not on timing.
        for bg in ["white", "transparent"] {
            let (mut e, layer) = ready(bg);
            // Whole-canvas, full-opacity fill with NO selection active → the
            // guarded fast path (uniform coverage 1.0, recompute skipped).
            e.apply_json(&fill_color(1, layer, [0.25, 0.5, 0.75], json!({})))
                .unwrap();
            assert!(
                !e.document().unwrap().selection().active(),
                "[{bg}] precondition: no selection ⇒ exercises the SKIPPED branch"
            );

            let buf = e
                .document()
                .unwrap()
                .layer(layer)
                .unwrap()
                .pixels()
                .unwrap();
            let eager = buf.bounds();

            // Exactly what the skipped recompute WOULD have produced — the
            // loader-agreeing pixel-exact scan, run here on a clone.
            let mut probe = buf.clone();
            probe.recompute_bounds_pixel_exact();

            assert_eq!(
                eager,
                probe.bounds(),
                "[{bg}] eager bounds must ALREADY equal the pixel-exact recompute — \
                 the guarded skip is a no-op, not a shortcut that loses tightening"
            );
            // Both are the full canvas: a whole-layer fill covers everything.
            assert_eq!(
                eager,
                IRect::new(0, 0, 128, 128),
                "[{bg}] whole-canvas fill ⇒ bounds == canvas"
            );
        }
    }

    #[test]
    fn subquantum_opacity_whole_layer_fill_keeps_bounds_pixel_exact() {
        // aud-perf r2 FINDING 1 — ADVERSARIAL regression (self-review R1).
        //
        // The perf guard must NOT over-skip. A whole-canvas fill with NO
        // selection but SUB-QUANTUM opacity stores all-transparent on a
        // premultiplied U8 layer, yet `changed` (a pre-quantization f32 delta)
        // counts every pixel — so the eager grow OVERSTATES to the full canvas.
        // A naive `!selection.active()` skip would leave those bloated bounds,
        // forking the loader's pixel-exact (EMPTY) bounds and breaking D-48
        // replay-as-recovery. The guard keys additionally on `opacity < 1.0`,
        // so it KEEPS the recompute here. Assert the applied fill already left
        // bounds pixel-exact (i.e. the recompute ran).
        let (mut e, layer) = ready("transparent");
        e.apply_json(&fill_color(
            1,
            layer,
            [0.25, 0.5, 0.75],
            json!({"opacity": 0.001}),
        ))
        .unwrap();
        let buf = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap();
        let live = buf.bounds();
        let mut probe = buf.clone();
        probe.recompute_bounds_pixel_exact();
        assert_eq!(
            live,
            probe.bounds(),
            "guard must recompute for sub-quantum opacity: live bounds must \
             already be pixel-exact, not the overstated eager extent"
        );
        // On a premultiplied U8 layer a 0.001-opacity fill stores nothing.
        assert!(
            live.is_empty(),
            "sub-quantum whole-canvas fill stores transparent ⇒ empty bounds, got {live:?}"
        );
    }

    #[test]
    fn fill_gates_on_selection_coverage() {
        let (mut e, layer) = ready("white");
        select_rect(&mut e, IRect::new(10, 10, 20, 20), 0.3);
        e.apply_json(&fill_color(2, layer, [0.0, 0.0, 0.0], json!({})))
            .unwrap();
        // 30 % black over white ⇒ 0.7 gray inside the selection.
        let inside = px(&e, layer, 12, 12);
        assert!(
            (inside[0] - 0.7).abs() < 0.01,
            "30% coverage blend, got {inside:?}"
        );
        // Outside the selection: untouched white.
        assert_eq!(px(&e, layer, 50, 50), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn fill_opacity_scales_deposit() {
        let (mut e, layer) = ready("white");
        e.apply_json(&fill_color(
            3,
            layer,
            [0.0, 0.0, 0.0],
            json!({"opacity": 0.5}),
        ))
        .unwrap();
        let p = px(&e, layer, 64, 64);
        assert!(
            (p[0] - 0.5).abs() < 0.01,
            "half-opacity black over white, got {p:?}"
        );
        assert!((p[3] - 1.0).abs() < 0.01);
    }

    #[test]
    fn fill_preserve_transparency_caps_by_dst_alpha() {
        // Transparent layer: the clamp deposits nothing at all.
        let (mut e, layer) = ready("transparent");
        let before = e.state_hash();
        let receipt = e
            .apply_json(&fill_color(
                4,
                layer,
                [1.0, 0.0, 0.0],
                json!({"preserveTransparency": true}),
            ))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "no coverage → §40.1 empty push");
        assert_eq!(e.state_hash(), before);
        assert_eq!(e.journal().len(), 2, "the empty DeltaSet still journals");
    }

    #[test]
    fn fill_transparency_lock_applies_the_same_clamp() {
        let (mut e, layer) = ready("transparent");
        lock(&mut e, layer, LockFlags::TRANSPARENCY);
        let before = e.state_hash();
        let receipt = e
            .apply_json(&fill_color(5, layer, [1.0, 0.0, 0.0], json!({})))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "§11.1 constrain, not a block");
        assert_eq!(e.state_hash(), before);

        // On full coverage the constrained fill deposits normally.
        let (mut e2, layer2) = ready("white");
        lock(&mut e2, layer2, LockFlags::TRANSPARENCY);
        e2.apply_json(&fill_color(6, layer2, [1.0, 0.0, 0.0], json!({})))
            .unwrap();
        assert_eq!(px(&e2, layer2, 5, 5), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn fill_headless_swatch_defaults() {
        let (mut e, layer) = ready("white");
        e.apply_json(&env(
            7,
            "fill",
            Some(layer),
            json!({"content": "foreground"}),
        ))
        .unwrap();
        assert_eq!(
            px(&e, layer, 1, 1),
            [0.0, 0.0, 0.0, 1.0],
            "foreground = black"
        );
        e.apply_json(&env(
            8,
            "fill",
            Some(layer),
            json!({"content": "background"}),
        ))
        .unwrap();
        assert_eq!(
            px(&e, layer, 1, 1),
            [1.0, 1.0, 1.0, 1.0],
            "background = white"
        );
    }

    #[test]
    fn fill_adversarial_rejects_are_typed() {
        let (mut e, layer) = ready("white");

        // content = color without the payload (§40.5 cross-field).
        let err = e
            .apply_json(&env(10, "fill", Some(layer), json!({"content": "color"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);

        // Unimplemented contents answer typed (payloads included where the
        // cross-field demands one).
        for (n, params) in [
            (
                11u32,
                json!({"content": "gradient", "gradient": {"kind": "linear", "stops": [
                    {"pos": 0.0, "color": {"model": "rgb", "comps": [0, 0, 0, 0]}},
                    {"pos": 1.0, "color": {"model": "rgb", "comps": [1, 1, 1, 0]}}]}}),
            ),
            (
                12,
                json!({"content": "pattern", "patternId": Uuid::new_v4().to_string()}),
            ),
            (13, json!({"content": "content_aware"})),
            (14, json!({"content": "foreground", "seed": [5, 5]})),
            (15, json!({"content": "foreground", "blend": "multiply"})),
        ] {
            let err = e
                .apply_json(&env(n, "fill", Some(layer), params))
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotImplemented, "case {n}");
        }

        // §22.1 color boundary: unimplemented model / pre-CMM profile.
        let err = e
            .apply_json(&env(
                16,
                "fill",
                Some(layer),
                json!({"content": "color", "color": {"model": "lab", "comps": [0.5, 0.5, 0.5, 0.0]}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        let err = e
            .apply_json(&env(
                17,
                "fill",
                Some(layer),
                json!({"content": "color", "color": {"model": "rgb", "comps": [1, 0, 0, 0],
                       "profileId": "eeeeeeee-0000-4000-8000-000000000001"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);

        // Target rules: unresolved → bad_target; group → bad_target (fill's
        // contract carries no target_kind_mismatch); mask surface → typed.
        let ghost = LayerId::from_uuid(Uuid::new_v4());
        let err = e
            .apply_json(&env(
                18,
                "fill",
                Some(ghost),
                json!({"content": "foreground"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        let group = add_group(&mut e);
        let err = e
            .apply_json(&env(
                19,
                "fill",
                Some(group),
                json!({"content": "foreground"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        let mut v: Value = serde_json::from_str(&env(
            20,
            "fill",
            Some(layer),
            json!({"content": "foreground"}),
        ))
        .unwrap();
        v["target"] = json!({"maskId": Uuid::new_v4().to_string()});
        let err = e.apply_json(&v.to_string()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("mask"), "milestone-naming message");

        // PIXELS lock blocks; ALL is the union (§2.6).
        lock(&mut e, layer, LockFlags::PIXELS);
        let err = e
            .apply_json(&env(
                21,
                "fill",
                Some(layer),
                json!({"content": "foreground"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        lock(&mut e, layer, LockFlags::ALL);
        let err = e
            .apply_json(&env(
                22,
                "fill",
                Some(layer),
                json!({"content": "foreground"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        lock(&mut e, layer, LockFlags::empty());

        // Every reject above journaled nothing beyond doc.new.
        assert_eq!(e.journal().len(), 1);
    }

    #[test]
    fn fill_determinism_same_inputs_same_state() {
        let build = || {
            let (mut e, layer) = ready("white");
            e.apply_json(&fill_color(
                30,
                layer,
                [0.2, 0.4, 0.8],
                json!({"opacity": 0.7}),
            ))
            .unwrap();
            e.state_hash()
        };
        assert_eq!(build(), build());
    }

    // ------------------------------------------------------------------
    // edit.cut
    // ------------------------------------------------------------------

    #[test]
    fn cut_whole_content_clears_and_round_trips() {
        let (mut e, layer) = ready("white");
        let before = e.state_hash();
        let receipt = e
            .apply_json(&env(40, "edit.cut", Some(layer), json!({})))
            .unwrap();
        assert!(!receipt.dirty.is_empty());
        assert_eq!(
            px(&e, layer, 64, 64),
            [0.0, 0.0, 0.0, 0.0],
            "cut to transparent"
        );
        let bounds = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert!(
            bounds.is_empty(),
            "no selection = whole content clears; bounds re-tighten"
        );

        let after = e.state_hash();
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after);
    }

    /// The contract's named unit test: a 30 %-selected pixel keeps 70 % of
    /// its alpha (§13.1 fractional coverage scales removed alpha).
    #[test]
    fn unit_cmd_edit_cut_fractional() {
        let (mut e, layer) = ready("white");
        select_rect(&mut e, IRect::new(10, 10, 11, 11), 0.3);
        e.apply_json(&env(41, "edit.cut", Some(layer), json!({})))
            .unwrap();
        let p = px(&e, layer, 10, 10);
        assert!((p[3] - 0.7).abs() < 0.01, "70% alpha kept, got {p:?}");
        // Unselected neighbors keep full alpha.
        assert_eq!(px(&e, layer, 11, 10), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn cut_on_empty_content_is_the_empty_delta_push() {
        let (mut e, layer) = ready("transparent");
        let before = e.state_hash();
        let receipt = e
            .apply_json(&env(42, "edit.cut", Some(layer), json!({})))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "empty content → §40.1 empty push");
        assert_eq!(e.state_hash(), before);
        assert_eq!(e.journal().len(), 2);
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
    }

    #[test]
    fn cut_locks_and_target_rules() {
        let (mut e, layer) = ready("white");
        lock(&mut e, layer, LockFlags::TRANSPARENCY);
        let err = e
            .apply_json(&env(43, "edit.cut", Some(layer), json!({})))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::Locked,
            "TRANSPARENCY hard-blocks cut (§40.5)"
        );
        lock(&mut e, layer, LockFlags::PIXELS);
        let err = e
            .apply_json(&env(44, "edit.cut", Some(layer), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        lock(&mut e, layer, LockFlags::empty());

        // Resolving non-raster target → the §40.5 target_kind_mismatch
        // addition; unresolved stays bad_target.
        let group = add_group(&mut e);
        let err = e
            .apply_json(&env(45, "edit.cut", Some(group), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
        let ghost = LayerId::from_uuid(Uuid::new_v4());
        let err = e
            .apply_json(&env(46, "edit.cut", Some(ghost), json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        let err = e
            .apply_json(&env(47, "edit.cut", None, json!({})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert_eq!(e.journal().len(), 1, "rejects journal nothing");
    }

    // ------------------------------------------------------------------
    // edit.paste / edit.paste_into
    // ------------------------------------------------------------------

    fn tiny_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(rgba).unwrap();
        }
        out
    }

    /// 2×2 opaque red.
    fn red_png() -> Vec<u8> {
        tiny_png(2, 2, &[255, 0, 0, 255].repeat(4))
    }

    fn children(e: &Engine) -> usize {
        let d = e.document().unwrap();
        d.children_of(d.root()).unwrap().len()
    }

    /// O-QA W18 finding 2 repro/regression: a pasted raster with a
    /// TRANSPARENT BORDER (the shape a PSD mask-rasterized-into-alpha
    /// layer lands as) must leave PIXEL-EXACT bounds, so live and
    /// reloaded region_hash agree (edit.paste is the core landing path
    /// io's import_into_engine drives; NOT an io-parser issue).
    #[test]
    fn paste_transparent_bordered_raster_leaves_canonical_bounds() {
        // 6×6 PNG: transparent border, opaque red 2×2 center at (2,2).
        let mut rgba = vec![0u8; 6 * 6 * 4];
        for y in 2..4 {
            for x in 2..4 {
                let i = (y * 6 + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let png = tiny_png(6, 6, &rgba);
        let (mut e, _bg) = ready("white");
        let asset = e.add_raster_asset(png).unwrap();
        let nl = Uuid::new_v4();
        e.apply_json(&env(
            80,
            "edit.paste",
            None,
            json!({"assetId": asset.to_string(), "newLayerId": nl.to_string(), "at": [10, 10]}),
        ))
        .unwrap();
        let layer = LayerId::from_uuid(nl);
        let live_bounds = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        // The opaque content is the 2×2 at (2,2) of the PNG, pasted at
        // (10,10) → pixel-exact [12,12,14,14], NOT the full [10,10,16,16].
        assert_eq!(
            live_bounds,
            IRect::new(12, 12, 14, 14),
            "paste leaves pixel-exact bounds (transparent border excluded)"
        );
        // Save + reload → bounds and persistent hash agree.
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&assembly(), &bytes).unwrap();
        let re_bounds = loaded
            .engine
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .bounds();
        assert_eq!(
            live_bounds, re_bounds,
            "live bounds == reloaded pixel-exact"
        );
        assert_eq!(
            e.state_hash_persistent(),
            loaded.engine.state_hash_persistent(),
            "persistent hash agrees across save/reload (W18 finding 2)"
        );
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash_persistent(), e.state_hash_persistent());
    }

    #[test]
    fn paste_creates_layer_undoes_redoes_replays() {
        let (mut e, _bg) = ready("white");
        let asset = e.add_raster_asset(red_png()).unwrap();
        let nl = Uuid::new_v4();
        let before = e.state_hash();
        let receipt = e
            .apply_json(&env(
                50,
                "edit.paste",
                None,
                json!({"assetId": asset.to_string(), "newLayerId": nl.to_string(), "at": [4, 6]}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![nl]);
        assert!(!receipt.dirty.is_empty(), "pasted tiles report dirty");
        assert_eq!(children(&e), 2, "new layer above the background");
        let pasted = LayerId::from_uuid(nl);
        let d = e.document().unwrap();
        assert_eq!(d.layer(pasted).unwrap().name(), "Pasted");
        assert_eq!(px(&e, pasted, 4, 6), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&e, pasted, 5, 7), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            px(&e, pasted, 3, 6),
            [0.0, 0.0, 0.0, 0.0],
            "outside the placement"
        );
        let after = e.state_hash();

        e.undo().unwrap();
        assert_eq!(children(&e), 1, "undo removes the pasted node");
        assert!(e.document().unwrap().layer(pasted).is_none());
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        assert_eq!(children(&e), 2, "redo re-inserts the SAME node");
        assert_eq!(e.state_hash(), after);
        assert_eq!(px(&e, pasted, 4, 6), [1.0, 0.0, 0.0, 1.0]);

        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            after,
            "journaled import replays byte-identically"
        );
    }

    #[test]
    fn paste_centers_on_the_canvas_when_at_is_absent() {
        let (mut e, _bg) = ready("white");
        let asset = e.add_raster_asset(red_png()).unwrap();
        let nl = Uuid::new_v4();
        e.apply_json(&env(
            51,
            "edit.paste",
            None,
            json!({"assetId": asset.to_string(), "newLayerId": nl.to_string()}),
        ))
        .unwrap();
        // (128 - 2) / 2 = 63: the 2×2 content sits at (63, 63)..(65, 65).
        let pasted = LayerId::from_uuid(nl);
        assert_eq!(px(&e, pasted, 63, 63), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&e, pasted, 64, 64), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(px(&e, pasted, 62, 63), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn paste_adversarial_rejects_are_typed() {
        let (mut e, bg) = ready("white");

        // Unknown asset id (stage-4 analog).
        let err = e
            .apply_json(&env(
                52,
                "edit.paste",
                None,
                json!({"assetId": Uuid::new_v4().to_string(),
                       "newLayerId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("ingested raster asset"));

        // A samples asset is not an ingested raster asset.
        let samples = e.add_samples_asset(b"opaque samples blob".to_vec());
        let err = e
            .apply_json(&env(
                53,
                "edit.paste",
                None,
                json!({"assetId": samples.to_string(),
                       "newLayerId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("samples recording"));

        // Garbage bytes refuse AT INGESTION (decode-validated door).
        let err = e.add_raster_asset(b"not a png".to_vec()).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);

        // newLayerId collision → duplicate_id (§9.6).
        let asset = e.add_raster_asset(red_png()).unwrap();
        let err = e
            .apply_json(&env(
                54,
                "edit.paste",
                None,
                json!({"assetId": asset.to_string(), "newLayerId": bg.as_uuid().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);

        assert_eq!(e.journal().len(), 1, "rejects journal nothing");
    }

    #[test]
    fn paste_into_requires_an_active_selection() {
        let (mut e, _bg) = ready("white");
        let asset = e.add_raster_asset(red_png()).unwrap();
        let err = e
            .apply_json(&env(
                55,
                "edit.paste_into",
                None,
                json!({"assetId": asset.to_string(),
                       "newLayerId": Uuid::new_v4().to_string(),
                       "newMaskId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        assert_eq!(e.journal().len(), 1);
    }

    #[test]
    fn paste_into_masks_from_the_selection_and_round_trips() {
        let (mut e, _bg) = ready("white");
        let asset = e.add_raster_asset(red_png()).unwrap();
        select_rect(&mut e, IRect::new(10, 10, 20, 20), 1.0);
        let sel_bounds = e.document().unwrap().selection().bounds();
        let (nl, nm) = (Uuid::new_v4(), Uuid::new_v4());
        let before = e.state_hash();
        let receipt = e
            .apply_json(&env(
                56,
                "edit.paste_into",
                None,
                json!({"assetId": asset.to_string(), "at": [0, 0],
                       "newLayerId": nl.to_string(), "newMaskId": nm.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![nl, nm], "layer then mask (§9.6)");
        let after = e.state_hash();
        assert_ne!(before, after);
        let d = e.document().unwrap();
        assert_eq!(d.mask_count(), 1);
        let pasted = LayerId::from_uuid(nl);
        assert_eq!(d.layer(pasted).unwrap().masks(), [MaskId::from_uuid(nm)]);
        let mask = d.mask(MaskId::from_uuid(nm)).unwrap();
        // unit_cmd_edit_paste_into_mask: coverage == selection bit-exactly.
        assert_eq!(mask.coverage().read_pixel_f32(IVec2::new(12, 12))[0], 1.0);
        assert_eq!(mask.coverage().read_pixel_f32(IVec2::new(5, 5))[0], 0.0);
        assert_eq!(mask.coverage().bounds(), sel_bounds);
        // The selection itself is unchanged.
        assert_eq!(d.selection().bounds(), sel_bounds);
        assert!(d.selection().active());

        e.undo().unwrap();
        assert_eq!(
            e.document().unwrap().mask_count(),
            0,
            "undo detaches + stashes the mask"
        );
        assert_eq!(children(&e), 1);
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        assert_eq!(
            e.document().unwrap().mask_count(),
            1,
            "redo re-attaches the SAME mask"
        );
        assert_eq!(e.state_hash(), after);
    }

    #[test]
    fn paste_into_mask_collision_is_duplicate_id() {
        let (mut e, _bg) = ready("white");
        let asset = e.add_raster_asset(red_png()).unwrap();
        select_rect(&mut e, IRect::new(0, 0, 8, 8), 1.0);
        let nm = Uuid::new_v4();
        e.apply_json(&env(
            57,
            "edit.paste_into",
            None,
            json!({"assetId": asset.to_string(), "at": [0, 0],
                   "newLayerId": Uuid::new_v4().to_string(), "newMaskId": nm.to_string()}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                58,
                "edit.paste_into",
                None,
                json!({"assetId": asset.to_string(), "at": [0, 0],
                       "newLayerId": Uuid::new_v4().to_string(), "newMaskId": nm.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);
    }

    // ------------------------------------------------------------------
    // mask.* family
    // ------------------------------------------------------------------

    fn mask_env(n: u32, kind: &str, layer: LayerId, mask: Option<Uuid>, params: Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("e1000000-0000-4000-8000-0002{n:08x}"),
            "kind": kind,
            "params": params,
        });
        v["target"] = match mask {
            Some(m) => json!({"layerId": layer.as_uuid().to_string(), "maskId": m.to_string()}),
            None => json!({"layerId": layer.as_uuid().to_string()}),
        };
        v.to_string()
    }

    fn add_mask(e: &mut Engine, n: u32, layer: LayerId, from: &str) -> MaskId {
        let m = Uuid::new_v4();
        e.apply_json(&mask_env(
            n,
            "mask.add",
            layer,
            None,
            json!({"kind": "raster", "newMaskId": m.to_string(), "from": from}),
        ))
        .unwrap();
        MaskId::from_uuid(m)
    }

    fn coverage_at(e: &Engine, m: MaskId, x: i32, y: i32) -> f32 {
        e.document()
            .unwrap()
            .mask(m)
            .unwrap()
            .coverage()
            .read_pixel_f32(IVec2::new(x, y))[0]
    }

    #[test]
    fn mask_add_reveal_all_round_trips() {
        let (mut e, layer) = ready("white");
        let before = e.state_hash();
        let m = Uuid::new_v4();
        let receipt = e
            .apply_json(&mask_env(
                1,
                "mask.add",
                layer,
                None,
                json!({"kind": "raster", "newMaskId": m.to_string()}),
            ))
            .unwrap();
        assert_eq!(receipt.created_ids, vec![m]);
        let mid = MaskId::from_uuid(m);
        assert_eq!(e.document().unwrap().mask_count(), 1);
        assert_eq!(
            coverage_at(&e, mid, 5, 5),
            1.0,
            "reveal_all covers the canvas"
        );
        let after = e.state_hash();
        assert_ne!(before, after, "masks fold into state_hash when present");
        e.undo().unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 0);
        assert_eq!(e.state_hash(), before);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        assert_eq!(coverage_at(&e, mid, 5, 5), 1.0);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), after);
    }

    #[test]
    fn mask_add_sources_and_rejects() {
        let (mut e, layer) = ready("white");
        // from: selection copies coverage bit-exactly; selection unchanged.
        select_rect(&mut e, IRect::new(10, 10, 20, 20), 0.4);
        let m = add_mask(&mut e, 10, layer, "selection");
        assert_eq!(coverage_at(&e, m, 12, 12), 0.4);
        assert_eq!(coverage_at(&e, m, 50, 50), 0.0);
        assert!(
            e.document().unwrap().selection().active(),
            "selection unchanged"
        );

        // Slot occupied -> target_in_use.
        let err = e
            .apply_json(&mask_env(
                11,
                "mask.add",
                layer,
                None,
                json!({"kind": "raster", "newMaskId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetInUse);

        // Duplicate creation uuid -> duplicate_id (fresh layer).
        let (mut e2, layer2) = ready("white");
        let m2 = add_mask(&mut e2, 12, layer2, "hide_all");
        assert_eq!(coverage_at(&e2, m2, 5, 5), 0.0, "hide_all reads 0");
        let g = add_group(&mut e2);
        let err = e2
            .apply_json(&mask_env(
                13,
                "mask.add",
                g,
                None,
                json!({"kind": "raster", "newMaskId": m2.as_uuid().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);

        // vector -> typed; from selection without one -> selection_required;
        // ALL lock -> locked; unresolved target -> bad_target.
        let err = e2
            .apply_json(&mask_env(
                14,
                "mask.add",
                g,
                None,
                json!({"kind": "vector", "newMaskId": Uuid::new_v4().to_string(),
                        "from": "path", "path": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        let err = e2
            .apply_json(&mask_env(
                15,
                "mask.add",
                g,
                None,
                json!({"kind": "raster", "newMaskId": Uuid::new_v4().to_string(),
                        "from": "selection"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        lock(&mut e2, g, LockFlags::ALL);
        let err = e2
            .apply_json(&mask_env(
                16,
                "mask.add",
                g,
                None,
                json!({"kind": "raster", "newMaskId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        let ghost = LayerId::from_uuid(Uuid::new_v4());
        let err = e2
            .apply_json(&mask_env(
                17,
                "mask.add",
                ghost,
                None,
                json!({"kind": "raster", "newMaskId": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    #[test]
    fn mask_add_from_transparency_copies_alpha() {
        let (mut e, layer) = ready("transparent");
        // Half-transparent content via a 50% fill.
        e.apply_json(&fill_color(
            20,
            layer,
            [1.0, 0.0, 0.0],
            json!({"opacity": 0.5}),
        ))
        .unwrap();
        let m = add_mask(&mut e, 21, layer, "transparency");
        let a = coverage_at(&e, m, 64, 64);
        assert!((a - 0.5).abs() < 0.01, "coverage == alpha, got {a}");
    }

    #[test]
    fn mask_edit_ops_round_trip_and_gate() {
        let (mut e, layer) = ready("white");
        let m = add_mask(&mut e, 30, layer, "hide_all");
        // fill 0.3 over the canvas.
        e.apply_json(&mask_env(
            31,
            "mask.edit",
            layer,
            Some(m.as_uuid()),
            json!({"edit": {"op": "fill", "value": 0.3}}),
        ))
        .unwrap();
        assert!((coverage_at(&e, m, 5, 5) - 0.3).abs() < 1e-6);
        let before_invert = e.state_hash();
        // invert -> 0.7.
        e.apply_json(&mask_env(
            32,
            "mask.edit",
            layer,
            Some(m.as_uuid()),
            json!({"edit": {"op": "invert"}}),
        ))
        .unwrap();
        assert!((coverage_at(&e, m, 5, 5) - 0.7).abs() < 1e-6);
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before_invert, "mask-surface deltas restore");
        e.redo().unwrap();
        // levels: inBlack .2 inWhite .8 -> t=(0.7-0.2)/0.6=0.8333.
        e.apply_json(&mask_env(
            33,
            "mask.edit",
            layer,
            Some(m.as_uuid()),
            json!({"edit": {"op": "levels", "params": {"inBlack": 0.2, "inWhite": 0.8}}}),
        ))
        .unwrap();
        assert!((coverage_at(&e, m, 5, 5) - 0.8333).abs() < 1e-3);
        // Selection gates the write (§13.1): sel 0.5 -> halfway blend.
        select_rect(&mut e, IRect::new(0, 0, 8, 8), 0.5);
        let old = coverage_at(&e, m, 2, 2);
        e.apply_json(&mask_env(
            34,
            "mask.edit",
            layer,
            Some(m.as_uuid()),
            json!({"edit": {"op": "fill", "value": 0.0}}),
        ))
        .unwrap();
        let got = coverage_at(&e, m, 2, 2);
        assert!(
            (got - old * 0.5).abs() < 1e-3,
            "gated halfway toward 0: {old} -> {got}"
        );

        // Rejects: gaussian typed; PIXELS lock blocks; bad op range.
        let err = e
            .apply_json(&mask_env(
                35,
                "mask.edit",
                layer,
                Some(m.as_uuid()),
                json!({"edit": {"op": "gaussian_blur", "params": {"sigma": 2.0}}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        lock(&mut e, layer, LockFlags::PIXELS);
        let err = e
            .apply_json(&mask_env(
                36,
                "mask.edit",
                layer,
                Some(m.as_uuid()),
                json!({"edit": {"op": "invert"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        lock(&mut e, layer, LockFlags::empty());
        let err = e
            .apply_json(&mask_env(
                37,
                "mask.edit",
                layer,
                Some(Uuid::new_v4()),
                json!({"edit": {"op": "invert"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget, "dangling maskId");
    }

    #[test]
    fn mask_set_props_delta_round_trips() {
        let (mut e, layer) = ready("white");
        let m = add_mask(&mut e, 40, layer, "reveal_all");
        let before = e.state_hash();
        e.apply_json(&mask_env(
            41,
            "mask.set_props",
            layer,
            Some(m.as_uuid()),
            json!({"feather": 4.0, "density": 0.5}),
        ))
        .unwrap();
        let p = e.document().unwrap().mask(m).unwrap().props();
        assert_eq!((p.feather, p.density), (4.0, 0.5));
        let after = e.state_hash();
        assert_ne!(before, after, "props fold into state_hash");
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before, "SetProp before-block restores");
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after);
        // Range reject + ALL lock.
        let err = e
            .apply_json(&mask_env(
                42,
                "mask.set_props",
                layer,
                Some(m.as_uuid()),
                json!({"density": 1.5}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        lock(&mut e, layer, LockFlags::ALL);
        let err = e
            .apply_json(&mask_env(
                43,
                "mask.set_props",
                layer,
                Some(m.as_uuid()),
                json!({"enabled": false}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    #[test]
    fn mask_delete_restores_the_same_node_on_undo() {
        let (mut e, layer) = ready("white");
        select_rect(&mut e, IRect::new(4, 4, 40, 40), 0.6);
        let m = add_mask(&mut e, 50, layer, "selection");
        let with_mask = e.state_hash();
        e.apply_json(&mask_env(
            51,
            "mask.delete",
            layer,
            Some(m.as_uuid()),
            json!({}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 0);
        e.undo().unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 1);
        assert_eq!(
            e.state_hash(),
            with_mask,
            "the SAME node re-attaches (tiles pinned)"
        );
        assert_eq!(coverage_at(&e, m, 10, 10), 0.6);
        e.redo().unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 0);
        // ALL lock blocks; PIXELS does NOT (§40.3 structural rule).
        e.undo().unwrap();
        lock(&mut e, layer, LockFlags::PIXELS);
        e.apply_json(&mask_env(
            52,
            "mask.delete",
            layer,
            Some(m.as_uuid()),
            json!({}),
        ))
        .unwrap();
        e.undo().unwrap();
        lock(&mut e, layer, LockFlags::ALL);
        let err = e
            .apply_json(&mask_env(
                53,
                "mask.delete",
                layer,
                Some(m.as_uuid()),
                json!({}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
    }

    #[test]
    fn mask_apply_bakes_the_effective_coverage_formula() {
        let (mut e, layer) = ready("white");
        let m = add_mask(&mut e, 60, layer, "hide_all");
        e.apply_json(&mask_env(
            61,
            "mask.edit",
            layer,
            Some(m.as_uuid()),
            json!({"edit": {"op": "fill", "value": 0.5}}),
        ))
        .unwrap();
        // density 0.5: c = 1 - 0.5*(1 - 0.5) = 0.75 (§11.2 step 3).
        e.apply_json(&mask_env(
            62,
            "mask.set_props",
            layer,
            Some(m.as_uuid()),
            json!({"density": 0.5}),
        ))
        .unwrap();
        let before = e.state_hash();
        e.apply_json(&mask_env(
            63,
            "mask.apply",
            layer,
            Some(m.as_uuid()),
            json!({}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().mask_count(), 0, "bake then remove");
        let px = px(&e, layer, 64, 64);
        assert!(
            (px[3] - 0.75).abs() < 0.01,
            "alpha == §11.2 formula output, got {px:?}"
        );
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            before,
            "tile deltas + mask re-attach restore"
        );
        e.redo().unwrap();

        // Rejects: TRANSPARENCY lock; feather typed; group owner mismatch.
        let (mut e2, layer2) = ready("white");
        let m2 = add_mask(&mut e2, 64, layer2, "reveal_all");
        lock(&mut e2, layer2, LockFlags::TRANSPARENCY);
        let err = e2
            .apply_json(&mask_env(
                65,
                "mask.apply",
                layer2,
                Some(m2.as_uuid()),
                json!({}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Locked);
        lock(&mut e2, layer2, LockFlags::empty());
        e2.apply_json(&mask_env(
            66,
            "mask.set_props",
            layer2,
            Some(m2.as_uuid()),
            json!({"feather": 2.0}),
        ))
        .unwrap();
        let err = e2
            .apply_json(&mask_env(
                67,
                "mask.apply",
                layer2,
                Some(m2.as_uuid()),
                json!({}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::NotImplemented,
            "feathered bake is the O-TOOLS seam"
        );
        let g = add_group(&mut e2);
        let mg = add_mask(&mut e2, 68, g, "reveal_all");
        let err = e2
            .apply_json(&mask_env(
                69,
                "mask.apply",
                g,
                Some(mg.as_uuid()),
                json!({}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetKindMismatch);
    }

    #[test]
    fn mask_to_selection_combines_with_exact_formulas() {
        let (mut e, layer) = ready("white");
        // Mask coverage: 0.5 over (0,0..16,16) via fill on a hide_all mask.
        let m = add_mask(&mut e, 70, layer, "hide_all");
        {
            // Constrain the fill with a selection over the target rect,
            // then clear the selection state for the real test below.
            select_rect(&mut e, IRect::new(0, 0, 16, 16), 1.0);
            e.apply_json(&mask_env(
                71,
                "mask.edit",
                layer,
                Some(m.as_uuid()),
                json!({"edit": {"op": "fill", "value": 0.5}}),
            ))
            .unwrap();
        }
        // Selection: 0.4 over (8,8..24,24).
        select_rect(&mut e, IRect::new(8, 8, 24, 24), 0.4);
        let before = e.state_hash();

        // add: overlap = min(1, 0.4+0.5) = 0.9; mask-only = 0.5; sel-only 0.4.
        e.apply_json(&mask_env(
            72,
            "mask.to_selection",
            layer,
            Some(m.as_uuid()),
            json!({"op": "add"}),
        ))
        .unwrap();
        let sel_at = |e: &Engine, x: i32, y: i32| {
            e.document()
                .unwrap()
                .selection()
                .mask()
                .read_pixel_f32(IVec2::new(x, y))[0]
        };
        assert!((sel_at(&e, 12, 12) - 0.9).abs() < 1e-6, "min(1, a+b)");
        assert!((sel_at(&e, 2, 2) - 0.5).abs() < 1e-6, "mask-only region");
        assert!(
            (sel_at(&e, 20, 20) - 0.4).abs() < 1e-6,
            "selection-only region"
        );
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            IRect::new(0, 0, 24, 24)
        );

        // Undo restores the previous selection EXACTLY (surface deltas).
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            IRect::new(8, 8, 24, 24)
        );

        // replace: selection becomes the coverage; intersect/subtract math.
        e.apply_json(&mask_env(
            73,
            "mask.to_selection",
            layer,
            Some(m.as_uuid()),
            json!({"op": "replace"}),
        ))
        .unwrap();
        assert!((sel_at(&e, 2, 2) - 0.5).abs() < 1e-6);
        assert_eq!(
            sel_at(&e, 20, 20),
            0.0,
            "replace clears outside the coverage"
        );
        assert_eq!(
            e.document().unwrap().selection().bounds(),
            IRect::new(0, 0, 16, 16)
        );

        select_rect(&mut e, IRect::new(8, 8, 24, 24), 0.4);
        e.apply_json(&mask_env(
            74,
            "mask.to_selection",
            layer,
            Some(m.as_uuid()),
            json!({"op": "subtract"}),
        ))
        .unwrap();
        assert_eq!(sel_at(&e, 12, 12), 0.0, "max(0, 0.4-0.5)");
        assert!(
            (sel_at(&e, 20, 20) - 0.4).abs() < 1e-6,
            "outside coverage untouched"
        );

        select_rect(&mut e, IRect::new(8, 8, 24, 24), 0.4);
        e.apply_json(&mask_env(
            75,
            "mask.to_selection",
            layer,
            Some(m.as_uuid()),
            json!({"op": "intersect"}),
        ))
        .unwrap();
        assert!((sel_at(&e, 12, 12) - 0.4).abs() < 1e-6, "min(a, b)");
        assert_eq!(sel_at(&e, 20, 20), 0.0, "intersect clears the non-overlap");

        // Locks never gate selection writes (§40.4).
        lock(&mut e, layer, LockFlags::ALL);
        e.apply_json(&mask_env(
            76,
            "mask.to_selection",
            layer,
            Some(m.as_uuid()),
            json!({"op": "replace"}),
        ))
        .unwrap();
    }

    // ------------------------------------------------------------------
    // doc.* color family
    // ------------------------------------------------------------------

    fn color_env(n: u32, kind: &str, params: Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("e1000000-0000-4000-8000-0003{n:08x}"),
            "kind": kind,
            "params": params,
        })
        .to_string()
    }

    #[test]
    fn assign_profile_retags_with_zero_deltas_and_param_inverse() {
        let (mut e, layer) = ready("white");
        let before_working = e.document().unwrap().color().working;
        let before_hash = e.state_hash();
        let px_before = px(&e, layer, 5, 5);
        // null = untagged.
        let receipt = e
            .apply_json(&color_env(
                1,
                "doc.assign_profile",
                json!({"profile": null}),
            ))
            .unwrap();
        assert!(receipt.dirty.is_empty(), "zero tile deltas (§5.3/§40.2)");
        assert_eq!(
            e.document().unwrap().color().working,
            crate::engine::color_glue::untagged_profile_id()
        );
        assert_eq!(px(&e, layer, 5, 5), px_before, "assign never converts");
        let after_hash = e.state_hash();
        assert_ne!(before_hash, after_hash, "working folds into state_hash");
        e.undo().unwrap();
        assert_eq!(
            e.document().unwrap().color().working,
            before_working,
            "param inverse"
        );
        assert_eq!(e.state_hash(), before_hash);
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after_hash);
        // Builtin assignment + typed rejects.
        let p3 =
            crate::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::DisplayP3);
        e.apply_json(&color_env(
            2,
            "doc.assign_profile",
            json!({"profile": p3.as_uuid().to_string()}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().color().working, p3);
        let gray =
            crate::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::GrayGamma22);
        let err = e
            .apply_json(&color_env(
                3,
                "doc.assign_profile",
                json!({"profile": gray.as_uuid().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);
        let err = e
            .apply_json(&color_env(
                4,
                "doc.assign_profile",
                json!({"profile": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::BadTarget,
            "unresolvable profile is stage 4"
        );
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn convert_profile_transforms_pixels_and_round_trips() {
        let (mut e, layer) = ready("transparent");
        // Mid-red content at fractional alpha (journaled only — replay must
        // reproduce it, so no test-door selection here).
        e.apply_json(&fill_color(
            10,
            layer,
            [0.8, 0.2, 0.1],
            json!({"opacity": 0.6}),
        ))
        .unwrap();
        let before_hash = e.state_hash();
        let px_before = px(&e, layer, 5, 5);
        let p3 =
            crate::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::DisplayP3);
        e.apply_json(&color_env(
            11,
            "doc.convert_profile",
            json!({"profile": p3.as_uuid().to_string()}),
        ))
        .unwrap();
        let px_after = px(&e, layer, 5, 5);
        assert_ne!(
            px_before, px_after,
            "appearance-preserving conversion changes device values"
        );
        assert!((px_after[3] - px_before[3]).abs() < 1e-6, "alpha untouched");
        assert_eq!(e.document().unwrap().color().working, p3, "retagged");

        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            before_hash,
            "full-surface deltas + SetProp restore"
        );
        e.redo().unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "convert replays deterministically"
        );
    }

    #[test]
    fn convert_profile_null_transform_and_rejects() {
        let (mut e, _layer) = ready("white");
        let srgb = crate::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::Srgb);
        let before_pixels = e.state_hash();
        // §5.8 test 2: src == dst → output byte-identical, still recorded.
        e.apply_json(&color_env(
            20,
            "doc.convert_profile",
            json!({"profile": srgb.as_uuid().to_string()}),
        ))
        .unwrap();
        assert_eq!(e.journal().len(), 2, "null transform still journals");
        // Pixels unchanged; only the working tag moved (alias -> builtin id).
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before_pixels);
        e.redo().unwrap();
        let gray =
            crate::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::SGray);
        let err = e
            .apply_json(&color_env(
                21,
                "doc.convert_profile",
                json!({"profile": gray.as_uuid().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);
        let err = e
            .apply_json(&color_env(
                22,
                "doc.convert_profile",
                json!({"profile": srgb.as_uuid().to_string(), "flatten": true}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        let err = e
            .apply_json(&color_env(
                23,
                "doc.convert_profile",
                json!({"profile": Uuid::new_v4().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    #[test]
    fn depth_convert_round_trips_and_refuses_typed() {
        let (mut e, layer) = ready("white");
        e.apply_json(&fill_color(
            30,
            layer,
            [0.3, 0.6, 0.9],
            json!({"opacity": 0.7}),
        ))
        .unwrap();
        let u8_hash = e.state_hash();
        assert_eq!(
            e.document().unwrap().color().depth,
            crate::tile::channel::ChannelKind::U8
        );
        // u8 -> f32 (snapshot step).
        e.apply_json(&color_env(31, "doc.depth_convert", json!({"to": "f32"})))
            .unwrap();
        assert_eq!(
            e.document().unwrap().color().depth,
            crate::tile::channel::ChannelKind::F32
        );
        let f32_hash = e.state_hash();
        assert_ne!(u8_hash, f32_hash);
        // §10.4 snap inverse: undo/redo are O(1) swaps.
        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            u8_hash,
            "snapshot undo restores byte identity"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), f32_hash);
        // §34.4 property 2: u8 -> f32 -> u8 identity.
        e.apply_json(&color_env(32, "doc.depth_convert", json!({"to": "u8"})))
            .unwrap();
        assert_eq!(
            e.state_hash(),
            u8_hash,
            "u8→f32→u8 round trip is the identity"
        );
        // No-op convert journals without changing state.
        let l = e.journal().len();
        e.apply_json(&color_env(33, "doc.depth_convert", json!({"to": "u8"})))
            .unwrap();
        assert_eq!((e.journal().len(), e.state_hash()), (l + 1, u8_hash));
        // Typed refusals: f16 paused; u16 feature-gated in this build.
        let err = e
            .apply_json(&color_env(34, "doc.depth_convert", json!({"to": "f16"})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        #[cfg(not(feature = "depth-u16"))]
        {
            let err = e
                .apply_json(&color_env(35, "doc.depth_convert", json!({"to": "u16"})))
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotImplemented);
        }
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "converts replay (value-capture reading)"
        );
    }

    #[test]
    fn icc_ingestion_rejects_are_typed_and_embedded_profiles_round_trip() {
        let (mut e, _layer) = ready("white");
        // Garbage bytes fail the §5.2 ladder typed.
        let err = e
            .add_icc_profile(b"definitely not an icc profile".to_vec())
            .unwrap_err();
        assert!(
            matches!(
                err.code,
                ErrorCode::ProfileMalformed | ErrorCode::ProfileClass
            ),
            "garbage refuses through the §5.2 ladder, got {:?}",
            err.code
        );
        // Oversized blob refuses at the ingress cap.
        let big = vec![0u8; 17 * 1024 * 1024];
        let err = e.add_icc_profile(big).unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileTooLarge);

        // Embedded bytes round-trip the package (registered via the
        // crate-internal door; CMM validation is use-time, §7.5 carriage is
        // byte-level).
        let bytes = std::sync::Arc::new(vec![7u8; 128]);
        let uid = appname_color::ProfileUid::of_icc_bytes(&bytes);
        let pid = crate::engine::color_glue::icc_profile_id(uid);
        e.doc_mut_for_test().register_profile(
            pid,
            crate::graph::document::DocProfile {
                uid,
                source: crate::graph::document::DocProfileSource::Icc(bytes.clone()),
            },
        );
        assert!(e.document().unwrap().has_embedded_profiles());
        let saved = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded =
            crate::io::package::load_package_bytes_with(&register(EngineAssembly::new()), &saved)
                .unwrap();
        let ldoc = loaded.engine.document().unwrap();
        assert!(ldoc.has_embedded_profiles());
        let entry = ldoc
            .profile(pid)
            .expect("re-registered from profiles/{id}.icc");
        assert_eq!(
            entry.uid, uid,
            "content-derived uid survives the round trip"
        );
        assert_eq!(
            loaded.engine.state_hash(),
            e.state_hash(),
            "embedded profiles fold"
        );
    }
}
