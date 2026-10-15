// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.6 (`ExifData` model: "the parsed view drives the UI; the
//! byte block drives re-embedding AFTER POLICY FILTERING"), §29.4 field
//! groups. A minimal TIFF-structured EXIF parser + re-emitter: parses
//! IFD0 / EXIF IFD / GPS IFD / Interop IFD into raw-byte entries,
//! filters tag sets, and re-emits a well-formed block with recomputed
//! offsets in the ORIGINAL byte order. Kept tags carry their value
//! bytes verbatim (the §8.6 byte-preservation rule at entry
//! granularity); IFD1 (the embedded thumbnail) is never re-emitted by
//! this writer — §29.4 drops it under StripAll/Selective and v1 drops
//! it under Preserve too (regeneration is a recorded deferral; a STALE
//! thumbnail is itself the leak the row exists to prevent).
//!
//! Untrusted-input discipline (the §34.2 bar): bounded entry counts,
//! checked reads, typed errors, no panics. A block this parser cannot
//! walk is reported up and the CALLER drops it whole (fail-closed in
//! the privacy direction — over-strip, never leak).

use crate::meta::MetadataInvalid;

const MAX_ENTRIES: usize = 4096; // a hostile IFD, not a camera's

const TAG_EXIF_IFD: u16 = 34665;
const TAG_GPS_IFD: u16 = 34853;
const TAG_INTEROP_IFD: u16 = 40965;

/// One IFD entry: raw value bytes, source byte order already applied to
/// the header fields (tag/type/count), value bytes verbatim.
#[derive(Debug, Clone)]
pub struct Entry {
    pub tag: u16,
    pub typ: u16,
    pub count: u32,
    pub data: Vec<u8>,
}

/// The parsed block: IFD0 + optional sub-IFDs, plus the byte order.
#[derive(Debug, Clone)]
pub struct ExifBlock {
    pub big_endian: bool,
    pub ifd0: Vec<Entry>,
    pub exif_ifd: Vec<Entry>,
    pub gps_ifd: Vec<Entry>,
    pub interop_ifd: Vec<Entry>,
    /// IFD1 present in the source (reported so the caller can record
    /// the thumbnail-drop `CompatItem`).
    pub had_thumbnail: bool,
}

fn type_size(typ: u16) -> Option<usize> {
    Some(match typ {
        1 | 2 | 6 | 7 => 1, // BYTE / ASCII / SBYTE / UNDEFINED
        3 | 8 => 2,         // SHORT / SSHORT
        4 | 9 | 11 => 4,    // LONG / SLONG / FLOAT
        5 | 10 | 12 => 8,   // RATIONAL / SRATIONAL / DOUBLE
        _ => return None,
    })
}

fn invalid(detail: &'static str) -> MetadataInvalid {
    MetadataInvalid {
        block: "exif",
        detail,
    }
}

struct Reader<'a> {
    b: &'a [u8],
    be: bool,
}

impl<'a> Reader<'a> {
    fn u16_at(&self, off: usize) -> Result<u16, MetadataInvalid> {
        let s: [u8; 2] = self
            .b
            .get(off..off + 2)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| invalid("short read (u16)"))?;
        Ok(if self.be {
            u16::from_be_bytes(s)
        } else {
            u16::from_le_bytes(s)
        })
    }
    fn u32_at(&self, off: usize) -> Result<u32, MetadataInvalid> {
        let s: [u8; 4] = self
            .b
            .get(off..off + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| invalid("short read (u32)"))?;
        Ok(if self.be {
            u32::from_be_bytes(s)
        } else {
            u32::from_le_bytes(s)
        })
    }

    /// Parse one IFD at `off`: entries + the next-IFD offset.
    fn ifd_at(&self, off: usize) -> Result<(Vec<Entry>, u32), MetadataInvalid> {
        let n = self.u16_at(off)? as usize;
        if n > MAX_ENTRIES {
            return Err(invalid("IFD entry count over bound"));
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let e = off + 2 + i * 12;
            let tag = self.u16_at(e)?;
            let typ = self.u16_at(e + 2)?;
            let count = self.u32_at(e + 4)?;
            let size = type_size(typ)
                .ok_or_else(|| invalid("unknown entry type"))?
                .checked_mul(count as usize)
                .ok_or_else(|| invalid("entry size overflow"))?;
            if size > self.b.len() {
                return Err(invalid("entry value larger than block"));
            }
            let data = if size <= 4 {
                self.b
                    .get(e + 8..e + 8 + size)
                    .ok_or_else(|| invalid("short inline value"))?
                    .to_vec()
            } else {
                let v = self.u32_at(e + 8)? as usize;
                self.b
                    .get(v..v + size)
                    .ok_or_else(|| invalid("value offset out of range"))?
                    .to_vec()
            };
            entries.push(Entry {
                tag,
                typ,
                count,
                data,
            });
        }
        let next = self.u32_at(off + 2 + n * 12)?;
        Ok((entries, next))
    }
}

/// Parse a raw TIFF-structured EXIF block (no `Exif\0\0` prefix — the
/// [`crate::meta::Metadata::exif`] convention).
pub fn parse(block: &[u8]) -> Result<ExifBlock, MetadataInvalid> {
    let be = match block.get(0..2) {
        Some(b"MM") => true,
        Some(b"II") => false,
        _ => return Err(invalid("missing byte-order mark")),
    };
    let r = Reader { b: block, be };
    if r.u16_at(2)? != 42 {
        return Err(invalid("bad TIFF magic"));
    }
    let ifd0_off = r.u32_at(4)? as usize;
    let (ifd0, next) = r.ifd_at(ifd0_off)?;

    let sub = |entries: &[Entry], tag: u16| -> Result<Vec<Entry>, MetadataInvalid> {
        match entries.iter().find(|e| e.tag == tag) {
            Some(e) if e.typ == 4 && e.count == 1 && e.data.len() == 4 => {
                let s: [u8; 4] = e.data[..4].try_into().expect("len checked");
                let off = if be {
                    u32::from_be_bytes(s)
                } else {
                    u32::from_le_bytes(s)
                } as usize;
                Ok(r.ifd_at(off)?.0)
            }
            Some(_) => Err(invalid("malformed sub-IFD pointer")),
            None => Ok(Vec::new()),
        }
    };
    let exif_ifd = sub(&ifd0, TAG_EXIF_IFD)?;
    let gps_ifd = sub(&ifd0, TAG_GPS_IFD)?;
    let interop_ifd = sub(&exif_ifd, TAG_INTEROP_IFD)?;

    Ok(ExifBlock {
        big_endian: be,
        ifd0,
        exif_ifd,
        gps_ifd,
        interop_ifd,
        had_thumbnail: next != 0,
    })
}

/// Re-emit the block: header + IFD0 (+ EXIF/GPS/Interop sub-IFDs as
/// present), recomputed offsets, next-IFD = 0 everywhere (no IFD1 —
/// module doc). Pointer entries for EMPTY sub-IFDs are omitted;
/// remaining entries keep their source order (§8.6 byte discipline at
/// entry granularity; TIFF readers accept the preserved order).
pub fn emit(block: &ExifBlock) -> Vec<u8> {
    let be = block.big_endian;
    let w16 = |v: u16| -> [u8; 2] {
        if be {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let w32 = |v: u32| -> [u8; 4] {
        if be {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };

    // Working copies with dead pointer entries dropped / live ones kept.
    let mut ifd0: Vec<Entry> = block
        .ifd0
        .iter()
        .filter(|e| {
            !(e.tag == TAG_EXIF_IFD && block.exif_ifd.is_empty())
                && !(e.tag == TAG_GPS_IFD && block.gps_ifd.is_empty())
        })
        .cloned()
        .collect();
    let mut exif_ifd: Vec<Entry> = block
        .exif_ifd
        .iter()
        .filter(|e| !(e.tag == TAG_INTEROP_IFD && block.interop_ifd.is_empty()))
        .cloned()
        .collect();
    // Pointer entries must exist where a sub-IFD does (a filtered parse
    // may have kept the IFD but the caller rebuilt entries) — ensure
    // presence, value patched below.
    let ensure_ptr = |list: &mut Vec<Entry>, tag: u16, present: bool| {
        if present && !list.iter().any(|e| e.tag == tag) {
            list.push(Entry {
                tag,
                typ: 4,
                count: 1,
                data: vec![0; 4],
            });
        }
    };
    ensure_ptr(&mut ifd0, TAG_EXIF_IFD, !exif_ifd.is_empty());
    ensure_ptr(&mut ifd0, TAG_GPS_IFD, !block.gps_ifd.is_empty());
    ensure_ptr(
        &mut exif_ifd,
        TAG_INTEROP_IFD,
        !block.interop_ifd.is_empty(),
    );

    // Layout: header(8) | IFD blocks in order, each = 2 + 12n + 4 +
    // external values.
    let ifd_len = |entries: &[Entry]| -> usize {
        2 + entries.len() * 12
            + 4
            + entries
                .iter()
                .map(|e| {
                    if e.data.len() > 4 {
                        (e.data.len() + 1) & !1
                    } else {
                        0
                    }
                })
                .sum::<usize>()
    };
    let ifd0_base = 8usize;
    let exif_base = ifd0_base + ifd_len(&ifd0);
    let gps_base = exif_base
        + if exif_ifd.is_empty() {
            0
        } else {
            ifd_len(&exif_ifd)
        };
    let interop_base = gps_base
        + if block.gps_ifd.is_empty() {
            0
        } else {
            ifd_len(&block.gps_ifd)
        };

    // Patch pointer values.
    let patch = |list: &mut [Entry], tag: u16, target: usize| {
        if let Some(e) = list.iter_mut().find(|e| e.tag == tag) {
            e.data = w32(target as u32).to_vec();
        }
    };
    patch(&mut ifd0, TAG_EXIF_IFD, exif_base);
    patch(&mut ifd0, TAG_GPS_IFD, gps_base);
    patch(&mut exif_ifd, TAG_INTEROP_IFD, interop_base);

    let mut out = Vec::with_capacity(interop_base + 64);
    out.extend_from_slice(if be { b"MM" } else { b"II" });
    out.extend_from_slice(&w16(42));
    out.extend_from_slice(&w32(ifd0_base as u32));

    let emit_ifd = |out: &mut Vec<u8>, base: usize, entries: &[Entry]| {
        debug_assert_eq!(out.len(), base, "IFD lands at its computed base");
        out.extend_from_slice(&w16(entries.len() as u16));
        // external values start after count + entries + next-offset
        let mut ext_off = base + 2 + entries.len() * 12 + 4;
        let mut ext: Vec<u8> = Vec::new();
        for e in entries {
            out.extend_from_slice(&w16(e.tag));
            out.extend_from_slice(&w16(e.typ));
            out.extend_from_slice(&w32(e.count));
            if e.data.len() <= 4 {
                let mut v = [0u8; 4];
                v[..e.data.len()].copy_from_slice(&e.data);
                out.extend_from_slice(&v);
            } else {
                out.extend_from_slice(&w32(ext_off as u32));
                ext.extend_from_slice(&e.data);
                if e.data.len() % 2 == 1 {
                    ext.push(0); // word alignment
                }
                ext_off += (e.data.len() + 1) & !1;
            }
        }
        out.extend_from_slice(&w32(0)); // next IFD: none
        out.extend_from_slice(&ext);
    };

    emit_ifd(&mut out, ifd0_base, &ifd0);
    if !exif_ifd.is_empty() {
        emit_ifd(&mut out, exif_base, &exif_ifd);
    }
    if !block.gps_ifd.is_empty() {
        emit_ifd(&mut out, gps_base, &block.gps_ifd);
    }
    if !block.interop_ifd.is_empty() {
        emit_ifd(&mut out, interop_base, &block.interop_ifd);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta;

    /// Build a tiny little-endian EXIF block: IFD0 {Make, Orientation,
    /// GPS ptr, EXIF ptr} + EXIF IFD {ISO} + GPS IFD {lat ref} + IFD1.
    fn fixture() -> Vec<u8> {
        // Assemble via the emitter itself, then splice an IFD1 by hand.
        let block = ExifBlock {
            big_endian: false,
            ifd0: vec![
                Entry {
                    tag: 271,
                    typ: 2,
                    count: 5,
                    data: b"Acme\0".to_vec(),
                },
                Entry {
                    tag: 274,
                    typ: 3,
                    count: 1,
                    data: vec![1, 0],
                },
            ],
            exif_ifd: vec![Entry {
                tag: 34855,
                typ: 3,
                count: 1,
                data: vec![100, 0],
            }],
            gps_ifd: vec![Entry {
                tag: 1,
                typ: 2,
                count: 2,
                data: b"N\0".to_vec(),
            }],
            interop_ifd: vec![],
            had_thumbnail: false,
        };
        emit(&block)
    }

    #[test]
    fn round_trip_preserves_entries_and_values() {
        let bytes = fixture();
        let parsed = parse(&bytes).unwrap();
        assert!(parsed
            .ifd0
            .iter()
            .any(|e| e.tag == 271 && e.data == b"Acme\0"));
        assert_eq!(parsed.exif_ifd.len(), 1);
        assert_eq!(parsed.gps_ifd.len(), 1);
        assert!(!parsed.had_thumbnail);
        // orientation survives and is readable by the existing reader
        assert_eq!(meta::orientation_from_exif(&bytes), Some(1));
        // emit(parse(x)) is stable
        assert_eq!(emit(&parsed), bytes);
    }

    #[test]
    fn dropping_gps_removes_ifd_and_pointer() {
        let bytes = fixture();
        let mut parsed = parse(&bytes).unwrap();
        parsed.gps_ifd.clear();
        let out = emit(&parsed);
        let re = parse(&out).unwrap();
        assert!(re.gps_ifd.is_empty());
        assert!(!re.ifd0.iter().any(|e| e.tag == TAG_GPS_IFD));
        assert_eq!(re.exif_ifd.len(), 1, "EXIF IFD untouched");
    }

    #[test]
    fn hostile_blocks_answer_typed() {
        assert!(parse(b"XX").is_err());
        assert!(parse(b"II\x2a\x00\xff\xff\xff\xff").is_err());
        // entry pointing outside the block
        let mut bytes = fixture();
        let len = bytes.len();
        bytes.truncate(len - 4);
        // may parse or err depending on where the cut lands — must not panic
        let _ = parse(&bytes);
    }
}
