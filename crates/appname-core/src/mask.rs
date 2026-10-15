// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §13.1 (selection as a float mask); DD ⑭㉙.
//!
//! [NORMATIVE, ES §13.1] There is **one** selection representation: a
//! single-channel `f32` mask in `[0, 1]` stored in the tile system (0 =
//! unselected, 1 = selected, §2.2 Gray row). Every tool writes into this one
//! buffer type; 8-bit masks are a storage/export optimization only. DD ㉙:
//! every mask/channel is one single-component tiled buffer — layer masks
//! (`mask.*`, CP-0.5) and alpha channels (§14) reuse [`Selection::new_mask_buffer`].
//!
//! CP-0.1 defines the type and its invariants; the operations (rasterize,
//! boolean combine §13.2, feather, bounds re-tighten) arrive at CP-0.5 with
//! `selection.set`.

use crate::buffer::Buffer;
use crate::geom::IRect;
use crate::graph::id::ProfileId;
use crate::tile::channel::ChannelKind;
use crate::tile::model::ColorModelKind;

/// The document's selection slot (ES §13.1): a Gray/f32 coverage buffer, an
/// active flag, and the tight bounds of `{p : m(p) > 0}`.
#[derive(Clone)]
pub struct Selection {
    mask: Buffer,
    active: bool,
    bounds: IRect,
}

impl Selection {
    /// The one mask buffer shape (DD ㉙): Gray model, f32 depth. Coverage is
    /// profile-less data — the profile slot holds the nil id.
    pub fn new_mask_buffer() -> Buffer {
        Buffer::new(
            ChannelKind::F32,
            ColorModelKind::Gray,
            ProfileId(uuid::Uuid::nil()),
        )
    }

    /// An inactive (empty) selection.
    pub fn new() -> Selection {
        Selection {
            mask: Self::new_mask_buffer(),
            active: false,
            bounds: IRect::EMPTY,
        }
    }

    pub fn mask(&self) -> &Buffer {
        &self.mask
    }

    /// [NORMATIVE, ES §13.1] `active = !bounds.is_empty()`.
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn bounds(&self) -> IRect {
        self.bounds
    }

    // Crate-internal mutation doors (plan rule 5); the CP-0.5 selection
    // commands are the consumers.

    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn mask_mut(&mut self) -> &mut Buffer {
        &mut self.mask
    }

    /// Maintain `bounds`/`active` after a mask edit (growing ops union in
    /// O(1); shrinking ops re-tighten tile-granularly — ES §13.1).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn set_bounds(&mut self, bounds: IRect) {
        self.bounds = if bounds.is_empty() {
            IRect::EMPTY
        } else {
            bounds
        };
        self.active = !self.bounds.is_empty();
    }
}

impl Default for Selection {
    fn default() -> Selection {
        Selection::new()
    }
}

/// Summary Debug (ES §1.6: no mask pixel dumps).
impl core::fmt::Debug for Selection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Selection")
            .field("active", &self.active)
            .field("bounds", &self.bounds)
            .field("tiles", &self.mask.tiles().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::IVec2;

    #[test]
    fn mask_buffer_is_gray_f32() {
        let s = Selection::new();
        assert_eq!(s.mask().kind(), ChannelKind::F32);
        assert_eq!(s.mask().model(), ColorModelKind::Gray);
        assert!(!s.active());
        assert_eq!(s.bounds(), IRect::EMPTY);
    }

    #[test]
    fn coverage_values_ride_the_tile_system() {
        let mut s = Selection::new();
        // A soft-edged coverage value (30 %-selected pixel, §13.1 semantics).
        s.mask_mut()
            .write_pixel_f32(IVec2::new(10, 10), [0.3, 0.0, 0.0, 0.0]);
        s.set_bounds(s.mask().bounds());
        assert!(s.active());
        assert_eq!(s.mask().read_pixel_f32(IVec2::new(10, 10))[0], 0.3);
        assert_eq!(s.bounds(), IRect::new(10, 10, 11, 11));
        // Absent mask space reads 0 = unselected.
        assert_eq!(s.mask().read_pixel_f32(IVec2::new(500, 500))[0], 0.0);
    }

    #[test]
    fn active_follows_bounds() {
        let mut s = Selection::new();
        s.set_bounds(IRect::new(0, 0, 5, 5));
        assert!(s.active());
        s.set_bounds(IRect::EMPTY);
        assert!(!s.active());
        // Degenerate rects normalize to EMPTY at this door.
        s.set_bounds(IRect::new(9, 9, 9, 20));
        assert!(!s.active());
        assert_eq!(s.bounds(), IRect::EMPTY);
    }
}
