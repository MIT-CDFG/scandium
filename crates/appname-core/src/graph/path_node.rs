// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §20.1–§20.3 (doc-level path storage), §40.7 vector kinds;
//! DD ㉕. TEXTVEC Door 2: the doc-path arena behind the existing
//! `document.v1` `pathRef` — core stores every path as OPAQUE
//! `appname.vector.v1` sidecar bytes + a content hash (geometry-blind;
//! validation crosses the [`crate::engine::vector_ops::VectorGeometry`]
//! seam). Paths are small: value-captured SNAPSHOT deltas are the R-class
//! inverses (never tile machinery).

use std::sync::Arc;

use uuid::Uuid;

use crate::geom::IRect;
use crate::graph::arena::HasId;
use crate::graph::id::PathId;

/// One doc-level path (paths panel; shape layers and vector masks
/// reference by [`PathId`]).
#[derive(Clone)]
pub struct PathNode {
    pub(crate) id: PathId,
    /// Optional panel name (`vector.path_new` `name` param).
    pub(crate) name: Option<String>,
    /// Normalized `appname.vector.v1` sidecar bytes (strict-writer
    /// re-emission from the geometry service).
    pub(crate) sidecar: Arc<Vec<u8>>,
    /// Content hash of `sidecar` (UUIDv5-over-BLAKE3, the §7.4 identity
    /// space) — feeds the §20.3 evaluation cache and render-DAG dedup.
    pub(crate) content_hash: Uuid,
    /// Integer hull in document pixels (§40.7 dirty rects).
    pub(crate) bounds: IRect,
}

impl PathNode {
    pub(crate) fn new(
        id: PathId,
        name: Option<String>,
        sidecar: Arc<Vec<u8>>,
        content_hash: Uuid,
        bounds: IRect,
    ) -> PathNode {
        PathNode {
            id,
            name,
            sidecar,
            content_hash,
            bounds,
        }
    }

    pub fn id(&self) -> PathId {
        self.id
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn sidecar(&self) -> &Arc<Vec<u8>> {
        &self.sidecar
    }

    pub fn content_hash(&self) -> Uuid {
        self.content_hash
    }

    pub fn bounds(&self) -> IRect {
        self.bounds
    }

    pub(crate) fn value_block(&self) -> PathValueBlock {
        PathValueBlock {
            name: self.name.clone(),
            sidecar: Arc::clone(&self.sidecar),
            content_hash: self.content_hash,
            bounds: self.bounds,
        }
    }

    pub(crate) fn apply_value_block(&mut self, block: &PathValueBlock) {
        self.name = block.name.clone();
        self.sidecar = Arc::clone(&block.sidecar);
        self.content_hash = block.content_hash;
        self.bounds = block.bounds;
    }
}

impl HasId for PathNode {
    fn uuid(&self) -> Uuid {
        self.id.into()
    }
}

/// The full mutable state of a path as one value block — the
/// `vector.path_edit`/`vector.edit` snapshot delta (§10.5 SetProp shape:
/// full before/after; sidecars share `Arc`s so a delta costs two pointers).
#[derive(Clone)]
pub struct PathValueBlock {
    pub name: Option<String>,
    pub sidecar: Arc<Vec<u8>>,
    pub content_hash: Uuid,
    pub bounds: IRect,
}

impl PartialEq for PathValueBlock {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.content_hash == other.content_hash
    }
}

impl std::fmt::Debug for PathValueBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathValueBlock")
            .field("name", &self.name)
            .field("content_hash", &self.content_hash)
            .field("bounds", &self.bounds)
            .field("sidecar_len", &self.sidecar.len())
            .finish()
    }
}
