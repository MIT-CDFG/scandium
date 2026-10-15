// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §20.1 "Anchor ↔ segment derivation" — the STORED form is
//! segments; editing views the path as anchors. Classification (exact
//! formulas, ε = 1e−4) and the [NORMATIVE] maintenance-on-drag math live
//! here as pure functions; the pen state machine that drives them is
//! CP-1.0 (§20.4).

use crate::geom::Vec2;
use crate::path::SubPath;

/// §20.1 anchor kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorKind {
    Corner,
    Smooth,
    Symmetric,
}

/// §20.1 `Anchor2` — the editing view of one on-curve point. A retracted
/// handle is stored equal to `point`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor2 {
    pub point: Vec2,
    pub in_handle: Vec2,
    pub out_handle: Vec2,
    pub kind: AnchorKind,
}

const CLASSIFY_EPS: f64 = 1e-4;

/// §20.1 classification with `u = point − in_handle`, `v = out_handle −
/// point`:
/// `Smooth: |u × v| ≤ 1e−4·|u|·|v| and u·v > 0` (collinear, same
/// direction); `Symmetric: Smooth and ||u| − |v|| ≤ 1e−4·max(|u|, |v|)`;
/// `Corner` otherwise, including any retracted handle (|u| = 0 or |v| = 0).
pub fn classify(point: Vec2, in_handle: Vec2, out_handle: Vec2) -> AnchorKind {
    let u = point - in_handle;
    let v = out_handle - point;
    let (lu, lv) = (u.length(), v.length());
    if lu == 0.0 || lv == 0.0 {
        return AnchorKind::Corner;
    }
    let smooth = u.cross(v).abs() <= CLASSIFY_EPS * lu * lv && u.dot(v) > 0.0;
    if !smooth {
        return AnchorKind::Corner;
    }
    if (lu - lv).abs() <= CLASSIFY_EPS * lu.max(lv) {
        AnchorKind::Symmetric
    } else {
        AnchorKind::Smooth
    }
}

/// Derive the anchor view of a subpath (§20.1): anchor `i` has
/// `point = segments[i].p0`, `in_handle = segments[i−1].c2` (wrapping if
/// closed; equal to `point` at an open end), `out_handle = segments[i].c1`.
/// An OPEN subpath additionally ends with the terminal anchor at
/// `last.p1` (retracted out-handle).
pub fn anchors(sub: &SubPath) -> Vec<Anchor2> {
    let n = sub.segments.len();
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let point = sub.segments[i].p0;
        let in_handle = if i > 0 {
            sub.segments[i - 1].c2
        } else if sub.closed {
            sub.segments[n - 1].c2
        } else {
            point // open start: retracted
        };
        let out_handle = sub.segments[i].c1;
        out.push(Anchor2 {
            point,
            in_handle,
            out_handle,
            kind: classify(point, in_handle, out_handle),
        });
    }
    if !sub.closed {
        let point = sub.segments[n - 1].p1;
        let in_handle = sub.segments[n - 1].c2;
        out.push(Anchor2 {
            point,
            in_handle,
            out_handle: point, // open end: retracted
            kind: classify(point, in_handle, point),
        });
    }
    out
}

impl Anchor2 {
    /// [NORMATIVE §20.1] drag `out_handle` to `new_out`. Symmetric mirrors
    /// the in-handle exactly (`in = 2·point − out`); Smooth keeps the
    /// in-handle's own LENGTH and rotates it collinear-opposite
    /// (`in = point − |point − in_old| · normalize(out − point)`; if the
    /// dragged handle retracts to zero length the rotation is undefined and
    /// the in-handle keeps its position); Corner handles are independent.
    /// Kind is re-derived from the resulting geometry.
    pub fn drag_out_handle(&self, new_out: Vec2) -> Anchor2 {
        let in_handle = match self.kind {
            AnchorKind::Symmetric => self.point + (self.point - new_out),
            AnchorKind::Smooth => match (new_out - self.point).normalize() {
                Some(dir) => self.point - dir * self.point.distance(self.in_handle),
                None => self.in_handle,
            },
            AnchorKind::Corner => self.in_handle,
        };
        Anchor2 {
            point: self.point,
            in_handle,
            out_handle: new_out,
            kind: classify(self.point, in_handle, new_out),
        }
    }

    /// Mirrored for the in side (§20.1 "and mirrored for the in side").
    pub fn drag_in_handle(&self, new_in: Vec2) -> Anchor2 {
        let out_handle = match self.kind {
            AnchorKind::Symmetric => self.point + (self.point - new_in),
            AnchorKind::Smooth => match (self.point - new_in).normalize() {
                Some(dir) => self.point + dir * self.point.distance(self.out_handle),
                None => self.out_handle,
            },
            AnchorKind::Corner => self.out_handle,
        };
        Anchor2 {
            point: self.point,
            in_handle: new_in,
            out_handle,
            kind: classify(self.point, new_in, out_handle),
        }
    }

    /// [NORMATIVE §20.1] "Dragging the anchor `point` translates both
    /// handles rigidly." Kind never changes under rigid translation.
    pub fn drag_point(&self, new_point: Vec2) -> Anchor2 {
        let d = new_point - self.point;
        Anchor2 {
            point: new_point,
            in_handle: self.in_handle + d,
            out_handle: self.out_handle + d,
            kind: self.kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::CubicSeg;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn classification_matrix() {
        let p = v(10.0, 10.0);
        // Symmetric: equal-length collinear opposite handles.
        assert_eq!(
            classify(p, v(8.0, 10.0), v(12.0, 10.0)),
            AnchorKind::Symmetric
        );
        // Smooth: collinear, same direction, different lengths.
        assert_eq!(classify(p, v(9.0, 10.0), v(15.0, 10.0)), AnchorKind::Smooth);
        // Corner: bent.
        assert_eq!(classify(p, v(8.0, 10.0), v(10.0, 14.0)), AnchorKind::Corner);
        // Corner: collinear but SAME side (u·v < 0).
        assert_eq!(
            classify(p, v(12.0, 10.0), v(12.5, 10.0)),
            AnchorKind::Corner
        );
        // Corner: retracted handle.
        assert_eq!(classify(p, p, v(12.0, 10.0)), AnchorKind::Corner);
    }

    #[test]
    fn symmetric_drag_mirrors_exactly() {
        let a = Anchor2 {
            point: v(5.0, 5.0),
            in_handle: v(3.0, 5.0),
            out_handle: v(7.0, 5.0),
            kind: AnchorKind::Symmetric,
        };
        let d = a.drag_out_handle(v(5.0, 9.0));
        assert_eq!(d.in_handle, v(5.0, 1.0));
        assert_eq!(d.kind, AnchorKind::Symmetric);
    }

    #[test]
    fn smooth_drag_keeps_opposite_length() {
        let a = Anchor2 {
            point: v(0.0, 0.0),
            in_handle: v(-2.0, 0.0),
            out_handle: v(6.0, 0.0),
            kind: AnchorKind::Smooth,
        };
        let d = a.drag_out_handle(v(0.0, 3.0));
        // In-handle length preserved (2.0), rotated opposite the new out.
        assert!((d.in_handle.distance(d.point) - 2.0).abs() < 1e-12);
        assert!(d.in_handle.distance(v(0.0, -2.0)) < 1e-12);
        assert_eq!(d.kind, AnchorKind::Smooth);
    }

    #[test]
    fn corner_drag_is_independent_and_point_drag_rigid() {
        let a = Anchor2 {
            point: v(1.0, 1.0),
            in_handle: v(0.0, 1.0),
            out_handle: v(1.0, 3.0),
            kind: AnchorKind::Corner,
        };
        let d = a.drag_out_handle(v(4.0, 4.0));
        assert_eq!(d.in_handle, v(0.0, 1.0));
        let m = a.drag_point(v(11.0, 21.0));
        assert_eq!(m.in_handle, v(10.0, 21.0));
        assert_eq!(m.out_handle, v(11.0, 23.0));
        assert_eq!(m.kind, AnchorKind::Corner);
    }

    #[test]
    fn derivation_open_and_closed() {
        // Open two-segment polyline: 3 anchors, ends retracted.
        let sub = SubPath {
            closed: false,
            segments: vec![
                CubicSeg::line(v(0.0, 0.0), v(4.0, 0.0)),
                CubicSeg::line(v(4.0, 0.0), v(4.0, 4.0)),
            ],
        };
        let a = anchors(&sub);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].in_handle, a[0].point);
        assert_eq!(a[2].out_handle, a[2].point);
        // Interior anchor of a straight polyline: collinear one-third
        // handles point the same direction? No — in = c2 of seg0 lies
        // BEFORE the point, out = c1 of seg1 lies after: Corner here
        // because the two edges meet at 90°.
        assert_eq!(a[1].kind, AnchorKind::Corner);

        // Closed square: 4 anchors, all corners, wrap-derived in-handles.
        let sq = {
            let p = [v(0.0, 0.0), v(4.0, 0.0), v(4.0, 4.0), v(0.0, 4.0)];
            SubPath {
                closed: true,
                segments: (0..4)
                    .map(|i| CubicSeg::line(p[i], p[(i + 1) % 4]))
                    .collect(),
            }
        };
        let a = anchors(&sq);
        assert_eq!(a.len(), 4);
        assert!(a.iter().all(|x| x.kind == AnchorKind::Corner));
        assert_eq!(a[0].in_handle, sq.segments[3].c2);
    }

    #[test]
    fn straight_line_interior_anchor_is_smooth_when_collinear() {
        let sub = SubPath {
            closed: false,
            segments: vec![
                CubicSeg::line(v(0.0, 0.0), v(4.0, 0.0)),
                CubicSeg::line(v(4.0, 0.0), v(8.0, 0.0)),
            ],
        };
        let a = anchors(&sub);
        // Equal one-third handle lengths on both sides → Symmetric.
        assert_eq!(a[1].kind, AnchorKind::Symmetric);
    }
}
