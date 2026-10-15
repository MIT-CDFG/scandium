// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.1 (ID types), §2.6 (`Document`/`LayerNode`), §2.7 (arenas and
//! tree integrity), §19.1 (document geometry); DD ⑧; schema
//! `contracts/schemas/document.v1.json`.
//!
//! Document graph v1: document → layer tree (raster layers + group skeleton),
//! UUID identities, bottom-to-top z-order in `children` vecs, and the
//! selection slot. The tree compiles to a render DAG later (§4.6, render
//! lane); parent links are never stored — `children` is the single source of
//! truth (§2.7).
//!
//! Mutation-door rule (plan rule 5 / D-48): all structural and property
//! mutation entry points are `pub(crate)`; the command engine (slice 2) is
//! the only public door. Read access is public.
//!
//! CP-0.1 scope fences: `LayerKind` carries `Raster` and `Group` (the other
//! five §2.6 variants land with their payload types — the enum is
//! `#[non_exhaustive]` so growing it is additive); `OpNode`/`MaskNode`
//! arenas, effects, and per-node `content_hash` (§4.6 folding) arrive with
//! their owning rows.

pub mod arena;
pub mod channel_node;
pub mod document;
pub mod id;
pub mod mask_eval;
pub mod mask_node;
pub mod node;
pub mod op_node;
pub mod path_node;
