// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.4 "Metadata privacy controls" [NORMATIVE] — the policy
//! matrix over EXIF/IPTC/XMP field groups; §8.6 (`MetadataPolicy`,
//! `SelectiveMeta`, the IPTC dataset table, the XMP filtering rules).
//! Privacy is a first-class export control (§35).
//!
//! The matrix as implemented (each row per the §29.4 letter):
//!
//! | group | Preserve | StripAll | Selective |
//! |---|---|---|---|
//! | camera + lens EXIF | keep | drop | keep iff `keep_camera` |
//! | GPS (IFD + `exif:GPS*` XMP) | keep | drop | drop iff `strip_gps` |
//! | original date/time | keep | drop | drop iff `strip_personal` |
//! | serials / owner | keep | drop | drop iff `strip_personal` — EVEN with `keep_camera` |
//! | author / copyright | keep | drop | keep iff `keep_copyright` |
//! | XMP packet remainder | keep | drop | kept, rules applied inside |
//! | EXIF thumbnail (IFD1) | regenerate→v1 drops (deferral) | drop | drop |
//! | ICC profile | — GOVERNED BY `ProfilePolicy` ALONE (§29.4 [NORMATIVE]); this module never touches `Metadata::icc` |
//!
//! **Cross-container consistency** (§29.4 [NORMATIVE]): the same logical
//! field is treated identically in EXIF, IPTC, and XMP — no leaking GPS
//! through a forgotten XMP copy. **Fail-closed rule:** wherever
//! selective surgery cannot be performed exactly (unparseable EXIF, an
//! XMP packet carrying a to-be-stripped property — the canonical
//! RDF/XML re-serializer is a recorded deferral, §8.6 — or an IIM block
//! with extended-length datasets), the WHOLE container drops with a
//! `CompatItem`: over-strip, never leak.
//!
//! Interpretation notes (recorded in the lane log): the original
//! date/time group is read as the logical datum — DateTime(306),
//! DateTimeOriginal/Digitized + SubSec/OffsetTime companions — not the
//! single named tag (the consistency rule makes a Digitized copy the
//! same leak); MakerNote (37500) drops under `strip_personal` (opaque
//! vendor blobs routinely embed serials/GPS — fail-closed).

use crate::export::exif_edit;
use crate::meta::Metadata;
use crate::opts::{MetadataPolicy, SelectiveMeta};
use crate::registry::{CompatAction, CompatReport};

/// IFD0: camera body tags.
const IFD0_CAMERA: &[u16] = &[271, 272]; // Make, Model
/// IFD0: authorship tags.
const IFD0_AUTHOR: &[u16] = &[315, 33432]; // Artist, Copyright
/// IFD0: date/time (the file-level stamp; consistency-rule member).
const IFD0_DATES: &[u16] = &[306];

/// EXIF IFD: camera exposure/program group (§29.4 "make, model,
/// exposure, ISO, program" + the §8.6 exposure triad's companions).
const EXIF_CAMERA: &[u16] = &[
    33434, // ExposureTime
    33437, // FNumber
    34850, // ExposureProgram
    34855, // ISOSpeedRatings
    34864, // SensitivityType
    34866, // RecommendedExposureIndex
    37377, // ShutterSpeedValue
    37379, // BrightnessValue
    37380, // ExposureBiasValue
    37383, // MeteringMode
    37384, // LightSource
    37385, // Flash
    41986, // ExposureMode
    41987, // WhiteBalance
    41989, // FocalLengthIn35mmFilm
    41990, // SceneCaptureType
];
/// EXIF IFD: lens group (§29.4 "lens model, focal length, aperture").
const EXIF_LENS: &[u16] = &[
    37378, // ApertureValue
    37381, // MaxApertureValue
    37386, // FocalLength
    42034, // LensSpecification
    42035, // LensMake
    42036, // LensModel
];
/// EXIF IFD: original date/time group (+ companions; module doc).
const EXIF_DATES: &[u16] = &[
    36867, // DateTimeOriginal
    36868, // DateTimeDigitized
    36880, // OffsetTime
    36881, // OffsetTimeOriginal
    36882, // OffsetTimeDigitized
    37520, // SubSecTime
    37521, // SubSecTimeOriginal
    37522, // SubSecTimeDigitized
];
/// EXIF IFD: serials/owner (§29.4: dropped by `strip_personal` even
/// when `keep_camera` is set) + MakerNote (module doc).
const EXIF_PERSONAL: &[u16] = &[
    37500, // MakerNote (fail-closed: vendor blobs carry serials)
    42032, // CameraOwnerName
    42033, // BodySerialNumber
    42037, // LensSerialNumber
];

/// The filtered metadata plus the honest ledger of what was dropped.
pub struct FilterOutcome {
    pub meta: Metadata,
    pub report: CompatReport,
}

/// Apply the §29.4 policy matrix. `Metadata::icc` passes through
/// UNTOUCHED in every arm — the ICC row is governed by `ProfilePolicy`
/// alone (§29.4 [NORMATIVE]; the pipeline's stage 3 owns it).
pub fn apply_policy(meta: &Metadata, policy: MetadataPolicy) -> FilterOutcome {
    let mut report = CompatReport::default();
    let out = match policy {
        MetadataPolicy::Preserve => Metadata {
            icc: meta.icc.clone(),
            exif: meta
                .exif
                .as_deref()
                .and_then(|b| preserve_exif(b, &mut report)),
            xmp: meta.xmp.clone(),
            iptc: meta.iptc.clone(),
            animation_loop: meta.animation_loop,
        },
        MetadataPolicy::StripAll => {
            if meta.exif.is_some() || meta.xmp.is_some() || meta.iptc.is_some() {
                report.push(
                    "metadata",
                    CompatAction::Dropped,
                    "MetadataPolicy::StripAll: EXIF/IPTC/XMP dropped (§29.4); \
                     ICC stays governed by ProfilePolicy",
                );
            }
            Metadata {
                icc: meta.icc.clone(),
                exif: None,
                xmp: None,
                iptc: None,
                // structural format parameter, not privacy metadata
                animation_loop: meta.animation_loop,
            }
        }
        MetadataPolicy::Selective(s) => Metadata {
            icc: meta.icc.clone(),
            exif: meta
                .exif
                .as_deref()
                .and_then(|b| selective_exif(b, s, &mut report)),
            xmp: meta
                .xmp
                .as_deref()
                .and_then(|b| selective_xmp(b, s, &mut report)),
            iptc: meta
                .iptc
                .as_deref()
                .and_then(|b| selective_iptc(b, s, &mut report)),
            animation_loop: meta.animation_loop,
        },
    };
    FilterOutcome { meta: out, report }
}

/// Preserve arm: the block passes whole EXCEPT IFD1 (§29.4 thumbnail
/// row: "regenerated from the export" — regeneration is a recorded
/// deferral, so v1 drops it; a stale thumbnail is the leak the row
/// exists to prevent). A block without IFD1 passes byte-verbatim.
fn preserve_exif(block: &[u8], report: &mut CompatReport) -> Option<Vec<u8>> {
    match exif_edit::parse(block) {
        Ok(parsed) if parsed.had_thumbnail => {
            report.push(
                "metadata.exif.thumbnail",
                CompatAction::Dropped,
                "IFD1 thumbnail dropped (§29.4 regeneration is a recorded deferral; \
                 stale thumbnails never re-export)",
            );
            Some(exif_edit::emit(&parsed))
        }
        Ok(_) => Some(block.to_vec()),
        Err(e) => {
            // Preserve of an unwalkable block: carried verbatim — it was
            // carried verbatim at import too (§8.6 byte-preservation);
            // nothing is being FILTERED, so passing it through leaks
            // nothing the source file didn't already show.
            report.push(
                "metadata.exif",
                CompatAction::Info,
                format!("unparseable block preserved verbatim under Preserve ({e})"),
            );
            Some(block.to_vec())
        }
    }
}

fn selective_exif(block: &[u8], s: SelectiveMeta, report: &mut CompatReport) -> Option<Vec<u8>> {
    let mut parsed = match exif_edit::parse(block) {
        Ok(p) => p,
        Err(e) => {
            report.push(
                "metadata.exif",
                CompatAction::Dropped,
                format!("selective filtering on an unparseable block: dropped whole (fail-closed, §29.4) ({e})"),
            );
            return None;
        }
    };
    if parsed.had_thumbnail {
        report.push(
            "metadata.exif.thumbnail",
            CompatAction::Dropped,
            "IFD1 thumbnail dropped (§29.4 Selective row)",
        );
        // emit() never writes IFD1
    }
    if s.strip_gps && !parsed.gps_ifd.is_empty() {
        parsed.gps_ifd.clear(); // the WHOLE IFD, never partial (§8.6)
        report.push(
            "metadata.exif.gps",
            CompatAction::Dropped,
            "GPS IFD dropped (strip_gps, §29.4)",
        );
    }
    let drop_ifd0: Vec<u16> = {
        let mut v = Vec::new();
        if !s.keep_camera {
            v.extend_from_slice(IFD0_CAMERA);
        }
        if !s.keep_copyright {
            v.extend_from_slice(IFD0_AUTHOR);
        }
        if s.strip_personal {
            v.extend_from_slice(IFD0_DATES);
        }
        v
    };
    let drop_exif: Vec<u16> = {
        let mut v = Vec::new();
        if !s.keep_camera {
            v.extend_from_slice(EXIF_CAMERA);
            v.extend_from_slice(EXIF_LENS);
        }
        if s.strip_personal {
            v.extend_from_slice(EXIF_DATES);
            v.extend_from_slice(EXIF_PERSONAL); // even when keep_camera
        }
        v
    };
    parsed.ifd0.retain(|e| !drop_ifd0.contains(&e.tag));
    parsed.exif_ifd.retain(|e| !drop_exif.contains(&e.tag));
    Some(exif_edit::emit(&parsed))
}

/// XMP under Selective: the §29.4 rules applied INSIDE the packet need
/// the §8.6 canonical RDF/XML re-serializer (a recorded deferral — the
/// File Info edit wave). v1 scans the packet for properties the policy
/// would strip: a clean packet passes VERBATIM (§8.6 letter); an
/// offending packet drops WHOLE with a `CompatItem` (fail-closed —
/// over-strip, never leak; the §29.4 consistency rule outranks
/// carriage).
fn selective_xmp(packet: &[u8], s: SelectiveMeta, report: &mut CompatReport) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(packet);
    let mut offending: Vec<&'static str> = Vec::new();
    if s.strip_gps && text.contains("exif:GPS") {
        offending.push("exif:GPS* (strip_gps)");
    }
    if s.strip_personal {
        for marker in [
            "dc:creator",
            "CreatorContactInfo",
            "xmp:CreateDate",
            "SerialNumber",
            "OwnerName",
            "photoshop:DateCreated",
        ] {
            if text.contains(marker) {
                offending.push("personal fields (strip_personal)");
                break;
            }
        }
    }
    if !s.keep_copyright && (text.contains("dc:rights") || text.contains("dc:creator")) {
        offending.push("dc:rights/dc:creator (keep_copyright = false)");
    }
    if !s.keep_camera {
        for marker in [
            "tiff:Make",
            "tiff:Model",
            "exif:ExposureTime",
            "exif:FNumber",
            "aux:Lens",
        ] {
            if text.contains(marker) {
                offending.push("camera/lens duplicates (keep_camera = false)");
                break;
            }
        }
    }
    if offending.is_empty() {
        Some(packet.to_vec())
    } else {
        report.push(
            "metadata.xmp",
            CompatAction::Dropped,
            format!(
                "packet carries {}; in-packet surgery is a recorded deferral — dropped whole \
                 (fail-closed, §29.4 cross-container rule)",
                offending.join(", ")
            ),
        );
        None
    }
}

/// IPTC-IIM under Selective: dataset-level filtering per the §8.6 table
/// (`strip_personal` removes Creator 2:80, City 2:90, State 2:95,
/// Country 2:101 — and DateCreated 2:55 per the date-group consistency
/// rule; `keep_copyright` retains 2:116 and gates 2:80). Unlisted
/// datasets pass byte-wise. Extended-length datasets: fail-closed whole
/// drop (module doc).
fn selective_iptc(block: &[u8], s: SelectiveMeta, report: &mut CompatReport) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(block.len());
    let mut i = 0usize;
    while i < block.len() {
        if block[i] != 0x1C {
            report.push(
                "metadata.iptc",
                CompatAction::Dropped,
                "malformed IIM block under Selective: dropped whole (fail-closed, §29.4)",
            );
            return None;
        }
        if i + 5 > block.len() {
            report.push(
                "metadata.iptc",
                CompatAction::Dropped,
                "truncated IIM dataset under Selective: dropped whole (fail-closed, §29.4)",
            );
            return None;
        }
        let record = block[i + 1];
        let dataset = block[i + 2];
        let len = u16::from_be_bytes([block[i + 3], block[i + 4]]) as usize;
        if len & 0x8000 != 0 {
            report.push(
                "metadata.iptc",
                CompatAction::Dropped,
                "extended-length IIM dataset: dropped whole (fail-closed deferral, §29.4)",
            );
            return None;
        }
        let end = i + 5 + len;
        if end > block.len() {
            report.push(
                "metadata.iptc",
                CompatAction::Dropped,
                "IIM dataset overruns block under Selective: dropped whole (fail-closed, §29.4)",
            );
            return None;
        }
        let drop = record == 2
            && match dataset {
                80 => s.strip_personal || !s.keep_copyright, // Creator (By-line)
                90 | 95 | 101 => s.strip_personal,           // City/State/Country
                55 => s.strip_personal,                      // DateCreated (date group)
                116 => !s.keep_copyright,                    // CopyrightNotice
                _ => false,
            };
        if drop {
            report.push(
                format!("metadata.iptc.2:{dataset:02}"),
                CompatAction::Dropped,
                "dropped per the §29.4 matrix",
            );
        } else {
            out.extend_from_slice(&block[i..end]);
        }
        i = end;
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::exif_edit::{emit, Entry, ExifBlock};

    fn exif_with_everything() -> Vec<u8> {
        emit(&ExifBlock {
            big_endian: false,
            ifd0: vec![
                Entry {
                    tag: 271,
                    typ: 2,
                    count: 5,
                    data: b"Acme\0".to_vec(),
                }, // Make
                Entry {
                    tag: 315,
                    typ: 2,
                    count: 4,
                    data: b"Me\0\0".to_vec(),
                }, // Artist
                Entry {
                    tag: 306,
                    typ: 2,
                    count: 20,
                    data: b"2026:08:17 00:00:00\0".to_vec(),
                },
                Entry {
                    tag: 282,
                    typ: 5,
                    count: 1,
                    data: vec![72, 0, 0, 0, 1, 0, 0, 0],
                }, // XRes
            ],
            exif_ifd: vec![
                Entry {
                    tag: 33434,
                    typ: 5,
                    count: 1,
                    data: vec![1, 0, 0, 0, 250, 0, 0, 0],
                }, // ExposureTime
                Entry {
                    tag: 42033,
                    typ: 2,
                    count: 6,
                    data: b"12345\0".to_vec(),
                }, // BodySerial
                Entry {
                    tag: 36867,
                    typ: 2,
                    count: 20,
                    data: b"2026:08:17 00:00:00\0".to_vec(),
                },
            ],
            gps_ifd: vec![Entry {
                tag: 1,
                typ: 2,
                count: 2,
                data: b"N\0".to_vec(),
            }],
            interop_ifd: vec![],
            had_thumbnail: false,
        })
    }

    fn meta_with(exif: Vec<u8>) -> Metadata {
        Metadata {
            icc: Some(vec![1, 2, 3]),
            exif: Some(exif),
            xmp: Some(b"<x:xmpmeta><rdf:RDF/></x:xmpmeta>".to_vec()),
            iptc: None,
            animation_loop: None,
        }
    }

    #[test]
    fn strip_all_drops_containers_but_never_icc() {
        let m = meta_with(exif_with_everything());
        let out = apply_policy(&m, MetadataPolicy::StripAll);
        assert!(out.meta.exif.is_none());
        assert!(out.meta.xmp.is_none());
        assert_eq!(
            out.meta.icc,
            Some(vec![1, 2, 3]),
            "§29.4: ICC governed by ProfilePolicy alone"
        );
    }

    #[test]
    fn selective_default_strips_gps_keeps_camera_and_copyright() {
        let m = meta_with(exif_with_everything());
        let out = apply_policy(&m, MetadataPolicy::Selective(SelectiveMeta::default()));
        let filtered = out.meta.exif.expect("block survives");
        let parsed = crate::export::exif_edit::parse(&filtered).unwrap();
        assert!(parsed.gps_ifd.is_empty(), "GPS dropped (dialog default on)");
        assert!(
            parsed.ifd0.iter().any(|e| e.tag == 271),
            "Make kept (keep_camera)"
        );
        assert!(
            parsed.ifd0.iter().any(|e| e.tag == 315),
            "Artist kept (keep_copyright)"
        );
        assert!(
            parsed.exif_ifd.iter().any(|e| e.tag == 36867),
            "DateTimeOriginal kept (strip_personal off by default)"
        );
        assert!(
            parsed.exif_ifd.iter().any(|e| e.tag == 42033),
            "serial kept while strip_personal is off"
        );
    }

    #[test]
    fn one_click_privacy_drops_personal_even_with_keep_camera() {
        let m = meta_with(exif_with_everything());
        let out = apply_policy(
            &m,
            MetadataPolicy::Selective(SelectiveMeta::privacy_one_click()),
        );
        let parsed = crate::export::exif_edit::parse(out.meta.exif.as_ref().unwrap()).unwrap();
        assert!(
            parsed.exif_ifd.iter().any(|e| e.tag == 33434),
            "exposure kept (keep_camera)"
        );
        assert!(
            !parsed.exif_ifd.iter().any(|e| e.tag == 42033),
            "§29.4: serial dropped even when keep_camera is set"
        );
        assert!(
            !parsed.exif_ifd.iter().any(|e| e.tag == 36867),
            "DateTimeOriginal dropped"
        );
        assert!(
            !parsed.ifd0.iter().any(|e| e.tag == 306),
            "IFD0 DateTime dropped (consistency)"
        );
        assert!(parsed.gps_ifd.is_empty());
    }

    #[test]
    fn xmp_with_gps_copy_drops_whole_fail_closed() {
        let mut m = meta_with(exif_with_everything());
        m.xmp = Some(b"<x:xmpmeta exif:GPSLatitude=\"1,2N\"/>".to_vec());
        let out = apply_policy(&m, MetadataPolicy::Selective(SelectiveMeta::default()));
        assert!(
            out.meta.xmp.is_none(),
            "no leaking GPS through a forgotten XMP copy"
        );
        assert!(out.report.items.iter().any(|i| i.feature == "metadata.xmp"));
    }

    #[test]
    fn clean_xmp_passes_verbatim_under_selective() {
        let m = meta_with(exif_with_everything());
        let out = apply_policy(&m, MetadataPolicy::Selective(SelectiveMeta::default()));
        assert_eq!(out.meta.xmp, m.xmp);
    }

    #[test]
    fn iptc_matrix_rows() {
        fn ds(record: u8, dataset: u8, v: &[u8]) -> Vec<u8> {
            let mut b = vec![0x1C, record, dataset];
            b.extend_from_slice(&(v.len() as u16).to_be_bytes());
            b.extend_from_slice(v);
            b
        }
        let mut block = Vec::new();
        block.extend(ds(2, 80, b"Author"));
        block.extend(ds(2, 90, b"Springfield"));
        block.extend(ds(2, 116, b"(c) me"));
        block.extend(ds(2, 25, b"keyword"));
        let m = Metadata {
            iptc: Some(block),
            ..Metadata::default()
        };
        let out = apply_policy(
            &m,
            MetadataPolicy::Selective(SelectiveMeta::privacy_one_click()),
        );
        let kept = out.meta.iptc.expect("block survives");
        let kept_str = String::from_utf8_lossy(&kept);
        assert!(
            !kept_str.contains("Author"),
            "2:80 dropped (strip_personal)"
        );
        assert!(!kept_str.contains("Springfield"), "2:90 dropped");
        assert!(kept_str.contains("(c) me"), "2:116 kept (keep_copyright)");
        assert!(
            kept_str.contains("keyword"),
            "unlisted dataset preserved byte-wise"
        );
    }

    #[test]
    fn preserve_passes_verbatim_when_no_thumbnail() {
        let m = meta_with(exif_with_everything());
        let out = apply_policy(&m, MetadataPolicy::Preserve);
        assert_eq!(out.meta.exif, m.exif);
        assert_eq!(out.meta.xmp, m.xmp);
        assert!(out.report.items.is_empty());
    }
}
