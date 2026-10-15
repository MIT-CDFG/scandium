// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! RAW develop v1 — the license-gated module (CP-1.0 O-IO row).
//!
//! Spec: ES §8.3 (RAW pipeline, LibRaw) [NORMATIVE]; ES Part 43 (RAW
//! develop specification — the executable contract: §43.1 v1 scope,
//! §43.2 camera profiles, §43.3 parameters + pipeline order, §43.4
//! determinism, §43.5 sidecars/unsupported flows, §43.7 acceptance);
//! ES §33.1 feature matrix row `raw`; ES Part 48 (LibRaw pin/licensing);
//! DD ⑩⑫. Algorithm authority: `spec/algorithms/dossiers/raw-demosaic.yaml`
//! (AHD election; see [`develop`]).
//!
//! **Module boundary (the license posture).** LibRaw is LGPL-2.1-elected
//! (of its LGPL-2.1/CDDL-1.0 dual), dynamic-link, OPTIONAL MODULE — never a
//! core link (spec-side record `bogos/deps/libraw.yaml`; ES §48.2 rules
//! 1–2). Concretely here:
//! * everything RAW rides the `raw` cargo feature (§33.1 row: desktop
//!   default ON, dropped ⇒ graceful fallback). With the feature off this
//!   module compiles down to [`params`] only (schema/validation — a v1
//!   reader must still validate `raw.develop` blocks in documents, §43.3 +
//!   §7.7) and the RAW registry slot does not exist, so RAW opens fail
//!   with the §43.5 absent-module flow.
//! * The deps record `deps/libraw.yaml` is MERGED and CONSUMED by this
//!   slice (its `preAuthorizedFor`): the binding is the OWN in-workspace
//!   facade `sys/libraw-sys` (ES §1.4 crate map; zero external crates;
//!   runtime dynamic loading only — never vendored, never a build-time
//!   link). Module resolution, the sanitized stage-1 wrapper, and the
//!   §43.5 absent-module/unsupported-camera arms live in [`stage1`].
//! * stages 2–6 are Appname's OWN implementations regardless of the
//!   binding (§43.4 stage ownership) — they live in [`develop`], are pure
//!   planar functions, tested without any dylib present.
//! * lens profiles: the Row-C lensdb reader (`appname-algo`, all four
//!   inferences O-QA-ratified 2026-08-17) is consumed through
//!   [`lensdb_seam`] under the O-INT consumer-seam SPLIT ruling
//!   (shipped db strict/fail-loud; user dir degrade-to-warnings-and-
//!   skip). Stage 4b machinery is live in [`develop`]; the v1
//!   `lensCorrection` PARAM stays reserved per §43.3 (rejected
//!   non-null, exactly as before).
//!
//! **Honest deferral inventory delta** (lane log carries the full
//! list): the stage-1 MOSAIC EGRESS (CFA plane + per-channel black
//! levels + orientation + EXIF) is blocked at the documented C API — a
//! RECORD-LEVEL FINDING escalated to O-INT/O-BUILD ([`stage1`] module
//! doc); the non-Bayer delegation and stage-7 profile generation ride
//! that resolution (colorant math itself is live:
//! [`develop::stage7_colorants_d50`], the bound D50/D65 advisory).

pub mod params;

#[cfg(feature = "raw")]
pub mod codec;
#[cfg(feature = "raw")]
pub mod develop;
#[cfg(feature = "raw")]
pub mod lensdb_seam;
#[cfg(feature = "raw")]
pub mod stage1;

/// The §8.1 registered extension list — also the open-dialog filter set
/// for the §43.5 / §48.4 absent-module fallback ("no RAW" builds filter
/// these extensions out with an explanatory `error.unsupported.*`
/// message), which is why it is compiled UNCONDITIONALLY.
pub const RAW_EXTENSIONS: &[&str] = &["cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2"];

/// Whether the RAW module is compiled into this build (§33.1 feature
/// `raw`; §43.5 absent-module flow keys on this).
pub const fn module_compiled() -> bool {
    cfg!(feature = "raw")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_list_is_lowercase_and_stable() {
        for e in RAW_EXTENSIONS {
            assert_eq!(*e, e.to_ascii_lowercase());
        }
        // The §8.1 table's enumerated set, in its order.
        assert_eq!(
            RAW_EXTENSIONS,
            &["cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2"]
        );
    }

    #[test]
    fn module_compiled_reflects_feature() {
        assert_eq!(module_compiled(), cfg!(feature = "raw"));
    }
}
