// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-vector` — the ONE Bézier path model (DD ㉕) + exact-coverage
//! scanline rasterizer v1 (CP-0.7 groundwork, plan §2.4 "Text/vector
//! groundwork" rows 1–2).
//!
//! Spec: ES §20.1 (path model, invariants, anchors, bounds, length), §20.2
//! (fill rasterization — sparse signed-area coverage, CPU reference), §20.3
//! (boolean numerical policy — the tolerance constants live here now so
//! CP-1.0 booleans inherit them), §7.5 (`appname.vector.v1` sidecar worked
//! example), §2.8 (serialization conventions), §13.1/§11.2 (coverage output
//! = the same Gray f32 [0,1] convention as selection masks — path →
//! selection conversion is literally free); DD ㉕ (one model for
//! shapes/masks/selections/text-on-path); dossier
//! `contracts/algorithms/dossiers/vector-rasterization.yaml` (era-basis:
//! scanline active-edge exact-area cell coverage, libart/FreeType-smooth
//! lineage — public art since the 1990s; clean-room own Rust; patents:
//! none; NV_path-style vendor GPU approaches rejected).
//!
//! Determinism spine (D-48, ES §10): everything in this crate is PURE —
//! values in, values out, no document mutation. Document mutations
//! (`vector.*` kinds, CP-1.0) go through `appname.command.v1` envelopes
//! into O-CORE's single apply entry; this crate only computes.
//!
//! Scope fence for CP-0.7 groundwork: path model + serde + flatten/length +
//! bounds + transforms + anchors + fill rasterizer. NOT here yet (CP-1.0):
//! strokes/tessellation, booleans, pen state machine, SVG interop,
//! `selection.from_path` installation (O-CORE doors).

pub mod anchor;
pub mod boolean;
pub mod bounds;
pub mod fit;
pub mod flatten;
pub mod geom;
pub mod op_bridge;
pub use op_bridge::register;
pub mod path;
pub mod primitives;
pub mod raster;
pub mod stroke;
pub mod wire;

pub use anchor::{Anchor2, AnchorKind};
pub use bounds::{control_bounds, tight_bounds, Rect2};
pub use flatten::{
    flatten_path, flatten_subpath, ArcLengthTable, MAX_SPLIT_DEPTH, TOL_BOOLEAN, TOL_DISPLAY,
    TOL_MEASURE,
};
pub use geom::Vec2;
pub use path::{CubicSeg, FillRule, PathError, SubPath, VectorPath, EPS_DEGENERATE, EPS_WELD};
pub use raster::{rasterize_path, rasterize_polylines, CoveragePlane};
pub use stroke::{
    shape_stroke_coverage, stroke_boundary_coverage, stroke_coverage, trace_iso_contours,
    STROKE_FLATTEN_TOL,
};
pub use wire::{path_from_sidecar_json, path_to_sidecar_json, LoadNote, SIDECAR_SCHEMA};
