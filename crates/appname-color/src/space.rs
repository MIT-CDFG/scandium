// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.5 (working spaces and color math; gray working spaces),
//! §5.2 [LEGAL] (bundled-profile policy: the sRGB, Display P3, ProPhoto
//! RGB, Lab, and generic-CMYK bundle carries no third-party profile
//! bytes — definitions synthesized at runtime, screened per the census;
//! "Adobe RGB (1998)" is an optional **user-supplied** profile, never
//! bundled), Appendix B (the normative linear-RGB → XYZ matrices).
//!
//! Builtin working-space definitions. Naming discipline (plan §1 rule 1 /
//! D-43): identifiers are generic; the only third-party space name that
//! appears anywhere is the §5.2-sanctioned compatibility reference above,
//! and only in documentation. The "AdobeRGB-compatible" §2.3 row item is
//! therefore NOT a bundled matrix here — the spec's own tables carry no
//! constants for it and §5.2 forbids bundling it — it is a **user-supplied
//! wide-gamut RGB profile slot** ([`WorkingSpace::UserRgbProfile`]), whose
//! ICC path activates with the lcms2 record.
//!
//! Matrix provenance: every constant below is copied verbatim from
//! Appendix B. Composite matrices (RGB→XYZ(D50), RGB→RGB) are DERIVED at
//! runtime from the normative parts (§5.5: composites "are derived at
//! startup from these normative parts"). TRC provenance note: the ProPhoto
//! TRC is not tabulated in Appendix B; `transfer.rs` carries the public
//! ROMM (ISO 22028-2) curve and the lane log records this as a derivation
//! flagged for review.

use crate::chromatic::{bradford_adaptation, WhitePoint};
use crate::mat3::Mat3;
use crate::profile::ProfileUid;
use crate::transfer::TransferFn;

/// Appendix B, verbatim: linear sRGB / Rec.709 → XYZ (D65).
pub const SRGB_TO_XYZ_D65: Mat3 = Mat3::new([
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
]);

/// Appendix B, verbatim: linear Display P3 → XYZ (D65).
pub const DISPLAY_P3_TO_XYZ_D65: Mat3 = Mat3::new([
    [0.4865709, 0.2656677, 0.1982173],
    [0.2289746, 0.6917385, 0.0792869],
    [0.0000000, 0.0451134, 1.0439444],
]);

/// Appendix B, verbatim: linear ProPhoto RGB → XYZ (D50).
pub const PROPHOTO_TO_XYZ_D50: Mat3 = Mat3::new([
    [0.7976749, 0.1351917, 0.0313534],
    [0.2880402, 0.7118741, 0.0000857],
    [0.0000000, 0.0000000, 0.8252100],
]);

/// A bundled working space with spec-tabulated math (matrix/TRC class —
/// exactly the §5.4 "shaper/matrix fast path" family).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum BuiltinSpace {
    /// sRGB (IEC 61966-2-1), D65 primaries, piecewise TRC. The default
    /// working space and the untagged-import assumption (§5.1).
    Srgb,
    /// Display P3, D65 primaries, sRGB TRC (the P3 display space uses the
    /// sRGB transfer).
    DisplayP3,
    /// ProPhoto RGB (ROMM), D50 primaries.
    ProPhotoRgb,
    /// Gray Gamma 2.2 — §5.5 gray spaces: "pure power 2.2 TRC, D50 white;
    /// the default for new Gray-mode documents and the assumed space for
    /// untagged grayscale imports."
    GrayGamma22,
    /// sGray — §5.5: "the sRGB piecewise transfer (IEC 61966-2-1) as a gray
    /// TRC, D50 white."
    SGray,
}

impl BuiltinSpace {
    /// All builtins, in registry order (deterministic — this order is part
    /// of the crate's stable behavior).
    pub const ALL: [BuiltinSpace; 5] = [
        BuiltinSpace::Srgb,
        BuiltinSpace::DisplayP3,
        BuiltinSpace::ProPhotoRgb,
        BuiltinSpace::GrayGamma22,
        BuiltinSpace::SGray,
    ];

    /// Stable machine tag. Feeds the content-derived builtin
    /// [`ProfileUid`]s and (later) the package-serialized space names —
    /// generic strings, no third-party marks (D-43).
    pub const fn tag(self) -> &'static str {
        match self {
            BuiltinSpace::Srgb => "builtin:srgb",
            BuiltinSpace::DisplayP3 => "builtin:display-p3",
            BuiltinSpace::ProPhotoRgb => "builtin:prophoto-rgb",
            BuiltinSpace::GrayGamma22 => "builtin:gray-gamma-2.2",
            BuiltinSpace::SGray => "builtin:sgray",
        }
    }

    /// Human-readable description (UI strings go through i18n; this is the
    /// registry `desc` per the §5.1 `IccProfile.desc` shape).
    pub const fn desc(self) -> &'static str {
        match self {
            BuiltinSpace::Srgb => "sRGB (built-in)",
            BuiltinSpace::DisplayP3 => "Display P3 (built-in)",
            BuiltinSpace::ProPhotoRgb => "ProPhoto RGB (built-in)",
            BuiltinSpace::GrayGamma22 => "Gray Gamma 2.2 (built-in)",
            BuiltinSpace::SGray => "sGray (built-in)",
        }
    }

    pub const fn is_gray(self) -> bool {
        matches!(self, BuiltinSpace::GrayGamma22 | BuiltinSpace::SGray)
    }

    pub const fn transfer(self) -> TransferFn {
        match self {
            BuiltinSpace::Srgb | BuiltinSpace::DisplayP3 | BuiltinSpace::SGray => {
                TransferFn::SrgbPiecewise
            }
            BuiltinSpace::ProPhotoRgb => TransferFn::ProPhoto,
            BuiltinSpace::GrayGamma22 => TransferFn::Gamma22,
        }
    }

    /// Native white point of the primaries (gray spaces: §5.5 fixes D50).
    pub const fn white(self) -> WhitePoint {
        match self {
            BuiltinSpace::Srgb | BuiltinSpace::DisplayP3 => WhitePoint::D65,
            BuiltinSpace::ProPhotoRgb | BuiltinSpace::GrayGamma22 | BuiltinSpace::SGray => {
                WhitePoint::D50
            }
        }
    }

    /// Linear RGB → XYZ at the space's NATIVE white (Appendix B constant).
    /// `None` for gray spaces (one channel; they convert through Y, §5.5).
    pub const fn rgb_to_xyz_native(self) -> Option<Mat3> {
        match self {
            BuiltinSpace::Srgb => Some(SRGB_TO_XYZ_D65),
            BuiltinSpace::DisplayP3 => Some(DISPLAY_P3_TO_XYZ_D65),
            BuiltinSpace::ProPhotoRgb => Some(PROPHOTO_TO_XYZ_D50),
            BuiltinSpace::GrayGamma22 | BuiltinSpace::SGray => None,
        }
    }

    /// Linear RGB → XYZ(D50) — the ICC connection space (§5.5: "model
    /// conversions go through CIE XYZ (D50 connection space, matching
    /// ICC)"). Derived: Bradford-adapts the native-white matrix when the
    /// native white is not D50.
    pub fn rgb_to_xyz_d50(self) -> Option<Mat3> {
        let native = self.rgb_to_xyz_native()?;
        Some(match self.white() {
            WhitePoint::D50 => native,
            w => bradford_adaptation(w, WhitePoint::D50).mul(&native),
        })
    }

    /// XYZ(D50) → linear RGB: the computed inverse (§5.5 inverse rule).
    pub fn xyz_d50_to_rgb(self) -> Option<Mat3> {
        self.rgb_to_xyz_d50()?.inverse()
    }

    /// The D50-adapted luminance row (Y of `rgb_to_xyz_d50`): the §5.5 /
    /// §6.2 "keep Y" vector for RGB → Gray. `None` for gray spaces.
    pub fn luminance_row_d50(self) -> Option<[f64; 3]> {
        Some(self.rgb_to_xyz_d50()?.m[1])
    }
}

/// Composite `linear src RGB → linear dst RGB` through XYZ(D50):
/// `M = M_dst⁻¹ · M_src` (both already Bradford-adapted to D50 — the §5.4
/// shaper/matrix fast-path shape `M_dst⁻¹ · M_adapt · M_src`).
/// `None` when either side is a gray space (gray goes through Y instead).
pub fn rgb_to_rgb_matrix(src: BuiltinSpace, dst: BuiltinSpace) -> Option<Mat3> {
    let m_src = src.rgb_to_xyz_d50()?;
    let m_dst_inv = dst.xyz_d50_to_rgb()?;
    Some(m_dst_inv.mul(&m_src))
}

/// A document/buffer working space as this crate models it (ES §5.1: the
/// profile identity a `Buffer`/`ColorState` points at resolves to one of
/// these through the registry).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum WorkingSpace {
    /// A bundled matrix/TRC space with spec-tabulated math.
    Builtin(BuiltinSpace),
    /// A user-supplied RGB profile (§5.2 [LEGAL]: e.g. an "Adobe RGB
    /// (1998)"-compatible profile the user installs — never bundled).
    /// Transforms for this arm are ICC-driven and await the lcms2 record.
    UserRgbProfile(ProfileUid),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chromatic::WhitePoint;

    /// Matrix-oracle derivation 1: each Appendix B matrix must carry its
    /// own white — the rows of linear-RGB→XYZ sum to the white point's XYZ
    /// (R=G=B=1 is white by definition). Tolerance 5e-4: the published
    /// P3 constants were evaluated against the chromaticity-derived D65
    /// (0.9504559, 1, 1.0890578), which differs from the §5.5 five-decimal
    /// D65 in the 4th decimal of Z; sRGB and ProPhoto agree to ~1e-7.
    #[test]
    fn matrix_rows_sum_to_the_native_white() {
        for (m, w) in [
            (SRGB_TO_XYZ_D65, WhitePoint::D65),
            (DISPLAY_P3_TO_XYZ_D65, WhitePoint::D65),
            (PROPHOTO_TO_XYZ_D50, WhitePoint::D50),
        ] {
            let white = w.xyz();
            for (r, want) in white.iter().enumerate() {
                let sum = m.m[r][0] + m.m[r][1] + m.m[r][2];
                assert!((sum - want).abs() < 5e-4, "row {r}: {sum} vs {want}");
            }
        }
    }

    /// Matrix-oracle derivation 2: primary columns. Unit linear red in sRGB
    /// is the matrix's first column (§5.5 constants, exact).
    #[test]
    fn srgb_red_primary_is_column_zero() {
        let xyz = SRGB_TO_XYZ_D65.mul_vec([1.0, 0.0, 0.0]);
        assert_eq!(xyz, [0.4124564, 0.2126729, 0.0193339]);
    }

    /// D50-adapted sRGB white must land exactly on D50 (the §5.5 composite
    /// `M_Bradford · M_sRGB→XYZ(D65)` carries sRGB white to PCS white).
    #[test]
    fn adapted_srgb_white_lands_on_d50() {
        let m = BuiltinSpace::Srgb.rgb_to_xyz_d50().unwrap();
        let white = m.mul_vec([1.0, 1.0, 1.0]);
        let d50 = WhitePoint::D50.xyz();
        for i in 0..3 {
            // Bounded by the row-sum provenance slack above + Bradford
            // rounding; sRGB agrees to ~1e-6.
            assert!((white[i] - d50[i]).abs() < 1e-4, "{:?}", white);
        }
    }

    /// ProPhoto is already D50: adapted == native, bit-for-bit.
    #[test]
    fn prophoto_needs_no_adaptation() {
        assert_eq!(
            BuiltinSpace::ProPhotoRgb.rgb_to_xyz_d50().unwrap(),
            PROPHOTO_TO_XYZ_D50
        );
    }

    /// RGB→RGB composite round trip: sRGB → P3 → sRGB is identity within
    /// f64 matrix conditioning.
    #[test]
    fn srgb_p3_matrix_round_trip() {
        let fwd = rgb_to_rgb_matrix(BuiltinSpace::Srgb, BuiltinSpace::DisplayP3).unwrap();
        let back = rgb_to_rgb_matrix(BuiltinSpace::DisplayP3, BuiltinSpace::Srgb).unwrap();
        let rt = back.mul(&fwd);
        assert!(rt.max_abs_diff(&Mat3::IDENTITY) < 1e-10, "{rt:?}");
    }

    /// Same-space composite is identity.
    #[test]
    fn same_space_matrix_is_identity() {
        let m = rgb_to_rgb_matrix(BuiltinSpace::Srgb, BuiltinSpace::Srgb).unwrap();
        assert!(m.max_abs_diff(&Mat3::IDENTITY) < 1e-12);
    }

    /// Gray spaces expose no RGB matrix (they convert through Y, §5.5).
    #[test]
    fn gray_spaces_have_no_matrix() {
        for g in [BuiltinSpace::GrayGamma22, BuiltinSpace::SGray] {
            assert!(g.is_gray());
            assert!(g.rgb_to_xyz_native().is_none());
            assert!(g.rgb_to_xyz_d50().is_none());
            assert!(rgb_to_rgb_matrix(g, BuiltinSpace::Srgb).is_none());
        }
    }

    /// The luminance row is the Y row and sums to 1 at white.
    #[test]
    fn luminance_row_sums_to_one() {
        for s in [
            BuiltinSpace::Srgb,
            BuiltinSpace::DisplayP3,
            BuiltinSpace::ProPhotoRgb,
        ] {
            let row = s.luminance_row_d50().unwrap();
            let sum = row[0] + row[1] + row[2];
            assert!((sum - 1.0).abs() < 5e-4, "{s:?}: {sum}");
        }
    }

    /// Registry-order and tag stability (deterministic vocabulary).
    #[test]
    fn builtin_tags_are_stable() {
        let tags: Vec<&str> = BuiltinSpace::ALL.iter().map(|s| s.tag()).collect();
        assert_eq!(
            tags,
            [
                "builtin:srgb",
                "builtin:display-p3",
                "builtin:prophoto-rgb",
                "builtin:gray-gamma-2.2",
                "builtin:sgray"
            ]
        );
    }
}
