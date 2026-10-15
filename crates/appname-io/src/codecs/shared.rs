// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §8.1 (decode contract: orientation applied on import,
//! metadata never fatal), §8.6; DD ⑫. The decode tail every raster codec
//! shares: EXIF-orientation bake (+ value capture), metadata
//! normalization, and the plane→Buffer ingestion hop.

use crate::error::CodecError;
use crate::meta::{self, Metadata};
use crate::opts::DecodeOpts;
use crate::plane::RgbaPlane;
use crate::registry::{CompatAction, CompatReport, Imported};

/// Finish a single-image decode: apply/capture orientation from the raw
/// EXIF block (if any), normalize the carried block to orientation 1
/// (§8.6), convert through the ingestion door, and assemble [`Imported`].
pub fn finish_single(
    plane: RgbaPlane,
    exif_raw: Option<Vec<u8>>,
    icc: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
    opts: &DecodeOpts,
    mut report: CompatReport,
) -> Result<Imported, CodecError> {
    let mut meta = Metadata {
        icc: icc.clone(),
        exif: None,
        xmp,
        iptc: None,
        animation_loop: None,
    };
    let mut orientation_applied = None;

    let mut plane = plane;
    if let Some(raw) = exif_raw {
        let parsed = meta::orientation_from_exif(&raw);
        let mut carried = raw;
        match parsed {
            Some(v @ 1..=8) => {
                if opts.apply_orientation {
                    plane = plane.apply_orientation(v)?;
                    orientation_applied = Some(v);
                    // §8.6: the carried block never re-exports a stale
                    // value.
                    if meta::normalize_exif_orientation(&mut carried).is_err() {
                        report.push(
                            "metadata.exif",
                            CompatAction::Dropped,
                            "orientation normalization failed on a malformed block",
                        );
                        carried.clear();
                    }
                }
            }
            Some(v) => {
                // Values outside 1–8 are treated as 1 with a CompatItem
                // (§8.6 [NORMATIVE]).
                report.push(
                    "metadata.exif.orientation",
                    CompatAction::Approximated,
                    format!("orientation {v} outside 1..=8 treated as 1"),
                );
                orientation_applied = Some(1);
            }
            None => {}
        }
        if opts.keep_unknown_metadata && !carried.is_empty() {
            meta.exif = Some(carried);
        }
    }

    let (ingress_png, buffer) = plane.to_png_and_buffer(opts.working_profile)?;
    Ok(Imported {
        buffer,
        ingress_png,
        size: (plane.width, plane.height),
        profile: meta.icc.clone(),
        meta,
        layers: None,
        orientation_applied,
        report,
        compat_report: None,
    })
}

/// Record a malformed metadata block per the §8.1 `MetadataInvalid`
/// non-fatal rule and return `None` (decode continues).
pub fn drop_bad_metadata<T>(
    report: &mut CompatReport,
    block: &'static str,
    err: impl std::fmt::Display,
) -> Option<T> {
    report.push(
        format!("metadata.{block}"),
        CompatAction::Dropped,
        format!("malformed block dropped: {err}"),
    );
    None
}
