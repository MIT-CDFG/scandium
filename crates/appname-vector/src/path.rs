// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §20.1 ([NORMATIVE] path model + invariants: (1) segments
//! contiguous `segments[i].p1 == segments[i+1].p0` BITWISE, coordinate
//! duplicated for locality, the editing API writes both; (2) closed ⇒
//! `last.p1 == first.p0`; (3) no degenerate segment survives an edit —
//! four points within 1e−9 doc px of one point ⇒ removed; (4) all path
//! math f64), §7.5 (reader MAY tolerate ≤ 1e−6 px drift and re-weld);
//! DD ㉕ (this ONE model feeds shapes, vector masks, selections, and
//! text-on-path — no second path type anywhere).

use appname_core::PathId;

use crate::geom::Vec2;

/// §20.1 [NORMATIVE] degenerate-segment epsilon (doc px): a segment whose
/// four control points lie within this distance of one point is removed by
/// normalization. Also §20.3's incidence/equality ε (numerical policy).
pub const EPS_DEGENERATE: f64 = 1e-9;

/// §7.5 [worked-example note] reader weld tolerance (doc px): readers MAY
/// tolerate up to this much contiguity drift and re-weld; more is an error.
pub const EPS_WELD: f64 = 1e-6;

/// §20.1 fill rule. Wire forms in wire.rs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// §20.1 cubic segment. `p0` duplicates the previous segment's `p1`
/// (locality; invariant 1) — the editing API writes both ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicSeg {
    pub p0: Vec2,
    pub c1: Vec2,
    pub c2: Vec2,
    pub p1: Vec2,
}

impl CubicSeg {
    pub const fn new(p0: Vec2, c1: Vec2, c2: Vec2, p1: Vec2) -> CubicSeg {
        CubicSeg { p0, c1, c2, p1 }
    }

    /// A straight line as a cubic with collinear one-third handles (the
    /// §7.5 worked example's straight-edge convention).
    pub fn line(p0: Vec2, p1: Vec2) -> CubicSeg {
        CubicSeg {
            p0,
            c1: p0.lerp(p1, 1.0 / 3.0),
            c2: p0.lerp(p1, 2.0 / 3.0),
            p1,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.p0.is_finite() && self.c1.is_finite() && self.c2.is_finite() && self.p1.is_finite()
    }

    /// Evaluate B(t) by De Casteljau (numerically stable; f64 per §20.1).
    pub fn eval(&self, t: f64) -> Vec2 {
        let a = self.p0.lerp(self.c1, t);
        let b = self.c1.lerp(self.c2, t);
        let c = self.c2.lerp(self.p1, t);
        a.lerp(b, t).lerp(b.lerp(c, t), t)
    }

    /// De Casteljau split at `t` — both halves EXACTLY reproduce the curve
    /// (the §20.4 "insert anchor preserves shape exactly" primitive).
    pub fn split(&self, t: f64) -> (CubicSeg, CubicSeg) {
        let q0 = self.p0.lerp(self.c1, t);
        let q1 = self.c1.lerp(self.c2, t);
        let q2 = self.c2.lerp(self.p1, t);
        let r0 = q0.lerp(q1, t);
        let r1 = q1.lerp(q2, t);
        let s = r0.lerp(r1, t);
        (
            CubicSeg::new(self.p0, q0, r0, s),
            CubicSeg::new(s, r1, q2, self.p1),
        )
    }

    /// Split at t = 0.5 using exact midpoint halving (bit-deterministic
    /// across platforms: only adds and multiplies by 0.5).
    pub fn split_half(&self) -> (CubicSeg, CubicSeg) {
        let q0 = self.p0.midpoint(self.c1);
        let q1 = self.c1.midpoint(self.c2);
        let q2 = self.c2.midpoint(self.p1);
        let r0 = q0.midpoint(q1);
        let r1 = q1.midpoint(q2);
        let s = r0.midpoint(r1);
        (
            CubicSeg::new(self.p0, q0, r0, s),
            CubicSeg::new(s, r1, q2, self.p1),
        )
    }

    /// §20.1 invariant 3 predicate. Pin (logged): "lie within 1e−9 of one
    /// point" evaluates against the four-point centroid — existence of a
    /// covering point within ε implies centroid distance ≤ ε (convexity),
    /// and centroid distance ≤ ε implies a covering point (the centroid),
    /// so the predicate is ε-equivalent and deterministic.
    pub fn is_degenerate(&self) -> bool {
        let cx = (self.p0.x + self.c1.x + self.c2.x + self.p1.x) * 0.25;
        let cy = (self.p0.y + self.c1.y + self.c2.y + self.p1.y) * 0.25;
        let c = Vec2::new(cx, cy);
        self.p0.distance(c) <= EPS_DEGENERATE
            && self.c1.distance(c) <= EPS_DEGENERATE
            && self.c2.distance(c) <= EPS_DEGENERATE
            && self.p1.distance(c) <= EPS_DEGENERATE
    }

    pub fn transform_affine(&self, m: &appname_core::Mat3) -> CubicSeg {
        CubicSeg {
            p0: self.p0.transform_affine(m),
            c1: self.c1.transform_affine(m),
            c2: self.c2.transform_affine(m),
            p1: self.p1.transform_affine(m),
        }
    }
}

/// §20.1 subpath: a contiguous run of cubic segments, open or closed.
#[derive(Clone, Debug, PartialEq)]
pub struct SubPath {
    pub segments: Vec<CubicSeg>,
    pub closed: bool,
}

/// §20.1 path: subpaths + fill rule, identified by core's `PathId`
/// (`vector/{pathId}.json` sidecar entry name, §7.5).
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPath {
    pub id: PathId,
    pub subpaths: Vec<SubPath>,
    pub fill_rule: FillRule,
}

/// Typed validation errors (strict WRITER rules; readers go through
/// `normalize`, §2.8 lenient-reader split).
#[derive(Clone, Debug, PartialEq)]
pub enum PathError {
    /// A coordinate is NaN/∞ (RFC 8259 finiteness, document.v1 x-spec-note).
    NonFinite { subpath: usize, segment: usize },
    /// `segments[i].p1 != segments[i+1].p0` beyond even the reader weld
    /// tolerance (`gap` = Euclidean distance).
    Discontiguous {
        subpath: usize,
        segment: usize,
        gap: f64,
    },
    /// Closed subpath whose `last.p1 != first.p0` beyond weld tolerance.
    UnclosedLoop { subpath: usize, gap: f64 },
    /// Sidecar `schema` field is not `appname.vector.v1`.
    SchemaMismatch { got: String },
    /// Sidecar JSON malformed (message from serde_json, stringified).
    Json { message: String },
}

impl VectorPath {
    pub fn new(id: PathId, fill_rule: FillRule) -> VectorPath {
        VectorPath {
            id,
            subpaths: Vec::new(),
            fill_rule,
        }
    }

    /// Strict §20.1 writer validation: finiteness, BITWISE contiguity,
    /// bitwise closure, no degenerate segments, no empty subpaths.
    pub fn validate(&self) -> Result<(), PathError> {
        for (si, sub) in self.subpaths.iter().enumerate() {
            if sub.segments.is_empty() {
                return Err(PathError::Discontiguous {
                    subpath: si,
                    segment: 0,
                    gap: f64::NAN,
                });
            }
            for (gi, seg) in sub.segments.iter().enumerate() {
                if !seg.is_finite() {
                    return Err(PathError::NonFinite {
                        subpath: si,
                        segment: gi,
                    });
                }
                if seg.is_degenerate() {
                    // Degenerates never survive an edit (invariant 3); a
                    // writer-side path carrying one is out of contract.
                    return Err(PathError::Discontiguous {
                        subpath: si,
                        segment: gi,
                        gap: 0.0,
                    });
                }
                if gi + 1 < sub.segments.len() && seg.p1 != sub.segments[gi + 1].p0 {
                    return Err(PathError::Discontiguous {
                        subpath: si,
                        segment: gi,
                        gap: seg.p1.distance(sub.segments[gi + 1].p0),
                    });
                }
            }
            if sub.closed {
                let (f, l) = (&sub.segments[0], &sub.segments[sub.segments.len() - 1]);
                if l.p1 != f.p0 {
                    return Err(PathError::UnclosedLoop {
                        subpath: si,
                        gap: l.p1.distance(f.p0),
                    });
                }
            }
        }
        Ok(())
    }

    /// Lenient READER normalization (§7.5 note + §20.1 invariant 3):
    /// re-weld contiguity/closure drift ≤ `EPS_WELD` (successor's `p0` is
    /// rewritten to the predecessor's `p1` bitwise — the duplicated
    /// coordinate convention), remove degenerate segments (welding across
    /// the removal), drop emptied subpaths. Drift beyond `EPS_WELD` is a
    /// typed error, never a silent fixup.
    pub fn normalize(&mut self) -> Result<(), PathError> {
        for (si, sub) in self.subpaths.iter_mut().enumerate() {
            for (gi, seg) in sub.segments.iter().enumerate() {
                if !seg.is_finite() {
                    return Err(PathError::NonFinite {
                        subpath: si,
                        segment: gi,
                    });
                }
            }
            // Weld pass.
            for gi in 1..sub.segments.len() {
                let prev_p1 = sub.segments[gi - 1].p1;
                let gap = sub.segments[gi].p0.distance(prev_p1);
                if gap > EPS_WELD {
                    return Err(PathError::Discontiguous {
                        subpath: si,
                        segment: gi - 1,
                        gap,
                    });
                }
                sub.segments[gi].p0 = prev_p1;
            }
            if sub.closed && !sub.segments.is_empty() {
                let first_p0 = sub.segments[0].p0;
                let last = sub.segments.len() - 1;
                let gap = sub.segments[last].p1.distance(first_p0);
                if gap > EPS_WELD {
                    return Err(PathError::UnclosedLoop { subpath: si, gap });
                }
                sub.segments[last].p1 = first_p0;
            }
            // Degenerate removal + weld across the removal (invariant 3;
            // the removed cluster is ≤ 2·EPS_DEGENERATE wide, far under
            // EPS_WELD, so the re-weld is in-tolerance by construction).
            let mut kept: Vec<CubicSeg> = Vec::with_capacity(sub.segments.len());
            for seg in sub.segments.drain(..) {
                if seg.is_degenerate() {
                    continue;
                }
                let mut seg = seg;
                if let Some(prev) = kept.last() {
                    seg.p0 = prev.p1;
                }
                kept.push(seg);
            }
            if sub.closed && !kept.is_empty() {
                let first_p0 = kept[0].p0;
                let last = kept.len() - 1;
                kept[last].p1 = first_p0;
            }
            sub.segments = kept;
        }
        self.subpaths.retain(|s| !s.segments.is_empty());
        Ok(())
    }

    /// Affine transform of every control point (§20.1 f64 math; the
    /// contiguity invariant is preserved bitwise because duplicated
    /// coordinates transform to identical bit patterns — same inputs, same
    /// operations).
    pub fn transform_affine(&self, m: &appname_core::Mat3) -> VectorPath {
        VectorPath {
            id: self.id,
            fill_rule: self.fill_rule,
            subpaths: self
                .subpaths
                .iter()
                .map(|s| SubPath {
                    closed: s.closed,
                    segments: s.segments.iter().map(|g| g.transform_affine(m)).collect(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    /// A unit square as four line-cubics, closed, CCW in y-down doc space.
    pub fn square(x0: f64, y0: f64, s: f64) -> SubPath {
        let p = [v(x0, y0), v(x0 + s, y0), v(x0 + s, y0 + s), v(x0, y0 + s)];
        SubPath {
            closed: true,
            segments: (0..4)
                .map(|i| CubicSeg::line(p[i], p[(i + 1) % 4]))
                .collect(),
        }
    }

    #[test]
    fn validate_accepts_square_and_rejects_gap() {
        let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
        path.subpaths.push(square(1.0, 2.0, 5.0));
        assert_eq!(path.validate(), Ok(()));

        // Nudge one shared coordinate: bitwise contiguity breaks.
        path.subpaths[0].segments[1].p0.x += 1e-12;
        assert!(matches!(
            path.validate(),
            Err(PathError::Discontiguous {
                subpath: 0,
                segment: 0,
                ..
            })
        ));
    }

    #[test]
    fn normalize_rewelds_small_drift_and_rejects_large() {
        let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
        path.subpaths.push(square(0.0, 0.0, 4.0));
        path.subpaths[0].segments[2].p0.x += 1e-7; // within EPS_WELD
        assert_eq!(path.normalize(), Ok(()));
        assert_eq!(path.validate(), Ok(()));

        let mut bad = VectorPath::new(PathId::nil(), FillRule::NonZero);
        bad.subpaths.push(square(0.0, 0.0, 4.0));
        bad.subpaths[0].segments[2].p0.x += 1e-5; // beyond EPS_WELD
        assert!(matches!(
            bad.normalize(),
            Err(PathError::Discontiguous {
                subpath: 0,
                segment: 1,
                ..
            })
        ));
    }

    #[test]
    fn normalize_removes_degenerate_and_rewelds() {
        let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
        let mut sub = square(0.0, 0.0, 4.0);
        // Insert a point-sized segment between segments 1 and 2.
        let p = sub.segments[1].p1;
        let tiny = CubicSeg::new(
            p,
            Vec2::new(p.x + 4e-10, p.y),
            Vec2::new(p.x, p.y + 4e-10),
            Vec2::new(p.x + 2e-10, p.y + 2e-10),
        );
        sub.segments.insert(2, tiny);
        // Successor p0 matches tiny.p1 so the input is contiguous.
        sub.segments[3].p0 = sub.segments[2].p1;
        path.subpaths.push(sub);
        assert_eq!(path.normalize(), Ok(()));
        assert_eq!(path.subpaths[0].segments.len(), 4);
        assert_eq!(path.validate(), Ok(()));
    }

    #[test]
    fn split_reproduces_curve_exactly() {
        let seg = CubicSeg::new(v(0.0, 0.0), v(1.0, 3.0), v(4.0, -1.0), v(5.0, 2.0));
        let (a, b) = seg.split(0.37);
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            let whole = seg.eval(t);
            let piece = if t <= 0.37 {
                a.eval(t / 0.37)
            } else {
                b.eval((t - 0.37) / 0.63)
            };
            assert!(whole.distance(piece) < 1e-12, "t={t}");
        }
        // Halving agrees with generic split at 0.5.
        let (h0, h1) = seg.split_half();
        let (g0, g1) = seg.split(0.5);
        assert!(h0.p1.distance(g0.p1) < 1e-15);
        assert!(h1.c1.distance(g1.c1) < 1e-15);
    }

    #[test]
    fn transform_preserves_bitwise_contiguity() {
        let mut path = VectorPath::new(PathId::nil(), FillRule::EvenOdd);
        path.subpaths.push(square(0.25, 0.75, 3.5));
        // A rotation-ish affine with irrational-looking entries.
        let m = appname_core::Mat3([0.8, 0.6, 0.0, -0.6, 0.8, 0.0, 12.3, -4.5, 1.0]);
        let t = path.transform_affine(&m);
        assert_eq!(t.validate(), Ok(()));
        assert_eq!(t.fill_rule, FillRule::EvenOdd);
    }
}
