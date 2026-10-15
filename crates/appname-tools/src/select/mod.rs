// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Selection interactive tiers v1 — pure coverage engines (CP-0.5 wave 2,
//! plan §2.3 row "Selections & masks", O-TOOLS share).
//!
//! Spec: ES §13.1 (one f32 mask [0,1]; [NORMATIVE] edges are coverage, not
//! blur; bounds maintenance), §13.2 ([NORMATIVE] boolean + refinement
//! math), §13.3 (per-source algorithms), §11.2 (analytic scanline
//! machinery), §22.6 (wand/bucket shared predicate + span-stack flood);
//! contracts `selection.set.yaml` / `selection.modify.yaml`; §34.4
//! properties 1, 11, 13. DD ⑭.
//!
//! # The seam (core installs, tools compute — the brush-handoff pattern)
//!
//! Everything here is pure: inputs are params + read-only views (the
//! existing mask through the brush module's `CoverageRead`, pixels through
//! `Backdrop`); the output is a [`SelectionEdit`] — full single-channel
//! tile planes keyed by core `TileCoord` + the post-edit tight bounds +
//! a shrinking flag (§13.1 bounds-maintenance split). O-CORE's
//! `selection.set`/`selection.modify` KindImpls install the patches into
//! `Selection::mask` through core doors, record selection-surface deltas,
//! and emit the T event with the returned bounds. Blocking core doors are
//! itemized in the lane log (no selection-surface install/bounds doors or
//! DeltaRecorder arm exist yet).
//!
//! v1 pins (frozen):
//! * `selection.set`'s `feather` param applies to the RASTERIZED SOURCE
//!   before the boolean op (§40.4 "source rasterized, combined with op");
//!   `selection.modify{feather}` blurs the whole mask. Logged.
//! * Wand metric operates on STRAIGHT color (premultiplied inputs
//!   un-premultiply; §19.3 trim precedent — §13.3 says only
//!   "working-space floats"). Logged.
//! * EDT lattice→contour offset: the §13.2 signed distance evaluates as
//!   `d = D_lattice − 0.5` (outside) / `−(D_lattice − 0.5)` (inside), so a
//!   pixel-boundary edge sits at d = ±0.5 and `smoothAA` reproduces the
//!   1-px AA band astride it. Logged.

pub mod grabcut_refine;
pub mod quick;
pub mod refine;
pub mod select_mask;
pub mod shape;
pub mod wand;

use appname_core::{IRect, IVec2, TileCoord, TILE, TILE_I32};

use crate::brush::present::CoverageRead;

/// §13.1 `SelectOp` (wire: replace/add/subtract/intersect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectOp {
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl SelectOp {
    pub fn from_wire(s: &str) -> Option<SelectOp> {
        Some(match s {
            "replace" => SelectOp::Replace,
            "add" => SelectOp::Add,
            "subtract" => SelectOp::Subtract,
            "intersect" => SelectOp::Intersect,
            _ => return None,
        })
    }

    /// §13.2 [NORMATIVE] per-pixel boolean.
    #[inline]
    pub fn apply(&self, a: f32, b: f32) -> f32 {
        match self {
            SelectOp::Add => (a + b).min(1.0),
            SelectOp::Subtract => (a - b).max(0.0),
            SelectOp::Intersect => a.min(b),
            SelectOp::Replace => b,
        }
    }

    /// §13.1 bounds-maintenance class: shrinking ops re-tighten; growing
    /// ops union in O(1).
    pub fn is_shrinking(&self) -> bool {
        matches!(
            self,
            SelectOp::Subtract | SelectOp::Intersect | SelectOp::Replace
        )
    }
}

/// A dense single-channel coverage plane over a document rect (row-major).
/// The working representation of every selection source and refinement.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskPlane {
    pub rect: IRect,
    pub v: Vec<f32>,
}

impl MaskPlane {
    pub fn new(rect: IRect) -> MaskPlane {
        let n = (rect.width().max(0) as usize) * (rect.height().max(0) as usize);
        MaskPlane {
            rect,
            v: vec![0.0; n],
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        if !self.rect.contains(IVec2::new(x, y)) {
            return 0.0; // absent reads zero (§2.4/§13.2)
        }
        self.v[((y - self.rect.min.y) * self.rect.width() + (x - self.rect.min.x)) as usize]
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, val: f32) {
        debug_assert!(self.rect.contains(IVec2::new(x, y)));
        let i = ((y - self.rect.min.y) * self.rect.width() + (x - self.rect.min.x)) as usize;
        self.v[i] = val;
    }

    /// Tight integer bounds of `{p : v(p) > 0}` (§13.1).
    pub fn tight_bounds(&self) -> IRect {
        let mut b = IRect::EMPTY;
        for y in self.rect.min.y..self.rect.max.y {
            for x in self.rect.min.x..self.rect.max.x {
                if self.get(x, y) > 0.0 {
                    b = b.union(&IRect::new(x, y, x + 1, y + 1));
                }
            }
        }
        b
    }

    /// `antialias:false` (§13.1/§13.3): threshold coverage at 0.5.
    pub fn threshold_half(&mut self) {
        for v in &mut self.v {
            *v = if *v >= 0.5 { 1.0 } else { 0.0 };
        }
    }
}

/// One result tile: the full post-edit coverage plane (row-major
/// `TILE × TILE`, values in [0, 1]).
pub struct MaskPatch {
    pub coord: TileCoord,
    pub values: Vec<f32>,
}

impl core::fmt::Debug for MaskPatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaskPatch")
            .field("coord", &self.coord)
            .finish_non_exhaustive()
    }
}

/// The computed selection edit O-CORE installs (§13.1/§40.4 seam).
#[derive(Debug)]
pub struct SelectionEdit {
    /// Patches sorted by `(y, x)` coord. A tile whose every value equals
    /// the existing mask is omitted (the §40.1 empty-DeltaSet case emerges
    /// when the list is empty).
    pub patches: Vec<MaskPatch>,
    /// Post-edit tight bounds of the WHOLE mask (§13.1 — the T-event
    /// payload). Exact when `shrinking` is false or the edit region covers
    /// the prior bounds; otherwise core re-tightens tile-granularly.
    pub bounds: IRect,
    /// §13.1 bounds-maintenance class of the applied op.
    pub shrinking: bool,
}

/// Combine a rasterized source with the existing mask (§13.2 op), emitting
/// patches over `touched ∩ clip`. `existing_bounds` is the current
/// selection bounds (drives the touched region for shrinking ops);
/// `existing` reads the current mask (core `Selection` implements
/// `CoverageRead`; inactive selections read 0 for combination purposes —
/// pass [`InactiveMask`] for that case, since core's impl reads 1).
pub fn combine(
    existing: &dyn CoverageRead,
    existing_bounds: IRect,
    source: &MaskPlane,
    op: SelectOp,
    clip: IRect,
) -> SelectionEdit {
    let touched = if op.is_shrinking() {
        source.rect.union(&existing_bounds)
    } else {
        source.rect
    }
    .intersect(&clip);

    let mut patches = Vec::new();
    let mut bounds = IRect::EMPTY;
    if !op.is_shrinking() {
        bounds = existing_bounds; // growing: O(1) union basis (§13.1)
    }
    if touched.is_empty() {
        return SelectionEdit {
            patches,
            bounds,
            shrinking: op.is_shrinking(),
        };
    }
    for (tx, ty) in touched.tiles(TILE_I32) {
        let coord = TileCoord::new(0, tx, ty);
        let trect = coord.pixel_rect();
        let mut values = vec![0.0f32; TILE * TILE];
        let mut any_change = false;
        let mut i = 0;
        for y in trect.min.y..trect.max.y {
            for x in trect.min.x..trect.max.x {
                let p = IVec2::new(x, y);
                let a = existing.coverage(p).clamp(0.0, 1.0);
                let out = if touched.contains(p) && clip.contains(p) {
                    let b = source.get(x, y).clamp(0.0, 1.0);
                    op.apply(a, b)
                } else {
                    a
                };
                values[i] = out;
                if out != a {
                    any_change = true;
                }
                if out > 0.0 {
                    bounds = bounds.union(&IRect::new(x, y, x + 1, y + 1));
                }
                i += 1;
            }
        }
        if any_change {
            patches.push(MaskPatch { coord, values });
        }
    }
    SelectionEdit {
        patches,
        bounds,
        shrinking: op.is_shrinking(),
    }
}

/// A zero-coverage mask view: what an INACTIVE selection combines as
/// (§13.2 `a = 0` everywhere; core's `Selection` reads 1 when inactive
/// because *consumers* treat no-selection as fully selected — combination
/// starts from the empty mask instead).
pub struct InactiveMask;

impl CoverageRead for InactiveMask {
    fn coverage(&self, _p: IVec2) -> f32 {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleans_are_the_13_2_forms_and_property_1_holds() {
        // Deterministic pseudo-random pairs; §34.4 property 1: add and
        // intersect commutative + associative; subtract pinned.
        let vals: Vec<f32> = (0..64)
            .map(|i| ((i * 37 + 11) % 101) as f32 / 100.0)
            .collect();
        for i in 0..vals.len() {
            for j in 0..vals.len() {
                let (a, b) = (vals[i], vals[j]);
                assert_eq!(SelectOp::Add.apply(a, b), (a + b).min(1.0));
                assert_eq!(SelectOp::Subtract.apply(a, b), (a - b).max(0.0));
                assert_eq!(SelectOp::Intersect.apply(a, b), a.min(b));
                assert_eq!(SelectOp::Replace.apply(a, b), b);
                // Commutativity.
                assert_eq!(SelectOp::Add.apply(a, b), SelectOp::Add.apply(b, a));
                assert_eq!(
                    SelectOp::Intersect.apply(a, b),
                    SelectOp::Intersect.apply(b, a)
                );
            }
        }
        // Associativity (add / intersect) over a triple sweep.
        for &a in &vals[..16] {
            for &b in &vals[..16] {
                for &c in &vals[..16] {
                    let l = SelectOp::Add.apply(SelectOp::Add.apply(a, b), c);
                    let r = SelectOp::Add.apply(a, SelectOp::Add.apply(b, c));
                    assert!((l - r).abs() < 1e-6);
                    let l = SelectOp::Intersect.apply(SelectOp::Intersect.apply(a, b), c);
                    let r = SelectOp::Intersect.apply(a, SelectOp::Intersect.apply(b, c));
                    assert_eq!(l, r);
                }
            }
        }
        // Subtract is neither commutative nor associative — pin an example.
        assert_ne!(
            SelectOp::Subtract.apply(1.0, 0.3),
            SelectOp::Subtract.apply(0.3, 1.0)
        );
        // Coverage stays in [0,1] (§34.4 property 13).
        for &a in &vals {
            for &b in &vals {
                for op in [
                    SelectOp::Replace,
                    SelectOp::Add,
                    SelectOp::Subtract,
                    SelectOp::Intersect,
                ] {
                    let v = op.apply(a, b);
                    assert!((0.0..=1.0).contains(&v));
                }
            }
        }
    }

    #[test]
    fn combine_emits_patches_and_tight_bounds() {
        let mut src = MaskPlane::new(IRect::new(10, 10, 20, 20));
        for y in 10..20 {
            for x in 10..20 {
                src.set(x, y, 1.0);
            }
        }
        let clip = IRect::new(0, 0, 512, 512);
        let edit = combine(&InactiveMask, IRect::EMPTY, &src, SelectOp::Replace, clip);
        assert_eq!(edit.patches.len(), 1);
        assert_eq!(edit.bounds, IRect::new(10, 10, 20, 20));
        assert!(edit.shrinking, "replace re-tightens (§13.1)");
        // The patch carries the full tile plane with the square set.
        let p = &edit.patches[0];
        assert_eq!(p.coord, TileCoord::new(0, 0, 0));
        assert_eq!(p.values[15 * TILE + 15], 1.0);
        assert_eq!(p.values[0], 0.0);
        // Add with an existing region unions bounds in O(1).
        struct Half;
        impl CoverageRead for Half {
            fn coverage(&self, p: IVec2) -> f32 {
                if p.x < 5 && p.y < 5 {
                    1.0
                } else {
                    0.0
                }
            }
        }
        let edit = combine(&Half, IRect::new(0, 0, 5, 5), &src, SelectOp::Add, clip);
        assert!(!edit.shrinking);
        assert_eq!(edit.bounds, IRect::new(0, 0, 20, 20));
        // No-change tiles are omitted: subtract of a non-overlapping rect.
        let edit = combine(
            &Half,
            IRect::new(0, 0, 5, 5),
            &src,
            SelectOp::Subtract,
            clip,
        );
        assert!(
            edit.patches.is_empty(),
            "nothing changed → empty edit (§40.1)"
        );
    }

    #[test]
    fn clip_gates_the_source() {
        let mut src = MaskPlane::new(IRect::new(-10, -10, 10, 10));
        for y in -10..10 {
            for x in -10..10 {
                src.set(x, y, 1.0);
            }
        }
        let clip = IRect::new(0, 0, 100, 100);
        let edit = combine(&InactiveMask, IRect::EMPTY, &src, SelectOp::Replace, clip);
        assert_eq!(
            edit.bounds,
            IRect::new(0, 0, 10, 10),
            "outside-canvas coverage clipped"
        );
    }
}
