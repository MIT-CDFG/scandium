// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.1 ("`Vec2 = glam::DVec2`" — all path math is f64, §20.1
//! invariant 4). The build workspace carries no glam record (deps/ has
//! none), so this is the minimal own f64 pair with exactly the operations
//! the path model needs; if a workspace-wide f64 vector type lands in core
//! later, this aliases to it (fold-in note in the lane log).

use serde::{Deserialize, Serialize};

/// f64 2-vector in document space. `PartialEq` is bitwise-exact on the
/// component bit patterns for the §20.1 contiguity invariant ("bitwise").
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl PartialEq for Vec2 {
    fn eq(&self, o: &Vec2) -> bool {
        // Bitwise: distinguishes 0.0 from -0.0 and is NaN-reflexive —
        // exactly what "p1 == p0 bitwise" (§20.1 invariant 1) asks for.
        self.x.to_bits() == o.x.to_bits() && self.y.to_bits() == o.y.to_bits()
    }
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Vec2 {
        Vec2 { x, y }
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    pub fn dot(&self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// z-component of the 3-D cross product (signed parallelogram area).
    pub fn cross(&self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }

    pub fn length(&self) -> f64 {
        self.x.hypot(self.y)
    }

    pub fn distance(&self, o: Vec2) -> f64 {
        (*self - o).length()
    }

    /// Unit vector; `None` for the zero vector (callers decide the
    /// retracted-handle policy explicitly, §20.1 classification).
    pub fn normalize(&self) -> Option<Vec2> {
        let l = self.length();
        if l == 0.0 {
            None
        } else {
            Some(Vec2::new(self.x / l, self.y / l))
        }
    }

    pub fn lerp(&self, o: Vec2, t: f64) -> Vec2 {
        Vec2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }

    /// Midpoint via the De Casteljau-stable form `(a + b) / 2`.
    pub fn midpoint(&self, o: Vec2) -> Vec2 {
        Vec2::new((self.x + o.x) * 0.5, (self.y + o.y) * 0.5)
    }

    /// Distance from `self` to the closed segment `a..b` (the §20.1
    /// flatness test's "chord p0p1"; degenerate chord → point distance,
    /// which makes the test conservative — see flatten.rs).
    pub fn distance_to_segment(&self, a: Vec2, b: Vec2) -> f64 {
        let ab = b - a;
        let len2 = ab.dot(ab);
        if len2 == 0.0 {
            return self.distance(a);
        }
        let t = ((*self - a).dot(ab) / len2).clamp(0.0, 1.0);
        self.distance(a.lerp(b, t))
    }

    /// Apply a column-major affine `Mat3` (core convention, ES §0.3):
    /// `x' = m0·x + m3·y + m6`, `y' = m1·x + m4·y + m7`. The projective
    /// row (m2, m5, m8) is required to be (0, 0, 1): document/layer
    /// transforms on paths are affine (§20; perspective goes through
    /// `transform.perspective` resampling, not the path model).
    pub fn transform_affine(&self, m: &appname_core::Mat3) -> Vec2 {
        debug_assert!(
            m.0[2] == 0.0 && m.0[5] == 0.0 && m.0[8] == 1.0,
            "path transforms must be affine (projective row (0,0,1))"
        );
        Vec2::new(
            m.0[0] * self.x + m.0[3] * self.y + m.0[6],
            m.0[1] * self.x + m.0[4] * self.y + m.0[7],
        )
    }
}

impl core::ops::Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl core::ops::Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl core::ops::Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}

impl core::ops::Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitwise_eq_distinguishes_signed_zero() {
        assert_ne!(Vec2::new(0.0, 0.0), Vec2::new(-0.0, 0.0));
        assert_eq!(Vec2::new(1.5, -2.25), Vec2::new(1.5, -2.25));
    }

    #[test]
    fn distance_to_segment_cases() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(10.0, 0.0);
        assert_eq!(Vec2::new(5.0, 3.0).distance_to_segment(a, b), 3.0);
        // Beyond the end: distance to the endpoint, not the infinite line.
        assert_eq!(Vec2::new(13.0, 4.0).distance_to_segment(a, b), 5.0);
        // Degenerate chord.
        assert_eq!(Vec2::new(3.0, 4.0).distance_to_segment(a, a), 5.0);
    }

    #[test]
    fn affine_transform_column_major() {
        // translate(3, 4) in column-major [f64; 9].
        let m = appname_core::Mat3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 3.0, 4.0, 1.0]);
        assert_eq!(
            Vec2::new(1.0, 2.0).transform_affine(&m),
            Vec2::new(4.0, 6.0)
        );
        // scale(2) about the origin.
        let s = appname_core::Mat3([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            Vec2::new(1.5, -2.0).transform_affine(&s),
            Vec2::new(3.0, -4.0)
        );
    }
}
