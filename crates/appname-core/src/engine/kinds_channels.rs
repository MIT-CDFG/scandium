// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: contracts `selection.save` / `selection.load` / `channel.add` /
//! `channel.edit` / `channel.delete` / `channel.duplicate`; ES §14.1
//! (named-channel model, component channels as derived views), §14.2
//! (alpha channels: selection round-trip, COW copy — zero pixel copies),
//! §14.3 (spot reserved Post-1.0), §13.2 (the ONE boolean-combine math),
//! §40.4 (family rules: T topic:"channels", never lock-gated except
//! component write-through, component channels structurally read-only);
//! doors C2–C3 of reports/O-COLOR-log.md (CP-1.0 wave-1 entry).
//!
//! CP-1.0 door slice 3, group 2. Divergences recorded in the lane log:
//!
//! * Component channels: core never MINTS component-channel ids
//!   (`ChannelSet::component` is a derived view, §14.1 — the group-1
//!   residue's `channel_node.rs` records the same), so every contract
//!   case "channelId resolves to a component channel →
//!   `target_kind_mismatch`" is structurally unreachable wire-side; an
//!   unresolvable channel uuid answers `bad_target` (stage 4). The
//!   mode-gated component SELECTORS (`c`/`m`/`y`/`k`/`l`/`a`/`b_lab`)
//!   do answer `target_kind_mismatch` in the apply-image grammar (group
//!   4), which is where typed component addressing exists.
//! * `channel.duplicate` `toDocument`: the engine hosts ONE document
//!   (§10.1 handle model; the session/multi-doc host is O-SCRIPT/O-UI
//!   surface). A `toDocument` equal to this document's id resolves
//!   (dims trivially identical); any other id answers `bad_target`
//!   naming the single-document posture. `error.command.size_mismatch`
//!   is minted (error.rs) and becomes reachable when a session host
//!   exists — recorded, not silent.
//! * Dynamic name defaults ("Selection N" / "Alpha N" / "<name> copy"
//!   §10.3): headless core has no localization catalog; the engine uses
//!   the English fixed stems with N = named-channel count + 1
//!   (deterministic, replay-stable). The i18n'd rendering is the UI's
//!   dialog-capture job (`actionRecording`: committed params carry the
//!   final name).
//! * `channel.edit` `edit.op:"gaussian_blur"` answers typed NI naming
//!   the O-TOOLS feather/refinement seam — the `mask.edit` precedent
//!   verbatim (adjacency flagged to O-TOOLS in the lane log).
//! * `selection.load` op:"replace" executes as a PIXEL-WISE §13.2
//!   combine over the operative rect, not §14.2's COW Arc copy-back
//!   (review MINOR-2, recorded): the `invert` arm and the three-source
//!   grammar need a value pass anyway, and replace = `b` is
//!   bit-identical to the copied values — the [NORMATIVE] guarantee
//!   holds; only the Arc-sharing optimization is not taken on the load
//!   direction (save's replace arm DOES take it).

use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::engine::error::{EngineError, ErrorCode};
use crate::engine::kind_impl::{KindContext, KindImpl, KindOutcome};
use crate::engine::mask_ops::SelectOp;
use crate::geom::{IRect, IVec2};
use crate::graph::id::ChannelId;

pub struct SelectionSaveKind;
pub struct SelectionLoadKind;
pub struct ChannelAddKind;
pub struct ChannelEditKind;
pub struct ChannelDeleteKind;
pub struct ChannelDuplicateKind;

pub(crate) fn impls() -> Vec<Arc<dyn KindImpl>> {
    vec![
        Arc::new(SelectionSaveKind),
        Arc::new(SelectionLoadKind),
        Arc::new(ChannelAddKind),
        Arc::new(ChannelEditKind),
        Arc::new(ChannelDeleteKind),
        Arc::new(ChannelDuplicateKind),
    ]
}

// ---------------------------------------------------------------------------
// Shared stage-4/6 helpers.
// ---------------------------------------------------------------------------

fn param_uuid(cx: &KindContext<'_>, name: &str) -> Result<Option<Uuid>, EngineError> {
    match cx.params().get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Uuid::parse_str(s).map(Some).map_err(|_| {
            EngineError::range(name, format!("`{name}` is not a valid UUID: \"{s}\""))
        }),
        Some(other) => Err(EngineError::range(
            name,
            format!("`{name}` must be a UUID string, got {other}"),
        )),
    }
}

fn op_param(cx: &KindContext<'_>) -> Result<SelectOp, EngineError> {
    match cx.params().get("op").and_then(Value::as_str) {
        None => Ok(SelectOp::Replace), // schema default
        Some(s) => SelectOp::from_wire(s)
            .ok_or_else(|| EngineError::range("op", format!("unknown op \"{s}\" (§13.2)"))),
    }
}

/// Stage-4 named-channel resolution from `target.channelId` (§40.4).
fn resolve_channel_target(cx: &KindContext<'_>) -> Result<ChannelId, EngineError> {
    let channel = cx
        .envelope()
        .target
        .as_ref()
        .and_then(|t| t.channel_id)
        .ok_or_else(|| {
            EngineError::bad_target("this kind requires target.channelId (§40.4 family rule)")
        })?;
    let id = ChannelId::from_uuid(channel);
    if cx.document().channel(id).is_none() {
        // Component channels have no independent identity (§14.1 derived
        // view) — core mints no component ids, so an unknown uuid is a
        // stage-4 bad_target (divergence recorded in the module docs).
        return Err(EngineError::bad_target(format!(
            "target.channelId {channel} does not resolve to a named channel (§14.1)"
        )));
    }
    Ok(id)
}

/// §14.3 spot-reserved gate: `ink`/`solidity` presence in v1 answers the
/// typed classification (`error.unsupported.spot_channels`, §40.4 —
/// "classification, not a promise").
fn refuse_spot_params(cx: &KindContext<'_>) -> Result<(), EngineError> {
    for key in ["ink", "solidity"] {
        if cx.params().contains_key(key) {
            return Err(EngineError::with_field(
                ErrorCode::SpotChannelsUnsupported,
                key,
                "spot-channel params (`ink`/`solidity`) are reserved Post-1.0 (§14.3); \
                 v1 refuses typed (§40.4)",
            ));
        }
    }
    Ok(())
}

fn require_active_selection(cx: &KindContext<'_>) -> Result<(), EngineError> {
    if !cx.document().selection().active() {
        return Err(EngineError::new(
            ErrorCode::SelectionRequired,
            "an active selection is required (§40.4)",
        ));
    }
    Ok(())
}

/// Read a single-channel coverage plane over `rect` from a source surface
/// reader, row-major.
fn read_plane(rect: IRect, mut read: impl FnMut(IVec2) -> f32) -> Vec<f32> {
    let mut values = Vec::with_capacity((rect.width() as usize) * (rect.height() as usize));
    for y in rect.min.y..rect.max.y {
        for x in rect.min.x..rect.max.x {
            values.push(read(IVec2::new(x, y)).clamp(0.0, 1.0));
        }
    }
    values
}

/// The post-combine bounds maintenance split on an arbitrary coverage
/// surface (the `apply_mask_asset_to_selection` pattern, §13.1).
fn maintain_surface_bounds(
    cx: &mut KindContext<'_>,
    surface: Uuid,
    rect: IRect,
    op: SelectOp,
) -> Result<(), EngineError> {
    match op {
        SelectOp::Replace => cx.set_surface_bounds(surface, rect),
        SelectOp::Add => cx.grow_surface_bounds(surface, rect),
        SelectOp::Subtract | SelectOp::Intersect => cx.recompute_surface_bounds(surface),
    }
}

// ---------------------------------------------------------------------------
// selection.save
// ---------------------------------------------------------------------------

impl KindImpl for SelectionSaveKind {
    fn kind(&self) -> &'static str {
        "selection.save"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        require_active_selection(cx)?;
        let op = op_param(cx)?;
        let sel_bounds = cx.document().selection().bounds();
        let sel_surface = cx.selection_surface();

        if let Some(channel) = param_uuid(cx, "channelId")? {
            // Merge into an existing alpha channel (§14.2).
            let id = ChannelId::from_uuid(channel);
            if cx.document().channel(id).is_none() {
                return Err(EngineError::bad_target(format!(
                    "channelId {channel} does not resolve to a named channel (§14.1; \
                     component channels have no independent identity)"
                )));
            }
            let surface = id.as_uuid();
            if matches!(op, SelectOp::Replace) {
                // §14.2 zero-pixel-copy arm: COW Arc copy of the
                // selection-surface tiles (§34 asserts per-tile ptr_eq).
                cx.copy_coverage_tiles_cow(sel_surface, surface, sel_bounds)?;
            } else {
                // Non-replace ops = core's EXISTING §13.2 float combine
                // (door C2: "do not duplicate the math").
                let chan_bounds = cx
                    .document()
                    .channel(id)
                    .expect("resolved above")
                    .buffer()
                    .bounds();
                let rect = sel_bounds.union(&chan_bounds);
                if !rect.is_empty() {
                    let values = {
                        let sel = cx.document().selection();
                        read_plane(rect, |p| sel.mask().read_pixel_f32(p)[0])
                    };
                    cx.combine_coverage_plane(surface, rect, &values, op)?;
                    maintain_surface_bounds(cx, surface, sel_bounds, op)?;
                }
            }
            return Ok(KindOutcome::default());
        }

        // Creating: newChannelId is REQUIRED (the contract's if/then
        // cross-field — stage-3's interpreter has no if/then arm, so the
        // arm enforces it: §40.1 taxonomy missing_field).
        let Some(new_id) = param_uuid(cx, "newChannelId")? else {
            return Err(EngineError::missing("newChannelId"));
        };
        let name = match cx.params().get("name").and_then(Value::as_str) {
            Some(n) => n.to_string(),
            // Dynamic default "Selection N" (§10.3) — headless English
            // stem, N = named-channel count + 1 (module-docs divergence).
            None => format!("Selection {}", cx.document().channel_count() + 1),
        };
        let id = cx.insert_named_channel(ChannelId::from_uuid(new_id), name)?;
        let surface = id.as_uuid();
        if matches!(op, SelectOp::Replace | SelectOp::Add) {
            // Onto an empty channel, replace ≡ add ≡ the selection itself:
            // take the §14.2 zero-pixel-copy arm.
            cx.copy_coverage_tiles_cow(sel_surface, surface, sel_bounds)?;
        }
        // Subtract/intersect against an empty channel yield the empty
        // channel exactly (max(0, 0−b) = 0, min(0, b) = 0, §13.2) — the
        // create records; no coverage write is the exact result.
        Ok(KindOutcome {
            created_ids: vec![surface],
        })
    }
}

// ---------------------------------------------------------------------------
// selection.load
// ---------------------------------------------------------------------------

impl KindImpl for SelectionLoadKind {
    fn kind(&self) -> &'static str {
        "selection.load"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let op = op_param(cx)?;
        let invert = cx
            .params()
            .get("invert")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let source = cx
            .params()
            .get("source")
            .and_then(Value::as_object)
            .ok_or_else(|| EngineError::missing("source"))?;
        let get_uuid = |key: &str| -> Result<Option<Uuid>, EngineError> {
            match source.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Uuid::parse_str(s).map(Some).map_err(|_| {
                    EngineError::range(
                        "source",
                        format!("`source.{key}` is not a valid UUID: \"{s}\""),
                    )
                }),
                Some(other) => Err(EngineError::range(
                    "source",
                    format!("`source.{key}` must be a UUID string, got {other}"),
                )),
            }
        };

        // Stage 4: resolve the one §10.3 source alternative (stage-3
        // oneOf guaranteed exactly one key present).
        enum Src {
            Channel(ChannelId),
            Mask(crate::graph::id::MaskId),
            LayerAlpha(crate::graph::id::LayerId),
        }
        let (src, src_bounds) = if let Some(c) = get_uuid("channelId")? {
            let id = ChannelId::from_uuid(c);
            let node = cx.document().channel(id).ok_or_else(|| {
                EngineError::bad_target(format!(
                    "source.channelId {c} does not resolve (§14.1; component channels load \
                     via the §14.1 soloing rule when component ids exist — none are minted)"
                ))
            })?;
            (Src::Channel(id), node.buffer().bounds())
        } else if let Some(m) = get_uuid("maskId")? {
            let id = crate::graph::id::MaskId::from_uuid(m);
            let node = cx.document().mask(id).ok_or_else(|| {
                EngineError::bad_target(format!("source.maskId {m} does not resolve"))
            })?;
            (Src::Mask(id), node.coverage().bounds())
        } else if let Some(l) = get_uuid("layerAlpha")? {
            let id = crate::graph::id::LayerId::from_uuid(l);
            let node = cx.document().layer(id).ok_or_else(|| {
                EngineError::bad_target(format!("source.layerAlpha {l} does not resolve"))
            })?;
            let Some(px) = node.pixels() else {
                return Err(EngineError::bad_target(
                    "source.layerAlpha must name a raster layer (alpha source)",
                ));
            };
            (Src::LayerAlpha(id), px.bounds())
        } else {
            return Err(EngineError::missing(
                "source.channelId|source.maskId|source.layerAlpha",
            ));
        };

        // Coverage domain: `invert` (1 − v BEFORE the op, §40.4) turns
        // absent coverage into 1, so the operative extent is the canvas
        // rect ∪ source bounds ∪ current selection bounds.
        let canvas = cx.document().geometry().canvas_rect();
        let rect = canvas
            .union(&src_bounds)
            .union(&cx.document().selection().bounds());
        let flip = |v: f32| if invert { 1.0 - v } else { v };
        let values = {
            let doc = cx.document();
            match &src {
                Src::Channel(id) => {
                    let buf = doc.channel(*id).expect("resolved").buffer();
                    read_plane(rect, |p| flip(buf.read_pixel_f32(p)[0]))
                }
                Src::Mask(id) => {
                    let cov = doc.mask(*id).expect("resolved").coverage();
                    read_plane(rect, |p| flip(cov.read_pixel_f32(p)[0]))
                }
                Src::LayerAlpha(id) => {
                    let px = doc.layer(*id).expect("resolved").pixels().expect("raster");
                    read_plane(rect, |p| flip(px.read_pixel_f32(p)[3]))
                }
            }
        };
        let sel_surface = cx.selection_surface();
        cx.combine_coverage_plane(sel_surface, rect, &values, op)?;
        match op {
            SelectOp::Replace => cx.set_selection_bounds(rect)?,
            SelectOp::Add => cx.grow_selection_bounds(rect)?,
            SelectOp::Subtract | SelectOp::Intersect => cx.recompute_selection_bounds()?,
        }
        cx.sync_selection_bounds();
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// channel.add
// ---------------------------------------------------------------------------

impl KindImpl for ChannelAddKind {
    fn kind(&self) -> &'static str {
        "channel.add"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        refuse_spot_params(cx)?;
        let from = cx
            .params()
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or("empty")
            .to_string();
        let from = from.as_str();
        if from == "selection" {
            require_active_selection(cx)?;
        } else if from != "empty" {
            return Err(EngineError::range(
                "from",
                format!("unknown source \"{from}\""),
            ));
        }
        let Some(new_id) = param_uuid(cx, "newChannelId")? else {
            return Err(EngineError::missing("newChannelId"));
        };
        let name = match cx.params().get("name").and_then(Value::as_str) {
            Some(n) => n.to_string(),
            None => format!("Alpha {}", cx.document().channel_count() + 1),
        };
        let id = cx.insert_named_channel(ChannelId::from_uuid(new_id), name)?;
        let surface = id.as_uuid();
        if from == "selection" {
            let sel_surface = cx.selection_surface();
            let sel_bounds = cx.document().selection().bounds();
            cx.copy_coverage_tiles_cow(sel_surface, surface, sel_bounds)?;
        }
        Ok(KindOutcome {
            created_ids: vec![surface],
        })
    }
}

// ---------------------------------------------------------------------------
// channel.edit
// ---------------------------------------------------------------------------

impl KindImpl for ChannelEditKind {
    fn kind(&self) -> &'static str {
        "channel.edit"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let id = resolve_channel_target(cx)?;
        refuse_spot_params(cx)?;
        // Alpha-channel targets are never lock-gated (§40.4 note; the
        // component write-through lock rows are unreachable — module docs).
        if let Some(name) = cx.params().get("name").and_then(Value::as_str) {
            cx.rename_channel(id, name.to_string())?;
        }
        let Some(edit) = cx.params().get("edit") else {
            return Ok(KindOutcome::default()); // rename-only (or no-op push)
        };
        let edit = edit
            .as_object()
            .ok_or_else(|| EngineError::range("edit", "`edit` must be an object"))?;
        let op = edit
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::missing("edit.op"))?;
        let canvas = cx.document().geometry().canvas_rect();
        let surface = id.as_uuid();
        // §40.4 "gates coverage": surface edits apply through sel(p) —
        // the shared coverage_edit door (channels joined it this slice).
        let nonzero = match op {
            "fill" => {
                let v = edit
                    .get("value")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| EngineError::missing("edit.value"))?
                    .clamp(0.0, 1.0) as f32;
                crate::engine::kinds::coverage_edit(cx, surface, canvas, true, &mut |_p, _old| v)?.1
            }
            "invert" => {
                crate::engine::kinds::coverage_edit(cx, surface, canvas, true, &mut |_p, old| {
                    1.0 - old
                })?
                .1
            }
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
                        "inWhite - inBlack must be >= 1/255 (§40.4 cross-field, the §17.2 shape)",
                    ));
                }
                crate::engine::kinds::coverage_edit(cx, surface, canvas, true, &mut |_p, old| {
                    let t = ((old - in_b) / (in_w - in_b)).clamp(0.0, 1.0);
                    out_b + t.powf(1.0 / gamma) * (out_w - out_b)
                })?
                .1
            }
            "gaussian_blur" => {
                return Err(EngineError::new(
                    ErrorCode::NotImplemented,
                    "channel gaussian_blur arrives with the O-TOOLS feather/refinement seam \
                     (§13.2; the mask.edit precedent — adjacency flagged in the lane log)",
                ))
            }
            other => {
                return Err(EngineError::range(
                    "edit.op",
                    format!("unknown op \"{other}\""),
                ))
            }
        };
        cx.set_surface_bounds(surface, nonzero)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// channel.delete
// ---------------------------------------------------------------------------

impl KindImpl for ChannelDeleteKind {
    fn kind(&self) -> &'static str {
        "channel.delete"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let id = resolve_channel_target(cx)?;
        cx.remove_named_channel(id)?;
        Ok(KindOutcome::default())
    }
}

// ---------------------------------------------------------------------------
// channel.duplicate
// ---------------------------------------------------------------------------

impl KindImpl for ChannelDuplicateKind {
    fn kind(&self) -> &'static str {
        "channel.duplicate"
    }

    fn apply(&self, cx: &mut KindContext<'_>) -> Result<KindOutcome, EngineError> {
        let src = resolve_channel_target(cx)?;
        if let Some(to_doc) = param_uuid(cx, "toDocument")? {
            if to_doc != cx.document().id().as_uuid() {
                // Single-document engine posture (module docs): the id
                // does not resolve to an OPEN document here. The §40.4
                // dims gate (`error.command.size_mismatch`) becomes
                // reachable with the session host — code minted, arm
                // recorded.
                return Err(EngineError::bad_target(format!(
                    "toDocument {to_doc} does not resolve to an open document (the engine \
                     hosts one document; §40.4 cross-document duplication needs the session \
                     host — divergence recorded)"
                )));
            }
            // toDocument == this document: identical pixel dimensions
            // trivially hold; the in-document arm below applies.
        }
        let Some(new_id) = param_uuid(cx, "newChannelId")? else {
            return Err(EngineError::missing("newChannelId"));
        };
        let (src_name, src_bounds) = {
            let node = cx.document().channel(src).expect("resolved");
            (node.name().to_string(), node.buffer().bounds())
        };
        let name = match cx.params().get("name").and_then(Value::as_str) {
            Some(n) => n.to_string(),
            // "<source> copy" (§10.3 dynamic default; headless English
            // stem — module-docs divergence).
            None => format!("{src_name} copy"),
        };
        let id = cx.insert_named_channel(ChannelId::from_uuid(new_id), name)?;
        let surface = id.as_uuid();
        // §14.2 COW discipline: the duplicate shares tile Arcs.
        cx.copy_coverage_tiles_cow(src.as_uuid(), surface, src_bounds)?;
        Ok(KindOutcome {
            created_ids: vec![surface],
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use std::sync::Arc;
    use uuid::Uuid;

    use crate::engine::error::ErrorCode;
    use crate::engine::kind_impl::EngineAssembly;
    use crate::engine::Engine;
    use crate::geom::{IRect, IVec2};
    use crate::graph::id::ChannelId;
    use crate::mask::Selection;

    fn assembly() -> EngineAssembly {
        crate::engine::kinds::register(EngineAssembly::new())
    }

    fn ready() -> Engine {
        let mut e = assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "c4a00000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 64, "height": 64, "background": "white"},
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    fn env(n: u32, kind: &str, target: Value, params: Value) -> String {
        let mut v = json!({
            "schema": "appname.command.v1",
            "id": format!("c4a00000-0000-4000-8000-0001{n:08x}"),
            "kind": kind,
            "params": params,
        });
        if !target.is_null() {
            v["target"] = target;
        }
        v.to_string()
    }

    /// JOURNALED selection recipe (replay-clean): clear any active
    /// selection (hide-all mask → to_selection replace — mask.edit gates
    /// through sel(p) otherwise), then reveal-all mask filled to `v`,
    /// loaded as the selection (replace). Scaffold masks are deleted so
    /// later assertions see only channel state.
    fn journaled_selection(e: &mut Engine, n_base: u32, v: f64) {
        let layer = e.document().unwrap().z_order_walk()[0];
        let ltarget = json!({"layerId": layer.as_uuid().to_string()});
        if e.document().unwrap().selection().active() {
            let zero =
                Uuid::parse_str(&format!("c4a0bbbb-0000-4000-8000-0002{n_base:08x}")).unwrap();
            let tz = json!({"layerId": layer.as_uuid().to_string(), "maskId": zero.to_string()});
            e.apply_json(&env(
                n_base + 4,
                "mask.add",
                ltarget.clone(),
                json!({"newMaskId": zero.to_string(), "kind": "raster", "from": "hide_all"}),
            ))
            .unwrap();
            e.apply_json(&env(
                n_base + 5,
                "mask.to_selection",
                tz.clone(),
                json!({"op": "replace"}),
            ))
            .unwrap();
            e.apply_json(&env(n_base + 6, "mask.delete", tz, json!({})))
                .unwrap();
        }
        let mask = Uuid::parse_str(&format!("c4a0aaaa-0000-4000-8000-0002{n_base:08x}")).unwrap();
        let t = json!({"layerId": layer.as_uuid().to_string(), "maskId": mask.to_string()});
        e.apply_json(&env(
            n_base,
            "mask.add",
            ltarget,
            json!({"newMaskId": mask.to_string(), "kind": "raster", "from": "reveal_all"}),
        ))
        .unwrap();
        e.apply_json(&env(
            n_base + 1,
            "mask.edit",
            t.clone(),
            json!({"edit": {"op": "fill", "value": v}}),
        ))
        .unwrap();
        e.apply_json(&env(
            n_base + 2,
            "mask.to_selection",
            t.clone(),
            json!({"op": "replace"}),
        ))
        .unwrap();
        // Drop the scaffold mask so later saves see only channel state.
        e.apply_json(&env(n_base + 3, "mask.delete", t, json!({})))
            .unwrap();
    }

    fn sel_at(e: &Engine, x: i32, y: i32) -> f32 {
        e.document()
            .unwrap()
            .selection()
            .mask()
            .read_pixel_f32(IVec2::new(x, y))[0]
    }

    fn chan_at(e: &Engine, id: ChannelId, x: i32, y: i32) -> f32 {
        e.document()
            .unwrap()
            .channel(id)
            .unwrap()
            .buffer()
            .read_pixel_f32(IVec2::new(x, y))[0]
    }

    const NEW_CHAN: &str = "feedfeed-0000-4000-8000-000000000001";

    #[test]
    fn selection_save_creates_channel_with_cow_tiles_and_round_trips() {
        let mut e = ready();
        journaled_selection(&mut e, 10, 0.4);
        assert!(e.document().unwrap().selection().active());
        let before = e.state_hash();

        let receipt = e
            .apply_json(&env(
                20,
                "selection.save",
                Value::Null,
                json!({"newChannelId": NEW_CHAN}),
            ))
            .unwrap();
        let id = ChannelId::from_uuid(Uuid::parse_str(NEW_CHAN).unwrap());
        assert_eq!(receipt.created_ids, vec![id.as_uuid()]);
        let doc = e.document().unwrap();
        assert_eq!(doc.channel_count(), 1);
        assert_eq!(doc.channel(id).unwrap().name(), "Selection 1");
        // §14.2 zero pixel copies — §34 per-tile Arc::ptr_eq.
        let sel_tiles: Vec<_> = doc.selection().mask().tiles().iter().collect();
        assert!(!sel_tiles.is_empty(), "selection carries coverage tiles");
        for (coord, arc) in &sel_tiles {
            let chan_arc = doc
                .channel(id)
                .unwrap()
                .buffer()
                .tiles()
                .get(*coord)
                .expect("channel shares the coord set");
            assert!(
                Arc::ptr_eq(arc, chan_arc),
                "§14.2 COW: channel tile {coord:?} is the SAME Arc"
            );
        }
        // The selection itself is unchanged; the state hash moved (channel
        // fold joins when present).
        assert!((sel_at(&e, 5, 5) - 0.4).abs() < 1e-6);
        assert_ne!(e.state_hash(), before);

        // Undo removes the channel; redo restores THE SAME node.
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        assert_eq!(e.document().unwrap().channel_count(), 0);
        e.redo().unwrap();
        assert_eq!(e.document().unwrap().channel_count(), 1);

        // Mutate the selection (journaled), then load the channel back.
        journaled_selection(&mut e, 30, 0.9);
        assert!((sel_at(&e, 5, 5) - 0.9).abs() < 1e-6);
        e.apply_json(&env(
            40,
            "selection.load",
            Value::Null,
            json!({"source": {"channelId": NEW_CHAN}}),
        ))
        .unwrap();
        assert!(
            (sel_at(&e, 5, 5) - 0.4).abs() < 1e-6,
            "save → load round-trips the stored coverage (§14.2)"
        );

        // Replay determinism: the WHOLE journal (selection came through
        // commands) reproduces the state hash.
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn selection_save_merges_with_the_exact_13_2_formulas() {
        let mut e = ready();
        journaled_selection(&mut e, 10, 0.5);
        e.apply_json(&env(
            20,
            "selection.save",
            Value::Null,
            json!({"newChannelId": NEW_CHAN}),
        ))
        .unwrap();
        let id = ChannelId::from_uuid(Uuid::parse_str(NEW_CHAN).unwrap());
        // New selection 0.8; intersect: min(0.5, 0.8) = 0.5; add:
        // min(1, .5+.8) = 1; subtract: max(0, .5−.8) = 0.
        journaled_selection(&mut e, 30, 0.8);
        e.apply_json(&env(
            40,
            "selection.save",
            Value::Null,
            json!({"channelId": NEW_CHAN, "op": "intersect"}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 3, 3) - 0.5).abs() < 1e-6, "min(a, b)");
        e.apply_json(&env(
            41,
            "selection.save",
            Value::Null,
            json!({"channelId": NEW_CHAN, "op": "add"}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 3, 3) - 1.0).abs() < 1e-6, "min(1, a+b)");
        e.apply_json(&env(
            42,
            "selection.save",
            Value::Null,
            json!({"channelId": NEW_CHAN, "op": "subtract"}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 3, 3) - 0.2).abs() < 1e-6, "max(0, a-b)");
        // Replace overwrites with the live selection.
        e.apply_json(&env(
            43,
            "selection.save",
            Value::Null,
            json!({"channelId": NEW_CHAN, "op": "replace"}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 3, 3) - 0.8).abs() < 1e-6, "replace = b");
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn selection_save_adversarial_rejects() {
        let mut e = ready();
        // No active selection → selection_required (stage 4).
        let err = e
            .apply_json(&env(
                1,
                "selection.save",
                Value::Null,
                json!({"newChannelId": NEW_CHAN}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        journaled_selection(&mut e, 10, 0.5);
        // Creating without newChannelId → the contract's if/then
        // cross-field (arm-enforced missing_field).
        let err = e
            .apply_json(&env(
                2,
                "selection.save",
                Value::Null,
                json!({"name": "sky"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        // newChannelId already in an arena (the background layer's id)
        // → §9.6 duplicate_id at the door.
        let layer = e.document().unwrap().z_order_walk()[0];
        let err = e
            .apply_json(&env(
                3,
                "selection.save",
                Value::Null,
                json!({"newChannelId": layer.as_uuid().to_string()}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);
        // channelId that does not resolve → bad_target (stage 4; the
        // contract's component-channel target_kind_mismatch row is
        // structurally unreachable — module-docs divergence).
        let err = e
            .apply_json(&env(
                4,
                "selection.save",
                Value::Null,
                json!({"channelId": "0badbad0-0000-4000-8000-00000000dead"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // Unknown op enum → stage-3 out_of_range.
        let err = e
            .apply_json(&env(
                5,
                "selection.save",
                Value::Null,
                json!({"newChannelId": NEW_CHAN, "op": "xor"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn selection_load_sources_ops_and_invert() {
        let mut e = ready();
        let layer = e.document().unwrap().z_order_walk()[0];
        journaled_selection(&mut e, 10, 0.25);
        e.apply_json(&env(
            20,
            "selection.save",
            Value::Null,
            json!({"newChannelId": NEW_CHAN}),
        ))
        .unwrap();
        // invert: 1 − v BEFORE the op (replace ⇒ 0.75 everywhere in
        // canvas).
        e.apply_json(&env(
            21,
            "selection.load",
            Value::Null,
            json!({"source": {"channelId": NEW_CHAN}, "op": "replace", "invert": true}),
        ))
        .unwrap();
        assert!((sel_at(&e, 5, 5) - 0.75).abs() < 1e-6, "invert precedes op");
        // layerAlpha source: white background alpha = 1 everywhere;
        // intersect keeps 0.75.
        e.apply_json(&env(
            22,
            "selection.load",
            Value::Null,
            json!({"source": {"layerAlpha": layer.as_uuid().to_string()}, "op": "intersect"}),
        ))
        .unwrap();
        assert!((sel_at(&e, 5, 5) - 0.75).abs() < 1e-6, "min(0.75, 1)");
        // subtract the channel (0.25): 0.75 − 0.25 = 0.5.
        e.apply_json(&env(
            23,
            "selection.load",
            Value::Null,
            json!({"source": {"channelId": NEW_CHAN}, "op": "subtract"}),
        ))
        .unwrap();
        assert!((sel_at(&e, 5, 5) - 0.5).abs() < 1e-6, "max(0, a-b)");
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn selection_load_adversarial_rejects() {
        let mut e = ready();
        // Empty source object → stage-3 property-level oneOf →
        // missing_field.
        let err = e
            .apply_json(&env(
                1,
                "selection.load",
                Value::Null,
                json!({"source": {}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        // Unresolvable channel → bad_target (stage 4).
        let err = e
            .apply_json(&env(
                2,
                "selection.load",
                Value::Null,
                json!({"source": {"channelId": "0badbad0-0000-4000-8000-00000000dead"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // op 'xor' → stage-3 enum out_of_range.
        journaled_selection(&mut e, 10, 0.5);
        e.apply_json(&env(
            20,
            "selection.save",
            Value::Null,
            json!({"newChannelId": NEW_CHAN}),
        ))
        .unwrap();
        let err = e
            .apply_json(&env(
                21,
                "selection.load",
                Value::Null,
                json!({"source": {"channelId": NEW_CHAN}, "op": "xor"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn channel_add_empty_selection_and_rejects() {
        let mut e = ready();
        // from:'empty' with a default name.
        e.apply_json(&env(
            1,
            "channel.add",
            Value::Null,
            json!({"newChannelId": NEW_CHAN}),
        ))
        .unwrap();
        let id = ChannelId::from_uuid(Uuid::parse_str(NEW_CHAN).unwrap());
        assert_eq!(e.document().unwrap().channel(id).unwrap().name(), "Alpha 1");
        assert_eq!(chan_at(&e, id, 5, 5), 0.0, "empty channel reads 0");
        // spot params → typed classification (§14.3).
        let err = e
            .apply_json(&env(
                2,
                "channel.add",
                Value::Null,
                json!({"newChannelId": "feedfeed-0000-4000-8000-000000000002",
                       "ink": {"model": "cmyk", "comps": [1, 0, 0, 0],
                               "profile": "eeeeeeee-0000-4000-8000-000000000001"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SpotChannelsUnsupported);
        // from:'selection' without one → selection_required.
        let err = e
            .apply_json(&env(
                3,
                "channel.add",
                Value::Null,
                json!({"newChannelId": "feedfeed-0000-4000-8000-000000000002", "from": "selection"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SelectionRequired);
        // Duplicate creation id → §9.6.
        let err = e
            .apply_json(&env(
                4,
                "channel.add",
                Value::Null,
                json!({"newChannelId": NEW_CHAN}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);
        // Bad from enum → stage-3 out_of_range.
        let err = e
            .apply_json(&env(
                5,
                "channel.add",
                Value::Null,
                json!({"newChannelId": "feedfeed-0000-4000-8000-000000000002", "from": "path"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        // from:'selection' copies the live selection (COW).
        journaled_selection(&mut e, 10, 0.6);
        e.apply_json(&env(
            20,
            "channel.add",
            Value::Null,
            json!({"newChannelId": "feedfeed-0000-4000-8000-000000000002", "from": "selection"}),
        ))
        .unwrap();
        let id2 =
            ChannelId::from_uuid(Uuid::parse_str("feedfeed-0000-4000-8000-000000000002").unwrap());
        assert!((chan_at(&e, id2, 5, 5) - 0.6).abs() < 1e-6);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn channel_edit_ops_rename_gating_and_rejects() {
        let mut e = ready();
        e.apply_json(&env(
            1,
            "channel.add",
            Value::Null,
            json!({"newChannelId": NEW_CHAN, "name": "sky"}),
        ))
        .unwrap();
        let id = ChannelId::from_uuid(Uuid::parse_str(NEW_CHAN).unwrap());
        let t = json!({"channelId": NEW_CHAN});
        // fill 0.6 (no selection: whole canvas).
        e.apply_json(&env(
            2,
            "channel.edit",
            t.clone(),
            json!({"edit": {"op": "fill", "value": 0.6}}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 5, 5) - 0.6).abs() < 1e-6);
        // levels: inBlack .2 inWhite .8 → t = (0.6−0.2)/0.6 = 0.6667.
        e.apply_json(&env(
            3,
            "channel.edit",
            t.clone(),
            json!({"edit": {"op": "levels", "params": {"inBlack": 0.2, "inWhite": 0.8}}}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 5, 5) - (0.6 - 0.2) / 0.6).abs() < 1e-5);
        // invert.
        e.apply_json(&env(
            4,
            "channel.edit",
            t.clone(),
            json!({"edit": {"op": "invert"}}),
        ))
        .unwrap();
        assert!((chan_at(&e, id, 5, 5) - (1.0 - (0.6 - 0.2) / 0.6)).abs() < 1e-5);
        // rename rides the same kind (value delta).
        e.apply_json(&env(
            5,
            "channel.edit",
            t.clone(),
            json!({"name": "sky matte"}),
        ))
        .unwrap();
        assert_eq!(
            e.document().unwrap().channel(id).unwrap().name(),
            "sky matte"
        );
        e.undo().unwrap();
        assert_eq!(e.document().unwrap().channel(id).unwrap().name(), "sky");
        e.redo().unwrap();
        // §40.4 selection gating: fill 1.0 under a half-coverage selection
        // blends v + (nv−v)·s.
        {
            let doc = e.doc_mut_for_test();
            let sel = doc.selection_mut();
            *sel = Selection::new();
            for y in 0..8 {
                for x in 0..8 {
                    sel.mask_mut()
                        .write_pixel_f32(IVec2::new(x, y), [0.5, 0.0, 0.0, 0.0]);
                }
            }
            sel.set_bounds(IRect::new(0, 0, 8, 8));
        }
        let base = chan_at(&e, id, 5, 5);
        e.apply_json(&env(
            6,
            "channel.edit",
            t.clone(),
            json!({"edit": {"op": "fill", "value": 1.0}}),
        ))
        .unwrap();
        assert!(
            (chan_at(&e, id, 5, 5) - (base + (1.0 - base) * 0.5)).abs() < 1e-5,
            "surface edits gate through sel(p) (§13.1)"
        );
        assert_eq!(
            chan_at(&e, id, 40, 40),
            base,
            "outside the selection: untouched"
        );
        // gaussian_blur → typed NI (O-TOOLS seam; mask.edit precedent).
        let err = e
            .apply_json(&env(
                7,
                "channel.edit",
                t.clone(),
                json!({"edit": {"op": "gaussian_blur", "params": {"sigma": 2.0}}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        // Unknown op → out_of_range; spot param → typed; unresolvable
        // target → bad_target.
        let err = e
            .apply_json(&env(
                8,
                "channel.edit",
                t.clone(),
                json!({"edit": {"op": "sharpen"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let err = e
            .apply_json(&env(9, "channel.edit", t.clone(), json!({"solidity": 0.5})))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SpotChannelsUnsupported);
        let err = e
            .apply_json(&env(
                10,
                "channel.edit",
                json!({"channelId": "0badbad0-0000-4000-8000-00000000dead"}),
                json!({"edit": {"op": "invert"}}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
    }

    #[test]
    fn channel_delete_restores_list_order_on_undo() {
        let mut e = ready();
        for (n, cid, name) in [
            (1, "feedfeed-0000-4000-8000-000000000001", "first"),
            (2, "feedfeed-0000-4000-8000-000000000002", "second"),
            (3, "feedfeed-0000-4000-8000-000000000003", "third"),
        ] {
            e.apply_json(&env(
                n,
                "channel.add",
                Value::Null,
                json!({"newChannelId": cid, "name": name}),
            ))
            .unwrap();
        }
        let ids: Vec<ChannelId> = e.document().unwrap().channel_ids_ordered().to_vec();
        let before = e.state_hash();
        // Delete the MIDDLE channel; undo must restore its §14.1 index.
        e.apply_json(&env(
            4,
            "channel.delete",
            json!({"channelId": ids[1].as_uuid().to_string()}),
            json!({}),
        ))
        .unwrap();
        assert_eq!(e.document().unwrap().channel_count(), 2);
        e.undo().unwrap();
        assert_eq!(e.state_hash(), before);
        assert_eq!(
            e.document().unwrap().channel_ids_ordered(),
            &ids[..],
            "undo re-inserts at the recorded list index (§14.1 order)"
        );
        e.redo().unwrap();
        // Rejects: unresolvable target; extra param (no-params row —
        // engine stage-3 taxonomy answers out_of_range for unknown keys;
        // the contract's missing_field spelling is the standing
        // validator-taxonomy divergence).
        let err = e
            .apply_json(&env(
                5,
                "channel.delete",
                json!({"channelId": "0badbad0-0000-4000-8000-00000000dead"}),
                json!({}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        let err = e
            .apply_json(&env(
                6,
                "channel.delete",
                json!({"channelId": ids[0].as_uuid().to_string()}),
                json!({"force": true}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn channel_duplicate_cow_name_default_and_single_doc_posture() {
        let mut e = ready();
        journaled_selection(&mut e, 10, 0.7);
        e.apply_json(&env(
            20,
            "channel.add",
            Value::Null,
            json!({"newChannelId": NEW_CHAN, "name": "sky", "from": "selection"}),
        ))
        .unwrap();
        let src = ChannelId::from_uuid(Uuid::parse_str(NEW_CHAN).unwrap());
        let dup_id = "feedfeed-0000-4000-8000-000000000002";
        e.apply_json(&env(
            21,
            "channel.duplicate",
            json!({"channelId": NEW_CHAN}),
            json!({"newChannelId": dup_id}),
        ))
        .unwrap();
        let dup = ChannelId::from_uuid(Uuid::parse_str(dup_id).unwrap());
        let doc = e.document().unwrap();
        assert_eq!(doc.channel(dup).unwrap().name(), "sky copy");
        // §14.2 COW: the duplicate shares tile Arcs with the source.
        let src_tiles: Vec<_> = doc.channel(src).unwrap().buffer().tiles().iter().collect();
        assert!(!src_tiles.is_empty());
        for (coord, arc) in &src_tiles {
            let dup_arc = doc
                .channel(dup)
                .unwrap()
                .buffer()
                .tiles()
                .get(*coord)
                .expect("duplicate shares the coord set");
            assert!(Arc::ptr_eq(arc, dup_arc), "COW-shared tile {coord:?}");
        }
        // toDocument == this document resolves (dims trivially equal).
        let this_doc = doc.id().as_uuid().to_string();
        e.apply_json(&env(
            22,
            "channel.duplicate",
            json!({"channelId": NEW_CHAN}),
            json!({"newChannelId": "feedfeed-0000-4000-8000-000000000003",
                   "toDocument": this_doc}),
        ))
        .unwrap();
        // toDocument elsewhere → bad_target (single-document posture;
        // size_mismatch reachable with the session host — recorded).
        let err = e
            .apply_json(&env(
                23,
                "channel.duplicate",
                json!({"channelId": NEW_CHAN}),
                json!({"newChannelId": "feedfeed-0000-4000-8000-000000000004",
                       "toDocument": "abababab-0000-4000-8000-000000000001"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        // Missing newChannelId → stage-3 required → missing_field.
        let err = e
            .apply_json(&env(
                24,
                "channel.duplicate",
                json!({"channelId": NEW_CHAN}),
                json!({"name": "x"}),
            ))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn channels_persist_through_the_package_round_trip() {
        let mut e = ready();
        journaled_selection(&mut e, 10, 0.35); // non-u8-exact → F32 leg
        e.apply_json(&env(
            20,
            "selection.save",
            Value::Null,
            json!({"newChannelId": NEW_CHAN, "name": "soft"}),
        ))
        .unwrap();
        e.apply_json(&env(
            21,
            "channel.add",
            Value::Null,
            json!({"newChannelId": "feedfeed-0000-4000-8000-000000000002", "name": "hard"}),
        ))
        .unwrap();
        // u8-exact leg: fill 0.5 is NOT k/255-exact; use 1.0 (exact).
        e.apply_json(&env(
            22,
            "channel.edit",
            json!({"channelId": "feedfeed-0000-4000-8000-000000000002"}),
            json!({"edit": {"op": "fill", "value": 1.0}}),
        ))
        .unwrap();
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let loaded = crate::io::package::load_package_bytes_with(&assembly(), &bytes).unwrap();
        assert_eq!(loaded.engine.document().unwrap().channel_count(), 2);
        assert_eq!(
            loaded.engine.state_hash_persistent(),
            e.state_hash_persistent(),
            "§14.2 channel trees round-trip into the same persistent hash"
        );
        let id = ChannelId::from_uuid(Uuid::parse_str(NEW_CHAN).unwrap());
        assert!((chan_at(&loaded.engine, id, 5, 5) - 0.35).abs() < 1e-6);
    }
}
