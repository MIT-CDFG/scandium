// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-color` — Appname color management: the CMM core (CP-0.5)
//! plus the CP-1.0 channels/mode-convert row, color-side.
//!
//! Spec: ES §5 (color management — §5.1 profile-carrying pipeline, §5.2
//! CMM integration + profile validation, §5.3 assign ≠ convert, §5.4
//! transform cache + 3D LUTs, §5.5 working spaces and color math, §5.6
//! soft proofing), ES §6 (depth, modes, premultiplied rules), ES §14
//! (channels — the §14.4 apply-image/calculations math, §14.1/§14.2
//! display + persistence math), ES §29.3 (print pipeline v1, color
//! side), Appendix B (normative matrices and transfer functions); DD
//! ⑦㉗㉞. Acceptance surface: ES §5.8 and workflow W04 (+ W11 print at
//! the QA harness).
//!
//! ## What is here now
//! The CP-0.5 CMM core, complete and tested:
//! * working-space definitions from the spec's own tables ([`space`]),
//! * the Bradford adaptation spine ([`chromatic`]),
//! * transfer curves ([`transfer`]) and Lab math ([`lab`]),
//! * depth-convert scaffolding ([`depth`]; f16 paused until core
//!   activates its F16 channels — engine-wide depth discipline),
//! * the content-keyed profile registry with §5.2 header-level
//!   validation ([`profile`]) + the deep CMM parse at the
//!   [`transform::Cms::open_profile`] ingress,
//! * the transform machinery + §5.4 cache ([`transform`]): the pure
//!   matrix/TRC fast path for builtin pairs, §5.8-test-2 null-transform
//!   detection, and the **lcms2-backed ICC path** (`icc` module behind
//!   the facade; deps record `deps/littlecms2.yaml`; determinism flag
//!   elections documented there).
//!
//! And the CP-1.0 channels/mode-convert row (plan §2.5), color-side:
//! * **CMYK/Lab CMS paths** — the §5.2 F32 format table made total
//!   (`K(1,4)` CmykA, float Lab/LabA), the normalized↔real Lab/CMYK
//!   boundary marshalling, and the builtin CIE Lab connection-space
//!   profile ([`Cms::builtin_lab`]);
//! * **`doc.mode_convert` machinery** ([`mode_convert`]): the §6.2
//!   [NORMATIVE] conversion matrix as an executable plan (lossiness +
//!   warning MessageIds), the mode vocabulary, and the §5.5 CMYK
//!   fallback formula (last resort, warning-bound, untagged output);
//! * **§14.4 apply-image / calculations math** ([`apply_image`]): the
//!   exact conditioning/lerp/mask semantics as pure planar functions,
//!   §4.5 kernels injected (one math library — render's);
//! * **soft proof §5.6 + display LUTs §5.4** ([`proof`]): the rgba proof
//!   LUT with the normative ΔE76 > 2.0 gamut flag, paper-white
//!   simulation, the 33³ bake loop verbatim, and the trilinear CPU
//!   reference sampler — the display-chain integration point for O-UI;
//! * **print pipeline v1 color-side §29.3** ([`print`]): the step-4
//!   working→printer strip conversion, the [PERF] strip-rows formula,
//!   and the preview-is-the-soft-proof tie-in;
//! * **channels groundwork §14.1–§14.3** ([`channels`]): colorized
//!   component views, the rubylith overlay, the §14.2 8-bit
//!   lossless-or-not-at-all predicate, spot display props.
//!
//! Channel STORAGE, the kind arms (`doc.mode_convert`, `channel.*`,
//! `selection.save/load`), and tile marshalling stay core's fence — the
//! precise door asks are filed in `reports/O-COLOR-log.md`. Still ahead:
//! CIEDE2000 (acceptance-suite slice), per-monitor acquisition (§5.7 —
//! consumer-side), non-RGB-working proof lattices (posture in
//! [`proof`]).
//!
//! ## Position in the crate DAG
//! ES §1.4 rule 4: `appname-core` depends on the domain crates, so this
//! crate sees no `appname-*` type. Core's `ColorState`/`ProfileId`
//! (its fence) map onto this crate's [`profile::ProfileUid`] and
//! [`space::WorkingSpace`] at the core boundary; the precise field/hook
//! needs are specified in `reports/O-COLOR-log.md` for O-INT brokering.
//! When the lcms2 record lands, the `*-sys` edge lives under this facade
//! (ES §1.4 rule 1) — this crate itself stays safe Rust, permanently.
//!
//! ## Determinism (D-48)
//! Same inputs → same bytes, always: fixed-order f64 math narrowed to f32
//! once per pixel, `BTreeMap`-keyed caches (no hash-seed or insertion
//! order anywhere), content-derived profile identity, and a byte-copy
//! identity path. The lcms2 flag elections that preserve this at the ICC
//! boundary (`cmsFLAGS_NOCACHE`, `cmsFLAGS_COPY_ALPHA`) are recorded in
//! [`transform`]'s module docs ahead of the backend landing.

#![forbid(unsafe_code)]

pub mod apply_image;
pub mod channels;
pub mod chromatic;
pub mod depth;
pub mod error;
mod icc; // lcms2 backend behind the Cms facade (ES §1.4 rule 1)
pub mod lab;
pub mod mat3;
pub mod mode_convert;
pub mod print;
pub mod profile;
pub mod proof;
pub mod space;
pub mod transfer;
pub mod transform;
pub mod types;

pub use apply_image::{
    apply_add_plane, apply_blend_plane, apply_blend_planes3, apply_subtract_plane,
    dissolve_reserved, ApplyShape, CoverageArm,
};
pub use channels::{
    colorize_multi, plane_is_u8_exact, rubylith_overlay, ComponentChannel, SpotDisplay,
    RUBYLITH_DEFAULT_INK, RUBYLITH_DEFAULT_SOLIDITY,
};
pub use chromatic::{
    bradford_adaptation, bradford_d50_to_d65, WhitePoint, BRADFORD_D65_TO_D50, BRADFORD_MA,
};
pub use depth::{
    classify_depth_convert, demote_f32_to_u16, demote_f32_to_u8, demote_u16_to_u8,
    promote_u16_to_f32, promote_u8_to_f32, promote_u8_to_u16, DepthConvertKind, DitherSource,
    NoDither,
};
pub use error::ColorError;
pub use lab::{delta_e76, lab_from_normalized, lab_to_normalized, lab_to_xyz, xyz_to_lab};
pub use mat3::Mat3;
pub use mode_convert::{
    cmyk_fallback_compose, cmyk_fallback_compose_px, cmyk_fallback_separate,
    cmyk_fallback_separate_px, plan_mode_convert, DocMode, ModeConvertPlan,
};
pub use print::{
    convert_print_strip, print_transform, strip_rows, PrintColorParams, STRIP_BUDGET_BYTES,
    WORKING_BYTES_PER_PX,
};
pub use profile::{
    parse_icc_header, DataColorSpace, IccHeader, ProfileClass, ProfileEntry, ProfileRegistry,
    ProfileSource, ProfileUid, PROFILE_SIZE_CAP,
};
pub use proof::{
    bake_display_lut, bake_proof_lut, gamut_flagged, sample_lut3d_rgba, DisplayLutData, ProofLut,
    SoftProof, GAMUT_WARNING_THRESHOLD_DE76, LUT_SIZE,
};
pub use space::{rgb_to_rgb_matrix, BuiltinSpace, WorkingSpace};
pub use transfer::TransferFn;
pub use transform::{Cms, Transform, TransformKey};
pub use types::{ColorModel, Depth, Intent, PixelFormat};

/// Crate name marker exercised by the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-color";

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-color");
    }
}
