// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! RAW binding slice — integration surface: the lensfun CONSUMER SEAM
//! (the O-INT split ruling against the REAL shipped database staged by
//! `deps/lensfun-db.yaml`) and the registry-level RAW decode arms with
//! the conformance-stub module.
//!
//! Spec: ES Part 43 §43.3 (stage 4b; param surface reserved), §43.5
//! table; ALGORITHMS.md §21 (lensfun data-only); the split ruling +
//! dossier-blessing ratification entries, INTEGRATION-log 2026-08-17.
//! NO goldens here — lensfun-consuming goldens route via O-QA only.

#![cfg(feature = "raw")]

use std::path::{Path, PathBuf};

use appname_io::raw::lensdb_seam::{load_shipped, load_with_user_dir, resolve_for_shot, ShotQuery};

/// The real shipped database (record `dataPaths: data/lensfun-db/db`),
/// resolved from the crate manifest to the repo root.
fn shipped_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("data/lensfun-db/db")
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("appname-rawlens-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A minimal valid lensfun-format document (matches the reader's
/// DTD/XSD-derived schema; own synthetic content, no upstream text).
const USER_DOC_OK: &str = r#"<lensdatabase version="2">
  <mount><name>TestMount</name></mount>
  <camera>
    <maker>Testco</maker>
    <model>Testco Proberling 9</model>
    <mount>TestMount</mount>
    <cropfactor>1.0</cropfactor>
  </camera>
  <lens>
    <maker>Testco</maker>
    <model>Testco Probe 50mm f/2</model>
    <mount>TestMount</mount>
    <cropfactor>1.0</cropfactor>
    <calibration>
      <distortion model="poly3" focal="50" k1="-0.01"/>
      <vignetting model="pa" focal="50" aperture="2" distance="1000" k1="-0.2" k2="0" k3="0"/>
    </calibration>
  </lens>
</lensdatabase>
"#;

// ---------------------------------------------------------------------------
// Ruling arm 1: shipped db, strict, fail-loud
// ---------------------------------------------------------------------------

/// The REAL shipped database loads strictly, warning-free — the record's
/// staged payload is a valid, clean corpus (a warning appearing here is
/// itself the shipped-db-modification tripwire).
#[test]
fn shipped_db_loads_strict_and_clean() {
    let db = load_shipped(&shipped_dir()).expect("shipped db must load");
    assert!(db.cameras.len() > 500, "cameras: {}", db.cameras.len());
    assert!(db.lenses.len() > 1000, "lenses: {}", db.lenses.len());
    assert!(!db.mounts.is_empty());
    assert!(
        db.warnings.is_empty(),
        "shipped corpus must be warning-free: {:?}",
        db.warnings
    );
}

/// Malformed SHIPPED data fails LOUD (typed error.internal — a broken
/// build, never a skip): the ruling's own rationale.
#[test]
fn corrupt_shipped_dir_fails_loud() {
    let dir = scratch("shipped-corrupt");
    std::fs::write(dir.join("broken.xml"), "<lensdatabase version=\"2\"><lens>").unwrap();
    let err = load_shipped(&dir).expect_err("must fail loud");
    assert_eq!(err.message_id(), "error.internal");
    assert!(err.to_string().contains("fail-loud"), "{err}");
    // And through the user-dir composer too: the shipped arm is strict
    // regardless of user-dir presence.
    let err = load_with_user_dir(&dir, None).expect_err("still loud");
    assert_eq!(err.message_id(), "error.internal");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------
// Ruling arm 2: user dir degrades to warnings-and-skip at this seam
// ---------------------------------------------------------------------------

/// One bad user XML costs exactly that file (warning surfaced); good
/// user files merge; the shipped data is untouched by the degradation.
#[test]
fn user_dir_degrades_per_file_and_merges_good_files() {
    let user = scratch("user-mixed");
    std::fs::write(user.join("a_good.xml"), USER_DOC_OK).unwrap();
    std::fs::write(
        user.join("b_bad.xml"),
        "<lensdatabase version=\"2\"><mount>",
    )
    .unwrap();
    std::fs::write(user.join("ignored.txt"), "not xml").unwrap();

    let shipped = shipped_dir();
    let baseline = load_shipped(&shipped).unwrap();
    let (db, warnings) = load_with_user_dir(&shipped, Some(&user)).expect("degrade, not fail");

    // The bad file is skipped with a per-file warning naming it.
    assert!(
        warnings
            .iter()
            .any(|w| w.file == "b_bad.xml" && w.detail.starts_with("skipped:")),
        "{warnings:?}"
    );
    // The good file's records merged on top of the shipped set.
    assert_eq!(db.cameras.len(), baseline.cameras.len() + 1);
    assert_eq!(db.lenses.len(), baseline.lenses.len() + 1);
    assert!(db.find_camera("Testco", "Testco Proberling 9").is_some());

    // And the merged database resolves the user lens for stage 4b.
    let cal = resolve_for_shot(
        &db,
        &ShotQuery {
            cam_maker: "Testco",
            cam_model: "Testco Proberling 9",
            lens_maker: "Testco",
            lens_model: "Testco Probe 50mm f/2",
            focal_mm: 50.0,
            aperture: Some(2.0),
            distance_m: Some(1000.0),
        },
    )
    .unwrap()
    .expect("calibration present");
    assert!(cal.distortion.is_some());
    assert!(cal.vignetting.is_some());
    std::fs::remove_dir_all(&user).unwrap();
}

/// An absent/empty user dir is simply "no user data".
#[test]
fn absent_user_dir_is_clean() {
    let shipped = shipped_dir();
    let (db, warnings) = load_with_user_dir(
        &shipped,
        Some(Path::new("/nonexistent/appname-user-lensdb")),
    )
    .unwrap();
    assert!(warnings.is_empty());
    assert!(!db.cameras.is_empty());
}

/// Non-finite lookup queries (EXIF 0/0 rationals at this seam) are a
/// typed refusal, never a panic — the Row-C review contract consumed.
#[test]
fn non_finite_lens_query_is_typed() {
    let db = load_shipped(&shipped_dir()).unwrap();
    // Any real camera/lens pair works for the query-validation check;
    // use the first lens's own names.
    let lens = &db.lenses[0];
    let err = resolve_for_shot(
        &db,
        &ShotQuery {
            cam_maker: "",
            cam_model: "",
            lens_maker: lens.maker(),
            lens_model: lens.model(),
            focal_mm: f64::NAN,
            aperture: None,
            distance_m: None,
        },
    )
    .expect_err("NaN focal must refuse");
    assert_eq!(err.message_id(), "error.codec.corrupt");
    assert!(err.to_string().contains("lens_query_non_finite"), "{err}");
}

// ---------------------------------------------------------------------------
// Registry-level RAW decode arms (stub module; sniff routes by real
// vendor magic, the stub spec rides behind it)
// ---------------------------------------------------------------------------

use appname_io::raw::codec::RawCodec;
use appname_io::registry::Codec;
use appname_io::DecodeOpts;
use libraw_sys::stub::{stub_bytes, stub_lib, with_prefix, StubSpec};

fn cr2_prefix() -> Vec<u8> {
    let mut v = vec![0x49, 0x49, 0x2A, 0x00, 0x10, 0x00, 0x00, 0x00, 0x43, 0x52];
    v.resize(16, 0);
    v
}

/// Registry resolution (CR2 magic) + the stub module: the decode call
/// runs the live stage-1 path and stops at the RECORDED egress gap —
/// typed, with the pre-gap arms (params, module, open, unpack, limits,
/// CFA, levels) all live above it.
#[test]
fn registry_routed_decode_reaches_egress_gap_typed() {
    let reg = appname_io::CodecRegistry::builtin();
    let bytes = with_prefix(&cr2_prefix(), &stub_bytes(&StubSpec::bayer_rggb(32, 24)));
    let codec = reg
        .resolve(&bytes[..16], Some("cr2"))
        .expect("routes to raw");
    assert_eq!(codec.id().0, "raw");
    // The registry's default RawCodec resolves the GLOBAL module (absent
    // in tests) — pin the §43.5 row-1 arm through the registry object.
    let err = codec.decode(&bytes, &DecodeOpts::default()).unwrap_err();
    assert_eq!(err.message_id(), "error.unsupported.raw_disabled");
    // With an explicit stub module: the full live path to the gap.
    let err = RawCodec::with_module(stub_lib())
        .decode(&bytes, &DecodeOpts::default())
        .unwrap_err();
    assert_eq!(err.message_id(), "error.codec.unsupported_feature");
    assert!(err.to_string().contains("stage1_mosaic_egress"), "{err}");
}
