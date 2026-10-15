// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: `contracts/commands/doc.new.yaml` (appname.command.DocNew.v1 —
//! the authority for params/ranges/rejects), ES §40.2, §10.3 doc family,
//! §9.6 (deterministic engine-generated ids via UUIDv5 over the creating
//! envelope id — the proto's `derive_id` convention, ported).
//!
//! The one CP-0.1-implemented kind: initialize a fresh document on the
//! slice-1 tile substrate — root group + background raster layer, the
//! background painted through the §10.1 recorder discipline so the commit
//! pipeline (prune → after-capture → per-context dedup intern) runs the
//! same path every pixel kind will use.
//!
//! CP-0.1 stage-6 divergences from the contract (recorded in the lane log
//! for the fold-in list; every one is a typed error, never a panic —
//! review minor-6 constraint):
//! * explicit `profile` → `error.color.profile_space_mismatch`: there is no
//!   profile registry/CMM before the color lane (§5.5), so an explicit
//!   profile's data space cannot be verified — the corpus's executed
//!   profile case (`G-DN-contract-invalid-2`) expects exactly this code;
//! * `mode: cmyk_a` → `error.command.not_implemented`: five-channel fills
//!   need the §14 full-width value path (slice-1 review minor 3);
//! * `depth: f16` (always) and `depth: u16` (without the `depth-u16`
//!   feature) → `error.command.not_implemented` (paused depths, slice-1
//!   review minor 6);
//! * `background: background_color` resolves to the headless default
//!   background swatch (white) — slice-2 open question CLOSED, reading at
//!   [`background_value`].

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::buffer::Buffer;
use crate::engine::delta::{DeltaRecorder, TreeOp};
use crate::engine::error::{EngineError, ErrorCode};
use crate::geom::IRect;
use crate::graph::document::{ColorState, Document, DocumentGeometry};
use crate::graph::id::{DocumentId, LayerId, ProfileId};
use crate::graph::node::LayerNode;
use crate::tile::channel::ChannelKind;
use crate::tile::map::TileCoord;
use crate::tile::model::ColorModelKind;
use crate::tile::TILE_I32;

/// Fixed namespace for deterministic v5 id derivation (proto convention,
/// ES §9.6): engine-generated ids hash the creating envelope's id so journal
/// replay reproduces them byte-for-byte.
pub const APPNAME_NS: Uuid = Uuid::from_u128(0x9a11_ce50_c0de_4a99_b0d5_0b7a_11ed_c0de);

pub fn derive_id(base: Uuid, tag: &str) -> Uuid {
    Uuid::new_v5(&APPNAME_NS, format!("{base}/{tag}").as_bytes())
}

/// Deterministic placeholder id for "the working space for `mode`" (the
/// contract's dynamic profile default, §5.5). The color lane (CP-0.5)
/// replaces the placeholder table with real profile resolution; the ids are
/// namespace-derived constants so documents and replays stay stable.
pub fn builtin_working_profile(mode: ColorModelKind) -> ProfileId {
    let tag = match mode {
        ColorModelKind::Rgba => "profile/working/rgba",
        ColorModelKind::GrayA => "profile/working/gray_a",
        ColorModelKind::CmykA => "profile/working/cmyk_a",
        ColorModelKind::LabA => "profile/working/lab_a",
        // doc.new's mode enum covers only the alpha-bearing four; other
        // models get working profiles when their creation paths exist.
        other => unreachable!("no doc.new working-profile tag for {other:?}"),
    };
    ProfileId(Uuid::new_v5(&APPNAME_NS, tag.as_bytes()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(clippy::enum_variant_names)] // wire literals (§7.3 background enum)
pub(crate) enum Background {
    Transparent,
    White,
    Black,
    BackgroundColor,
}
// (BackgroundColor mirrors the contract's wire literal "background_color";
// wire naming beats the variant-name style lint.)

pub(crate) struct DocNewParams {
    pub width: i32,
    pub height: i32,
    pub resolution_ppi: f64,
    pub mode: ColorModelKind,
    pub depth: ChannelKind,
    pub background: Background,
    pub profile: Option<Uuid>,
}

/// Decode schema-validated effective params (defaults already filled by the
/// stage-5 validator) into typed form. Wire vocabularies are the contract's:
/// mode `rgba|gray_a|cmyk_a|lab_a`, depth `u8|u16|f16|f32`,
/// background `transparent|white|black|background_color`.
pub(crate) fn decode_params(effective: &Map<String, Value>) -> DocNewParams {
    let int = |k: &str| effective[k].as_f64().expect("validated") as i64;
    let s = |k: &str| effective[k].as_str().expect("validated");
    DocNewParams {
        width: int("width") as i32,
        height: int("height") as i32,
        resolution_ppi: effective["resolutionPpi"].as_f64().expect("validated"),
        mode: match s("mode") {
            "rgba" => ColorModelKind::Rgba,
            "gray_a" => ColorModelKind::GrayA,
            "cmyk_a" => ColorModelKind::CmykA,
            "lab_a" => ColorModelKind::LabA,
            other => unreachable!("schema-validated mode {other:?}"),
        },
        depth: match s("depth") {
            "u8" => ChannelKind::U8,
            "u16" => ChannelKind::U16,
            "f16" => ChannelKind::F16,
            "f32" => ChannelKind::F32,
            other => unreachable!("schema-validated depth {other:?}"),
        },
        background: match s("background") {
            "transparent" => Background::Transparent,
            "white" => Background::White,
            "black" => Background::Black,
            "background_color" => Background::BackgroundColor,
            other => unreachable!("schema-validated background {other:?}"),
        },
        profile: effective
            .get("profile")
            .and_then(Value::as_str)
            .map(|p| Uuid::parse_str(p).expect("validated uuid")),
    }
}

/// Stage-6 live-state checks in pinned order (see `engine` module docs):
/// (6b) profile space, (6c) paused depth, (6d) unsupported mode,
/// (6e) unsupported background. (6a — already-initialized — is the engine's,
/// it owns the document slot.)
pub(crate) fn check_supported(p: &DocNewParams) -> Result<(), EngineError> {
    if let Some(profile) = p.profile {
        return Err(EngineError::with_field(
            ErrorCode::ProfileSpaceMismatch,
            "profile",
            format!(
                "profile {profile} cannot be verified against mode at CP-0.1 (no profile \
                 registry/CMM before the color lane, §5.5); omit `profile` to take the \
                 working-space default"
            ),
        ));
    }
    let depth_ok = match p.depth {
        ChannelKind::U8 | ChannelKind::F32 => true,
        ChannelKind::U16 => cfg!(feature = "depth-u16"),
        ChannelKind::F16 => false,
    };
    if !depth_ok {
        return Err(EngineError::with_field(
            ErrorCode::NotImplemented,
            "depth",
            format!(
                "storage depth {:?} is paused in this build (u16 needs the depth-u16 feature; \
                 f16 awaits its dependency record) — typed refusal per the slice-2 \
                 reachability constraint",
                p.depth
            ),
        ));
    }
    if p.mode == ColorModelKind::CmykA {
        return Err(EngineError::with_field(
            ErrorCode::NotImplemented,
            "mode",
            "cmyk_a documents need the five-channel value path of the channels work (§14); \
             typed refusal at CP-0.1",
        ));
    }
    Ok(())
}

/// Storage-normalized premultiplied fill value for a background choice in a
/// mode (ES §2.2 channel semantics; Lab neutral a/b encode as 128/255 —
/// byte-exact ICC scaling, deliberately not 0.5).
/// `background_color` (slice-2 open question, CLOSED — reading recorded in
/// the lane log): the param names the tool-state background swatch, and the
/// contract's `actionRecording` note says the New Document dialog "captures
/// committed params" — i.e. capture happens at the UI, and what the engine
/// receives it resolves against tool state **at apply time**. Headless
/// CP-0.1 has no tool state, so the engine resolves the swatch at its
/// documented startup default — white — deterministically; replay resolves
/// identically (constant), so D-48 replay equality holds. When O-TOOLS
/// lands mutable tool state, UI-originated envelopes carry captured
/// concrete params per `actionRecording`, and the headless default remains
/// this constant.
pub(crate) fn background_value(mode: ColorModelKind, bg: Background) -> Option<[f32; 4]> {
    const LAB_NEUTRAL: f32 = 128.0 / 255.0;
    match bg {
        Background::Transparent => None,
        // Headless default background swatch = white (reading above).
        Background::BackgroundColor | Background::White => Some(match mode {
            ColorModelKind::Rgba => [1.0, 1.0, 1.0, 1.0],
            ColorModelKind::GrayA => [1.0, 1.0, 0.0, 0.0],
            ColorModelKind::LabA => [1.0, LAB_NEUTRAL, LAB_NEUTRAL, 1.0],
            other => unreachable!("unsupported doc.new mode {other:?}"),
        }),
        Background::Black => Some(match mode {
            ColorModelKind::Rgba => [0.0, 0.0, 0.0, 1.0],
            ColorModelKind::GrayA => [0.0, 1.0, 0.0, 0.0],
            ColorModelKind::LabA => [0.0, LAB_NEUTRAL, LAB_NEUTRAL, 1.0],
            other => unreachable!("unsupported doc.new mode {other:?}"),
        }),
    }
}

/// Execute on a fresh handle: build the document (root group + background
/// layer), painting the background through the recorder discipline.
/// Returns the document and the created ids `[root, background]` (§40.2
/// DeltaSet note: tree_after = AddNode(root, background) + background
/// tiles).
pub(crate) fn execute(
    env_id: Uuid,
    p: &DocNewParams,
    rec: &mut DeltaRecorder,
) -> Result<(Document, Vec<Uuid>), EngineError> {
    let doc_id = DocumentId(derive_id(env_id, "doc"));
    let root_id = LayerId(derive_id(env_id, "tree/root"));
    let layer_id = LayerId(derive_id(env_id, "layer/0"));
    let working = builtin_working_profile(p.mode);

    let mut geometry = DocumentGeometry::new(p.width, p.height);
    geometry.resolution_ppi = p.resolution_ppi;
    let color = ColorState {
        working,
        mode: p.mode,
        depth: p.depth,
        rendering_intent: None,
        black_point_compensation: None,
    };

    #[allow(clippy::needless_late_init)]
    let mut doc;
    doc = Document::new(doc_id, root_id, geometry, color).map_err(|e| {
        // Unreachable through the validated path (schema bounds fit §19.1,
        // paused depths were refused at 6c) — surfaced honestly if internal
        // invariants ever drift.
        EngineError::new(
            ErrorCode::InvalidArg,
            format!("document construction refused: {e:?}"),
        )
    })?;
    crate::engine::color_glue::seed_builtin_profiles(&mut doc);
    rec.tree(TreeOp::AddNode {
        parent: None,
        index: 0,
        id: root_id,
    });

    // Background layer: paint first (recorder discipline), then attach.
    let mut pixels = Buffer::new(p.depth, p.mode, working);
    if let Some(value) = background_value(p.mode, p.background) {
        let canvas = IRect::new(0, 0, p.width, p.height);
        let surface = layer_id.as_uuid();
        rec.will_edit_bounds(surface, &pixels);
        for (tx, ty) in canvas.tiles(TILE_I32) {
            rec.will_edit(surface, pixels.tiles(), TileCoord::new(0, tx, ty));
        }
        pixels.fill_rect(canvas, value);
    }
    let node = LayerNode::new_raster(layer_id, "Background", pixels);
    doc.insert_child(root_id, 0, node).map_err(|e| {
        EngineError::new(
            ErrorCode::InvalidArg,
            format!("background layer attach refused: {e:?}"),
        )
    })?;
    rec.tree(TreeOp::AddNode {
        parent: Some(root_id),
        index: 0,
        id: layer_id,
    });

    Ok((doc, vec![root_id.as_uuid(), layer_id.as_uuid()]))
}
