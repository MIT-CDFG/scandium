// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.1–§5.5 (profiles, working spaces, conversion), §6.3 (depth
//! conversion formulas), §7.5 (`profiles/{id}.icc` carriage);
//! reports/O-COLOR-log.md "CORE-SIDE NEEDS" items 1–6 (the brokered list
//! this module discharges; each re-checked against the merged CMM as
//! wired).
//!
//! Core↔CMM glue: deterministic `ProfileId`s for the §5.1 builtins and the
//! untagged state, per-document seeding, `ProfileId → ProfileUid`
//! resolution (need 1 — the document profile store IS the map; embedded
//! ICC bytes re-open in the per-engine [`appname_color::Cms`] on demand,
//! content-deduped by construction), and the `ColorError → ErrorCode`
//! boundary mapping (need 6).
//!
//! Value-capture reading for converts (dispatch item — §10/§7.8/§53.10
//! read): JOURNAL replay re-executes the convert command through the same
//! CMM — deterministic per machine/build because the lcms2 flag elections
//! make output a pure function of (profile bytes, formats, intent, flags)
//! (O-COLOR log, the D-48 replay surface). §7.8's tile-blob value capture
//! (crash replay restoring captured after-image BYTES without re-running
//! the CMM) rides the autosave TILEREF frames when they land; §53.10's
//! model-value-capture rule is about AI tools and does not bind the CMM.
//! Reading recorded in the lane log.

use uuid::Uuid;

use appname_color::{BuiltinSpace, ColorError, ProfileUid};

use crate::engine::doc_new::APPNAME_NS;
use crate::engine::error::{EngineError, ErrorCode};
use crate::graph::document::{DocProfile, DocProfileSource, Document};
use crate::graph::id::ProfileId;

/// Deterministic id for a §5.1 builtin working space.
pub fn builtin_profile_id(space: BuiltinSpace) -> ProfileId {
    ProfileId(Uuid::new_v5(
        &APPNAME_NS,
        format!("profile/builtin/{}", space.tag()).as_bytes(),
    ))
}

/// Deterministic id for the untagged state (`assign_profile{profile:null}`).
pub fn untagged_profile_id() -> ProfileId {
    ProfileId(Uuid::new_v5(&APPNAME_NS, b"profile/untagged"))
}

/// Deterministic id for the CIELAB connection space (`doc.mode_convert`
/// to `lab_a`; the CMM builds it via `Cms::builtin_lab`, §5.3).
pub fn builtin_lab_profile_id() -> ProfileId {
    ProfileId(Uuid::new_v5(&APPNAME_NS, b"profile/builtin/lab"))
}

/// Deterministic id for embedded ICC bytes (content-derived via the
/// `ProfileUid`, so identical bytes mint identical ids — §0.3 discipline).
pub fn icc_profile_id(uid: ProfileUid) -> ProfileId {
    ProfileId(Uuid::new_v5(
        &APPNAME_NS,
        format!("profile/icc/{:032x}", uid.as_u128()).as_bytes(),
    ))
}

/// Seed the deterministic entries every document carries: the five §5.1
/// builtins, the untagged state, and the doc.new working alias (which maps
/// to sRGB interim). Builtin entries never fold into hashes or
/// document.json — seeding is hash-invisible (mask precedent).
pub(crate) fn seed_builtin_profiles(doc: &mut Document) {
    for space in BuiltinSpace::ALL {
        doc.register_profile(
            builtin_profile_id(space),
            DocProfile {
                uid: ProfileUid::of_builtin(space),
                source: DocProfileSource::Builtin(space),
            },
        );
    }
    doc.register_profile(
        builtin_lab_profile_id(),
        DocProfile {
            // The uid slot is a placeholder — resolution goes through
            // `Cms::builtin_lab()` (the Lab space is CMM-built, not a
            // BuiltinSpace row); deterministic like every seeded entry.
            uid: ProfileUid::of_builtin(BuiltinSpace::Srgb),
            source: DocProfileSource::BuiltinLab,
        },
    );
    doc.register_profile(
        untagged_profile_id(),
        DocProfile {
            uid: ProfileUid::of_builtin(BuiltinSpace::Srgb),
            source: DocProfileSource::Untagged,
        },
    );
    // The doc.new working alias (`profile/working/rgba`): sRGB interim.
    let alias =
        crate::engine::doc_new::builtin_working_profile(crate::tile::model::ColorModelKind::Rgba);
    doc.register_profile(
        alias,
        DocProfile {
            uid: ProfileUid::of_builtin(BuiltinSpace::Srgb),
            source: DocProfileSource::Builtin(BuiltinSpace::Srgb),
        },
    );
}

/// §5.2 `ColorError` → wire taxonomy (need 6): the four `error.color.*`
/// codes map onto their rows; scope refusals map typed; everything else is
/// an internal-shaped bad_target with the CMM detail carried in prose.
pub(crate) fn map_color_error(e: ColorError) -> EngineError {
    let msg = format!("{e:?}");
    match e {
        ColorError::ProfileTooLarge { .. } => EngineError::new(
            ErrorCode::ProfileTooLarge,
            "ICC profile exceeds the 16 MiB ingress cap (§5.2)",
        ),
        ColorError::ProfileMalformed(detail) => EngineError::new(
            ErrorCode::ProfileMalformed,
            format!("ICC profile failed §5.2 validation: {detail}"),
        ),
        ColorError::ProfileClass(_) => EngineError::new(
            ErrorCode::ProfileClass,
            "the profile's class is not usable as a working space (§5.2)",
        ),
        ColorError::ProfileSpaceMismatch => EngineError::new(
            ErrorCode::ProfileSpaceMismatch,
            "the profile's data color space does not match the document mode (§5.5)",
        ),
        ColorError::DepthPaused(_) | ColorError::ImplicitU16ToF16 => EngineError::new(
            ErrorCode::NotImplemented,
            "f16 endpoints are paused engine-wide until the half-record adoption (§6.3)",
        ),
        ColorError::Reserved(detail) => EngineError::new(
            ErrorCode::NotImplemented,
            format!("contract-valid value with reserved engine semantics: {detail}"),
        ),
        ColorError::IccBackendPending | ColorError::NoMatrixPath => EngineError::new(
            ErrorCode::NotImplemented,
            format!("conversion path not built at CP-0.5: {msg}"),
        ),
        _ => EngineError::bad_target(format!("CMM refused: {msg}")),
    }
}

/// Resolve a document profile entry to its CMM uid, opening embedded ICC
/// bytes in the per-engine [`appname_color::Cms`] on demand (content-keyed
/// — reopening is a dedup hit).
pub(crate) fn resolve_uid(
    cms: &mut appname_color::Cms,
    entry: &DocProfile,
) -> Result<ProfileUid, EngineError> {
    match &entry.source {
        DocProfileSource::Builtin(space) => Ok(cms.builtin(*space)),
        DocProfileSource::Untagged => Ok(cms.builtin(BuiltinSpace::Srgb)),
        DocProfileSource::BuiltinLab => Ok(cms.builtin_lab()),
        DocProfileSource::Icc(bytes) => cms
            .open_profile((**bytes).clone(), appname_color::ColorModel::Rgb)
            .map_err(map_color_error),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// Door C6 smoke: the present-pass payloads bake against the LIVE
    /// document's working space — deterministic bytes (§5.4 bake loop),
    /// proof LUT carrying clean gamut flags for an in-gamut proof space.
    #[test]
    fn display_and_proof_luts_bake_from_the_document_working_space() {
        let mut e =
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new()).build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "d15b0000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 8, "height": 8},
            })
            .to_string(),
        )
        .unwrap();
        let intent = appname_color::Intent::RelativeColorimetric;
        let lut = e.bake_display_lut(None, intent, true).unwrap();
        assert_eq!(lut.size as usize, appname_color::LUT_SIZE);
        assert_eq!(lut.data.len(), appname_color::LUT_SIZE.pow(3));
        // Determinism: a second bake is value-identical (cache or not).
        let lut2 = e.bake_display_lut(None, intent, true).unwrap();
        assert_eq!(lut, lut2);
        // Proof leg: sRGB proof over an sRGB working/display chain is
        // in-gamut everywhere — every flag clean.
        let proof = appname_color::SoftProof {
            proof_profile: e.builtin_profile_uid(appname_color::BuiltinSpace::Srgb),
            intent,
            bpc: true,
            simulate_paper: false,
            gamut_warning: true,
        };
        let plut = e.bake_proof_lut(None, &proof).unwrap();
        assert_eq!(plut.data.len(), appname_color::LUT_SIZE.pow(3));
        assert!(
            plut.data.iter().all(|px| px[3] < 0.5),
            "sRGB→sRGB proof flags nothing out of gamut (§5.6)"
        );
        // No document: the chain refuses typed.
        let mut bare =
            crate::engine::kinds::register(crate::engine::kind_impl::EngineAssembly::new()).build();
        assert!(bare.bake_display_lut(None, intent, true).is_err());
    }
}
