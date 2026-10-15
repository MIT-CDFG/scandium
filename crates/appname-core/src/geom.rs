// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.1 (scalars, geometry) — integer pixel geometry used by the tile
//! substrate and document graph.
//!
//! The spec's `appname-geom` sketch types `Vec2`/`Mat3` on `glam`; `glam` has
//! no deps/ record at CP-0.1, so this module carries minimal own definitions
//! with the exact §2.1 semantics for the parts CP-0.1 uses: [`IVec2`],
//! [`IRect`] (half-open, tile iteration with Euclidean division), and a
//! column-major [`Mat3`] value type (identity only for now — composition /
//! decomposition arrive with the transform work, ES §2.1/§18). Swapping the
//! backing to `glam` later is a representation change, not an API change.

use serde::{Deserialize, Serialize};

/// Integer pixel-coordinate pair (ES §2.1 `IVec2`).
///
/// Serializes as a 2-array `[x, y]` to match the `document.v1.json` size
/// fields (`imageSize`/`canvasSize` are `[w, h]` arrays).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IVec2 {
    pub x: i32,
    pub y: i32,
}

impl IVec2 {
    pub const ZERO: IVec2 = IVec2 { x: 0, y: 0 };

    pub const fn new(x: i32, y: i32) -> IVec2 {
        IVec2 { x, y }
    }
}

impl core::ops::Add for IVec2 {
    type Output = IVec2;
    fn add(self, o: IVec2) -> IVec2 {
        IVec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Serialize for IVec2 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        [self.x, self.y].serialize(s)
    }
}

impl<'de> Deserialize<'de> for IVec2 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let [x, y] = <[i32; 2]>::deserialize(d)?;
        Ok(IVec2 { x, y })
    }
}

/// Integer pixel rectangle, half-open `[min, max)`. Empty if `min >= max` on
/// any axis (ES §2.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct IRect {
    pub min: IVec2,
    pub max: IVec2,
}

impl IRect {
    pub const EMPTY: IRect = IRect {
        min: IVec2::ZERO,
        max: IVec2::ZERO,
    };

    pub const fn new(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> IRect {
        IRect {
            min: IVec2::new(min_x, min_y),
            max: IVec2::new(max_x, max_y),
        }
    }

    pub fn width(&self) -> i32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> i32 {
        self.max.y - self.min.y
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Component-wise max(min)/min(max). May yield `min >= max`; `is_empty` is
    /// the only validity test — code MUST NOT assume normalized empties compare
    /// equal to `EMPTY` (ES §2.1).
    pub fn intersect(&self, o: &IRect) -> IRect {
        IRect {
            min: IVec2::new(self.min.x.max(o.min.x), self.min.y.max(o.min.y)),
            max: IVec2::new(self.max.x.min(o.max.x), self.max.y.min(o.max.y)),
        }
    }

    /// Component-wise min(min)/max(max), empty-aware: union with an empty rect
    /// returns the other operand unchanged — an empty rect has no location
    /// (ES §2.1).
    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        IRect {
            min: IVec2::new(self.min.x.min(o.min.x), self.min.y.min(o.min.y)),
            max: IVec2::new(self.max.x.max(o.max.x), self.max.y.max(o.max.y)),
        }
    }

    pub fn translate(&self, d: IVec2) -> IRect {
        IRect {
            min: self.min + d,
            max: self.max + d,
        }
    }

    /// Grow (positive) or shrink (negative) on all four sides; shrinking may
    /// produce an empty rect, which every consumer must tolerate (ES §2.1).
    pub fn inflate(&self, dx: i32, dy: i32) -> IRect {
        IRect {
            min: IVec2::new(self.min.x - dx, self.min.y - dy),
            max: IVec2::new(self.max.x + dx, self.max.y + dy),
        }
    }

    pub fn contains(&self, p: IVec2) -> bool {
        p.x >= self.min.x && p.x < self.max.x && p.y >= self.min.y && p.y < self.max.y
    }

    pub fn contains_rect(&self, o: &IRect) -> bool {
        o.is_empty()
            || (o.min.x >= self.min.x
                && o.min.y >= self.min.y
                && o.max.x <= self.max.x
                && o.max.y <= self.max.y)
    }

    /// u64 because a gigapixel-scale rect overflows i32*i32 (ES §2.1).
    pub fn area(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            self.width() as u64 * self.height() as u64
        }
    }

    /// Tiles (at a given size) that this rect touches.
    ///
    /// [NORMATIVE, ES §2.1] Grid cell `(tx, ty)` covers pixels
    /// `[tx*tile, (tx+1)*tile) × [ty*tile, (ty+1)*tile)`. The first column is
    /// `floor(min.x / tile)` computed with Euclidean division — for negative
    /// coordinates `(-1).div_euclid(256) == -1`, whereas truncating `/` would
    /// wrongly give 0. The exclusive end column is
    /// `ceil(max.x / tile) = (max.x + tile - 1).div_euclid(tile)`, an identity
    /// that holds for all i32 values when `tile > 0`. Yields row-major
    /// deterministic order (ty ascending outer, tx ascending inner); an empty
    /// rect yields nothing.
    pub fn tiles(&self, tile: i32) -> impl Iterator<Item = (i32, i32)> {
        debug_assert!(tile > 0);
        let (x0, x1, y0, y1) = if self.is_empty() {
            (0, 0, 0, 0)
        } else {
            (
                self.min.x.div_euclid(tile),
                (self.max.x + tile - 1).div_euclid(tile),
                self.min.y.div_euclid(tile),
                (self.max.y + tile - 1).div_euclid(tile),
            )
        };
        (y0..y1).flat_map(move |ty| (x0..x1).map(move |tx| (tx, ty)))
    }
}

/// Column-major 3×3 f64 affine matrix (ES §0.3/§2.1 convention), stored as the
/// 9 column-major scalars — the same layout `document.v1.json` serializes for
/// the layer `transform` field (a 9-array).
///
/// CP-0.1 carries only the identity value and (de)serialization; `compose` /
/// `decompose` per the §2.1 `Trs2` convention land with the transform commands
/// (ES §18).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mat3(pub [f64; 9]);

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn is_identity(&self) -> bool {
        *self == Mat3::IDENTITY
    }
}

impl Default for Mat3 {
    fn default() -> Mat3 {
        Mat3::IDENTITY
    }
}

impl Serialize for Mat3 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Mat3 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Mat3(<[f64; 9]>::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_worked_cases_from_spec() {
        // ES §2.1 worked cases at tile = 256.
        let cols = |r: IRect| r.tiles(256).collect::<Vec<_>>();
        assert_eq!(cols(IRect::new(-1, 0, 1, 1)), vec![(-1, 0), (0, 0)]);
        assert_eq!(cols(IRect::new(-256, 0, -255, 1)), vec![(-1, 0)]);
        assert_eq!(cols(IRect::new(0, 0, 256, 1)), vec![(0, 0)]);
        assert_eq!(cols(IRect::new(0, 0, 257, 1)), vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn tiles_row_major_order_and_empty() {
        let r = IRect::new(-10, -10, 300, 300);
        let got: Vec<_> = r.tiles(256).collect();
        assert_eq!(
            got,
            vec![
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (0, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1)
            ]
        );
        assert_eq!(IRect::EMPTY.tiles(256).count(), 0);
        // Degenerate (min >= max on one axis) also yields nothing.
        assert_eq!(IRect::new(5, 5, 5, 900).tiles(256).count(), 0);
    }

    #[test]
    fn union_with_empty_returns_other_operand() {
        let a = IRect::new(10, 10, 20, 20);
        assert_eq!(a.union(&IRect::EMPTY), a);
        assert_eq!(IRect::EMPTY.union(&a), a);
        // A degenerate non-EMPTY empty rect also has no location.
        let degenerate = IRect::new(500, 500, 500, 600);
        assert_eq!(a.union(&degenerate), a);
    }

    #[test]
    fn intersect_may_be_degenerate_and_is_empty_is_the_test() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(20, 20, 30, 30);
        let i = a.intersect(&b);
        assert!(i.is_empty());
        // Not normalized to EMPTY — is_empty is the only validity test.
        assert_ne!(i, IRect::EMPTY);
    }

    #[test]
    fn contains_half_open_semantics() {
        let r = IRect::new(0, 0, 256, 256);
        assert!(r.contains(IVec2::new(0, 0)));
        assert!(r.contains(IVec2::new(255, 255)));
        assert!(!r.contains(IVec2::new(256, 0)));
        assert!(!r.contains(IVec2::new(-1, 0)));
        assert!(r.contains_rect(&IRect::EMPTY));
        assert!(r.contains_rect(&IRect::new(0, 0, 256, 256)));
        assert!(!r.contains_rect(&IRect::new(0, 0, 257, 256)));
    }

    #[test]
    fn area_uses_u64() {
        // A gigapixel-scale rect: 2^16 × 2^16 = 2^32 > i32::MAX.
        let r = IRect::new(0, 0, 65536, 65536);
        assert_eq!(r.area(), 1u64 << 32);
        assert_eq!(IRect::EMPTY.area(), 0);
    }

    #[test]
    fn ivec2_serializes_as_2_array() {
        let v = IVec2::new(1920, 1080);
        assert_eq!(serde_json::to_string(&v).unwrap(), "[1920,1080]");
        let back: IVec2 = serde_json::from_str("[1920,1080]").unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn mat3_identity_serializes_as_9_array() {
        let m = Mat3::IDENTITY;
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            "[1.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0]"
        );
        let back: Mat3 = serde_json::from_str("[1,0,0,0,1,0,0,0,1]").unwrap();
        assert!(back.is_identity());
    }
}
