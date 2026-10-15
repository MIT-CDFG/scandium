// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Conformance suite for the ABI 1.6 font-provisioning door (Appendix-N
//! discipline: every additive block lands WITH its conformance rows).
//!
//! Rows: symbol coverage/version gate; registration (a host face lands
//! and enumerates with its metadata + content-addressed id); idempotence
//! per (asset,bytes); the §23.5/§44.2 refusals (docEmbedded source,
//! unknown source, unusable bytes, psName collision, different bytes
//! under a live id) surfaced as TYPED ABI errors; fonts-stamp BUMP on a
//! change; the fresh-doc readback shape; handle misuse; and the
//! CALL-ORDER-PRESERVED line (TEXTVEC scope note) — the door is faithful
//! to ITS caller's order; document order is the loader's §44.2
//! asset-index contract, upstream of this door.
//!
//! The face bytes are a MINIMAL TrueType font built procedurally here
//! (no font file is vendored — §3.10; the spike-fixture approach), just
//! rich enough that BOTH HarfBuzz and FreeType accept it (that is
//! exactly what `register_provided_face` validates).

use std::ffi::{CStr, CString};

use serde_json::Value;

use appname_cabi::doc::{appname_doc_close, appname_doc_new, AppnameDoc};
use appname_cabi::fonts::{appname_doc_font_faces, appname_doc_provide_face};
use appname_cabi::mem::{
    appname_error_free, appname_error_json, appname_error_status, appname_string_data,
    appname_string_free, AppnameError, AppnameString,
};
use appname_cabi::{appname_version, AppnameStatus, ABI_1_6_SYMBOLS};

// ---------------------------------------------------------------------------
// Minimal TrueType face builder (independent implementation of the public
// sfnt format; two glyphs: .notdef + 'A' as a filled rectangle).
// ---------------------------------------------------------------------------

fn be16(o: &mut Vec<u8>, v: u16) {
    o.extend_from_slice(&v.to_be_bytes());
}
fn bei16(o: &mut Vec<u8>, v: i16) {
    o.extend_from_slice(&v.to_be_bytes());
}
fn be32(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_be_bytes());
}
fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}
fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut chunk = [0u8; 4];
    for i in (0..data.len()).step_by(4) {
        chunk.fill(0);
        let n = (data.len() - i).min(4);
        chunk[..n].copy_from_slice(&data[i..i + n]);
        sum = sum.wrapping_add(u32::from_be_bytes(chunk));
    }
    sum
}

/// A rectangle glyph `[x0,x1] × [y0,y1]`, one closed contour (4 on-curve
/// points).
fn rect_glyph(x0: i16, x1: i16, y0: i16, y1: i16) -> Vec<u8> {
    let mut g = Vec::new();
    bei16(&mut g, 1); // numberOfContours
    bei16(&mut g, x0);
    bei16(&mut g, y0);
    bei16(&mut g, x1);
    bei16(&mut g, y1);
    be16(&mut g, 3); // endPtsOfContours[0]
    be16(&mut g, 0); // instructionLength
    g.extend_from_slice(&[0x01; 4]); // 4 on-curve flags
    for d in [x0, x1 - x0, 0, x0 - x1] {
        bei16(&mut g, d); // x deltas
    }
    for d in [y0, 0, y1 - y0, 0] {
        bei16(&mut g, d); // y deltas
    }
    pad4(&mut g);
    g
}

/// Build a minimal valid TrueType face mapping `A_ADV` to a rectangle.
/// `variant` lets a test produce DIFFERENT bytes for the same conceptual
/// face (a distinct advance) to exercise the "different bytes" refusal.
fn build_face(variant: u16) -> Vec<u8> {
    const UPM: u16 = 1000;
    let a_adv: u16 = 600 + variant; // varying advance => varying bytes/id

    // glyf + loca (long): gid0 .notdef empty, gid1 = 'A' rectangle.
    let mut glyf = Vec::new();
    let mut loca: Vec<u32> = vec![0];
    loca.push(glyf.len() as u32); // .notdef: empty
    glyf.extend_from_slice(&rect_glyph(100, 500, 0, 700));
    loca.push(glyf.len() as u32);
    let mut loca_b = Vec::new();
    for off in &loca {
        be32(&mut loca_b, *off);
    }

    // head (checkSumAdjustment patched last).
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000); // version
    be32(&mut head, 0x0001_0000); // fontRevision
    be32(&mut head, 0); // checkSumAdjustment (patched)
    be32(&mut head, 0x5F0F_3CF5); // magic
    be16(&mut head, 0); // flags
    be16(&mut head, UPM); // unitsPerEm
    be32(&mut head, 0); // created hi
    be32(&mut head, 0); // created lo
    be32(&mut head, 0); // modified hi
    be32(&mut head, 0); // modified lo
    bei16(&mut head, 100); // xMin
    bei16(&mut head, 0); // yMin
    bei16(&mut head, 500); // xMax
    bei16(&mut head, 700); // yMax
    be16(&mut head, 0); // macStyle
    be16(&mut head, 8); // lowestRecPPEM
    bei16(&mut head, 2); // fontDirectionHint
    bei16(&mut head, 1); // indexToLocFormat: long
    bei16(&mut head, 0); // glyphDataFormat

    // maxp 1.0. Fields after numGlyphs (13 uint16): maxPoints,
    // maxContours, … — give FreeType the true 'A' extents (4 points,
    // 1 contour) as allocation hints; the rest zero.
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, 2); // numGlyphs
    be16(&mut maxp, 4); // maxPoints
    be16(&mut maxp, 1); // maxContours
    for _ in 0..11 {
        be16(&mut maxp, 0);
    }

    // hhea.
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    bei16(&mut hhea, 800); // ascender
    bei16(&mut hhea, -200); // descender
    bei16(&mut hhea, 90); // lineGap
    be16(&mut hhea, a_adv); // advanceWidthMax
    bei16(&mut hhea, 0); // minLeftSideBearing
    bei16(&mut hhea, 0); // minRightSideBearing
    bei16(&mut hhea, 500); // xMaxExtent
    bei16(&mut hhea, 1); // caretSlopeRise
    bei16(&mut hhea, 0); // caretSlopeRun
    bei16(&mut hhea, 0); // caretOffset
    for _ in 0..4 {
        bei16(&mut hhea, 0); // reserved
    }
    bei16(&mut hhea, 0); // metricDataFormat
    be16(&mut hhea, 2); // numberOfHMetrics

    // hmtx: one longHorMetric per glyph (numberOfHMetrics=2).
    let mut hmtx = Vec::new();
    be16(&mut hmtx, 500); // .notdef advance
    bei16(&mut hmtx, 0); // lsb
    be16(&mut hmtx, a_adv); // 'A' advance
    bei16(&mut hmtx, 100); // lsb

    // cmap: format 4, one segment mapping 'A' (0x41) -> gid 1, plus the
    // required 0xFFFF terminator segment.
    let mut sub4 = Vec::new();
    let seg_count: u16 = 2;
    let seg_x2 = seg_count * 2;
    // seg_count = 2: searchRange = 2*2^floor(log2(2)) = 4, entrySelector
    // = 1, rangeShift = 2*segCount - searchRange = 0.
    let (search_range, entry_selector, range_shift) = (4u16, 1u16, 0u16);
    be16(&mut sub4, 4); // format
    be16(&mut sub4, 16 + 8 * seg_count); // length
    be16(&mut sub4, 0); // language
    be16(&mut sub4, seg_x2);
    be16(&mut sub4, search_range);
    be16(&mut sub4, entry_selector);
    be16(&mut sub4, range_shift);
    // endCode[]: 'A', 0xFFFF
    be16(&mut sub4, 0x0041);
    be16(&mut sub4, 0xFFFF);
    be16(&mut sub4, 0); // reservedPad
                        // startCode[]: 'A', 0xFFFF
    be16(&mut sub4, 0x0041);
    be16(&mut sub4, 0xFFFF);
    // idDelta[]: gid1 for 'A' means delta = 1 - 0x41; last segment delta 1
    be16(&mut sub4, (1i32 - 0x41i32) as u16);
    be16(&mut sub4, 1);
    // idRangeOffset[]: 0, 0
    be16(&mut sub4, 0);
    be16(&mut sub4, 0);
    let mut cmap = Vec::new();
    be16(&mut cmap, 0); // version
    be16(&mut cmap, 1); // numTables
    be16(&mut cmap, 3); // platformID Windows
    be16(&mut cmap, 1); // encodingID Unicode BMP
    be32(&mut cmap, 12); // offset to subtable
    cmap.extend_from_slice(&sub4);

    // name: format 0, zero records.
    let mut name = Vec::new();
    be16(&mut name, 0);
    be16(&mut name, 0);
    be16(&mut name, 6);

    // post 3.0.
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0); // italicAngle
    bei16(&mut post, 0); // underlinePosition
    bei16(&mut post, 0); // underlineThickness
    be32(&mut post, 0); // isFixedPitch
    for _ in 0..4 {
        be32(&mut post, 0); // min/max Mem*
    }

    // Assemble with a table directory (tags sorted ascending).
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca_b),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    tables.sort_by(|a, b| a.0.cmp(b.0));

    let num_tables = tables.len() as u16;
    // entrySelector = floor(log2(numTables)) = 31 - leading_zeros (u32).
    let entry_selector = (31u32 - (num_tables as u32).leading_zeros()) as u16;
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables * 16 - search_range;

    let dir_len = 12 + tables.len() * 16;
    let mut offset = dir_len;
    let mut records: Vec<(u32, u32, u32)> = Vec::new(); // (checksum, offset, len)
    for (_, body) in &mut tables {
        let len = body.len() as u32;
        let cs = checksum(body);
        records.push((cs, offset as u32, len));
        pad4(body);
        offset += body.len();
    }

    let mut font = Vec::new();
    be32(&mut font, 0x0001_0000); // sfntVersion
    be16(&mut font, num_tables);
    be16(&mut font, search_range);
    be16(&mut font, entry_selector);
    be16(&mut font, range_shift);
    for (i, (tag, _)) in tables.iter().enumerate() {
        font.extend_from_slice(*tag);
        let (cs, off, len) = records[i];
        be32(&mut font, cs);
        be32(&mut font, off);
        be32(&mut font, len);
    }
    for (_, body) in &tables {
        font.extend_from_slice(body);
    }

    // Patch head.checkSumAdjustment = 0xB1B0AFBA - checksum(whole font).
    let head_off = records[tables.iter().position(|(t, _)| *t == b"head").unwrap()].1 as usize;
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&font));
    font[head_off + 8..head_off + 12].copy_from_slice(&adjust.to_be_bytes());
    font
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn take_string(s: *mut AppnameString) -> String {
    let mut len: usize = 0;
    let p = unsafe { appname_string_data(s, &mut len) };
    assert!(!p.is_null());
    let t = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_owned();
    assert_eq!(unsafe { appname_string_free(s) }, AppnameStatus::AppnameOk);
    t
}

fn take_error(e: *mut AppnameError) -> (AppnameStatus, Value) {
    assert!(!e.is_null());
    let st = unsafe { appname_error_status(e) };
    let mut len: usize = 0;
    let p = unsafe { appname_error_json(e, &mut len) };
    assert!(!p.is_null());
    let t = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_owned();
    assert_eq!(unsafe { appname_error_free(e) }, AppnameStatus::AppnameOk);
    (st, serde_json::from_str(&t).unwrap())
}

fn new_doc() -> *mut AppnameDoc {
    let mut doc: *mut AppnameDoc = std::ptr::null_mut();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    assert_eq!(
        unsafe { appname_doc_new(&mut doc, &mut err) },
        AppnameStatus::AppnameOk
    );
    doc
}

#[allow(clippy::too_many_arguments)]
fn provide(
    doc: *mut AppnameDoc,
    bytes: &[u8],
    source: &str,
    family: &str,
    style: &str,
    psname: &str,
) -> Result<String, (AppnameStatus, Value)> {
    let (csrc, cfam, csty, cps) = (
        CString::new(source).unwrap(),
        CString::new(family).unwrap(),
        CString::new(style).unwrap(),
        CString::new(psname).unwrap(),
    );
    let mut out: *mut AppnameString = std::ptr::null_mut();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe {
        appname_doc_provide_face(
            doc,
            bytes.as_ptr(),
            bytes.len(),
            csrc.as_ptr(),
            cfam.as_ptr(),
            csty.as_ptr(),
            cps.as_ptr(),
            &mut out,
            &mut err,
        )
    };
    if st == AppnameStatus::AppnameOk {
        Ok(take_string(out))
    } else {
        Err(take_error(err))
    }
}

fn faces(doc: *mut AppnameDoc) -> Value {
    let mut out: *mut AppnameString = std::ptr::null_mut();
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe { appname_doc_font_faces(doc, &mut out, &mut err) };
    assert_eq!(st, AppnameStatus::AppnameOk, "{:?}", take_error(err));
    serde_json::from_str(&take_string(out)).unwrap()
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[test]
fn version_gates_the_block() {
    let v = appname_version();
    assert!(v.abi_minor >= 6, "the 1.6 block ships with abi_minor >= 6");
    assert_eq!(ABI_1_6_SYMBOLS.len(), 2);
}

#[test]
fn fresh_doc_readback_shape() {
    let doc = new_doc();
    let f = faces(doc);
    assert_eq!(f["schema"], "appname.fontfaces.v1");
    assert!(f["faces"].as_array().unwrap().is_empty());
    assert!(f["stamp"].is_u64());
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
}

#[test]
fn registration_enumerates_with_metadata_and_content_addressed_id() {
    let doc = new_doc();
    let before = faces(doc)["stamp"].as_u64().unwrap();
    let bytes = build_face(0);
    let id = provide(
        doc,
        &bytes,
        "user",
        "Cabi Sans",
        "Regular",
        "CabiSans-Regular",
    )
    .unwrap();

    // The id is the content-addressed asset id of the bytes.
    let expect = appname_core::engine::assets::asset_id_for(&bytes).to_string();
    assert_eq!(id, expect, "id is content-addressed");

    let f = faces(doc);
    assert!(
        f["stamp"].as_u64().unwrap() > before,
        "provision bumps the stamp"
    );
    let arr = f["faces"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["family"], "Cabi Sans");
    assert_eq!(arr[0]["style"], "Regular");
    assert_eq!(arr[0]["postscriptName"], "CabiSans-Regular");
    assert_eq!(
        arr[0]["embeddable"], true,
        "no fsType restriction => embeddable"
    );
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
}

#[test]
fn idempotent_per_asset_bytes() {
    let doc = new_doc();
    let bytes = build_face(0);
    let id1 = provide(
        doc,
        &bytes,
        "system",
        "Cabi Sans",
        "Regular",
        "CabiSans-Regular",
    )
    .unwrap();
    let stamp1 = faces(doc)["stamp"].as_u64().unwrap();
    // Re-provide IDENTICAL bytes: same id, no new face, stamp unchanged.
    let id2 = provide(
        doc,
        &bytes,
        "system",
        "Cabi Sans",
        "Regular",
        "CabiSans-Regular",
    )
    .unwrap();
    assert_eq!(id1, id2);
    assert_eq!(faces(doc)["faces"].as_array().unwrap().len(), 1);
    assert_eq!(
        faces(doc)["stamp"].as_u64().unwrap(),
        stamp1,
        "identical re-provide is a pure no-op"
    );
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
}

#[test]
fn refusals_are_typed() {
    let doc = new_doc();
    let bytes = build_face(0);

    // DOOR-LEVEL refusals (bad argument tokens) surface as
    // APPNAME_ERR_INVALID_ARG — the door parses the source before
    // touching the engine.
    //
    // docEmbedded source refused at the door (loader's class), typed.
    let (st, body) = provide(doc, &bytes, "docEmbedded", "X", "X", "X").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    assert_eq!(body["code"], "error.api.invalid_arg");
    assert!(body["message"].as_str().unwrap().contains("docEmbedded"));

    // Unknown source token refused.
    let (st, _) = provide(doc, &bytes, "kernel", "X", "X", "X").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);

    // ENGINE-LEVEL refusals (§44.1 validation, §23.5 dedup) ride the
    // engine's `OutOfRange` code, which buckets to APPNAME_ERR_SCHEMA;
    // the fine-grained wire code rides in the JSON. The door surfaces
    // them faithfully.
    //
    // Unusable bytes refused (neither HarfBuzz nor FreeType accept).
    let (st, _) = provide(doc, b"not a font at all", "user", "X", "X", "X").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrSchema);

    // A DIFFERENT face providing a psName already held refuses (§23.5
    // dedup would silently shadow one).
    provide(
        doc,
        &bytes,
        "user",
        "Cabi Sans",
        "Regular",
        "CabiSans-Regular",
    )
    .unwrap();
    let other = build_face(7); // different bytes => different asset id
    let (st, body) =
        provide(doc, &other, "user", "Other", "Regular", "CabiSans-Regular").unwrap_err();
    assert_eq!(st, AppnameStatus::AppnameErrSchema);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("postscript"));
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
}

/// CALL-ORDER-PRESERVED [conformance line, TEXTVEC scope]: the door
/// registers faces in the order its caller provides them (A, then B,
/// then C) — the door is faithful to ITS caller's order. Whether a
/// *loader* calls in document order is the loader's §44.2 asset-index
/// contract, upstream of this door and not asserted here.
#[test]
fn door_preserves_caller_order() {
    let doc = new_doc();
    // Three distinct faces, distinct psNames, provided A→B→C.
    let names = ["Aaa-Regular", "Bbb-Regular", "Ccc-Regular"];
    for (i, ps) in names.iter().enumerate() {
        let bytes = build_face(i as u16 + 1);
        provide(doc, &bytes, "user", "Fam", "Regular", ps).unwrap();
    }
    let f = faces(doc);
    let got: Vec<String> = f["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["postscriptName"].as_str().unwrap().to_string())
        .collect();
    // The door preserved call order into the service's enumeration.
    assert_eq!(got, names, "enumeration reflects caller (grant) order");
    assert_eq!(unsafe { appname_doc_close(doc) }, AppnameStatus::AppnameOk);
}

#[test]
fn handle_misuse_is_typed() {
    // provide on NULL doc.
    let bytes = build_face(0);
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let (csrc, cfam, csty, cps) = (
        CString::new("user").unwrap(),
        CString::new("F").unwrap(),
        CString::new("R").unwrap(),
        CString::new("P").unwrap(),
    );
    let st = unsafe {
        appname_doc_provide_face(
            std::ptr::null_mut(),
            bytes.as_ptr(),
            bytes.len(),
            csrc.as_ptr(),
            cfam.as_ptr(),
            csty.as_ptr(),
            cps.as_ptr(),
            std::ptr::null_mut(),
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);

    // font_faces on a garbage handle.
    let mut err: *mut AppnameError = std::ptr::null_mut();
    let st = unsafe {
        appname_doc_font_faces(
            0xdead_beefusize as *mut AppnameDoc,
            std::ptr::null_mut(),
            &mut err,
        )
    };
    assert_eq!(st, AppnameStatus::AppnameErrInvalidArg);
    let _ = take_error(err);
}
