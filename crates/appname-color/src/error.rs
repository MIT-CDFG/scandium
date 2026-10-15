// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.2 (profile validation failure taxonomy: `error.color.*`),
//! §1.6 (structured errors, never a crash at the wrapper boundary), §6.3
//! (the never-implicit `u16 → f16` rule).
//!
//! `ColorError` is this crate's whole failure surface. Variants that
//! correspond to a spec-named wire code expose it via [`ColorError::wire_code`];
//! the rest are engine-internal conditions that `appname-core` maps into its
//! own `error.engine.*` taxonomy at the command boundary (this crate does
//! not invent wire codes the spec does not name).

use core::fmt;

use crate::profile::{ProfileClass, ProfileUid};
use crate::types::Depth;

/// Structured color-management failure (ES §5.2 / §1.6).
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ColorError {
    /// §5.2 validation 1: byte stream over the 16 MiB cap.
    ProfileTooLarge { len: usize },
    /// §5.2 validation 2: truncated/inconsistent profile bytes. The static
    /// string names the exact header check that failed (test-stable).
    ProfileMalformed(&'static str),
    /// §5.2 validation 3: class outside {Display, Input, Output, ColorSpace}
    /// where a document/working profile is expected.
    ProfileClass(ProfileClass),
    /// §5.2 validation 4: profile data color space ≠ buffer model.
    ProfileSpaceMismatch,
    /// A `ProfileUid` that is not in the registry (dangling reference).
    UnknownProfile(ProfileUid),
    /// f16 path requested while the `half` record is pending (paused-depth
    /// discipline; see `types::Depth`).
    DepthPaused(Depth),
    /// §6.3 [NORMATIVE]: `u16 → f16` is never performed implicitly; the
    /// explicit user path additionally needs the f16 machinery (paused).
    ImplicitU16ToF16,
    /// A capability that waits on a later slice was requested (today:
    /// only the §5.6 soft-proof chain, which lands with the display-LUT
    /// slice). Historical note: before `deps/littlecms2.yaml` landed this
    /// variant also gated every ICC-bytes-backed transform.
    IccBackendPending,
    /// §5.2 validation step 5 (lazy buildability): the profiles parsed but
    /// `cmsCreateTransform` (or profile/curve synthesis feeding it)
    /// refused — "a profile that parses but lacks required tags fails at
    /// `cmsCreateTransform` with a clean error". Core maps this to its
    /// §1.6 taxonomy and falls back to the working default with a warning.
    TransformBuild(&'static str),
    /// The slice-1 apply path moves f32 only (working precision, ES §6.1);
    /// integer/half marshalling arrives with the ICC backend.
    UnsupportedFormat(crate::types::PixelFormat),
    /// A transform between these spaces has no pure-matrix construction
    /// (e.g. anything touching CMYK, which MUST be profile-driven, §5.5).
    NoMatrixPath,
    /// Two distinct byte streams collided on one content identity —
    /// astronomically improbable; refused loudly rather than mis-keyed.
    IdentityCollision(ProfileUid),
    /// A math-entry parameter outside its §10.3 contract range (or a
    /// plane-length mismatch) reached the engine math (§14.4 apply
    /// shapes, coverage planes). Envelope validation is core's stage-3
    /// job and maps this to `error.schema.out_of_range`; this variant is
    /// the engine-side refusal so out-of-contract values can never run.
    ParamOutOfRange(&'static str),
    /// A contract-listed capability whose engine semantics the spec does
    /// not (yet) define — typed-reserved with the divergence recorded to
    /// the fold-in list (today: only `dissolve` in apply-image, §14.4 —
    /// see `apply_image::dissolve_reserved`). Distinct from
    /// out-of-range: the wire value IS valid; core maps this to its
    /// not-implemented taxonomy.
    Reserved(&'static str),
}

impl ColorError {
    /// The spec-named wire code, where one exists (ES §5.2). `None` means
    /// "engine-internal": core maps it at the command boundary.
    pub const fn wire_code(&self) -> Option<&'static str> {
        match self {
            ColorError::ProfileTooLarge { .. } => Some("error.color.profile_too_large"),
            ColorError::ProfileMalformed(_) => Some("error.color.profile_malformed"),
            ColorError::ProfileClass(_) => Some("error.color.profile_class"),
            ColorError::ProfileSpaceMismatch => Some("error.color.profile_space_mismatch"),
            _ => None,
        }
    }
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorError::ProfileTooLarge { len } => {
                write!(
                    f,
                    "ICC profile too large: {len} bytes exceeds the 16 MiB cap (ES §5.2)"
                )
            }
            ColorError::ProfileMalformed(what) => {
                write!(f, "ICC profile malformed: {what} (ES §5.2)")
            }
            ColorError::ProfileClass(c) => {
                write!(
                    f,
                    "ICC profile class {c:?} not usable as a document/working profile (ES §5.2)"
                )
            }
            ColorError::ProfileSpaceMismatch => {
                write!(
                    f,
                    "ICC profile data color space does not match the buffer model (ES §5.2)"
                )
            }
            ColorError::UnknownProfile(uid) => write!(f, "unknown profile {uid}"),
            ColorError::DepthPaused(d) => write!(
                f,
                "depth {} is paused pending the `half` crate deps/ record",
                d.as_wire()
            ),
            ColorError::ImplicitU16ToF16 => {
                write!(
                    f,
                    "u16 -> f16 is never implicit (ES §6.3); explicit path awaits the half record"
                )
            }
            ColorError::IccBackendPending => {
                write!(
                    f,
                    "requested color capability arrives with a later slice (soft-proof chain)"
                )
            }
            ColorError::TransformBuild(what) => {
                write!(f, "transform not buildable: {what} (ES §5.2 step 5)")
            }
            ColorError::UnsupportedFormat(fmt_) => {
                write!(f, "slice-1 apply moves f32 layouts only; got {fmt_}")
            }
            ColorError::NoMatrixPath => {
                write!(f, "no pure-matrix construction between these spaces (profile-driven only, ES §5.5)")
            }
            ColorError::IdentityCollision(uid) => {
                write!(
                    f,
                    "profile content-identity collision on {uid}; registration refused"
                )
            }
            ColorError::ParamOutOfRange(what) => {
                write!(f, "parameter outside its contract range: {what} (§10.3)")
            }
            ColorError::Reserved(what) => {
                write!(f, "typed-reserved capability: {what}")
            }
        }
    }
}

impl std::error::Error for ColorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_codes_match_spec_taxonomy() {
        assert_eq!(
            ColorError::ProfileTooLarge { len: 0 }.wire_code(),
            Some("error.color.profile_too_large")
        );
        assert_eq!(
            ColorError::ProfileMalformed("x").wire_code(),
            Some("error.color.profile_malformed")
        );
        assert_eq!(
            ColorError::ProfileClass(ProfileClass::DeviceLink).wire_code(),
            Some("error.color.profile_class")
        );
        assert_eq!(
            ColorError::ProfileSpaceMismatch.wire_code(),
            Some("error.color.profile_space_mismatch")
        );
        // Internal conditions carry no invented wire code.
        assert_eq!(ColorError::IccBackendPending.wire_code(), None);
        assert_eq!(ColorError::DepthPaused(Depth::F16).wire_code(), None);
        assert_eq!(ColorError::TransformBuild("x").wire_code(), None);
    }
}
