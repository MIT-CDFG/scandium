// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.2 (boolean combination — EXACT normative formulas), §13.1
//! (single f32 coverage representation), §40.4 (family rule: all boolean
//! combination uses §13.2 math).
//!
//! The pure combine kernel shared by `mask.to_selection` now and the
//! `selection.*` kinds at their row (the O-TOOLS seam mirrors the brush
//! handoff: geometry/wand/feather produce COVERAGE, core combines with
//! these formulas and installs under the recorder). The §13.2 refinement
//! family (feather σ, EDT grow/shrink/border, smooth) is O-TOOLS' half of
//! the split.

/// §13.2 boolean combination ops (the `op` wire enum of `selection.set`,
/// `selection.load`, `mask.to_selection`, `selection.save`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectOp {
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl SelectOp {
    /// The contract wire literals (`replace | add | subtract | intersect`).
    pub fn from_wire(s: &str) -> Option<SelectOp> {
        Some(match s {
            "replace" => SelectOp::Replace,
            "add" => SelectOp::Add,
            "subtract" => SelectOp::Subtract,
            "intersect" => SelectOp::Intersect,
            _ => return None,
        })
    }
}

/// [NORMATIVE §13.2] For mask values `a` (existing) and `b` (new), per
/// pixel — these exact expressions:
///
/// ```text
/// Add:        min(1, a + b)
/// Subtract:   max(0, a − b)
/// Intersect:  min(a, b)
/// Replace:    b
/// ```
#[inline]
pub fn combine(op: SelectOp, a: f32, b: f32) -> f32 {
    match op {
        SelectOp::Add => (a + b).min(1.0),
        SelectOp::Subtract => (a - b).max(0.0),
        SelectOp::Intersect => a.min(b),
        SelectOp::Replace => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_are_the_normative_expressions() {
        // Boundary + interior probes per op (§13.2 exactness).
        assert_eq!(
            combine(SelectOp::Add, 0.75, 0.5),
            1.0,
            "min(1, a+b) saturates"
        );
        assert_eq!(combine(SelectOp::Add, 0.25, 0.5), 0.75);
        assert_eq!(
            combine(SelectOp::Subtract, 0.25, 0.5),
            0.0,
            "max(0, a-b) floors"
        );
        assert_eq!(combine(SelectOp::Subtract, 0.75, 0.5), 0.25);
        assert_eq!(combine(SelectOp::Intersect, 0.75, 0.5), 0.5);
        assert_eq!(combine(SelectOp::Intersect, 0.25, 0.5), 0.25);
        assert_eq!(combine(SelectOp::Replace, 0.75, 0.5), 0.5);
        // Exactness: no epsilon, no re-association — the f32 expressions
        // are the identity (bit-stable across platforms; IEEE min/max/add).
        let (a, b) = (0.1f32, 0.2f32);
        assert_eq!(
            combine(SelectOp::Add, a, b).to_bits(),
            (a + b).min(1.0).to_bits()
        );
    }

    #[test]
    fn wire_literals_round_trip() {
        for (s, op) in [
            ("replace", SelectOp::Replace),
            ("add", SelectOp::Add),
            ("subtract", SelectOp::Subtract),
            ("intersect", SelectOp::Intersect),
        ] {
            assert_eq!(SelectOp::from_wire(s), Some(op));
        }
        assert_eq!(SelectOp::from_wire("xor"), None, "§13.2 has no xor");
    }
}
