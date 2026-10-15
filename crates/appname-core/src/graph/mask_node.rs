// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.6 (`MaskNode` field semantics), §13.1/DD ㉙ (one
//! single-component tiled buffer per mask), §11.2 (evaluation — the render
//! lane's); schema `document.v1.json` `$defs.mask`.
//!
//! CP-0.5 minimal mask doc-model: raster masks only (`Vector`/`Clip` arrive
//! with the §20 path work), created by `edit.paste_into` and stored/round-
//! tripped by the package. Mask COMPOSITING (§11.2 density/feather/invert
//! evaluation) is O-RENDER's carve-out — flagged handoff; this module owns
//! the model and its §2.6 defaults.

use uuid::Uuid;

use crate::buffer::Buffer;
use crate::graph::arena::HasId;
use crate::graph::id::MaskId;

/// Mask content (v1: raster only — Gray/f32 coverage per §13.1).
#[non_exhaustive]
#[derive(Clone)]
pub enum MaskKind {
    Raster(Buffer),
}

/// One mask node (§2.6 field table, CP-0.5 subset; defaults per the table).
#[derive(Clone)]
pub struct MaskNode {
    pub(crate) id: MaskId,
    pub(crate) kind: MaskKind,
    pub(crate) enabled: bool,
    pub(crate) inverted: bool,
    /// `[0,1]` overall mask strength.
    pub(crate) density: f32,
    /// px at lod 0, applied live at render (§11.2) — never baked.
    pub(crate) feather: f32,
    /// Moves with the layer transform when true.
    pub(crate) linked: bool,
}

impl MaskNode {
    /// A raster mask with the §2.6 defaults.
    pub(crate) fn new_raster(id: MaskId, coverage: Buffer) -> MaskNode {
        MaskNode {
            id,
            kind: MaskKind::Raster(coverage),
            enabled: true,
            inverted: false,
            density: 1.0,
            feather: 0.0,
            linked: true,
        }
    }

    pub(crate) fn set_id(&mut self, id: MaskId) {
        self.id = id;
    }

    pub fn id(&self) -> MaskId {
        self.id
    }

    pub fn kind(&self) -> &MaskKind {
        &self.kind
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn inverted(&self) -> bool {
        self.inverted
    }

    pub fn density(&self) -> f32 {
        self.density
    }

    pub fn feather(&self) -> f32 {
        self.feather
    }

    pub fn linked(&self) -> bool {
        self.linked
    }

    /// The raster coverage buffer (every v1 mask is raster).
    pub fn coverage(&self) -> &Buffer {
        match &self.kind {
            MaskKind::Raster(b) => b,
        }
    }

    /// The five §2.6 properties as one value block (SetProp delta unit).
    pub fn props(&self) -> crate::engine::delta::MaskProps {
        crate::engine::delta::MaskProps {
            enabled: self.enabled,
            inverted: self.inverted,
            density: self.density,
            feather: self.feather,
            linked: self.linked,
        }
    }

    /// Apply a full §2.6 property block (mask.set_props + its undo/redo).
    pub(crate) fn set_props(&mut self, p: crate::engine::delta::MaskProps) {
        self.enabled = p.enabled;
        self.inverted = p.inverted;
        self.density = p.density;
        self.feather = p.feather;
        self.linked = p.linked;
    }

    pub(crate) fn coverage_mut(&mut self) -> &mut Buffer {
        match &mut self.kind {
            MaskKind::Raster(b) => b,
        }
    }
}

impl HasId for MaskNode {
    fn uuid(&self) -> Uuid {
        self.id.as_uuid()
    }
}

/// Summary Debug (ES §1.6: never dump coverage payloads).
impl core::fmt::Debug for MaskNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaskNode")
            .field("id", &self.id)
            .field("enabled", &self.enabled)
            .field("tiles", &self.coverage().tiles().len())
            .finish()
    }
}
