// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-core` — Appname engine kernel: tile substrate, document graph,
//! command engine, journal/undo.
//!
//! Spec: ES §2 (core data model), §3 (tile engine), §10 (commands/undo),
//! §13.1 (selection type), §19.1 (geometry); DD ⑥⑧⑰⑲㉙.
//!
//! Determinism spine (D-48, ES §10): every document mutation is an
//! `appname.command.v1` envelope through the single apply entry. This crate
//! enforces the shape structurally — **every mutation entry point in the
//! tile and graph layers is `pub(crate)`** ([`tile::map::TileMap::make_mut`]
//! is the single COW door, [`graph::document::Document`]'s structural doors
//! likewise), so the command engine (slice 2) is the only public door; there
//! are no side-door mutation APIs.
//!
//! CP-0.1 slice 1 content (plan §2.1 rows "Tile substrate v1" + "Document
//! graph v1"): 256×256 premultiplied tiles generic over depth × model,
//! content-addressed COW tile maps (BLAKE3-256 identity, ES §0.3/§3.8), the
//! `Buffer` surface with `region_hash`, raster/group layer trees with UUID
//! identity and bottom-to-top z-order, and the f32 mask selection slot.

pub mod buffer;
pub mod engine;
pub mod geom;
pub mod graph;
pub mod hash;
pub mod io;
pub mod mask;
pub mod serial;
pub mod tile;

pub use buffer::{Buffer, BufferUid, PremulViolation};
pub use engine::assets::{asset_id_for, AssetKind, AssetStore};
pub use engine::delta::{DedupIndex, DeltaRecorder, DeltaSet, TileDelta, TreeOp};
pub use engine::envelope::{Envelope, Target, ENVELOPE_SCHEMA, MAX_ENVELOPE_BYTES};
pub use engine::error::{EngineError, ErrorCode};
pub use engine::kind_impl::{
    BlendKernelRouting, EngineAssembly, FillConvertRouting, FillToLinearFn, KindContext, KindImpl,
    KindOutcome, PremultiplyCoverageFn, SeparableKernel, TripleKernel,
};
pub use engine::op_eval::{fold_adjustment_node_weight, ChainEvaluator, OpPlane};
pub use engine::sampling::{
    resolve_sampling_composite, source_over_storage, SampleRequest, SampleScope, SamplingComposite,
};
pub use engine::text_ops::{TextLayout, TextPageData, TEXT_SIDECAR_SCHEMA};
pub use engine::vector_ops::{
    CoveragePlaneData, GeometryResult, VectorBoolOp, VectorEvalCache, VectorGeometry,
    VECTOR_SIDECAR_SCHEMA,
};
pub use engine::{ApplyReceipt, DirtyRect, Engine, HistoryRow};
pub use geom::{IRect, IVec2, Mat3};
pub use graph::arena::{Arena, ArenaError, HasId};
pub use graph::document::{
    ColorState, Document, DocumentGeometry, GraphError, RenderingIntent, TreeError,
};
pub use graph::id::{
    AssetId, ChannelId, CommandId, DocumentId, LayerId, MaskId, OpId, PathId, ProfileId,
};
pub use graph::mask_eval::{
    apply_coverage, feather_coverage, feather_reach_px, feather_sigma, layer_effective_coverage,
    mask_needs_feather, resolve_effective_coverage, resolve_effective_coverage_with,
    stack_needs_feather, FeatheredCoverage, ResolvedMasks,
};
pub use graph::mask_node::{MaskKind, MaskNode};
pub use graph::node::{BlendMode, ColorLabel, LayerKind, LayerNode, LockFlags};
pub use graph::node::{ShapeLayerData, SmartObjectData, SmartSource};
pub use graph::op_node::{OpNode, OpValueBlock};
pub use graph::path_node::{PathNode, PathValueBlock};
pub use hash::ContentHash;
pub use io::package::{
    load_package, load_package_bytes, load_package_bytes_with, load_package_with, save_package,
    save_package_bytes, LoadedPackage,
};
pub use io::png_io::{
    composite_plane_f32, decode_png_to_buffer, encode_composite_png, export_png_bytes,
    has_visible_parametric_content, resolve_parametric_page,
};
pub use mask::Selection;
pub use serial::Maybe;
pub use tile::channel::{Channel, ChannelKind};
pub use tile::dedup::DedupTable;
pub use tile::map::{TileCoord, TileMap};
pub use tile::model::{model, ColorModel, ColorModelKind};
pub use tile::storage::{TilePixels, TileStorage};
pub use tile::tile::Tile;
pub use tile::{TILE, TILE_I32};

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-core";

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-core");
    }
}
