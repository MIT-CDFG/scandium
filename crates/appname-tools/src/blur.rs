// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §16.3 Gaussian Blur block, §11.2 step 2 (mask feather).
//!
//! **ADAPTER ONLY — the kernel itself lives in
//! [`appname_algo::rgba_blur`] and exists nowhere else.** This module is
//! the `IRect`/[`Plane`]-shaped door onto it, kept here so every existing
//! §16.3 caller (`appname_filters::corpus::gaussian`, `sharpen`, the
//! filter registry) keeps compiling against `appname_tools::blur`
//! unchanged.
//!
//! WHY THE KERNEL MOVED (O-INT architecture re-ruling, this slice). The
//! §11.2 mask feather is evaluated in `appname_core::graph::mask_eval`,
//! and `appname-tools` already depends on `appname-core`, so `core →
//! tools` is a hard cargo cycle — the kernel could not stay here and still
//! be reachable from core. `appname-algo` is a TRUE LEAF (zero `appname-*`
//! dependencies), so `core → algo` cannot cycle now or later, and both
//! consumers share ONE implementation: §16.3 filter blur and §11.2 mask
//! feather are at parity BY CONSTRUCTION, not by matched pairs of code.
//!
//! **The adaptation is free.** `appname_algo::rgba_blur::PlaneRect` is the
//! same four `i32`s as `IRect`, and the pixel buffers move whole — the
//! input crosses as a borrowed [`appname_algo::rgba_blur::RgbaView`] and
//! the output's `Vec` is moved into the returned [`Plane`]. No pixel is
//! copied on either side, so this door costs nothing over calling the
//! kernel directly.
//!
//! See the kernel module's **tile-invariance caveat** before calling
//! [`gaussian_blur_plane`] once per output tile: above σ = 32 the result
//! is NOT rect-invariant.

use appname_algo::rgba_blur::{self, PlaneRect, RgbaView};
use appname_core::IRect;

use crate::resample::Plane;

/// [NORMATIVE, §16.3] The kernel's reach in px: `⌈3σ⌉` — the ROI
/// inflation and, for the §11.2 mask feather, the `Rmask` invalidation
/// margin. Re-exported from the kernel so there is exactly ONE definition
/// of the reach in the workspace.
pub use appname_algo::rgba_blur::gaussian_reach;

/// `IRect` → the kernel's document-free rect. Field-for-field; `IRect` is
/// half-open `[min, max)` and so is [`PlaneRect`].
#[inline]
fn to_plane_rect(r: IRect) -> PlaneRect {
    PlaneRect::new(r.min.x, r.min.y, r.max.x, r.max.y)
}

/// The §16.3 pipeline over [`Plane`]s: direct separable kernel for σ ≤ 32,
/// Kovesi 3-box beyond. Reads outside `input.rect` clamp to its edge.
///
/// A thin, copy-free adapter over [`appname_algo::rgba_blur::gaussian_blur_rgba`];
/// see the module doc's tile-invariance caveat before calling this once
/// per output tile.
pub fn gaussian_blur_plane(input: &Plane, out_rect: IRect, sigma: f64) -> Plane {
    let view = RgbaView::new(to_plane_rect(input.rect), &input.px);
    let out = rgba_blur::gaussian_blur_rgba(view, to_plane_rect(out_rect), sigma);
    // The kernel's rect IS `out_rect` re-expressed; move the pixels back
    // under the caller's `IRect` without touching them.
    debug_assert_eq!(out.rect, to_plane_rect(out_rect));
    Plane {
        rect: out_rect,
        px: out.px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resample::EdgePolicy;
    use appname_algo::rgba_blur::EdgeMode;

    /// The adapter is a RE-EXPRESSION, not a second implementation: for a
    /// content-bearing plane, over BOTH kernel paths (σ = 4 direct, σ = 40
    /// box) and over an out_rect that differs from the input rect, the
    /// `Plane`-shaped door returns exactly the bytes the kernel returns
    /// when called directly on the same samples.
    ///
    /// This is the test that would catch the adapter drifting — a
    /// transposed rect field, an off-by-one origin, a stray copy that
    /// re-rounds.
    #[test]
    fn the_adapter_reproduces_the_kernel_byte_for_byte() {
        let extent = IRect::new(-7, 5, 57, 45);
        let mut src = Plane::new(extent);
        for y in extent.min.y..extent.max.y {
            for x in extent.min.x..extent.max.x {
                let v = ((x * 5 + y * 9).rem_euclid(13)) as f32 / 12.0;
                src.set(x, y, [v, v * 0.5, v * 0.25, 1.0]);
            }
        }
        // An out_rect that is NOT the input rect, so a bad origin shows.
        let out_rect = IRect::new(3, 12, 41, 33);
        for sigma in [4.0f64, 40.0f64] {
            let via_adapter = gaussian_blur_plane(&src, out_rect, sigma);
            let direct = rgba_blur::gaussian_blur_rgba(
                RgbaView::new(to_plane_rect(extent), &src.px),
                to_plane_rect(out_rect),
                sigma,
            );
            assert_eq!(via_adapter.rect, out_rect, "σ={sigma}: rect not preserved");
            assert_eq!(
                via_adapter.px, direct.px,
                "σ={sigma}: the adapter is not the kernel"
            );
            // And the rect really does place the pixels: sample through
            // the `Plane` door and the kernel door at the same document
            // coordinate and demand the same bits.
            for (x, y) in [(3, 12), (20, 20), (40, 32)] {
                assert_eq!(
                    via_adapter.get(x, y, EdgePolicy::Transparent),
                    direct.get(x, y, EdgeMode::Transparent),
                    "σ={sigma}: coordinate mapping differs at ({x},{y})"
                );
            }
        }
    }

    /// `IRect` and `PlaneRect` must agree on emptiness and extent, since
    /// the adapter hands one to code that reasons with the other.
    #[test]
    fn rect_translation_preserves_extent_and_emptiness() {
        for r in [
            IRect::new(0, 0, 10, 4),
            IRect::new(-9, -3, -1, 7),
            IRect::new(5, 5, 5, 9), // empty on x
            IRect::new(5, 5, 9, 5), // empty on y
            IRect::new(2, 2, 1, 1), // inverted ⇒ empty
        ] {
            let p = to_plane_rect(r);
            assert_eq!(p.width(), r.width(), "{r:?}");
            assert_eq!(p.height(), r.height(), "{r:?}");
            assert_eq!(p.is_empty(), r.is_empty(), "{r:?}");
            assert_eq!(p.inflate(3, 2).width(), r.inflate(3, 2).width(), "{r:?}");
            assert_eq!(p.inflate(3, 2).height(), r.inflate(3, 2).height(), "{r:?}");
        }
    }

    /// The §16.3 caller-visible contract still holds through the door:
    /// σ ≤ 0 is an identity copy over `out_rect`, and the reach is
    /// `⌈3σ⌉`.
    #[test]
    fn zero_sigma_is_an_identity_copy_and_reach_is_ceil_three_sigma() {
        assert_eq!(gaussian_reach(4.0), 12);
        assert_eq!(gaussian_reach(0.0), 0);
        let extent = IRect::new(0, 0, 8, 8);
        let mut src = Plane::new(extent);
        src.set(3, 4, [0.5, 0.25, 0.125, 1.0]);
        let out = gaussian_blur_plane(&src, extent, 0.0);
        assert_eq!(out.px, src.px, "σ ≤ 0 must copy, not filter");
    }
}
