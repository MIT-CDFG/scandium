// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The lens-profile CONSUMER SEAM — the O-INT split ruling (2026-08-17),
//! implemented exactly where the ruling put it: at the O-IO consumer,
//! with the Row-C reader's semantics untouched.
//!
//! Spec: ALGORITHMS.md §21 (lensfun consumed as DATA only, own reader);
//! ES Part 43 §43.3 stage 4b (the consuming stage; its PARAM surface
//! stays reserved — [`crate::raw::develop::LensCorrection`] doc);
//! record `deps/lensfun-db.yaml` (`stagedDirContract`, `dataPaths:
//! data/lensfun-db/db`). The §43.3 "file/LibRaw public data only" vs
//! ALGORITHMS §21 lensfun-route wording tension is a KNOWN O-INT
//! fold-in register item — this module follows the merged record + the
//! ruling, and resolves nothing about that wording.
//!
//! **THE SPLIT RULING, verbatim discipline:**
//! * the SHIPPED db (`data/lensfun-db/db`, payload-sha-pinned by the
//!   record) is read STRICTLY, all-or-nothing — malformed shipped data
//!   means a broken build, and [`load_shipped`] fails LOUD (typed
//!   `error.internal`), never skips;
//! * the USER-REPLACEABLE dir (an OS-user-location override outside the
//!   repo) degrades to warnings-and-skip AT THIS SEAM: one bad
//!   user-dropped XML costs exactly that file, surfaced as a
//!   [`LensDbWarning`], never the whole database;
//! * the reader's pinned all-or-nothing contract is UNCHANGED — this
//!   seam COMPOSES strict per-file reads
//!   ([`appname_algo::lensdb::from_named_documents`] over one file at a
//!   time for user data); no reader semantics move.
//!
//! All four documented reader inferences (interpolation scheme,
//! ptlens/poly3 normalization, TCA degree assignment, crop direction)
//! were O-QA-RATIFIED 2026-08-17 (dossier-blessing round riding this
//! slice's dispatch) — consumption is unblocked; goldens still route
//! via O-QA only.

use std::path::{Path, PathBuf};

use appname_algo::lensdb::{
    from_named_documents, read_dir, resolve_calibration, LensDatabase, LookupError,
    ResolvedCalibration,
};

use crate::error::CodecError;

/// The record's `dataPaths` staging area, relative to the application
/// data root (`deps/lensfun-db.yaml`; packaging ships it read-only).
pub const SHIPPED_DB_RELPATH: &str = "data/lensfun-db/db";

/// One degraded user-dir finding (the ruling's warnings-and-skip arm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensDbWarning {
    /// File (or `<merged>` for cross-file reader findings).
    pub file: String,
    /// Plain-words detail (reader error text or reader warning).
    pub detail: String,
}

/// Resolve the shipped db dir under an application data root.
pub fn shipped_db_dir(data_root: &Path) -> PathBuf {
    data_root.join(SHIPPED_DB_RELPATH)
}

/// STRICT shipped-database load (ruling arm 1): any reader error is a
/// loud, typed failure — corruption of what we ship is never skipped.
/// Reader-level warnings (duplicate identities, mixed versions) are
/// surfaced to the caller; the shipped corpus is warning-free today and
/// a warning appearing on shipped data is itself a tripwire.
pub fn load_shipped(shipped_dir: &Path) -> Result<LensDatabase, CodecError> {
    read_dir(shipped_dir).map_err(|e| {
        CodecError::internal(format!(
            "lensfun shipped db (strict, fail-loud per the consumer-seam ruling): {e}"
        ))
    })
}

/// Shipped + optional user dir (ruling arms 1 + 2). Returns the merged
/// database plus every user-dir degradation, deterministically ordered.
///
/// Merge mechanics (documented): shipped files are read strictly; each
/// user file is parsed ALONE through the strict reader (per-file
/// composition, semantics unmoved) and dropped-with-warning on any
/// error; the surviving set is parsed as one corpus, shipped files
/// first then user files (both name-sorted), so cross-file identity
/// resolution follows the reader's own documented duplicate rule.
/// In the (unexpected) case that the union parse fails even though
/// every user file parsed alone, ALL user files are dropped with a
/// warning and the shipped database stands — degradation never reaches
/// the shipped data.
pub fn load_with_user_dir(
    shipped_dir: &Path,
    user_dir: Option<&Path>,
) -> Result<(LensDatabase, Vec<LensDbWarning>), CodecError> {
    // Arm 1: strict shipped read (also validates the dir as a corpus).
    let shipped_db = load_shipped(shipped_dir)?;
    let mut warnings: Vec<LensDbWarning> = shipped_db
        .warnings
        .iter()
        .map(|w| LensDbWarning {
            file: "<shipped>".to_owned(),
            detail: w.clone(),
        })
        .collect();

    let Some(user) = user_dir else {
        return Ok((shipped_db, warnings));
    };
    if !user.is_dir() {
        // An absent user dir is simply "no user data" (the override is
        // optional by design), not an error.
        return Ok((shipped_db, warnings));
    }

    // Arm 2: per-file strict reads over the user dir, degrade-and-skip.
    let mut user_docs: Vec<(String, String)> = Vec::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(user)
        .map_err(|e| CodecError::internal(format!("lensfun user dir listing: {e}")))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
        })
        .collect();
    names.sort();
    for path in names {
        let display = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<non-utf8 name>")
            .to_owned();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(LensDbWarning {
                    file: display,
                    detail: format!("skipped: {e}"),
                });
                continue;
            }
        };
        // The STRICT per-file read the ruling composes.
        match from_named_documents(&[(display.clone(), content.clone())]) {
            Ok(_) => user_docs.push((display, content)),
            Err(e) => warnings.push(LensDbWarning {
                file: display,
                detail: format!("skipped: {e}"),
            }),
        }
    }

    if user_docs.is_empty() {
        return Ok((shipped_db, warnings));
    }

    // Union parse: shipped (name-sorted) then surviving user docs.
    let mut all_docs: Vec<(String, String)> = Vec::new();
    let mut shipped_names: Vec<PathBuf> = std::fs::read_dir(shipped_dir)
        .map_err(|e| CodecError::internal(format!("lensfun shipped dir listing: {e}")))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
        })
        .collect();
    shipped_names.sort();
    for path in shipped_names {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let content = std::fs::read_to_string(&path)
            .map_err(|e| CodecError::internal(format!("lensfun shipped db read: {e}")))?;
        all_docs.push((name, content));
    }
    all_docs.extend(user_docs);
    match from_named_documents(&all_docs) {
        Ok(merged) => {
            warnings.extend(merged.warnings.iter().map(|w| LensDbWarning {
                file: "<merged>".to_owned(),
                detail: w.clone(),
            }));
            Ok((merged, warnings))
        }
        Err(e) => {
            // Degradation never reaches the shipped data: drop the user
            // set whole, keep the strict shipped db.
            warnings.push(LensDbWarning {
                file: "<user-dir>".to_owned(),
                detail: format!("all user files dropped (union parse failed): {e}"),
            });
            Ok((shipped_db, warnings))
        }
    }
}

/// One shot's lookup query (EXIF-derived; every field untrusted input).
#[derive(Debug, Clone, PartialEq)]
pub struct ShotQuery<'a> {
    pub cam_maker: &'a str,
    pub cam_model: &'a str,
    pub lens_maker: &'a str,
    pub lens_model: &'a str,
    /// Focal length in mm.
    pub focal_mm: f64,
    /// f-number, when known.
    pub aperture: Option<f64>,
    /// Subject distance in meters, when known.
    pub distance_m: Option<f64>,
}

/// Resolve a per-shot calibration for stage 4b. `Ok(None)` = no match /
/// no calibration (correction simply unavailable, never an error);
/// non-finite query values (EXIF 0/0 rationals and friends) are typed
/// `error.codec.corrupt` — the Row-C review's own contract at this seam.
pub fn resolve_for_shot(
    db: &LensDatabase,
    q: &ShotQuery<'_>,
) -> Result<Option<ResolvedCalibration>, CodecError> {
    let camera = db.find_camera(q.cam_maker, q.cam_model);
    let Some(lens) = db.find_lens(q.lens_maker, q.lens_model, camera) else {
        return Ok(None);
    };
    match resolve_calibration(lens, q.focal_mm, q.aperture, q.distance_m) {
        Ok(resolved) => Ok(resolved),
        Err(LookupError::NonFiniteQuery { param }) => Err(CodecError::corrupt(format!(
            "raw.lens_query_non_finite: {param}"
        ))),
    }
}
