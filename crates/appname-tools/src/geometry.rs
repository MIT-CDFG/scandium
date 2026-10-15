// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §19.1 (document geometry, [NORMATIVE] lossless orientation
//! permutations), §19.2 (crop/straighten — spec-verbatim inscribed-rect
//! functions, hide-vs-delete semantics), §19.3 ([NORMATIVE] canvas-size
//! never resamples; anchor offset math; trim algorithm; resize coupling
//! rules); contracts `doc.crop/trim/rotate/canvas_resize/resize.yaml`;
//! DD ㉔ ("crop/canvas ops never resample implicitly" — nothing in this
//! module touches a kernel; resampling happens only in `resample`/`xform`
//! when a command explicitly selects it). Plan §2.3 row.
//!
//! Pure engines only: the `doc.*` KindImpls are blocked on core doors
//! (document-geometry mutation, all-surface enumeration, selection-surface
//! deltas — lane log handoff); every algorithm below is complete, oracle-
//! tested, and consumes core's PUBLIC read surface, so the future wiring
//! is assembly-only.
//!
//! v1 pins: trim reads STRAIGHT color (§19.3 "straight alpha, normalized
//! storage values"): premultiplied storage un-premultiplies (`c/a`, `a>0`)
//! before comparison; the wire has no tolerance param (contract) — the
//! engine takes `t ∈ [0,1]` and the KindImpl passes 0.

use appname_core::{Buffer, IRect, IVec2};

use crate::resample::Plane;

/// §19.1 `Anchor`, snake_case wire order per `doc.canvas_resize.yaml`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub fn from_wire(s: &str) -> Option<Anchor> {
        Some(match s {
            "top_left" => Anchor::TopLeft,
            "top" => Anchor::Top,
            "top_right" => Anchor::TopRight,
            "left" => Anchor::Left,
            "center" => Anchor::Center,
            "right" => Anchor::Right,
            "bottom_left" => Anchor::BottomLeft,
            "bottom" => Anchor::Bottom,
            "bottom_right" => Anchor::BottomRight,
            _ => return None,
        })
    }

    /// §19.3 anchor factors `(ax, ay)`.
    pub fn factors(&self) -> (f64, f64) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        }
    }
}

/// §19.3 [NORMATIVE] anchored canvas-resize offset:
/// `offset = (round(ax·(new.w − old.w)), round(ay·(new.h − old.h)))` —
/// every document-space coordinate translates by `+offset`.
pub fn canvas_anchor_offset(old: IVec2, new: IVec2, anchor: Anchor) -> IVec2 {
    let (ax, ay) = anchor.factors();
    IVec2::new(
        (ax * (new.x - old.x) as f64).round() as i32,
        (ay * (new.y - old.y) as f64).round() as i32,
    )
}

/// §19.2 spec-verbatim: largest centered axis-aligned rect of fixed aspect
/// `a = W/H` inside a `w×h` rect rotated by `alpha` (|alpha| < 90°).
pub fn inscribed_aspect(w: f64, h: f64, alpha: f64, a: f64) -> (f64, f64) {
    let (s, c) = alpha.abs().sin_cos();
    let hh = (w / (a * c + s)).min(h / (a * s + c));
    (a * hh, hh)
}

/// §19.2 spec-verbatim: largest-area inscribed axis-aligned rect, aspect
/// unconstrained.
pub fn inscribed_max_area(w: f64, h: f64, alpha: f64) -> (f64, f64) {
    let (s, c) = alpha.abs().sin_cos();
    let (long, short) = if w >= h { (w, h) } else { (h, w) };
    if short <= 2.0 * s * c * long || (c - s).abs() < 1e-10 {
        let x = 0.5 * short;
        if w >= h {
            (x / s, x / c)
        } else {
            (x / c, x / s)
        }
    } else {
        let cos2 = c * c - s * s;
        ((w * c - h * s) / cos2, (h * c - w * s) / cos2)
    }
}

/// §19.2 straighten normalization: fold `alpha_raw` (radians) into
/// `(−45°, +45°]` by subtracting the nearest multiple of 90°; the document
/// then rotates by the NEGATED result.
pub fn straighten_angle(alpha_raw: f64) -> f64 {
    use core::f64::consts::FRAC_PI_2;
    let mut a = alpha_raw.rem_euclid(FRAC_PI_2); // [0, 90°)
    if a > FRAC_PI_2 / 2.0 {
        a -= FRAC_PI_2; // → (−45°, +45°]
    }
    a
}

/// §19.1 lossless orientation permutations. `angle_q` = quarter-turns CW
/// (0..=3); flips apply AFTER rotation (pin: `doc.rotate` composes
/// rotation-then-flip; combinable with `angle_q = 0` per §10.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orientation {
    pub angle_q: u8,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Orientation {
    /// Whether `angleDeg` takes the lossless path (≡ 0 mod 90, §10.3).
    pub fn from_angle_deg(angle_deg: f64, flip_h: bool, flip_v: bool) -> Option<Orientation> {
        let r = angle_deg.rem_euclid(360.0);
        if (r / 90.0 - (r / 90.0).round()).abs() > 1e-9 {
            return None;
        }
        Some(Orientation {
            angle_q: ((r / 90.0).round() as i64 % 4) as u8,
            flip_h,
            flip_v,
        })
    }

    pub fn is_identity(&self) -> bool {
        self.angle_q == 0 && !self.flip_h && !self.flip_v
    }

    /// New canvas size for a `w×h` canvas.
    pub fn map_size(&self, size: IVec2) -> IVec2 {
        if self.angle_q % 2 == 1 {
            IVec2::new(size.y, size.x)
        } else {
            size
        }
    }

    /// Map a source pixel to its destination position on a `w×h` canvas
    /// (top-left origin, +y down; rotation CW about the canvas center
    /// lattice). Exact integer permutation (§19.1: bit-identical data).
    pub fn map_pixel(&self, p: IVec2, size: IVec2) -> IVec2 {
        let (w, h) = (size.x, size.y);
        // Quarter-turn CW: (x, y) → (h−1−y, x); iterate.
        let (mut x, mut y) = (p.x, p.y);
        let (mut cw, mut ch) = (w, h);
        for _ in 0..self.angle_q {
            let (nx, ny) = (ch - 1 - y, x);
            x = nx;
            y = ny;
            core::mem::swap(&mut cw, &mut ch);
        }
        if self.flip_h {
            x = cw - 1 - x;
        }
        if self.flip_v {
            y = ch - 1 - y;
        }
        IVec2::new(x, y)
    }

    /// Permute a plane (exact copy of channel values — no arithmetic).
    pub fn permute_plane(&self, src: &Plane) -> Plane {
        let size = IVec2::new(src.rect.width(), src.rect.height());
        let out_size = self.map_size(size);
        let mut out = Plane::new(IRect::new(0, 0, out_size.x, out_size.y));
        for y in 0..size.y {
            for x in 0..size.x {
                let d = self.map_pixel(IVec2::new(x, y), size);
                let v = src.px[(y * size.x + x) as usize];
                out.set(d.x, d.y, v);
            }
        }
        out
    }
}

/// §19.3 trim inputs (wire: `doc.trim.yaml`; tolerance is engine-level with
/// the wire pinned at 0 — module pin).
#[derive(Clone, Copy, Debug)]
pub struct TrimSpec {
    pub basis: TrimBasis,
    pub tolerance: f32,
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimBasis {
    Transparent,
    TopLeftColor,
    BottomRightColor,
}

impl TrimBasis {
    pub fn from_wire(s: &str) -> Option<TrimBasis> {
        Some(match s {
            "transparent" => TrimBasis::Transparent,
            "top_left_color" => TrimBasis::TopLeftColor,
            "bottom_right_color" => TrimBasis::BottomRightColor,
            _ => return None,
        })
    }
}

/// A composited-pixel reader for trim (the doc-level trim scans the
/// flattened composite; a single-layer scan passes the layer buffer).
/// `appname_core::Buffer` implements the brush module's `Backdrop`, reused
/// here as the read seam.
pub trait TrimSource {
    fn read(&self, p: IVec2) -> [f32; 4];
}

impl TrimSource for Buffer {
    fn read(&self, p: IVec2) -> [f32; 4] {
        self.read_pixel_f32(p)
    }
}

/// §19.3 trim scan over `canvas`: returns the kept rect `R`, or `None`
/// when everything matches (the §40.1 no-op push /
/// `dialog.trim.nothing_left` case). Straight-alpha comparison (module
/// pin): premultiplied inputs un-premultiply before the color compare.
pub fn trim_rect<S: TrimSource + ?Sized>(
    src: &S,
    canvas: IRect,
    spec: &TrimSpec,
    premultiplied: bool,
) -> Option<IRect> {
    let straight = |p: IVec2| -> [f32; 4] {
        let v = src.read(p);
        if premultiplied && v[3] > 0.0 {
            [v[0] / v[3], v[1] / v[3], v[2] / v[3], v[3]]
        } else {
            v
        }
    };
    let reference = match spec.basis {
        TrimBasis::Transparent => None,
        TrimBasis::TopLeftColor => Some(straight(canvas.min)),
        TrimBasis::BottomRightColor => {
            Some(straight(IVec2::new(canvas.max.x - 1, canvas.max.y - 1)))
        }
    };
    let matches = |p: IVec2| -> bool {
        let v = straight(p);
        match &reference {
            None => v[3] <= spec.tolerance,
            Some(r) => {
                (0..3).all(|c| (v[c] - r[c]).abs() <= spec.tolerance)
                    && (v[3] - r[3]).abs() <= spec.tolerance
            }
        }
    };
    let row_matches = |y: i32, x0: i32, x1: i32| (x0..x1).all(|x| matches(IVec2::new(x, y)));
    let col_matches = |x: i32, y0: i32, y1: i32| (y0..y1).all(|y| matches(IVec2::new(x, y)));

    let mut r = canvas;
    if spec.top {
        while r.min.y < r.max.y && row_matches(r.min.y, r.min.x, r.max.x) {
            r.min.y += 1;
        }
    }
    if spec.bottom {
        while r.max.y > r.min.y && row_matches(r.max.y - 1, r.min.x, r.max.x) {
            r.max.y -= 1;
        }
    }
    if spec.left {
        while r.min.x < r.max.x && col_matches(r.min.x, r.min.y, r.max.y) {
            r.min.x += 1;
        }
    }
    if spec.right {
        while r.max.x > r.min.x && col_matches(r.max.x - 1, r.min.y, r.max.y) {
            r.max.x -= 1;
        }
    }
    if r.is_empty() {
        None // everything matched — nothing left
    } else {
        Some(r)
    }
}

/// `doc.resize` derived-dimension math (§19.3 coupling + contract cross
/// fields): exactly one of `width`/`height`/`longest_edge` is `Some`; the
/// aspect ratio is the ORIGINAL `W:H` ("repeated edits never drift through
/// rounding"). Returns `(w, h)` or `None` when the derived dimension lands
/// outside `1..=1_000_000` (contract → `error.schema.out_of_range`).
pub fn resize_dims(
    old: IVec2,
    width: Option<i64>,
    height: Option<i64>,
    longest_edge: Option<i64>,
    constrain_aspect: bool,
) -> Option<IVec2> {
    let in_range = |v: i64| (1..=1_000_000).contains(&v);
    let derive = |known: i64, from: i32, other: i32| -> i64 {
        ((known as f64) * (other as f64) / (from as f64))
            .round()
            .max(1.0) as i64
    };
    let (w, h) = match (width, height, longest_edge) {
        (Some(w), None, None) => {
            let h = if constrain_aspect {
                derive(w, old.x, old.y)
            } else {
                old.y as i64
            };
            (w, h)
        }
        (None, Some(h), None) => {
            let w = if constrain_aspect {
                derive(h, old.y, old.x)
            } else {
                old.x as i64
            };
            (w, h)
        }
        (None, None, Some(e)) => {
            // longestEdge implies aspect (contract cross-field).
            if old.x >= old.y {
                (e, derive(e, old.x, old.y))
            } else {
                (derive(e, old.y, old.x), e)
            }
        }
        _ => return None,
    };
    if in_range(w) && in_range(h) {
        Some(IVec2::new(w as i32, h as i32))
    } else {
        None
    }
}

/// `doc.crop` semantic gate (contract): rect non-empty and, unless
/// `allow_growth`, intersecting the canvas.
pub fn crop_rect_valid(rect: IRect, canvas: IRect, allow_growth: bool) -> bool {
    !rect.is_empty() && (allow_growth || !rect.intersect(&canvas).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_offsets_match_the_19_3_table() {
        let old = IVec2::new(100, 80);
        let grown = IVec2::new(140, 120);
        let cases = [
            (Anchor::TopLeft, (0, 0)),
            (Anchor::Top, (20, 0)),
            (Anchor::TopRight, (40, 0)),
            (Anchor::Left, (0, 20)),
            (Anchor::Center, (20, 20)),
            (Anchor::Right, (40, 20)),
            (Anchor::BottomLeft, (0, 40)),
            (Anchor::Bottom, (20, 40)),
            (Anchor::BottomRight, (40, 40)),
        ];
        for (a, (ox, oy)) in cases {
            assert_eq!(
                canvas_anchor_offset(old, grown, a),
                IVec2::new(ox, oy),
                "{a:?}"
            );
        }
        // Shrink with odd delta: round() applies (Center of −15 → −7.5 → −8
        // by round-half-away, Rust f64::round).
        let shrunk = IVec2::new(85, 80);
        assert_eq!(
            canvas_anchor_offset(old, shrunk, Anchor::Center),
            IVec2::new(-8, 0)
        );
        // Wire names parse.
        assert_eq!(Anchor::from_wire("bottom_right"), Some(Anchor::BottomRight));
        assert_eq!(Anchor::from_wire("middle"), None);
    }

    #[test]
    fn inscribed_rects_match_known_geometry() {
        // No rotation: the full rect (aspect-locked to its own aspect).
        let (w, h) = inscribed_aspect(100.0, 50.0, 0.0, 2.0);
        assert!((w - 100.0).abs() < 1e-9 && (h - 50.0).abs() < 1e-9);
        // Square at 45°, aspect 1: inscribed square has side s/√2… the
        // formula: hh = min(w/(c+s), h/(s+c)) = 100/√2 → W=H=100/√2.
        let (w, h) = inscribed_aspect(100.0, 100.0, core::f64::consts::FRAC_PI_4, 1.0);
        assert!((w - 100.0 / 2f64.sqrt()).abs() < 1e-9);
        assert!((h - 100.0 / 2f64.sqrt()).abs() < 1e-9);
        // Max-area at 0°… degenerate guard branch: c−s far from 0 requires
        // rotation; use a small angle and check containment constraints.
        let (w, h) = inscribed_max_area(100.0, 50.0, 0.1);
        let (s, c) = 0.1f64.sin_cos();
        // Support constraints of the worst corner hold (within eps).
        assert!(w * c + h * s <= 100.0 + 1e-9);
        assert!(w * s + h * c <= 50.0 + 1e-9);
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn straighten_folds_into_the_pm45_interval() {
        use core::f64::consts::PI;
        let deg = |d: f64| d * PI / 180.0;
        assert!((straighten_angle(deg(10.0)) - deg(10.0)).abs() < 1e-12);
        assert!((straighten_angle(deg(80.0)) - deg(-10.0)).abs() < 1e-12);
        assert!(
            (straighten_angle(deg(45.0)) - deg(45.0)).abs() < 1e-12,
            "+45 stays (interval is half-open at -45)"
        );
        assert!((straighten_angle(deg(135.0)) - deg(45.0)).abs() < 1e-12);
        assert!((straighten_angle(deg(-10.0)) - deg(-10.0)).abs() < 1e-12);
        assert!((straighten_angle(deg(91.0)) - deg(1.0)).abs() < 1e-12);
    }

    fn plane_from(vals: &[&[u32]]) -> Plane {
        let h = vals.len() as i32;
        let w = vals[0].len() as i32;
        let mut p = Plane::new(IRect::new(0, 0, w, h));
        for (y, row) in vals.iter().enumerate() {
            for (x, v) in row.iter().enumerate() {
                p.set(x as i32, y as i32, [*v as f32, 0.0, 0.0, 1.0]);
            }
        }
        p
    }

    #[test]
    fn quarter_turn_permutations_are_exact_and_compose() {
        let src = plane_from(&[&[1, 2, 3], &[4, 5, 6]]); // 3×2
        let r1 = Orientation {
            angle_q: 1,
            flip_h: false,
            flip_v: false,
        };
        let out = r1.permute_plane(&src);
        // CW: (x,y) → (h−1−y, x); 3×2 → 2×3. Top row of src (1,2,3) becomes
        // right column.
        assert_eq!(out.rect, IRect::new(0, 0, 2, 3));
        assert_eq!(
            out.get(1, 0, crate::resample::EdgePolicy::Transparent)[0],
            1.0
        );
        assert_eq!(
            out.get(1, 1, crate::resample::EdgePolicy::Transparent)[0],
            2.0
        );
        assert_eq!(
            out.get(0, 0, crate::resample::EdgePolicy::Transparent)[0],
            4.0
        );
        // Four quarter turns = identity, bit-exact (§19.1 lossless).
        let mut cur = src.clone();
        for _ in 0..4 {
            cur = r1.permute_plane(&cur);
        }
        assert_eq!(cur.px, src.px);
        // flipH twice = identity; 180° = flipH∘flipV.
        let fh = Orientation {
            angle_q: 0,
            flip_h: true,
            flip_v: false,
        };
        assert_eq!(fh.permute_plane(&fh.permute_plane(&src)).px, src.px);
        let r2 = Orientation {
            angle_q: 2,
            flip_h: false,
            flip_v: false,
        };
        let fhv = Orientation {
            angle_q: 0,
            flip_h: true,
            flip_v: true,
        };
        assert_eq!(r2.permute_plane(&src).px, fhv.permute_plane(&src).px);
    }

    #[test]
    fn angle_gate_detects_the_lossless_path() {
        assert_eq!(
            Orientation::from_angle_deg(90.0, false, false),
            Some(Orientation {
                angle_q: 1,
                flip_h: false,
                flip_v: false
            })
        );
        assert_eq!(
            Orientation::from_angle_deg(-90.0, false, false)
                .unwrap()
                .angle_q,
            3,
            "−90 ≡ 270 CW"
        );
        assert_eq!(
            Orientation::from_angle_deg(360.0, false, false)
                .unwrap()
                .angle_q,
            0
        );
        assert!(
            Orientation::from_angle_deg(37.0, false, false).is_none(),
            "arbitrary → resample path"
        );
    }

    struct PlaneSource(Plane);
    impl TrimSource for PlaneSource {
        fn read(&self, p: IVec2) -> [f32; 4] {
            self.0
                .get(p.x, p.y, crate::resample::EdgePolicy::Transparent)
        }
    }

    #[test]
    fn trim_finds_the_tight_content_rect() {
        // 8×6 with content in [2,4)×[1,3) (alpha 1), transparent elsewhere.
        let mut p = Plane::new(IRect::new(0, 0, 8, 6));
        for y in 1..3 {
            for x in 2..4 {
                p.set(x, y, [0.5, 0.25, 0.0, 1.0]);
            }
        }
        let src = PlaneSource(p);
        let spec = TrimSpec {
            basis: TrimBasis::Transparent,
            tolerance: 0.0,
            top: true,
            bottom: true,
            left: true,
            right: true,
        };
        assert_eq!(
            trim_rect(&src, IRect::new(0, 0, 8, 6), &spec, true),
            Some(IRect::new(2, 1, 4, 3))
        );
        // Per-side toggles: only left enabled.
        let spec_l = TrimSpec {
            top: false,
            bottom: false,
            right: false,
            ..spec
        };
        assert_eq!(
            trim_rect(&src, IRect::new(0, 0, 8, 6), &spec_l, true),
            Some(IRect::new(2, 0, 8, 6))
        );
        // Everything transparent → None (nothing left).
        let empty = PlaneSource(Plane::new(IRect::new(0, 0, 4, 4)));
        assert_eq!(trim_rect(&empty, IRect::new(0, 0, 4, 4), &spec, true), None);
    }

    #[test]
    fn trim_color_basis_compares_straight_values() {
        // Border = opaque white; content = one red pixel. top_left_color
        // basis removes the white margin exactly.
        let mut p = Plane::new(IRect::new(0, 0, 5, 5));
        for y in 0..5 {
            for x in 0..5 {
                p.set(x, y, [1.0, 1.0, 1.0, 1.0]);
            }
        }
        p.set(2, 2, [1.0, 0.0, 0.0, 1.0]);
        let src = PlaneSource(p);
        let spec = TrimSpec {
            basis: TrimBasis::TopLeftColor,
            tolerance: 0.0,
            top: true,
            bottom: true,
            left: true,
            right: true,
        };
        assert_eq!(
            trim_rect(&src, IRect::new(0, 0, 5, 5), &spec, true),
            Some(IRect::new(2, 2, 3, 3))
        );
        // Half-covered premultiplied border (0.5,0.5,0.5,0.5) vs opaque gray
        // (straight color equal): straight-alpha compare keeps them DISTINCT
        // because alpha differs (§19.3 match includes the alpha term).
        let mut p2 = Plane::new(IRect::new(0, 0, 3, 1));
        p2.set(0, 0, [0.5, 0.5, 0.5, 0.5]);
        p2.set(1, 0, [1.0, 1.0, 1.0, 1.0]);
        p2.set(2, 0, [0.5, 0.5, 0.5, 0.5]);
        let src2 = PlaneSource(p2);
        let spec2 = TrimSpec {
            basis: TrimBasis::TopLeftColor,
            ..spec
        };
        assert_eq!(
            trim_rect(&src2, IRect::new(0, 0, 3, 1), &spec2, true),
            Some(IRect::new(1, 0, 2, 1))
        );
    }

    #[test]
    fn resize_dims_derive_from_the_original_ratio() {
        let old = IVec2::new(3000, 2000);
        assert_eq!(
            resize_dims(old, Some(1500), None, None, true),
            Some(IVec2::new(1500, 1000))
        );
        assert_eq!(
            resize_dims(old, None, Some(500), None, true),
            Some(IVec2::new(750, 500))
        );
        assert_eq!(
            resize_dims(old, None, None, Some(1200), true),
            Some(IVec2::new(1200, 800))
        );
        // Portrait longest edge.
        assert_eq!(
            resize_dims(IVec2::new(2000, 3000), None, None, Some(1200), true),
            Some(IVec2::new(800, 1200))
        );
        // Unconstrained width keeps height.
        assert_eq!(
            resize_dims(old, Some(100), None, None, false),
            Some(IVec2::new(100, 2000))
        );
        // Derived dimension out of range (contract invalid case analogue).
        assert_eq!(resize_dims(old, None, None, Some(1_000_001), true), None);
        assert_eq!(
            resize_dims(IVec2::new(1_000_000, 10), Some(1_000_000), None, None, true),
            Some(IVec2::new(1_000_000, 10))
        );
        // Rounding to minimum 1.
        assert_eq!(
            resize_dims(IVec2::new(10_000, 10), Some(10), None, None, true),
            Some(IVec2::new(10, 1))
        );
    }

    #[test]
    fn crop_gate_matches_the_contract_examples() {
        let canvas = IRect::new(0, 0, 1000, 800);
        assert!(crop_rect_valid(IRect::new(10, 10, 500, 400), canvas, false));
        // The contract's invalid case: far-away rect without allowGrowth.
        assert!(!crop_rect_valid(
            IRect::new(999_999, 999_999, 1_000_099, 1_000_099),
            canvas,
            false
        ));
        assert!(crop_rect_valid(
            IRect::new(999_999, 999_999, 1_000_099, 1_000_099),
            canvas,
            true
        ));
        assert!(
            !crop_rect_valid(IRect::EMPTY, canvas, true),
            "empty rect always rejects"
        );
    }
}
