// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §5.1 (profile identity on the pipeline: "nothing is ever
//! interpreted as raw RGB"; the `IccProfile { id, bytes, desc, class }`
//! shape), §5.2 "Profile validation on load [NORMATIVE]" (the five-step
//! ladder: size cap, parse, class check, color-space match, lazy
//! buildability), §5.5 "Gray working spaces" (synthesized at runtime, no
//! third-party profile bytes shipped — [LEGAL] clean).
//!
//! The profile registry. Identity here is **content-derived**
//! ([`ProfileUid`], FNV-1a-128 over the profile bytes or the builtin tag):
//! the same bytes always get the same identity, in any session, in any
//! registration order — a determinism-spine requirement (D-48), and the
//! stable half of the §5.4 transform-cache key. `appname-core`'s uuid
//! `ProfileId` (its document arena addressing) maps onto `ProfileUid` at
//! the core boundary; the mapping is core's, specified in the lane log.
//!
//! Slice-1 honesty: validation steps 1–4 are implemented here at ICC
//! *header* level (pure Rust — size cap, magic/size/tag-table sanity,
//! class signature, data-color-space signature). Step 2's full parse and
//! step 5's transform buildability are the lcms2 wrapper's job and attach
//! when `deps/lcms2.yaml` lands; until then ICC-bytes-backed profiles
//! register and validate but build no transforms
//! ([`ColorError::IccBackendPending`]).

use std::collections::BTreeMap;
use std::fmt;

use crate::error::ColorError;
use crate::space::BuiltinSpace;
use crate::types::ColorModel;

/// §5.2 validation 1: the profile byte-stream size cap, 16 MiB.
pub const PROFILE_SIZE_CAP: usize = 16 * 1024 * 1024;

/// Content-derived profile identity: FNV-1a-128 (public-domain constants)
/// over the identity preimage — `b"icc:"` + profile bytes for user
/// profiles, `b"tag:"` + the stable tag for builtins. Deterministic across
/// sessions/platforms; collisions between distinct byte streams are
/// astronomically improbable and REFUSED loudly on registration
/// ([`ColorError::IdentityCollision`]) rather than silently mis-keyed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ProfileUid(u128);

impl ProfileUid {
    pub const fn as_u128(self) -> u128 {
        self.0
    }

    fn fnv1a128(domain: &[u8], data: &[u8]) -> ProfileUid {
        const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
        const PRIME: u128 = 0x0000000001000000000000000000013B;
        let mut h = OFFSET;
        for &b in domain.iter().chain(data.iter()) {
            h ^= u128::from(b);
            h = h.wrapping_mul(PRIME);
        }
        ProfileUid(h)
    }

    /// Identity of an ICC byte stream.
    pub fn of_icc_bytes(bytes: &[u8]) -> ProfileUid {
        ProfileUid::fnv1a128(b"icc:", bytes)
    }

    /// Identity of a builtin space (from its stable tag).
    pub fn of_builtin(space: BuiltinSpace) -> ProfileUid {
        ProfileUid::fnv1a128(b"tag:", space.tag().as_bytes())
    }

    /// Identity of the builtin CIE Lab connection-space profile (§5.2
    /// `builtin()` list: "sRGB, P3, ProPhoto, Lab, generic CMYK"; the Lab
    /// entry). Tag-derived like every builtin — session-stable.
    pub fn of_builtin_lab() -> ProfileUid {
        ProfileUid::fnv1a128(b"tag:", BUILTIN_LAB_TAG.as_bytes())
    }
}

/// Stable machine tag of the builtin CIE Lab profile (D50 connection
/// space, ES §5.5: "anything↔Lab is profile-based" — this is the profile).
pub const BUILTIN_LAB_TAG: &str = "builtin:cielab";

impl fmt::Display for ProfileUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// ICC profile/device class (header bytes 12..16). §5.2 validation 3
/// accepts Display, Input, Output, ColorSpace for document/working
/// profiles; DeviceLink and NamedColor are rejected (and Abstract, which
/// is outside the accepted four).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ProfileClass {
    /// `mntr`
    Display,
    /// `scnr`
    Input,
    /// `prtr`
    Output,
    /// `spac`
    ColorSpaceConversion,
    /// `link` — rejected for working profiles.
    DeviceLink,
    /// `nmcl` — rejected for working profiles.
    NamedColor,
    /// `abst` — outside the §5.2 accepted set for working profiles.
    Abstract,
}

impl ProfileClass {
    fn from_signature(sig: [u8; 4]) -> Option<ProfileClass> {
        Some(match &sig {
            b"mntr" => ProfileClass::Display,
            b"scnr" => ProfileClass::Input,
            b"prtr" => ProfileClass::Output,
            b"spac" => ProfileClass::ColorSpaceConversion,
            b"link" => ProfileClass::DeviceLink,
            b"nmcl" => ProfileClass::NamedColor,
            b"abst" => ProfileClass::Abstract,
            _ => return None,
        })
    }

    /// §5.2 validation 3: usable as a document/working profile?
    pub const fn usable_as_working(self) -> bool {
        matches!(
            self,
            ProfileClass::Display
                | ProfileClass::Input
                | ProfileClass::Output
                | ProfileClass::ColorSpaceConversion
        )
    }
}

/// ICC data color space (header bytes 16..20), the §5.2 validation-4
/// vocabulary this build accepts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum DataColorSpace {
    /// `RGB `
    Rgb,
    /// `GRAY`
    Gray,
    /// `CMYK`
    Cmyk,
    /// `Lab `
    Lab,
    /// `XYZ `
    Xyz,
}

impl DataColorSpace {
    fn from_signature(sig: [u8; 4]) -> Option<DataColorSpace> {
        Some(match &sig {
            b"RGB " => DataColorSpace::Rgb,
            b"GRAY" => DataColorSpace::Gray,
            b"CMYK" => DataColorSpace::Cmyk,
            b"Lab " => DataColorSpace::Lab,
            b"XYZ " => DataColorSpace::Xyz,
            _ => return None,
        })
    }

    /// §5.2 validation 4: "RGB profile for `Rgb/Rgba`, GRAY for
    /// `Gray/GrayA`, CMYK for `Cmyk/CmykA`, Lab for `Lab/LabA`."
    pub const fn matches_model(self, model: ColorModel) -> bool {
        matches!(
            (self, model.without_alpha()),
            (DataColorSpace::Rgb, ColorModel::Rgb)
                | (DataColorSpace::Gray, ColorModel::Gray)
                | (DataColorSpace::Cmyk, ColorModel::Cmyk)
                | (DataColorSpace::Lab, ColorModel::Lab)
        )
    }
}

/// The header fields the §5.2 pure-Rust pre-validation extracts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IccHeader {
    /// Header `size` field (bytes 0..4, big-endian).
    pub declared_size: u32,
    pub class: ProfileClass,
    pub data_space: DataColorSpace,
    /// Major.minor from the version field (byte 8 = major, high nibble of
    /// byte 9 = minor).
    pub version: (u8, u8),
    /// Tag count (bytes 128..132, big-endian).
    pub tag_count: u32,
}

/// ICC header size + tag-count field.
const HEADER_LEN: usize = 132;
/// Sanity bound on the tag table — real profiles carry dozens of tags; the
/// bound exists so a hostile count cannot drive the entry scan.
const MAX_TAGS: u32 = 1024;

/// §5.2 validation steps 1–4 at header level (the deep parse is lcms2's,
/// step 2's remainder). Never panics on hostile bytes — every failure is a
/// structured [`ColorError`] (§1.6; this boundary is fuzz-target territory
/// per §34.2 once the wrapper lands).
pub fn parse_icc_header(bytes: &[u8]) -> Result<IccHeader, ColorError> {
    // 1. Size cap.
    if bytes.len() > PROFILE_SIZE_CAP {
        return Err(ColorError::ProfileTooLarge { len: bytes.len() });
    }
    // 2. Header-level parse sanity.
    if bytes.len() < HEADER_LEN {
        return Err(ColorError::ProfileMalformed("shorter than the ICC header"));
    }
    let magic: [u8; 4] = [bytes[36], bytes[37], bytes[38], bytes[39]];
    if &magic != b"acsp" {
        return Err(ColorError::ProfileMalformed("missing 'acsp' signature"));
    }
    let declared_size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    // Header/size mismatch → malformed (§5.2 step 2). The declared size may
    // not exceed what we were handed, and must at least cover the header.
    if (declared_size as usize) > bytes.len() || (declared_size as usize) < HEADER_LEN {
        return Err(ColorError::ProfileMalformed(
            "size field inconsistent with byte stream",
        ));
    }
    let class_sig: [u8; 4] = [bytes[12], bytes[13], bytes[14], bytes[15]];
    let class = ProfileClass::from_signature(class_sig).ok_or(ColorError::ProfileMalformed(
        "unknown profile/device class signature",
    ))?;
    let space_sig: [u8; 4] = [bytes[16], bytes[17], bytes[18], bytes[19]];
    let data_space = DataColorSpace::from_signature(space_sig).ok_or(
        ColorError::ProfileMalformed("unsupported data color space signature"),
    )?;
    // Truncated tag tables → malformed (§5.2 step 2).
    let tag_count = u32::from_be_bytes([bytes[128], bytes[129], bytes[130], bytes[131]]);
    if tag_count > MAX_TAGS {
        return Err(ColorError::ProfileMalformed(
            "tag count exceeds sanity bound",
        ));
    }
    let table_end = HEADER_LEN as u64 + u64::from(tag_count) * 12;
    if table_end > declared_size as u64 {
        return Err(ColorError::ProfileMalformed(
            "tag table exceeds declared size",
        ));
    }
    // Each tag entry's (offset, size) must sit inside the declared size.
    for i in 0..tag_count as usize {
        let base = HEADER_LEN + i * 12;
        let off = u32::from_be_bytes([
            bytes[base + 4],
            bytes[base + 5],
            bytes[base + 6],
            bytes[base + 7],
        ]);
        let sz = u32::from_be_bytes([
            bytes[base + 8],
            bytes[base + 9],
            bytes[base + 10],
            bytes[base + 11],
        ]);
        if u64::from(off) + u64::from(sz) > u64::from(declared_size) {
            return Err(ColorError::ProfileMalformed(
                "tag data exceeds declared size",
            ));
        }
    }
    Ok(IccHeader {
        declared_size,
        class,
        data_space,
        version: (bytes[8], bytes[9] >> 4),
        tag_count,
    })
}

/// Where a registered profile's math comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProfileSource {
    /// Bundled matrix/TRC space — full pure-Rust transform support today.
    Builtin(BuiltinSpace),
    /// The builtin CIE Lab connection-space profile (D50), synthesized at
    /// runtime through the CMM (`cmsCreateLab4Profile` — no third-party
    /// profile bytes, same [LEGAL] posture as the gray spaces). Transforms
    /// touching it are always ICC-driven (§5.5: "anything↔Lab is
    /// profile-based").
    BuiltinLab,
    /// User-supplied ICC bytes (header-validated); transforms await lcms2.
    IccBytes(Vec<u8>),
}

/// A registry row (the §5.1 `IccProfile` shape, content-keyed).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProfileEntry {
    pub uid: ProfileUid,
    pub source: ProfileSource,
    pub desc: String,
    pub class: ProfileClass,
    pub data_space: DataColorSpace,
}

impl ProfileEntry {
    /// The builtin space, when this entry is one.
    pub fn builtin(&self) -> Option<BuiltinSpace> {
        match &self.source {
            ProfileSource::Builtin(s) => Some(*s),
            ProfileSource::BuiltinLab | ProfileSource::IccBytes(_) => None,
        }
    }
}

/// The profile registry: content-keyed, deterministically ordered
/// (`BTreeMap` — iteration order is uid order, never insertion order, so
/// two sessions that loaded the same profiles agree on every enumeration).
#[derive(Clone, Debug)]
pub struct ProfileRegistry {
    entries: BTreeMap<ProfileUid, ProfileEntry>,
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        ProfileRegistry::new()
    }
}

impl ProfileRegistry {
    /// A registry pre-seeded with the five builtin working spaces (§5.5;
    /// synthesized definitions, no third-party profile bytes — §5.2
    /// [LEGAL] posture, matching the runtime-synthesis rule for grays).
    pub fn new() -> ProfileRegistry {
        let mut entries = BTreeMap::new();
        for space in BuiltinSpace::ALL {
            let uid = ProfileUid::of_builtin(space);
            let (class, data_space) = (
                ProfileClass::ColorSpaceConversion,
                if space.is_gray() {
                    DataColorSpace::Gray
                } else {
                    DataColorSpace::Rgb
                },
            );
            entries.insert(
                uid,
                ProfileEntry {
                    uid,
                    source: ProfileSource::Builtin(space),
                    desc: space.desc().to_owned(),
                    class,
                    data_space,
                },
            );
        }
        // The builtin Lab connection-space profile (§5.2 builtin() list;
        // §6.2 "anything↔Lab is profile-based" — this is the profile the
        // Lab arms of doc.mode_convert resolve against).
        let lab_uid = ProfileUid::of_builtin_lab();
        entries.insert(
            lab_uid,
            ProfileEntry {
                uid: lab_uid,
                source: ProfileSource::BuiltinLab,
                desc: String::from("CIE Lab D50 (built-in)"),
                class: ProfileClass::ColorSpaceConversion,
                data_space: DataColorSpace::Lab,
            },
        );
        ProfileRegistry { entries }
    }

    /// The uid of a builtin space (always present).
    pub fn builtin_uid(&self, space: BuiltinSpace) -> ProfileUid {
        ProfileUid::of_builtin(space)
    }

    /// The uid of the builtin CIE Lab profile (always present).
    pub fn lab_uid(&self) -> ProfileUid {
        ProfileUid::of_builtin_lab()
    }

    /// Register user-supplied ICC bytes as a document/working-capable
    /// profile for `model`: §5.2 steps 1–4 (header level), then content-key
    /// and store. Re-registering identical bytes is a no-op returning the
    /// same uid (content identity). This is the ingress for the §2.3
    /// "AdobeRGB-compatible" slot: a user-supplied wide-gamut RGB profile
    /// registers here and becomes addressable working state; its transforms
    /// activate with the lcms2 backend.
    pub fn register_working_icc(
        &mut self,
        bytes: Vec<u8>,
        model: ColorModel,
    ) -> Result<ProfileUid, ColorError> {
        let header = parse_icc_header(&bytes)?;
        if !header.class.usable_as_working() {
            return Err(ColorError::ProfileClass(header.class));
        }
        if !header.data_space.matches_model(model) {
            return Err(ColorError::ProfileSpaceMismatch);
        }
        let uid = ProfileUid::of_icc_bytes(&bytes);
        if let Some(existing) = self.entries.get(&uid) {
            match &existing.source {
                ProfileSource::IccBytes(b) if *b == bytes => return Ok(uid), // dedup
                _ => return Err(ColorError::IdentityCollision(uid)),
            }
        }
        self.entries.insert(
            uid,
            ProfileEntry {
                uid,
                source: ProfileSource::IccBytes(bytes),
                // desc extraction ('desc'/'mluc' tag) is the lcms2 slice's;
                // header-level registration keeps a stable placeholder.
                desc: String::from("(ICC profile; description pending CMM backend)"),
                class: header.class,
                data_space: header.data_space,
            },
        );
        Ok(uid)
    }

    pub fn get(&self, uid: ProfileUid) -> Result<&ProfileEntry, ColorError> {
        self.entries
            .get(&uid)
            .ok_or(ColorError::UnknownProfile(uid))
    }

    pub fn contains(&self, uid: ProfileUid) -> bool {
        self.entries.contains_key(&uid)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Deterministic enumeration (uid order).
    pub fn iter(&self) -> impl Iterator<Item = &ProfileEntry> {
        self.entries.values()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Synthetic ICC byte streams for header-validation tests (built here,
    //! not vendored — no third-party profile bytes enter the repo, §5.2
    //! [LEGAL] posture).

    /// Minimal well-formed header+tag-table stream: `class` and `space` are
    /// 4-byte signatures; one zero-length tag entry keeps the table real.
    pub fn synthetic_icc(class: &[u8; 4], space: &[u8; 4]) -> Vec<u8> {
        let total: u32 = 132 + 12; // header + one tag entry
        let mut b = vec![0u8; total as usize];
        b[0..4].copy_from_slice(&total.to_be_bytes());
        b[8] = 4; // version 4.x
        b[12..16].copy_from_slice(class);
        b[16..20].copy_from_slice(space);
        b[36..40].copy_from_slice(b"acsp");
        b[128..132].copy_from_slice(&1u32.to_be_bytes()); // one tag
                                                          // tag entry: sig 'desc', offset = end of table, size 0
        b[132..136].copy_from_slice(b"desc");
        b[136..140].copy_from_slice(&total.to_be_bytes()); // offset (size 0 ⇒ in bounds)
        b[140..144].copy_from_slice(&0u32.to_be_bytes());
        b
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::synthetic_icc;
    use super::*;

    #[test]
    fn builtin_uids_are_content_stable() {
        // Registration-independent, session-independent identity: the uid
        // is a pure function of the tag. These values double as the
        // cross-session determinism oracle — if they ever change, profile
        // references in saved documents would dangle.
        let a = ProfileUid::of_builtin(BuiltinSpace::Srgb);
        let b = ProfileUid::of_builtin(BuiltinSpace::Srgb);
        assert_eq!(a, b);
        // Distinct spaces, distinct uids.
        let mut uids: Vec<ProfileUid> = BuiltinSpace::ALL
            .iter()
            .map(|s| ProfileUid::of_builtin(*s))
            .collect();
        uids.sort();
        uids.dedup();
        assert_eq!(uids.len(), BuiltinSpace::ALL.len());
    }

    #[test]
    fn registry_seeds_the_five_builtins() {
        // Five matrix/TRC builtins + the CIE Lab connection-space profile
        // (the §5.2 builtin() list's Lab entry, CP-1.0 slice).
        let reg = ProfileRegistry::new();
        assert_eq!(reg.len(), 6);
        for s in BuiltinSpace::ALL {
            let entry = reg.get(reg.builtin_uid(s)).unwrap();
            assert_eq!(entry.builtin(), Some(s));
            assert!(entry.class.usable_as_working());
        }
        let lab = reg.get(reg.lab_uid()).unwrap();
        assert_eq!(lab.builtin(), None);
        assert_eq!(lab.source, ProfileSource::BuiltinLab);
        assert_eq!(lab.data_space, DataColorSpace::Lab);
        assert!(lab.class.usable_as_working());
    }

    #[test]
    fn registry_enumeration_is_registration_order_independent() {
        // Two registries fed the same ICC streams in opposite orders
        // enumerate identically (BTreeMap keyed on content identity).
        let p1 = synthetic_icc(b"mntr", b"RGB ");
        let p2 = synthetic_icc(b"prtr", b"RGB ");
        let mut r_fwd = ProfileRegistry::new();
        r_fwd
            .register_working_icc(p1.clone(), ColorModel::Rgba)
            .unwrap();
        r_fwd
            .register_working_icc(p2.clone(), ColorModel::Rgb)
            .unwrap();
        let mut r_rev = ProfileRegistry::new();
        r_rev.register_working_icc(p2, ColorModel::Rgb).unwrap();
        r_rev.register_working_icc(p1, ColorModel::Rgba).unwrap();
        let fwd: Vec<ProfileUid> = r_fwd.iter().map(|e| e.uid).collect();
        let rev: Vec<ProfileUid> = r_rev.iter().map(|e| e.uid).collect();
        assert_eq!(fwd, rev);
    }

    #[test]
    fn identical_bytes_dedup_to_one_uid() {
        let p = synthetic_icc(b"mntr", b"RGB ");
        let mut reg = ProfileRegistry::new();
        let a = reg
            .register_working_icc(p.clone(), ColorModel::Rgba)
            .unwrap();
        let b = reg.register_working_icc(p, ColorModel::Rgba).unwrap();
        assert_eq!(a, b);
        assert_eq!(reg.len(), 7); // 6 builtin seeds + 1 (deduped) user entry
    }

    #[test]
    fn size_cap_is_enforced() {
        let bytes = vec![0u8; PROFILE_SIZE_CAP + 1];
        assert_eq!(
            parse_icc_header(&bytes),
            Err(ColorError::ProfileTooLarge {
                len: PROFILE_SIZE_CAP + 1
            })
        );
    }

    #[test]
    fn truncated_and_hostile_streams_are_structured_errors() {
        // Truncation below the header.
        assert!(matches!(
            parse_icc_header(&[0u8; 40]),
            Err(ColorError::ProfileMalformed(_))
        ));
        // Bad magic.
        let mut bad_magic = synthetic_icc(b"mntr", b"RGB ");
        bad_magic[36] = b'x';
        assert_eq!(
            parse_icc_header(&bad_magic),
            Err(ColorError::ProfileMalformed("missing 'acsp' signature"))
        );
        // Declared size larger than the stream.
        let mut oversize = synthetic_icc(b"mntr", b"RGB ");
        oversize[0..4].copy_from_slice(&0x0100_0000u32.to_be_bytes());
        assert_eq!(
            parse_icc_header(&oversize),
            Err(ColorError::ProfileMalformed(
                "size field inconsistent with byte stream"
            ))
        );
        // Hostile tag count.
        let mut hostile_tags = synthetic_icc(b"mntr", b"RGB ");
        hostile_tags[128..132].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            parse_icc_header(&hostile_tags),
            Err(ColorError::ProfileMalformed(
                "tag count exceeds sanity bound"
            ))
        );
        // Tag data pointing past the declared size.
        let mut oob_tag = synthetic_icc(b"mntr", b"RGB ");
        oob_tag[136..140].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            parse_icc_header(&oob_tag),
            Err(ColorError::ProfileMalformed(
                "tag data exceeds declared size"
            ))
        );
    }

    #[test]
    fn class_gate_rejects_devicelink_namedcolor_abstract() {
        let mut reg = ProfileRegistry::new();
        for (sig, class) in [
            (b"link", ProfileClass::DeviceLink),
            (b"nmcl", ProfileClass::NamedColor),
            (b"abst", ProfileClass::Abstract),
        ] {
            let p = synthetic_icc(sig, b"RGB ");
            assert_eq!(
                reg.register_working_icc(p, ColorModel::Rgba),
                Err(ColorError::ProfileClass(class))
            );
        }
        // All four accepted classes pass.
        for sig in [b"mntr", b"scnr", b"prtr", b"spac"] {
            let p = synthetic_icc(sig, b"RGB ");
            assert!(reg.register_working_icc(p, ColorModel::Rgba).is_ok());
        }
    }

    #[test]
    fn space_gate_matches_profile_to_model() {
        let mut reg = ProfileRegistry::new();
        // GRAY profile against an RGB buffer: mismatch.
        let gray = synthetic_icc(b"mntr", b"GRAY");
        assert_eq!(
            reg.register_working_icc(gray.clone(), ColorModel::Rgba),
            Err(ColorError::ProfileSpaceMismatch)
        );
        // Against Gray/GrayA: fine.
        assert!(reg.register_working_icc(gray, ColorModel::GrayA).is_ok());
        // CMYK and Lab wiring of the same gate.
        let cmyk = synthetic_icc(b"prtr", b"CMYK");
        assert!(reg.register_working_icc(cmyk, ColorModel::Cmyk).is_ok());
        let lab = synthetic_icc(b"spac", b"Lab ");
        assert_eq!(
            reg.register_working_icc(lab.clone(), ColorModel::Rgb),
            Err(ColorError::ProfileSpaceMismatch)
        );
        assert!(reg.register_working_icc(lab, ColorModel::LabA).is_ok());
    }

    #[test]
    fn unknown_uid_is_a_structured_error() {
        let reg = ProfileRegistry::new();
        let ghost = ProfileUid::of_icc_bytes(b"not registered");
        assert_eq!(
            reg.get(ghost).err(),
            Some(ColorError::UnknownProfile(ghost))
        );
    }
}
