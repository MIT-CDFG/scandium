// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.6 (metadata: EXIF/IPTC/XMP; orientation NORMATIVE rules) —
//! slice-1 subset: ICC + raw-EXIF + raw-XMP CARRIAGE plus the parsed
//! orientation surface. DD ⑫㉞.
//!
//! §8.6 [NORMATIVE]: orientation (tag 274) is applied on import via the
//! transform table (see [`crate::plane`]) and the stored tag reset to 1 so
//! stored pixels are upright — a preserved-then-re-embedded block never
//! re-exports a stale value. `ExifData`'s full parsed `BTreeMap` surface,
//! IPTC, XMP field editing, and `Selective` policy arrive with the CP-1.0
//! metadata row; this slice preserves the original TIFF-structured byte
//! block byte-wise (`DecodeOpts::keep_unknown_metadata`) with exactly the
//! orientation word patched.
//!
//! EXIF parsing is `kamadak-exif` per its deps record. Malformed blocks
//! NEVER fail a decode (§8.1 `MetadataInvalid` rule): callers drop the
//! block, record a `CompatItem`, and continue. Because that rule keeps
//! metadata failures off the API boundary, this module's error type
//! [`MetadataInvalid`] is NON-WIRE — it carries no MessageId and maps to
//! nothing in the §8.1 table (review round 1 blocker: the table
//! deliberately has no row, and no id may be minted for it).

use std::fmt;

/// Non-wire metadata parse failure (module doc): consumed inside decode
/// paths, converted to a `CompatItem`, never surfaced through
/// [`crate::error::CodecError`].
#[derive(Debug)]
pub struct MetadataInvalid {
    pub block: &'static str,
    pub detail: &'static str,
}

impl fmt::Display for MetadataInvalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid metadata block {}: {}", self.block, self.detail)
    }
}

impl std::error::Error for MetadataInvalid {}

/// Slice-1 metadata carriage (§8.6 subset — see module doc).
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    /// Embedded ICC profile bytes, CARRIED verbatim (extracted on decode,
    /// re-embedded on encode). Conversion waits for the CMM (§5) — the
    /// documented CP-0.5-interim seam.
    pub icc: Option<Vec<u8>>,
    /// The original TIFF-structured EXIF block (no "Exif\0\0" prefix),
    /// byte-preserved except the §8.6 orientation normalization.
    pub exif: Option<Vec<u8>>,
    /// Raw XMP packet bytes, preserved verbatim (§8.6).
    pub xmp: Option<Vec<u8>>,
    /// Raw IPTC-IIM record block (the JPEG APP13 `8BIM` 0x0404 resource
    /// payload / TIFF tag 33723 value), preserved byte-wise (§8.6
    /// "unlisted datasets are preserved byte-wise"). Carriage wiring in
    /// the JPEG/TIFF codecs is a recorded gap (lane log, export wave);
    /// the §29.4 policy filter (`crate::export::policy`) handles the
    /// block once carried.
    pub iptc: Option<Vec<u8>>,
    /// Animation loop count where the container carries one (GIF NETSCAPE
    /// / WebP ANIM); surfaced for the consumer's `meta.animation.loopCount`
    /// (§8.2 GIF mapping). 0 = infinite.
    pub animation_loop: Option<u16>,
}

/// Parse the §8.6 orientation (tag 274, IFD0) out of a raw TIFF-structured
/// EXIF block. `None` = absent or unparseable (caller records a
/// `CompatItem` for the latter; §8.1 `MetadataInvalid` is non-fatal).
pub fn orientation_from_exif(block: &[u8]) -> Option<u16> {
    let exif = exif::Reader::new().read_raw(block.to_vec()).ok()?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    match field.value.get_uint(0) {
        Some(v) if v <= u32::from(u16::MAX) => Some(v as u16),
        _ => None,
    }
}

/// §8.6 normalization: patch the IFD0 orientation word (tag 274) to 1
/// in-place, byte-preserving everything else. A minimal bounds-checked
/// TIFF-IFD walk (endianness from the block header); blocks without the
/// tag come back unchanged. Errors are typed, never panics — untrusted
/// input (the D-series bar).
pub fn normalize_exif_orientation(block: &mut [u8]) -> Result<(), MetadataInvalid> {
    let bad = |d: &'static str| MetadataInvalid {
        block: "exif",
        detail: d,
    };
    if block.len() < 8 {
        return Err(bad("short header"));
    }
    let big = match &block[0..2] {
        b"MM" => true,
        b"II" => false,
        _ => return Err(bad("bad byte order")),
    };
    let u16_at = |b: &[u8], o: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(o..o + 2)?.try_into().ok()?;
        Some(if big {
            u16::from_be_bytes(s)
        } else {
            u16::from_le_bytes(s)
        })
    };
    let u32_at = |b: &[u8], o: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(s)
        } else {
            u32::from_le_bytes(s)
        })
    };
    if u16_at(block, 2) != Some(42) {
        return Err(bad("bad TIFF magic"));
    }
    let ifd0 = u32_at(block, 4).ok_or_else(|| bad("short IFD offset"))? as usize;
    let count = u16_at(block, ifd0).ok_or_else(|| bad("IFD0 out of bounds"))? as usize;
    for i in 0..count {
        let entry = ifd0 + 2 + i * 12;
        let tag = u16_at(block, entry).ok_or_else(|| bad("entry out of bounds"))?;
        if tag != 274 {
            continue;
        }
        let ty = u16_at(block, entry + 2).ok_or_else(|| bad("entry out of bounds"))?;
        if ty != 3 {
            return Err(bad("orientation is not SHORT"));
        }
        // SHORT count 1 stores the value inline in the first two bytes of
        // the 4-byte value field.
        let vo = entry + 8;
        let word: [u8; 2] = if big {
            1u16.to_be_bytes()
        } else {
            1u16.to_le_bytes()
        };
        let slot = block
            .get_mut(vo..vo + 2)
            .ok_or_else(|| bad("value slot out of bounds"))?;
        slot.copy_from_slice(&word);
        return Ok(());
    }
    Ok(()) // no orientation tag — nothing to normalize
}

/// Build a minimal valid TIFF-structured EXIF block carrying exactly one
/// IFD0 with the orientation tag — the test-fixture builder for the §8.6
/// orientation matrix (kept in the crate proper so the codec integration
/// tests and future fuzz seeds share it).
pub fn exif_block_with_orientation(v: u16, big_endian: bool) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + 2 + 12 + 4);
    let w16 = |b: &mut Vec<u8>, v: u16| {
        b.extend_from_slice(&if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        })
    };
    let w32 = |b: &mut Vec<u8>, v: u32| {
        b.extend_from_slice(&if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        })
    };
    b.extend_from_slice(if big_endian { b"MM" } else { b"II" });
    w16(&mut b, 42);
    w32(&mut b, 8); // IFD0 right after the header
    w16(&mut b, 1); // one entry
    w16(&mut b, 274); // Orientation
    w16(&mut b, 3); // SHORT
    w32(&mut b, 1); // count
    w16(&mut b, v); // inline value
    w16(&mut b, 0); // value-field padding
    w32(&mut b, 0); // next-IFD terminator
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_reads_both_endiannesses() {
        for big in [false, true] {
            for v in 1..=8u16 {
                let block = exif_block_with_orientation(v, big);
                assert_eq!(orientation_from_exif(&block), Some(v), "big={big} v={v}");
            }
        }
    }

    #[test]
    fn normalize_patches_only_the_orientation_word() {
        for big in [false, true] {
            let mut block = exif_block_with_orientation(6, big);
            let before = block.clone();
            normalize_exif_orientation(&mut block).unwrap();
            assert_eq!(orientation_from_exif(&block), Some(1));
            // Exactly the two value bytes differ.
            let diff: Vec<usize> = (0..block.len())
                .filter(|&i| block[i] != before[i])
                .collect();
            assert!(diff.len() <= 2, "diff bytes: {diff:?}");
        }
    }

    #[test]
    fn normalize_rejects_garbage_typed_and_non_wire() {
        let mut junk = b"not an exif block at all".to_vec();
        let err = normalize_exif_orientation(&mut junk).unwrap_err();
        assert_eq!(
            err.block, "exif",
            "typed, block-tagged, non-wire (no MessageId exists)"
        );
        let mut short = b"II".to_vec();
        assert!(normalize_exif_orientation(&mut short).is_err());
    }

    #[test]
    fn normalize_without_tag_is_a_no_op() {
        // A block whose single entry is Make (271), not orientation.
        let mut b = exif_block_with_orientation(6, false);
        b[10] = 15; // tag 274 -> 271 (0x112 -> 0x10f little-endian low byte)
        b[11] = 1;
        let before = b.clone();
        normalize_exif_orientation(&mut b).unwrap();
        assert_eq!(b, before);
    }
}
