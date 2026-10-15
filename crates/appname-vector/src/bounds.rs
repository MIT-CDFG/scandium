// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §20.1 "Bounds" — control-polygon AABBs are the conservative
//! bound (culling); tight bounds evaluate the axis-wise extrema of each
//! cubic: solve `a·t² + b·t + c = 0` with `a = 3(p1 − 3c2 + 3c1 − p0)`,
//! `b = 6(p0 − 2c1 + c2)`, `c = 3(c1 − p0)`; `|a| < 1e−12` → linear root
//! `t = −c/b`; keep roots in (0, 1); evaluate B at {0, roots, 1}.

use crate::geom::Vec2;
use crate::path::{CubicSeg, VectorPath};

/// f64 document-space AABB (mirrors ES §2.1 `Rect`; local until a shared
/// float rect lands in core — fold-in note in the lane log).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect2 {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect2 {
    fn including(mut self, p: Vec2) -> Rect2 {
        self.min = Vec2::new(self.min.x.min(p.x), self.min.y.min(p.y));
        self.max = Vec2::new(self.max.x.max(p.x), self.max.y.max(p.y));
        self
    }

    fn seed(p: Vec2) -> Rect2 {
        Rect2 { min: p, max: p }
    }

    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    pub fn contains_rect(&self, o: &Rect2) -> bool {
        o.min.x >= self.min.x
            && o.min.y >= self.min.y
            && o.max.x <= self.max.x
            && o.max.y <= self.max.y
    }
}

/// Conservative control-polygon AABB (§20.1). `None` for an empty path.
pub fn control_bounds(path: &VectorPath) -> Option<Rect2> {
    let mut acc: Option<Rect2> = None;
    for sub in &path.subpaths {
        for seg in &sub.segments {
            for p in [seg.p0, seg.c1, seg.c2, seg.p1] {
                acc = Some(match acc {
                    None => Rect2::seed(p),
                    Some(r) => r.including(p),
                });
            }
        }
    }
    acc
}

/// Per-axis interior extremum parameters of one cubic (§20.1 formulas).
fn axis_extrema(p0: f64, c1: f64, c2: f64, p1: f64, out: &mut Vec<f64>) {
    let a = 3.0 * (p1 - 3.0 * c2 + 3.0 * c1 - p0);
    let b = 6.0 * (p0 - 2.0 * c1 + c2);
    let c = 3.0 * (c1 - p0);
    if a.abs() < 1e-12 {
        // Linear: t = −c/b (guarded: b ≈ 0 means constant derivative —
        // no interior extremum).
        if b.abs() > 0.0 {
            let t = -c / b;
            if t > 0.0 && t < 1.0 {
                out.push(t);
            }
        }
        return;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return;
    }
    let sq = disc.sqrt();
    for t in [(-b + sq) / (2.0 * a), (-b - sq) / (2.0 * a)] {
        if t > 0.0 && t < 1.0 {
            out.push(t);
        }
    }
}

/// Tight per-segment bounds: B evaluated at {0, interior extrema, 1}.
pub fn seg_tight_bounds(seg: &CubicSeg) -> Rect2 {
    let mut r = Rect2::seed(seg.p0).including(seg.p1);
    let mut ts: Vec<f64> = Vec::with_capacity(4);
    axis_extrema(seg.p0.x, seg.c1.x, seg.c2.x, seg.p1.x, &mut ts);
    axis_extrema(seg.p0.y, seg.c1.y, seg.c2.y, seg.p1.y, &mut ts);
    for t in ts {
        r = r.including(seg.eval(t));
    }
    r
}

/// Tight path bounds (§20.1). `None` for an empty path.
pub fn tight_bounds(path: &VectorPath) -> Option<Rect2> {
    let mut acc: Option<Rect2> = None;
    for sub in &path.subpaths {
        for seg in &sub.segments {
            let r = seg_tight_bounds(seg);
            acc = Some(match acc {
                None => r,
                Some(a) => a.including(r.min).including(r.max),
            });
        }
    }
    acc
}

/// Test-only shared fixtures (used by flatten/raster oracle tests too).
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::path::{FillRule, SubPath};
    use appname_core::PathId;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    /// κ-cubic full circle (§20.4 primitive convention), center c radius r.
    pub fn circle_path(cx: f64, cy: f64, r: f64) -> VectorPath {
        const K: f64 = 0.5522847498; // §7.5 worked-example constant
        let k = K * r;
        let p = [v(cx + r, cy), v(cx, cy + r), v(cx - r, cy), v(cx, cy - r)];
        let segs = vec![
            CubicSeg::new(p[0], v(cx + r, cy + k), v(cx + k, cy + r), p[1]),
            CubicSeg::new(p[1], v(cx - k, cy + r), v(cx - r, cy + k), p[2]),
            CubicSeg::new(p[2], v(cx - r, cy - k), v(cx - k, cy - r), p[3]),
            CubicSeg::new(p[3], v(cx + k, cy - r), v(cx + r, cy - k), p[0]),
        ];
        let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
        path.subpaths.push(SubPath {
            closed: true,
            segments: segs,
        });
        path
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::circle_path;
    use super::*;
    use crate::path::{FillRule, SubPath};
    use appname_core::PathId;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn line_cubic_tight_bounds_is_endpoint_box() {
        let seg = CubicSeg::line(v(2.0, 7.0), v(9.0, 3.0));
        let r = seg_tight_bounds(&seg);
        assert_eq!(
            r,
            Rect2 {
                min: v(2.0, 3.0),
                max: v(9.0, 7.0)
            }
        );
    }

    #[test]
    fn circle_tight_bounds_hits_extrema_and_control_bounds_contain() {
        let path = circle_path(10.0, 20.0, 5.0);
        let tight = tight_bounds(&path).unwrap();
        // κ-approximation stays within ~2.8e-4·r of the true circle.
        let tol = 5.0 * 3e-4;
        assert!((tight.min.x - 5.0).abs() < tol);
        assert!((tight.max.x - 15.0).abs() < tol);
        assert!((tight.min.y - 15.0).abs() < tol);
        assert!((tight.max.y - 25.0).abs() < tol);
        let ctrl = control_bounds(&path).unwrap();
        assert!(ctrl.contains_rect(&tight));
    }

    #[test]
    fn interior_extremum_found_on_arch() {
        // Symmetric arch: apex above both endpoints at t = 0.5.
        let seg = CubicSeg::new(v(0.0, 0.0), v(2.0, -6.0), v(4.0, -6.0), v(6.0, 0.0));
        let r = seg_tight_bounds(&seg);
        let apex = seg.eval(0.5);
        assert!((r.min.y - apex.y).abs() < 1e-12);
        assert_eq!(r.min.x, 0.0);
        assert_eq!(r.max.x, 6.0);
    }

    #[test]
    fn control_bounds_always_contain_tight_bounds_seeded_property() {
        // Fixed-seed xorshift64* — deterministic property sweep (rule:
        // fixed-seed property tests, plan §3.10 task note).
        let mut s: u64 = 0x9E3779B97F4A7C15;
        let mut next = move || {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            let r = s.wrapping_mul(0x2545F4914F6CDD1D);
            (r >> 11) as f64 / (1u64 << 53) as f64 * 200.0 - 100.0
        };
        for _ in 0..200 {
            let seg = CubicSeg::new(
                v(next(), next()),
                v(next(), next()),
                v(next(), next()),
                v(next(), next()),
            );
            let mut path = VectorPath::new(PathId::nil(), FillRule::NonZero);
            path.subpaths.push(SubPath {
                closed: false,
                segments: vec![seg],
            });
            let tight = tight_bounds(&path).unwrap();
            let ctrl = control_bounds(&path).unwrap();
            assert!(ctrl.contains_rect(&tight));
            // And the tight box really bounds dense samples.
            for i in 0..=64 {
                let p = seg.eval(i as f64 / 64.0);
                assert!(p.x >= tight.min.x - 1e-9 && p.x <= tight.max.x + 1e-9);
                assert!(p.y >= tight.min.y - 1e-9 && p.y <= tight.max.y + 1e-9);
            }
        }
    }
}
