// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §29.1 [NORMATIVE] "An exported file embeds the correct
//! profile (tested)" + §5.2 [LEGAL] posture (builtin working spaces are
//! SYNTHESIZED definitions — no third-party profile bytes are bundled
//! or copied, matching appname-color's runtime-synthesis rule). This
//! module emits a minimal, deterministic ICC v4 matrix/TRC profile for
//! a [`BuiltinSpace`], entirely from the §5.5/Appendix-B constants that
//! `appname_color::space` already exposes (`rgb_to_xyz_d50`,
//! `transfer`): header + `desc`/`cprt` (mluc), `wtpt` (D50), and for
//! RGB spaces `rXYZ`/`gXYZ`/`bXYZ` + parametric TRCs; gray spaces get
//! `kTRC`.
//!
//! Determinism: creation date zeroed, profile ID zeroed, fixed tag
//! order — same space, same bytes, forever (the W21 "equal pixels equal
//! bytes" bar extends to embedded profiles).
//!
//! Structure note: the ICC container layout implemented here follows
//! the published ICC.1 specification (a public interface standard);
//! every VALUE is our own §5.5 constant.

use appname_color::{BuiltinSpace, TransferFn};

/// D50 PCS illuminant, s15Fixed16 (ICC.1 fixed values).
const D50_X: u32 = 0x0000_F6D6;
const D50_Y: u32 = 0x0001_0000;
const D50_Z: u32 = 0x0000_D32D;

fn s15f16(v: f64) -> u32 {
    // round-half-away from zero; range-clamped (profiles never carry
    // values outside s15.16 here)
    let x = (v * 65536.0).round();
    (x.clamp(-(1i64 << 31) as f64, ((1i64 << 31) - 1) as f64) as i64 as i32) as u32
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// `mluc` with one `enUS` record.
fn mluc(text: &str) -> Vec<u8> {
    let utf16: Vec<u8> = text.encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
    let mut t = Vec::with_capacity(28 + utf16.len());
    t.extend_from_slice(b"mluc");
    t.extend_from_slice(&be32(0));
    t.extend_from_slice(&be32(1)); // record count
    t.extend_from_slice(&be32(12)); // record size
    t.extend_from_slice(b"enUS");
    t.extend_from_slice(&be32(utf16.len() as u32));
    t.extend_from_slice(&be32(28)); // offset from tag start
    t.extend_from_slice(&utf16);
    t
}

fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
    let mut t = Vec::with_capacity(20);
    t.extend_from_slice(b"XYZ ");
    t.extend_from_slice(&be32(0));
    t.extend_from_slice(&be32(s15f16(x)));
    t.extend_from_slice(&be32(s15f16(y)));
    t.extend_from_slice(&be32(s15f16(z)));
    t
}

fn wtpt_d50() -> Vec<u8> {
    let mut t = Vec::with_capacity(20);
    t.extend_from_slice(b"XYZ ");
    t.extend_from_slice(&be32(0));
    t.extend_from_slice(&be32(D50_X));
    t.extend_from_slice(&be32(D50_Y));
    t.extend_from_slice(&be32(D50_Z));
    t
}

/// `para` parametric curve for a §5.5 transfer (exact — these curves
/// are parametric by construction).
fn para_tag(tf: TransferFn) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(b"para");
    t.extend_from_slice(&be32(0));
    match tf {
        TransferFn::Linear => {
            t.extend_from_slice(&[0, 0, 0, 0]); // function type 0
            t.extend_from_slice(&be32(s15f16(1.0))); // g = 1
        }
        TransferFn::Gamma22 => {
            t.extend_from_slice(&[0, 0, 0, 0]);
            t.extend_from_slice(&be32(s15f16(2.2)));
        }
        TransferFn::SrgbPiecewise => {
            // type 3: Y = (aX+b)^g for X ≥ d, cX below — the IEC
            // 61966-2-1 constants (§5.5)
            t.extend_from_slice(&[0, 3, 0, 0]);
            for p in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
                t.extend_from_slice(&be32(s15f16(p)));
            }
        }
        TransferFn::ProPhoto => {
            // ROMM (§5.5): g 1.8, linear toe slope 1/16, break 16/512
            t.extend_from_slice(&[0, 3, 0, 0]);
            for p in [1.8, 1.0, 0.0, 1.0 / 16.0, 16.0 / 512.0] {
                t.extend_from_slice(&be32(s15f16(p)));
            }
        }
    }
    t
}

/// Synthesize the deterministic ICC v4 profile for a builtin space.
pub fn builtin_icc_bytes(space: BuiltinSpace) -> Vec<u8> {
    let gray = space.is_gray();
    let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    tags.push((*b"desc", mluc(space.desc())));
    tags.push((
        *b"cprt",
        mluc("CC0: synthesized colorimetric definition, no rights reserved"),
    ));
    tags.push((*b"wtpt", wtpt_d50()));
    if gray {
        tags.push((*b"kTRC", para_tag(space.transfer())));
    } else {
        let m = space
            .rgb_to_xyz_d50()
            .expect("RGB builtin has a D50 matrix")
            .m;
        // columns are the primaries' XYZ (rows are X/Y/Z components)
        tags.push((*b"rXYZ", xyz_tag(m[0][0], m[1][0], m[2][0])));
        tags.push((*b"gXYZ", xyz_tag(m[0][1], m[1][1], m[2][1])));
        tags.push((*b"bXYZ", xyz_tag(m[0][2], m[1][2], m[2][2])));
        let trc = para_tag(space.transfer());
        tags.push((*b"rTRC", trc.clone()));
        tags.push((*b"gTRC", trc.clone()));
        tags.push((*b"bTRC", trc));
    }

    // Layout: header (128) + tag count (4) + table (12 each) + data,
    // each datum 4-byte aligned.
    let table_end = 128 + 4 + tags.len() * 12;
    let mut offsets = Vec::with_capacity(tags.len());
    let mut cursor = table_end;
    for (_, data) in &tags {
        offsets.push(cursor);
        cursor += (data.len() + 3) & !3;
    }
    let total = cursor as u32;

    let mut out = Vec::with_capacity(cursor);
    // ---- header ----
    out.extend_from_slice(&be32(total)); // size
    out.extend_from_slice(b"\0\0\0\0"); // CMM: none
    out.extend_from_slice(&be32(0x0420_0000)); // version 4.2
    out.extend_from_slice(b"mntr"); // display class
    out.extend_from_slice(if gray { b"GRAY" } else { b"RGB " });
    out.extend_from_slice(b"XYZ "); // PCS
    out.extend_from_slice(&[0u8; 12]); // creation date: zero (determinism)
    out.extend_from_slice(b"acsp");
    out.extend_from_slice(&[0u8; 4]); // platform: none
    out.extend_from_slice(&be32(0)); // flags
    out.extend_from_slice(&[0u8; 8]); // manufacturer + model
    out.extend_from_slice(&[0u8; 8]); // attributes
    out.extend_from_slice(&be32(0)); // intent: perceptual
    out.extend_from_slice(&be32(D50_X));
    out.extend_from_slice(&be32(D50_Y));
    out.extend_from_slice(&be32(D50_Z));
    out.extend_from_slice(&[0u8; 4]); // creator
    out.extend_from_slice(&[0u8; 16]); // profile ID: zero (determinism)
    out.extend_from_slice(&[0u8; 28]); // reserved
    debug_assert_eq!(out.len(), 128);
    // ---- tag table ----
    out.extend_from_slice(&be32(tags.len() as u32));
    for ((sig, data), off) in tags.iter().zip(&offsets) {
        out.extend_from_slice(sig);
        out.extend_from_slice(&be32(*off as u32));
        out.extend_from_slice(&be32(data.len() as u32));
    }
    // ---- tag data ----
    for (_, data) in &tags {
        out.extend_from_slice(data);
        let pad = ((data.len() + 3) & !3) - data.len();
        out.extend_from_slice(&[0u8; 3][..pad]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_parse_and_are_deterministic() {
        for space in BuiltinSpace::ALL {
            let a = builtin_icc_bytes(space);
            let b = builtin_icc_bytes(space);
            assert_eq!(a, b, "{space:?} deterministic");
            let hdr = appname_color::parse_icc_header(&a)
                .unwrap_or_else(|e| panic!("{space:?} header must parse: {e}"));
            assert_eq!(hdr.declared_size as usize, a.len());
        }
    }

    #[test]
    fn srgb_primaries_land_in_the_matrix_tags() {
        let bytes = builtin_icc_bytes(BuiltinSpace::Srgb);
        // rXYZ tag data present with a plausible red X (~0.436 D50-adapted)
        let pos = bytes
            .windows(4)
            .position(|w| w == b"rXYZ")
            .expect("rXYZ in table");
        // table entry: sig, offset, size
        let off = u32::from_be_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let x = u32::from_be_bytes(bytes[off + 8..off + 12].try_into().unwrap());
        let xf = x as f64 / 65536.0;
        assert!(
            (0.40..0.47).contains(&xf),
            "red primary X (D50) ≈ 0.436, got {xf}"
        );
    }
}
