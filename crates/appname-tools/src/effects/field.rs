// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The shared machinery of §11.3 "Rendering algorithms".
//!
//! Every one of the five MVP effects is a construction over exactly two
//! primitives, and this module owns the reuse of both rather than letting
//! each effect reach for its own:
//!
//! * **`A(p)`** — the layer's own composited coverage **at full fill
//!   opacity**, and **`d(p)`** — its signed Euclidean distance field
//!   (positive outside the 0.5-thresholded `A`), from the §13.2
//!   Felzenszwalb–Huttenlocher EDT in [`crate::select::refine`].
//! * **blur** — the §16.3 separable Gaussian at `σ = radius/3`, via the
//!   crate's single blur dispatch, [`refine::blur_plane_sigma`].
//!
//! "Dilate by `r`" is §13.2's threshold-expand through `d` with the 1-px
//! smoothstep re-AA — i.e. the same `smoothAA(d − r)` that §13.2 `Grow`
//! uses. Nothing here re-derives a distance transform, a Gaussian kernel or
//! an AA ramp; a second implementation of any of the three would be a
//! second answer to a question the engine has already answered once.
//!
//! **Nothing in this module blends.** Effects produce coverage; the
//! Porter-Duff/blend arithmetic of §4.4/§4.5 stays in `appname-render`,
//! which is the one compositing truth. That split is also why the stack
//! lives here and not there: `appname-render` depends only on
//! `appname-core` and cannot see the EDT, while inverting the layering to
//! give it one would be a far larger change than layer styles warrant.

use appname_core::IRect;

use crate::select::refine::{self, smooth_aa};
use crate::select::MaskPlane;

/// §11.3: all effect blurs are the §16.3 Gaussian with `σ = radius/3`
/// (radius ≈ 3σ). The one place that ratio is written down.
#[inline]
pub(crate) fn sigma_for_radius(radius_px: f32) -> f64 {
    (radius_px as f64 / 3.0).max(0.0)
}

/// The §11.3 shadow offset for UI angle `α`.
///
/// Document convention is +x right, +y **down**. §11.3 pins it: for UI
/// angle `α` (CCW, `0` = light from the right) the light direction is
/// `(cos α, −sin α)`, and a shadow offsets **away** from the light along
/// `(−cos α, +sin α)`. The `θ` of the section's earlier prose formula
/// `(distance·cos θ, distance·sin θ)` is therefore `θ = α + 180°`, not `α`
/// — reading the prose alone and substituting the UI angle directly puts
/// every shadow on the wrong side of its layer.
#[inline]
pub(crate) fn shadow_offset(angle_deg: f32, distance_px: f32) -> (f32, f32) {
    let a = (angle_deg as f64).to_radians();
    let dx = -(a.cos() as f32) * distance_px;
    let dy = (a.sin() as f32) * distance_px;
    (dx, dy)
}

/// §11.3 `f` — "a linear ramp clamped to `[0, 1]`", decreasing: `1` at the
/// contour (`t = 0`), `0` at and beyond `t = 1`.
///
/// **The clamp is load-bearing, not cosmetic.** Both glows form `t` by
/// dividing a signed distance by a size, and `size_px = 0` is inside the
/// §11.3 range for a glow, so `t = ±∞` is reachable. The clamp is the
/// single thing that keeps a non-finite sample out of the pass coverage —
/// nothing downstream of a coverage plane re-checks, and an infinity or a
/// NaN would ride straight through the compositor into an exported pixel.
/// `effects::tests::degenerate_inputs_stay_finite` is the falsifier.
#[inline]
pub(crate) fn ramp(t: f32) -> f32 {
    (1.0 - t).clamp(0.0, 1.0)
}

/// The precomputed field an effect stack evaluates against: `A` and `d`
/// over one common working rect.
///
/// One rect for the whole stack (the union of every effect's reach) keeps
/// each pass index-aligned, so the compositor can walk them without
/// per-pass resampling.
pub(crate) struct Field {
    pub rect: IRect,
    /// `A` — coverage at full fill opacity, resampled onto `rect`.
    pub a: MaskPlane,
    /// `d` — signed distance, positive outside, row-major over `rect`.
    pub d: Vec<f32>,
}

impl Field {
    pub(crate) fn new(a: &MaskPlane, rect: IRect) -> Field {
        let mut plane = MaskPlane::new(rect);
        let w = rect.width().max(0) as usize;
        for y in 0..rect.height().max(0) as usize {
            for x in 0..w {
                plane.v[y * w + x] = a.get(rect.min.x + x as i32, rect.min.y + y as i32);
            }
        }
        let d = refine::signed_distance(&plane, rect);
        Field { rect, a: plane, d }
    }

    #[inline]
    pub(crate) fn width(&self) -> usize {
        self.rect.width().max(0) as usize
    }

    #[inline]
    pub(crate) fn height(&self) -> usize {
        self.rect.height().max(0) as usize
    }

    /// A new zeroed plane over the field's rect.
    pub(crate) fn empty_plane(&self) -> MaskPlane {
        MaskPlane::new(self.rect)
    }
}

/// Bilinear sample of a coverage plane at a fractional document position;
/// absent reads are 0 (§2.4/§13.2).
#[inline]
fn sample_bilinear(src: &MaskPlane, x: f32, y: f32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let (x0, y0) = (x0 as i32, y0 as i32);
    let v00 = src.get(x0, y0);
    let v10 = src.get(x0 + 1, y0);
    let v01 = src.get(x0, y0 + 1);
    let v11 = src.get(x0 + 1, y0 + 1);
    let top = v00 + (v10 - v00) * fx;
    let bot = v01 + (v11 - v01) * fx;
    top + (bot - top) * fy
}

/// Translate a coverage plane by `(dx, dy)` document px onto `rect`,
/// bilinear for the subpixel part (`distance_px` is an `f32`, so subpixel
/// offsets are reachable and must not snap).
pub(crate) fn translate(src: &MaskPlane, rect: IRect, dx: f32, dy: f32) -> MaskPlane {
    let mut out = MaskPlane::new(rect);
    let w = rect.width().max(0) as usize;
    for y in 0..rect.height().max(0) as usize {
        for x in 0..w {
            let sx = (rect.min.x + x as i32) as f32 - dx;
            let sy = (rect.min.y + y as i32) as f32 - dy;
            out.v[y * w + x] = sample_bilinear(src, sx, sy).clamp(0.0, 1.0);
        }
    }
    out
}

/// §11.3 "dilate by `r`" = §13.2 threshold-expand through the plane's own
/// signed distance with the 1-px smoothstep re-AA: `m′ = smoothAA(d − r)`.
///
/// A no-op at `r <= 0`, which matters: `spread = 0` is the default, and
/// re-running an EDT to compute `smoothAA(d)` would *re-threshold* an
/// already-antialiased plane and quietly harden its edge.
pub(crate) fn dilate(src: &MaskPlane, rect: IRect, r: f32) -> MaskPlane {
    if r <= 0.0 {
        return src.clone();
    }
    let d = refine::signed_distance(src, rect);
    let mut out = MaskPlane::new(rect);
    for (o, di) in out.v.iter_mut().zip(d.iter()) {
        *o = smooth_aa(di - r).clamp(0.0, 1.0);
    }
    out
}

/// §16.3 separable Gaussian at `σ`, through the crate's single blur
/// dispatch. In place on a coverage plane, clamped to `[0, 1]`.
pub(crate) fn blur(plane: &mut MaskPlane, sigma: f64) {
    if sigma <= 0.0 {
        return;
    }
    let w = plane.rect.width().max(0) as usize;
    let h = plane.rect.height().max(0) as usize;
    refine::blur_plane_sigma(&mut plane.v, w, h, sigma);
    for v in &mut plane.v {
        *v = v.clamp(0.0, 1.0);
    }
}

/// Multiply one plane by another, elementwise (the knockout `S ← S·(1−A)`
/// and the interior clips `G ← G·A` both land here).
pub(crate) fn mul_in_place(dst: &mut MaskPlane, src: &MaskPlane) {
    // `zip` would silently truncate to the shorter plane, turning a rect
    // mismatch into a partially-applied clip instead of a failure.
    debug_assert_eq!(dst.rect, src.rect, "elementwise op across differing rects");
    for (d, s) in dst.v.iter_mut().zip(src.v.iter()) {
        *d *= *s;
    }
}

/// Multiply one plane by `(1 − other)`, elementwise.
pub(crate) fn mul_inverse_in_place(dst: &mut MaskPlane, src: &MaskPlane) {
    debug_assert_eq!(dst.rect, src.rect, "elementwise op across differing rects");
    for (d, s) in dst.v.iter_mut().zip(src.v.iter()) {
        *d *= 1.0 - *s;
    }
}

/// Copy the sub-rect `rect` out of a coverage plane.
///
/// A pure extraction: every output sample is a sample that was already
/// computed, so this **cannot** change a value the way narrowing an
/// *evaluation* rect can (see [`super::evaluate_in`]). `rect` must be
/// contained in `src.rect` — a crop that reached outside would be
/// manufacturing the absent-read zeros of §2.4 and passing them off as
/// computed coverage, which is precisely the false-zero mechanism that
/// makes tile-local effect evaluation wrong.
///
/// A no-op rect returns a clone rather than paying for the copy loop.
pub(crate) fn crop(src: &MaskPlane, rect: IRect) -> MaskPlane {
    debug_assert!(
        src.rect.contains_rect(&rect),
        "crop to {rect:?} reaches outside the plane's own {:?}",
        src.rect
    );
    if rect == src.rect {
        return src.clone();
    }
    let mut out = MaskPlane::new(rect);
    let w = rect.width().max(0) as usize;
    for y in 0..rect.height().max(0) as usize {
        for x in 0..w {
            out.v[y * w + x] = src.get(rect.min.x + x as i32, rect.min.y + y as i32);
        }
    }
    out
}

/// §11.3 Stroke: select the band `lo ≤ d ≤ hi` from the signed distance,
/// antialiased by a 1-px smoothstep at **both** edges.
///
/// `smoothAA(t)` is 1 below the contour and 0 above it, so the band is the
/// product of "inside `hi`" and "outside `lo`":
/// `band = smoothAA(d − hi) · (1 − smoothAA(d − lo))`.
pub(crate) fn band(d: &[f32], lo: f32, hi: f32, out: &mut MaskPlane) {
    debug_assert_eq!(
        out.v.len(),
        d.len(),
        "band over a mismatched distance field"
    );
    for (o, dv) in out.v.iter_mut().zip(d.iter()) {
        let upper = smooth_aa(dv - hi);
        let lower = 1.0 - smooth_aa(dv - lo);
        *o = (upper * lower).clamp(0.0, 1.0);
    }
}
