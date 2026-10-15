// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-shape-sys` — the shaping wrapper: Appname's FFI boundary over
//! the vendored HarfBuzz (the "shaping wrapper (§23.2, fuzzed at the
//! `*-sys` boundary §1.4)" the spec's fuzz matrix names).
//!
//! Spec: ES §23.2 (HarfBuzz inputs/outputs, [NORMATIVE] cluster level 1,
//! pinned vendored library); DD ① (FFI-boundary `unsafe` confined to this
//! crate, every block justified — the `appname-cabi` discipline); DD ㉒.
//! Dependency authorization: deps/harfbuzz.yaml (default-features OFF +
//! `bundled` is PART of the authorization; HarfBuzz 8.4.0 vendored C).
//!
//! Design rules of this crate:
//!
//! * The public API is SAFE and value-based: bytes + params in,
//!   [`RawGlyph`] values out. No HarfBuzz type, pointer, or callback
//!   crosses the boundary.
//! * No panics on the C side of any call; every returned pointer is
//!   null-checked before use (HarfBuzz returns non-null singletons for
//!   allocation failure — those are detected via
//!   `hb_buffer_allocation_successful`).
//! * Determinism (D-48/§23.2): the vendored pin means identical inputs
//!   produce identical outputs on every OS; nothing here reads system
//!   state, fonts, or locale.
//! * NO font data ships in this crate; faces arrive as caller bytes
//!   (§3.10 font-licensing rule — fixture fonts are the caller's concern).

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::c_char;
use std::os::raw::{c_int, c_uint};

use harfbuzz_sys as hb;

/// Errors surfaced by the wrapper. All are recoverable; none abort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeSysError {
    /// Face bytes were rejected (blob/face creation failed or the index
    /// is out of range — HarfBuzz yields an empty face; we detect it via
    /// a zero glyph count and refuse it).
    InvalidFace,
    /// The requested scale is out of the i32 fixed-point domain.
    InvalidScale,
    /// HarfBuzz buffer allocation failed mid-shape.
    Allocation,
}

impl std::fmt::Display for ShapeSysError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeSysError::InvalidFace => write!(f, "face bytes rejected by HarfBuzz"),
            ShapeSysError::InvalidScale => write!(f, "font scale outside i32 domain"),
            ShapeSysError::Allocation => write!(f, "HarfBuzz buffer allocation failed"),
        }
    }
}

impl std::error::Error for ShapeSysError {}

/// The vendored HarfBuzz version string (e.g. `"8.4.0"`). Pinned by
/// deps/harfbuzz.yaml; the spike test asserts the pin so a silent crate
/// bump can never drift the golden text corpus (§34.1).
pub fn harfbuzz_version() -> String {
    // SAFETY: hb_version_string returns a static NUL-terminated string
    // owned by the library; it is never null and never freed.
    let p = unsafe { hb::hb_version_string() };
    debug_assert!(!p.is_null());
    // SAFETY: p is a valid NUL-terminated C string (above).
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

/// One OpenType feature setting (§23.2: engine defaults overlaid by the
/// run's features). `start`/`end` are UTF-8 byte indices into the shaped
/// paragraph (u32::MAX = unbounded, the HarfBuzz convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Feature {
    pub tag: [u8; 4],
    pub value: u32,
    pub start: u32,
    pub end: u32,
}

impl Feature {
    /// A feature applied to the whole buffer.
    pub const fn global(tag: [u8; 4], value: u32) -> Self {
        Feature {
            tag,
            value,
            start: 0,
            end: u32::MAX,
        }
    }
}

/// Shaping parameters for one item (§23.2 "HarfBuzz inputs").
#[derive(Debug, Clone, Copy)]
pub struct ShapeParams<'a> {
    /// Direction from the item's bidi level (odd = RTL).
    pub rtl: bool,
    /// ISO 15924 script tag (e.g. `*b"Latn"`); HarfBuzz maps it to its
    /// OpenType script internally.
    pub script: [u8; 4],
    /// BCP 47 language tag, or `None` for the HarfBuzz default language
    /// resolution (deterministic: no locale is read — unset means the
    /// implicit `dflt` lookup path).
    pub language: Option<&'a str>,
    /// Feature settings, already overlaid (engine defaults first, run
    /// features after — the caller owns the §23.2 overlay order).
    pub features: &'a [Feature],
}

/// One shaped glyph, verbatim from HarfBuzz (§23.2 outputs). Positions
/// are in the font-scale units set at [`ShapeFont::from_bytes`]
/// (`scale` per em); `cluster` is a UTF-8 byte index into the shaped
/// paragraph (cluster level 1 — monotone character clusters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawGlyph {
    pub glyph: u32,
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    /// HB_GLYPH_FLAG_UNSAFE_TO_BREAK: breaking the text at this
    /// glyph's cluster start would change shaping — the §23.4
    /// cursive-join/ligature-context discriminator (C-5 slice 2).
    pub unsafe_to_break: bool,
}

/// An owned HarfBuzz face+font pair over caller-provided bytes.
///
/// The bytes are copied once into a stable heap allocation that outlives
/// the blob (HarfBuzz `READONLY` blobs borrow their data; the copy makes
/// the safety local instead of a caller contract).
pub struct ShapeFont {
    font: *mut hb::hb_font_t,
    face: *mut hb::hb_face_t,
    /// Keeps the blob's borrowed data alive until Drop (never read back).
    _bytes: Box<[u8]>,
}

impl ShapeFont {
    /// Build a font from raw face bytes (TTF/OTF), a face index, and a
    /// per-em scale (both axes; §23.2 `em_px·64` fixed-point is the
    /// caller's convention — this crate treats scale as opaque units).
    pub fn from_bytes(data: &[u8], face_index: u32, scale: i32) -> Result<Self, ShapeSysError> {
        if scale <= 0 {
            return Err(ShapeSysError::InvalidScale);
        }
        let bytes: Box<[u8]> = data.into();
        // SAFETY: `bytes` is a stable heap allocation owned by the
        // returned struct and outlives blob+face+font (Drop order below
        // destroys them first). Length fits c_uint (u32) or we refuse.
        let len: c_uint = match c_uint::try_from(bytes.len()) {
            Ok(l) => l,
            Err(_) => return Err(ShapeSysError::InvalidFace),
        };
        let blob = unsafe {
            hb::hb_blob_create(
                bytes.as_ptr() as *const c_char,
                len,
                hb::HB_MEMORY_MODE_READONLY,
                std::ptr::null_mut(),
                None,
            )
        };
        if blob.is_null() {
            return Err(ShapeSysError::InvalidFace);
        }
        // SAFETY: blob is a valid hb_blob_t just created; face takes its
        // own reference, so our reference is released right after.
        let face = unsafe { hb::hb_face_create(blob, face_index) };
        // SAFETY: releasing our blob reference (face holds one).
        unsafe { hb::hb_blob_destroy(blob) };
        if face.is_null() {
            return Err(ShapeSysError::InvalidFace);
        }
        // Reject the empty face HarfBuzz substitutes for unparseable
        // bytes / out-of-range index (glyph count 0).
        // SAFETY: face is valid (above).
        if unsafe { hb::hb_face_get_glyph_count(face) } == 0 {
            // SAFETY: releasing the face reference we hold.
            unsafe { hb::hb_face_destroy(face) };
            return Err(ShapeSysError::InvalidFace);
        }
        // SAFETY: face is valid; font takes its own face reference.
        let font = unsafe { hb::hb_font_create(face) };
        if font.is_null() {
            // SAFETY: releasing the face reference we hold.
            unsafe { hb::hb_face_destroy(face) };
            return Err(ShapeSysError::InvalidFace);
        }
        // SAFETY: font is valid; set_scale has no failure mode.
        unsafe { hb::hb_font_set_scale(font, scale, scale) };
        Ok(ShapeFont {
            font,
            face,
            _bytes: bytes,
        })
    }

    /// The face's glyph count (test/probe surface).
    pub fn glyph_count(&self) -> u32 {
        // SAFETY: self.face is valid for self's lifetime.
        (unsafe { hb::hb_face_get_glyph_count(self.face) }) as u32
    }

    /// cmap lookup: the nominal glyph for a character, if mapped
    /// (§44.2 coverage probes ride this).
    pub fn glyph_index(&self, c: char) -> Option<u32> {
        let mut g: hb::hb_codepoint_t = 0;
        // SAFETY: self.font is valid; out-pointer is a local.
        let ok = unsafe { hb::hb_font_get_nominal_glyph(self.font, c as u32, &mut g) };
        (ok != 0).then_some(g)
    }

    /// Shape `paragraph[item_start .. item_start+item_len]` with the rest
    /// of the paragraph as pre/post context (§23.2 "shaping context
    /// crosses item edges"). Returns glyphs in the buffer's output order
    /// (visual within the item; RTL items therefore come back reversed,
    /// clusters monotone decreasing). Cluster values are paragraph byte
    /// indices (cluster level 1).
    ///
    /// # Panics
    ///
    /// Panics if the item range overflows, exceeds the paragraph, or
    /// does not lie on `char` boundaries — a caller bug at the FFI
    /// boundary, refused loudly in every build profile (never fed to C).
    pub fn shape(
        &self,
        paragraph: &str,
        item_start: usize,
        item_len: usize,
        params: &ShapeParams<'_>,
    ) -> Result<Vec<RawGlyph>, ShapeSysError> {
        // checked_add: an overflowing range must fail the guard, not
        // wrap past it (review finding, slice-2 re-review round).
        let item_end = item_start
            .checked_add(item_len)
            .expect("item range overflows usize");
        assert!(
            item_end <= paragraph.len()
                && paragraph.is_char_boundary(item_start)
                && paragraph.is_char_boundary(item_end),
            "item range must lie on char boundaries inside the paragraph"
        );
        if item_len == 0 {
            return Ok(Vec::new());
        }
        let text_len: c_int = match c_int::try_from(paragraph.len()) {
            Ok(l) => l,
            Err(_) => return Err(ShapeSysError::Allocation),
        };
        // SAFETY: script tag bytes are a 4-byte local; hb parses ISO 15924.
        let script =
            unsafe { hb::hb_script_from_string(params.script.as_ptr() as *const c_char, 4) };
        // SAFETY: buffer creation; the inert singleton (allocation
        // failure) is detected via hb_buffer_allocation_successful below.
        let buf = unsafe { hb::hb_buffer_create() };
        // SAFETY: buf is a valid hb_buffer_t (possibly the inert
        // singleton, on which every call below is a documented no-op);
        // text pointer/length describe a live &str borrow for the whole
        // scope; item offset/length were bounds- and boundary-checked.
        unsafe {
            hb::hb_buffer_add_utf8(
                buf,
                paragraph.as_ptr() as *const c_char,
                text_len,
                item_start as c_uint,
                item_len as c_int,
            );
            hb::hb_buffer_set_cluster_level(buf, hb::HB_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS);
            hb::hb_buffer_set_direction(
                buf,
                if params.rtl {
                    hb::HB_DIRECTION_RTL
                } else {
                    hb::HB_DIRECTION_LTR
                },
            );
            hb::hb_buffer_set_script(buf, script);
            if let Some(lang) = params.language {
                let l = hb::hb_language_from_string(
                    lang.as_ptr() as *const c_char,
                    lang.len() as c_int,
                );
                hb::hb_buffer_set_language(buf, l);
            }
        }
        let features: Vec<hb::hb_feature_t> = params
            .features
            .iter()
            .map(|f| hb::hb_feature_t {
                tag: u32::from_be_bytes(f.tag),
                value: f.value,
                start: f.start,
                end: f.end,
            })
            .collect();
        // SAFETY: font and buf are valid; the features slice lives across
        // the call; a null/empty features pointer pair is permitted.
        unsafe {
            hb::hb_shape(
                self.font,
                buf,
                if features.is_empty() {
                    std::ptr::null()
                } else {
                    features.as_ptr()
                },
                features.len() as c_uint,
            );
        }
        // SAFETY: buf is valid; allocation check is the documented way to
        // detect the inert buffer / mid-shape allocation failure.
        if unsafe { hb::hb_buffer_allocation_successful(buf) } == 0 {
            // SAFETY: releasing our buffer reference.
            unsafe { hb::hb_buffer_destroy(buf) };
            return Err(ShapeSysError::Allocation);
        }
        let mut n_info: c_uint = 0;
        let mut n_pos: c_uint = 0;
        // SAFETY: buf is valid; out-lengths are locals; the returned
        // arrays are owned by the buffer and read before it is destroyed.
        let (infos, positions) = unsafe {
            (
                hb::hb_buffer_get_glyph_infos(buf, &mut n_info),
                hb::hb_buffer_get_glyph_positions(buf, &mut n_pos),
            )
        };
        debug_assert_eq!(n_info, n_pos);
        let n = n_info.min(n_pos) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            // SAFETY: i < n ≤ both array lengths reported by the buffer;
            // arrays are valid until hb_buffer_destroy below.
            let (gi, gp) = unsafe { (&*infos.add(i), &*positions.add(i)) };
            // SAFETY: gi points into the live glyph-info array (bounds
            // as above); the flag accessor only reads the struct.
            let flags = unsafe { hb::hb_glyph_info_get_glyph_flags(gi) };
            out.push(RawGlyph {
                glyph: gi.codepoint,
                cluster: gi.cluster,
                x_advance: gp.x_advance,
                y_advance: gp.y_advance,
                x_offset: gp.x_offset,
                y_offset: gp.y_offset,
                unsafe_to_break: flags & hb::HB_GLYPH_FLAG_UNSAFE_TO_BREAK != 0,
            });
        }
        // SAFETY: releasing our buffer reference (arrays above are not
        // used past this point).
        unsafe { hb::hb_buffer_destroy(buf) };
        Ok(out)
    }
}

impl Drop for ShapeFont {
    fn drop(&mut self) {
        // SAFETY: font and face hold the references taken at creation;
        // destroy order font→face matches the reference graph. `_bytes`
        // drops after both, keeping the blob's borrowed data alive.
        unsafe {
            hb::hb_font_destroy(self.font);
            hb::hb_face_destroy(self.face);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// deps/harfbuzz.yaml pins the vendored C at 8.4.0; a crate bump that
    /// changed the vendored version would drift the §34.1 golden text
    /// corpus, so the pin is asserted, not assumed.
    #[test]
    fn vendored_harfbuzz_is_the_recorded_pin() {
        assert_eq!(harfbuzz_version(), "8.4.0");
    }

    /// Garbage bytes must be refused, not shaped against an empty face
    /// (HarfBuzz substitutes an empty face; we detect and reject it).
    #[test]
    fn garbage_face_bytes_are_rejected() {
        assert_eq!(
            ShapeFont::from_bytes(b"not a font", 0, 1000).err(),
            Some(ShapeSysError::InvalidFace)
        );
        assert_eq!(
            ShapeFont::from_bytes(&[], 0, 1000).err(),
            Some(ShapeSysError::InvalidFace)
        );
    }

    /// Scale domain guard.
    #[test]
    fn nonpositive_scale_is_rejected() {
        assert_eq!(
            ShapeFont::from_bytes(b"x", 0, 0).err(),
            Some(ShapeSysError::InvalidScale)
        );
    }
}
