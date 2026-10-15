// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contract `doc.mode_convert`; ES §6.2 (mode matrix, warnings
//! BEFORE execution, masks/channels/selection NEVER converted
//! [NORMATIVE]), §5.3/§5.5 (profile resolution, CMYK fallback), §10.4
//! (class C — snapshot inverse); door C5 of reports/O-COLOR-log.md in
//! its SUPERSEDED-BY-ADDENDUM reading (the addendum touches C4 only —
//! C5 stands as the wave-1 entry wrote it).
//!
//! CP-1.0 door slice 3, group 5. The reachable v1 arms, honestly:
//!
//! * `rgba ↔ gray_a`, `rgba ↔ lab_a`, `gray_a ↔ lab_a` execute fully:
//!   per-tile straight-alpha f32 marshalling through ONE cached CMM
//!   transform (`Cms::transform`, the `doc.convert_profile` marshalling
//!   pattern mirrored — any TRC-convention change lands in the Cms and
//!   both paths inherit it); COPY_ALPHA discipline (alpha re-taken from
//!   the saved plane, never color-managed); masks/channels/selection
//!   untouched (Gray, §6.2 [NORMATIVE]); ColorState re-tags through the
//!   CheckpointOnly doors; inverse = the engine's class-C SNAPSHOT.
//! * `cmyk_a` endpoints answer typed NI **after** the §40.2 profile
//!   gates fire (the contract's invalid case 3 — an RGB profile for
//!   `to:'cmyk_a'` — answers `error.color.profile_space_mismatch`
//!   first). CAUSE, recorded: core's tile pixel doors are 4-wide
//!   (`read/write_pixel_f32: [f32; 4]`; `write_px_dense` zeroes CmykA's
//!   5th channel by documented interim, `alpha_index` 4 is outside the
//!   value, premul validation is 4-slot) — the 5-channel value path
//!   exists COLOR-side (§5.5 fallback + CMM formats) but not
//!   substrate-side. The doc.new CmykA refusal therefore also STANDS
//!   (lifting it would mint broken documents). Flagged to O-INT as the
//!   open question of this slice: the C5 "CmykA refusal lift" needs a
//!   scoped substrate slice (wide pixel doors) first.
//! * Documents carrying parametric layers (adjustment/text/shape/smart
//!   object) answer typed NI naming the §40.2 rasterize-or-drop compat
//!   matrix as the deferred arm (per-layer CompatReport detail rides
//!   that arm). Raster/group documents convert fully.
//! * Integer-depth documents marshal as PREMULTIPLIED-normalized
//!   storage exactly like `doc.convert_profile` (unpremultiply →
//!   transform → re-premultiply; content rides its storage encoding
//!   through the CMM — the recorded O-COLOR TRC interim). RULED at the
//!   fix round for the O-UI §6.4 filing: the first cut of this arm
//!   treated integer storage as encoded-STRAIGHT (srgb decode/encode
//!   with untouched linear alpha), which mints c > a pixels — a §6.4
//!   write-door breach — for ANY fractional-alpha content (a
//!   fractional-coverage edit.cut being the first minter of such
//!   pixels; the cut itself is uniform-premul-scaled and CLEAN). The
//!   premul posture makes c ≤ a hold by construction.
//! * `cmyk_profile_available` resolves FALSE headless (OQ-COLOR-7:
//!   `pref.color` CMYK-profile plumbing unowned) — CMYK plans would
//!   take the §5.5 fallback leg when the substrate arm lands.
//! * Warnings surface BEFORE execution in the command RECEIPT
//!   (`ApplyReceipt.warnings`, §6.2 MessageIds — additive field,
//!   serialized only when non-empty so every prior receipt shape is
//!   byte-identical) and ride the history event via the receipt echo.

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use appname_color::DocMode;

use crate::engine::color_glue;
use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::geom::IVec2;
use crate::graph::document::{DocProfile, DocProfileSource};
use crate::graph::id::{LayerId, ProfileId};
use crate::tile::channel::ChannelKind;
use crate::tile::map::TileCoord;
use crate::tile::model::ColorModelKind;
use crate::tile::{TILE, TILE_I32};

pub struct DocModeConvertKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![Arc::new(DocModeConvertKind)]
}

/// Document `ColorState.mode` → the §6.2 wire mode.
fn doc_mode_of(mode: ColorModelKind) -> Result<DocMode, EngineError> {
    Ok(match mode {
        ColorModelKind::Rgba => DocMode::Rgba,
        ColorModelKind::GrayA => DocMode::GrayA,
        ColorModelKind::CmykA => DocMode::CmykA,
        ColorModelKind::LabA => DocMode::LabA,
        other => {
            return Err(EngineError::bad_target(format!(
                "document mode {other:?} is not a §6.2 document mode"
            )))
        }
    })
}

fn model_of(mode: DocMode) -> ColorModelKind {
    match mode {
        DocMode::Rgba => ColorModelKind::Rgba,
        DocMode::GrayA => ColorModelKind::GrayA,
        DocMode::CmykA => ColorModelKind::CmykA,
        DocMode::LabA => ColorModelKind::LabA,
    }
}

fn depth_of(kind: ChannelKind) -> appname_color::Depth {
    match kind {
        ChannelKind::U8 => appname_color::Depth::U8,
        ChannelKind::U16 => appname_color::Depth::U16,
        ChannelKind::F16 => appname_color::Depth::F16,
        ChannelKind::F32 => appname_color::Depth::F32,
    }
}

/// The §5.3 space class a mode's profiles must carry.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum SpaceClass {
    Rgb,
    Gray,
    Cmyk,
    Lab,
}

fn required_space(mode: DocMode) -> SpaceClass {
    match mode {
        DocMode::Rgba => SpaceClass::Rgb,
        DocMode::GrayA => SpaceClass::Gray,
        DocMode::CmykA => SpaceClass::Cmyk,
        DocMode::LabA => SpaceClass::Lab,
    }
}

fn space_of_entry(entry: &DocProfile) -> Result<SpaceClass, EngineError> {
    Ok(match &entry.source {
        DocProfileSource::Builtin(space) => match space {
            appname_color::BuiltinSpace::GrayGamma22 | appname_color::BuiltinSpace::SGray => {
                SpaceClass::Gray
            }
            _ => SpaceClass::Rgb,
        },
        DocProfileSource::Untagged => SpaceClass::Rgb, // presented-as-sRGB interim (§5.3)
        DocProfileSource::BuiltinLab => SpaceClass::Lab,
        DocProfileSource::Icc(bytes) => {
            let header =
                appname_color::parse_icc_header(bytes).map_err(color_glue::map_color_error)?;
            match header.data_space {
                appname_color::DataColorSpace::Rgb => SpaceClass::Rgb,
                appname_color::DataColorSpace::Gray => SpaceClass::Gray,
                appname_color::DataColorSpace::Cmyk => SpaceClass::Cmyk,
                appname_color::DataColorSpace::Lab => SpaceClass::Lab,
                other => {
                    return Err(EngineError::new(
                        ErrorCode::ProfileSpaceMismatch,
                        format!("profile data space {other:?} matches no §6.2 document mode"),
                    ))
                }
            }
        }
    })
}

impl KindImpl for DocModeConvertKind {
    fn kind(&self) -> &'static str {
        "doc.mode_convert"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let to = cx
            .params()
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("to"))?;
        let to = DocMode::from_wire(to)
            .ok_or_else(|| EngineError::range("to", format!("unknown mode literal \"{to}\"")))?;
        let from = doc_mode_of(cx.document().color().mode)?;
        let depth = cx.document().color().depth;
        if cx.params().get("flatten").and_then(Value::as_bool) == Some(true) {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "flatten-on-convert needs the compositor flatten row (§40.2; the \
                 doc.convert_profile posture, mirrored)",
            ));
        }
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

        // §40.2 profile gate — BEFORE any NI refusal (the contract's
        // invalid case 3 must answer profile_space_mismatch).
        let profile_param: Option<(ProfileId, DocProfile)> =
            match cx.params().get("profile").and_then(Value::as_str) {
                None => None,
                Some(s) => {
                    let id = ProfileId(Uuid::parse_str(s).map_err(|_| {
                        EngineError::range("profile", format!("not a valid UUID: \"{s}\""))
                    })?);
                    let entry = cx.document().profile(id).cloned().ok_or_else(|| {
                        EngineError::bad_target(format!(
                            "profile {s} does not resolve in the document store (§5.1)"
                        ))
                    })?;
                    if space_of_entry(&entry)? != required_space(to) {
                        return Err(EngineError::new(
                            ErrorCode::ProfileSpaceMismatch,
                            format!(
                                "the given profile is not of the destination space ({:?} \
                                 required for to:'{}', §40.2)",
                                required_space(to),
                                to.as_wire()
                            ),
                        ));
                    }
                    Some((id, entry))
                }
            };

        // OQ-COLOR-7: no CMYK-profile preference plumbing exists headless.
        let cmyk_profile_available = false;
        let plan =
            appname_color::plan_mode_convert(from, to, depth_of(depth), cmyk_profile_available);
        for w in &plan.warnings {
            cx.push_warning(*w); // §6.2 warnings BEFORE execution
        }

        if plan.identity {
            // §6.2: same-mode is the identity — recorded, zero change
            // (§40.1 empty push; class C stores the snapshot regardless).
            return Ok(KindOutcome::default());
        }
        if from == DocMode::CmykA || to == DocMode::CmykA {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                "cmyk_a mode conversion needs the 5-channel tile pixel doors (the substrate's \
                 [f32; 4] read/write interim, recorded in the lane log) — the §5.5 fallback \
                 and CMM legs are color-side ready; typed refusal until the substrate slice",
            ));
        }
        // Parametric content: the §40.2 rasterize-or-drop compat matrix is
        // a named deferred arm (per-layer CompatReport detail rides it).
        let parametric: Vec<LayerId> = cx
            .document()
            .z_order_walk()
            .into_iter()
            .filter(|id| {
                cx.document()
                    .layer(*id)
                    .map(|n| !n.is_group() && n.pixels().is_none())
                    .unwrap_or(false)
            })
            .collect();
        if !parametric.is_empty() {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "mode conversion over parametric layers ({} present) needs the §40.2 \
                     rasterize-or-drop compat arm (per-layer report) — typed refusal, \
                     recorded",
                    parametric.len()
                ),
            ));
        }

        // Resolve endpoints: src = the working profile; dst = the given
        // profile or the destination mode's deterministic default.
        let working = cx.document().color().working;
        let src_entry = cx.document().profile(working).cloned().ok_or_else(|| {
            EngineError::bad_target("the working profile does not resolve in the store (§5.1)")
        })?;
        let src_uid = color_glue::resolve_uid(cx.cms(), &src_entry)?;
        let (dst_pid, dst_uid) = match profile_param {
            Some((pid, entry)) => {
                let uid = color_glue::resolve_uid(cx.cms(), &entry)?;
                (pid, uid)
            }
            None => match to {
                DocMode::Rgba => {
                    let space = appname_color::BuiltinSpace::Srgb;
                    (
                        color_glue::builtin_profile_id(space),
                        cx.cms().builtin(space),
                    )
                }
                DocMode::GrayA => {
                    let space = appname_color::BuiltinSpace::GrayGamma22;
                    (
                        color_glue::builtin_profile_id(space),
                        cx.cms().builtin(space),
                    )
                }
                DocMode::LabA => (color_glue::builtin_lab_profile_id(), cx.cms().builtin_lab()),
                DocMode::CmykA => unreachable!("refused above"),
            },
        };
        let fmt_in = appname_color::PixelFormat {
            model: from.color_model(),
            depth: appname_color::Depth::F32,
        };
        let fmt_out = appname_color::PixelFormat {
            model: to.color_model(),
            depth: appname_color::Depth::F32,
        };
        let key = appname_color::TransformKey {
            src: src_uid,
            dst: dst_uid,
            proof: None,
            intent,
            bpc: true,
            fmt_in,
            fmt_out,
        };
        let transform = cx
            .cms()
            .transform(key)
            .map_err(color_glue::map_color_error)?;

        let in_ch = from.color_model().channels();
        let out_ch = to.color_model().channels();
        let to_model = model_of(to);

        // Convert every raster layer through the ONE cached transform
        // (§6.2 per-tile marshalling; masks/channels/selection untouched).
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
            let bounds = {
                let buf = cx
                    .document()
                    .layer(layer)
                    .expect("walked")
                    .pixels()
                    .expect("raster");
                buf.bounds()
            };
            let mut out_buf = crate::buffer::Buffer::new(depth, to_model, dst_pid);
            if !bounds.is_empty() {
                let coords: Vec<(i32, i32)> = bounds.tiles(TILE_I32).collect();
                for (tx, ty) in coords {
                    let coord = TileCoord::new(0, tx, ty);
                    let rect = coord.pixel_rect();
                    if rect.intersect(&bounds).is_empty() {
                        continue;
                    }
                    let mut straight = vec![0.0f32; TILE * TILE * in_ch];
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
                                let a = v[in_ch - 1];
                                alphas[i] = a;
                                // Storage is PREMULTIPLIED-normalized at
                                // EVERY depth (the doc.convert_profile
                                // posture verbatim; the fix-round
                                // §6.4-breach ruling — integer content
                                // rides its storage encoding through the
                                // CMM, the recorded O-COLOR TRC interim):
                                // unpremultiply, transform, re-premultiply.
                                let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
                                for c in 0..in_ch - 1 {
                                    straight[i * in_ch + c] = v[c] * inv;
                                }
                                straight[i * in_ch + in_ch - 1] = a;
                                i += 1;
                            }
                        }
                    }
                    let mut out = vec![0.0f32; TILE * TILE * out_ch];
                    transform
                        .apply_f32(&straight, &mut out)
                        .map_err(color_glue::map_color_error)?;
                    for (i, a) in alphas.iter().enumerate() {
                        let a = *a;
                        let mut px = [0.0f32; 4];
                        if a > 0.0 {
                            for c in 0..out_ch - 1 {
                                let s = out[i * out_ch + c].clamp(0.0, 1.0);
                                // Re-premultiply at EVERY depth (§6.4:
                                // c ≤ a by construction — the write door
                                // validation can never fire; a = 0 pixels
                                // stay all-zero canonically).
                                px[c] = (s * a).clamp(0.0, a);
                            }
                            px[out_ch - 1] = a;
                            let x = rect.min.x + (i % TILE) as i32;
                            let y = rect.min.y + (i / TILE) as i32;
                            out_buf.write_pixel_f32(IVec2::new(x, y), px);
                        }
                    }
                }
                out_buf.set_bounds_recorded(bounds);
            }
            cx.checkpoint_set_layer_pixels(layer, out_buf)?;
        }
        cx.checkpoint_set_color_mode(to_model, dst_pid)?;
        Ok(KindOutcome::default())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::engine::error::ErrorCode;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;
    use crate::geom::IVec2;
    use crate::tile::model::ColorModelKind;

    fn assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
    }

    fn ready(depth: &str) -> Engine {
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0dec0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 32, "height": 32, "background": "white", "depth": depth},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, params: Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": format!("0dec0000-0000-4000-8000-0001{n:08x}"),
            "kind": "doc.mode_convert",
            "params": params,
        })
        .to_string()
    }

    fn px(e: &Engine, x: i32, y: i32) -> [f32; 4] {
        let layer = e.document().unwrap().z_order_walk()[0];
        e.document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .read_pixel_f32(IVec2::new(x, y))
    }

    /// §6.4 fix-round regression (O-UI filing, ede03b5 escalation): the
    /// EXACT filed sequence — u8 doc, fractional selection, edit.cut,
    /// doc.mode_convert — journaled end-to-end. Pre-fix this panicked at
    /// the §6.4 write door (buffer.rs "into U8/GrayA": encode(g) > a);
    /// the verified mechanism was THIS kind's integer arm writing
    /// sRGB-encoded straight color with linear alpha — NOT the filed
    /// cut hypothesis (see the cut-audit test below).
    #[test]
    fn filed_repro_cut_then_convert_succeeds_and_stays_premul_valid() {
        let mut e = ready("u8");
        let layer = e.document().unwrap().z_order_walk()[0];
        // Journaled fractional selection (the channels-battery recipe).
        let mask = "0dec0000-0000-4000-8000-00000000aaaa";
        for (id, kind, target, params) in [
            (
                "0dec0000-0000-4000-8000-000000000011",
                "mask.add",
                json!({"layerId": layer.as_uuid().to_string()}),
                json!({"newMaskId": mask, "kind": "raster", "from": "reveal_all"}),
            ),
            (
                "0dec0000-0000-4000-8000-000000000012",
                "mask.edit",
                json!({"layerId": layer.as_uuid().to_string(), "maskId": mask}),
                json!({"edit": {"op": "fill", "value": 0.35}}),
            ),
            (
                "0dec0000-0000-4000-8000-000000000013",
                "mask.to_selection",
                json!({"layerId": layer.as_uuid().to_string(), "maskId": mask}),
                json!({"op": "replace"}),
            ),
            (
                "0dec0000-0000-4000-8000-000000000014",
                "mask.delete",
                json!({"layerId": layer.as_uuid().to_string(), "maskId": mask}),
                json!({}),
            ),
        ] {
            let mut v = json!({"schema": "appname.command.v1", "id": id,
                               "kind": kind, "params": params});
            v["target"] = target;
            e.apply_json(&v.to_string()).unwrap();
        }
        e.apply_json(
            &json!({"schema": "appname.command.v1",
                    "id": "0dec0000-0000-4000-8000-000000000015",
                    "kind": "edit.cut",
                    "target": {"layerId": layer.as_uuid().to_string()}})
            .to_string(),
        )
        .unwrap();
        // The discriminating call: pre-fix, PANIC here.
        let r = e.apply_json(&env(22, json!({"to": "gray_a"}))).unwrap();
        assert_eq!(r.warnings, ["color.mode.to_gray_lossy"]);
        // Post-convert: every buffer honors §6.4 (the write door already
        // debug-asserts; this is the release-visible check).
        let buf = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap();
        assert!(buf.validate_premultiplied().is_ok(), "§6.4 whole-surface");
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
        // Same class, C4's layer write (swept + fixed together): a §14.4
        // apply on the fractional-alpha u8 layer stays §6.4-clean too.
        e.apply_json(
            &json!({"schema": "appname.command.v1",
                    "id": "0dec0000-0000-4000-8000-000000000016",
                    "kind": "channel.apply_image",
                    "target": {"layerId": layer.as_uuid().to_string()},
                    "params": {"source": {"layer": "merged",
                                           "channel": "transparency"},
                               "blend": "add", "opacity": 0.5}})
            .to_string(),
        )
        .unwrap();
        assert!(e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap()
            .validate_premultiplied()
            .is_ok());
    }

    /// Review DEFECT-1 (fix round 2) regression: the depth-convert
    /// sibling of the §6.4 mint — f32 doc, fractional cut, THEN
    /// doc.depth_convert{u8}. Pre-fix, Document::convert_depth's
    /// f32→integer arm wrote enc(c) against untouched linear alpha and
    /// panicked at the same §6.4 door; the arm now marshals
    /// premul-normalized (enc(c/a)·a).
    #[test]
    fn fractional_cut_then_depth_convert_stays_premul_valid() {
        let mut e = ready("f32");
        let layer = e.document().unwrap().z_order_walk()[0];
        let mask = "0dec0000-0000-4000-8000-00000000bbbb";
        for (id, kind, target, params) in [
            (
                "0dec0000-0000-4000-8000-000000000031",
                "mask.add",
                json!({"layerId": layer.as_uuid().to_string()}),
                json!({"newMaskId": mask, "kind": "raster", "from": "reveal_all"}),
            ),
            (
                "0dec0000-0000-4000-8000-000000000032",
                "mask.edit",
                json!({"layerId": layer.as_uuid().to_string(), "maskId": mask}),
                json!({"edit": {"op": "fill", "value": 0.35}}),
            ),
            (
                "0dec0000-0000-4000-8000-000000000033",
                "mask.to_selection",
                json!({"layerId": layer.as_uuid().to_string(), "maskId": mask}),
                json!({"op": "replace"}),
            ),
            (
                "0dec0000-0000-4000-8000-000000000034",
                "mask.delete",
                json!({"layerId": layer.as_uuid().to_string(), "maskId": mask}),
                json!({}),
            ),
        ] {
            let mut v = json!({"schema": "appname.command.v1", "id": id,
                               "kind": kind, "params": params});
            v["target"] = target;
            e.apply_json(&v.to_string()).unwrap();
        }
        e.apply_json(
            &json!({"schema": "appname.command.v1",
                    "id": "0dec0000-0000-4000-8000-000000000035",
                    "kind": "edit.cut",
                    "target": {"layerId": layer.as_uuid().to_string()}})
            .to_string(),
        )
        .unwrap();
        // The discriminating call: pre-fix, PANIC at buffer.rs' §6.4 door.
        e.apply_json(
            &json!({"schema": "appname.command.v1",
                    "id": "0dec0000-0000-4000-8000-000000000036",
                    "kind": "doc.depth_convert",
                    "params": {"to": "u8"}})
            .to_string(),
        )
        .unwrap();
        let buf = e
            .document()
            .unwrap()
            .layer(layer)
            .unwrap()
            .pixels()
            .unwrap();
        assert_eq!(buf.kind(), crate::tile::channel::ChannelKind::U8);
        assert!(buf.validate_premultiplied().is_ok(), "§6.4 whole-surface");
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    /// The tasked cut audit: edit.cut itself is CLEAN — full coverage
    /// zeroes pixels EXACTLY (a = 0 ⟹ every component 0, the §6.4
    /// canonical form), fractional coverage scales premultiplied
    /// uniformly (c ≤ a throughout). The filed hypothesis (cut leaves
    /// color under zero alpha) is DISCONFIRMED by this scan; paste_into
    /// masks via attach_mask + §13.1 coverage (no alpha-clear write
    /// path — audited, no equivalent site).
    #[test]
    fn edit_cut_leaves_the_premul_invariant_intact_at_both_coverages() {
        for depth in ["u8", "f32"] {
            let mut e = ready(depth);
            let layer = e.document().unwrap().z_order_walk()[0];
            // Fractional selection over the left half, hard full
            // coverage over a strip (test door; no replay assert here —
            // the repro test above owns the journaled leg).
            {
                let doc = e.doc_mut_for_test();
                let sel = doc.selection_mut();
                *sel = crate::mask::Selection::new();
                for y in 0..32 {
                    for x in 0..16 {
                        sel.mask_mut()
                            .write_pixel_f32(IVec2::new(x, y), [0.35, 0.0, 0.0, 0.0]);
                    }
                    for x in 16..24 {
                        sel.mask_mut()
                            .write_pixel_f32(IVec2::new(x, y), [1.0, 0.0, 0.0, 0.0]);
                    }
                }
                sel.set_bounds(crate::geom::IRect::new(0, 0, 24, 32));
            }
            e.apply_json(
                &json!({"schema": "appname.command.v1",
                        "id": format!("0dec0000-0000-4000-8000-00000000c{:03x}",
                                      if depth == "u8" { 1 } else { 2 }),
                        "kind": "edit.cut",
                        "target": {"layerId": layer.as_uuid().to_string()}})
                .to_string(),
            )
            .unwrap();
            let buf = e
                .document()
                .unwrap()
                .layer(layer)
                .unwrap()
                .pixels()
                .unwrap();
            assert!(buf.validate_premultiplied().is_ok(), "{depth}: §6.4");
            for y in 0..32 {
                // Full coverage: canonical all-zero.
                let v = buf.read_pixel_f32(IVec2::new(20, y));
                assert_eq!(v, [0.0; 4], "{depth}: cov=1 zeroes exactly");
                // Fractional: uniform premul scale, c <= a, a != 0.
                let v = buf.read_pixel_f32(IVec2::new(4, y));
                assert!(v[3] > 0.0, "{depth}: fractional keeps alpha");
                for c in 0..3 {
                    assert!(
                        v[c] <= v[3] + 1e-6,
                        "{depth}: c <= a after fractional cut, got {v:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn rgba_to_gray_a_converts_warns_and_snapshots() {
        let mut e = ready("f32");
        let before = e.state_hash();
        let receipt = e.apply_json(&env(1, json!({"to": "gray_a"}))).unwrap();
        // §6.2 RGB→Gray: lossy, warned BEFORE execution (receipt).
        assert_eq!(receipt.warnings, ["color.mode.to_gray_lossy"]);
        let doc = e.document().unwrap();
        assert_eq!(doc.color().mode, ColorModelKind::GrayA);
        // White → gray 1.0 (premultiplied f32 GrayA: [g·a, a, 0, 0]).
        let v = px(&e, 4, 4);
        assert!((v[0] - 1.0).abs() < 1e-3, "white maps to white, got {v:?}");
        assert!((v[1] - 1.0).abs() < 1e-6, "alpha untouched (COPY_ALPHA)");
        // Class-C snapshot inverse: undo restores byte identity.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        assert_eq!(e.document().unwrap().color().mode, ColorModelKind::Rgba);
        e.redo().unwrap();
        assert_eq!(e.document().unwrap().color().mode, ColorModelKind::GrayA);
        // Replay determinism (CMM output is a pure function per the
        // O-COLOR flag elections — journal replay re-executes it).
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn rgba_to_lab_a_and_back_stays_near_neutral() {
        let mut e = ready("f32");
        let r1 = e.apply_json(&env(1, json!({"to": "lab_a"}))).unwrap();
        assert!(r1.warnings.is_empty(), "f32 RGB→Lab warns nothing (§6.2)");
        assert_eq!(e.document().unwrap().color().mode, ColorModelKind::LabA);
        // White in normalized Lab: L ≈ 1, a/b ≈ the 128/255 neutral.
        let v = px(&e, 4, 4);
        assert!((v[0] - 1.0).abs() < 1e-2, "L(white) ≈ 1, got {v:?}");
        assert!(
            (v[1] - 128.0 / 255.0).abs() < 2e-2,
            "a ≈ neutral, got {v:?}"
        );
        // Round trip back to rgba (lossy per §6.2 Lab→RGB: gamut clip).
        let r2 = e.apply_json(&env(2, json!({"to": "rgba"}))).unwrap();
        assert_eq!(r2.warnings, ["color.mode.lab_clip"]);
        let v = px(&e, 4, 4);
        for c in 0..3 {
            assert!((v[c] - 1.0).abs() < 1e-2, "white round-trips, got {v:?}");
        }
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn u8_depth_marshals_through_the_interim_encoding() {
        let mut e = ready("u8");
        // u8 RGB→Lab carries the §6.2 precision caveat.
        let r = e.apply_json(&env(1, json!({"to": "lab_a"}))).unwrap();
        assert_eq!(r.warnings, ["color.mode.lab_u8_precision"]);
        assert_eq!(
            e.document().unwrap().color().depth,
            crate::tile::channel::ChannelKind::U8
        );
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn identity_is_a_recorded_no_op() {
        let mut e = ready("f32");
        let before = e.state_hash();
        let journal = e.journal().len();
        let r = e.apply_json(&env(1, json!({"to": "rgba"}))).unwrap();
        assert!(r.warnings.is_empty());
        assert_eq!(e.state_hash(), before, "§6.2 same-mode identity");
        assert_eq!(e.journal().len(), journal + 1, "recorded (§40.1 push)");
    }

    #[test]
    fn mode_convert_adversarial_rejects() {
        let mut e = ready("f32");
        // Missing to → stage-3 required.
        let err = e.apply_json(&env(1, json!({}))).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        // 'rgb' is not a mode literal → stage-3 enum.
        let err = e.apply_json(&env(2, json!({"to": "rgb"}))).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // RGB profile for to:'gray_a' → profile_space_mismatch (the
        // working sRGB builtin id is an RGB-space profile).
        let srgb = crate::engine::color_glue::builtin_profile_id(appname_color::BuiltinSpace::Srgb);
        let err = e
            .apply_json(&env(
                3,
                json!({"to": "gray_a", "profile": srgb.0.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);
        // Same gate fires for cmyk_a BEFORE the substrate NI refusal
        // (contract invalid case 3).
        let err = e
            .apply_json(&env(
                4,
                json!({"to": "cmyk_a", "profile": srgb.0.to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);
        // cmyk_a without a profile → the typed substrate refusal.
        let err = e.apply_json(&env(5, json!({"to": "cmyk_a"}))).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // Unresolvable profile id → bad_target.
        let err = e
            .apply_json(&env(
                6,
                json!({"to": "gray_a", "profile": "0badbad0-0000-4000-8000-00000000dead"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // flatten:true → typed NI (compositor flatten row).
        let err = e
            .apply_json(&env(7, json!({"to": "gray_a", "flatten": true})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    #[test]
    fn masks_channels_and_selection_stay_gray_untouched() {
        let mut e = ready("f32");
        // A named channel with coverage 0.35 (journaled recipe from the
        // channels battery, condensed: add + fill).
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0dec0000-0000-4000-8000-000200000001",
                "kind": "channel.add",
                "params": {"newChannelId": "feedfeed-0000-4000-8000-000000000001"},
            })
            .to_string(),
        )
        .unwrap();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "0dec0000-0000-4000-8000-000200000002",
                "kind": "channel.edit",
                "target": {"channelId": "feedfeed-0000-4000-8000-000000000001"},
                "params": {"edit": {"op": "fill", "value": 0.35}},
            })
            .to_string(),
        )
        .unwrap();
        let chan_id = crate::graph::id::ChannelId::from_uuid(
            uuid::Uuid::parse_str("feedfeed-0000-4000-8000-000000000001").unwrap(),
        );
        let chan_before = e
            .document()
            .unwrap()
            .channel(chan_id)
            .unwrap()
            .buffer()
            .region_hash();
        e.apply_json(&env(10, json!({"to": "gray_a"}))).unwrap();
        assert_eq!(
            e.document()
                .unwrap()
                .channel(chan_id)
                .unwrap()
                .buffer()
                .region_hash(),
            chan_before,
            "§6.2 [NORMATIVE]: channels are NEVER converted by a mode change"
        );
    }
}
