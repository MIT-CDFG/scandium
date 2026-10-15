// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Data-directory reader: a directory of `*.xml` profile files → one
//! indexed [`LensDatabase`].
//!
//! USER-REPLACEABLE-DIR posture (the CC-BY-SA-3.0 database ships as a
//! clearly-separated data package the user may swap out — ALGORITHMS.md
//! §1.3 data rule / §21; rights-register lensfun-db row): every input is
//! untrusted. Malformed files produce a typed [`DbError`] naming the
//! file and position; nothing panics.
//!
//! Determinism (D-48):
//! - files are processed in SORTED FILE-NAME ORDER (byte order of the
//!   names), independent of directory-listing order;
//! - merged entries are re-sorted into one canonical order, so two
//!   reads of equal content are `==` and produce identical
//!   [`LensDatabase::canonical_summary`] strings;
//! - later files NEVER silently override earlier entries: a duplicate
//!   camera/lens identity is skipped and surfaced in
//!   [`LensDatabase::warnings`] (first definition wins — the `.myb`
//!   importer's warning-list precedent); duplicate mount declarations
//!   merge their compat lists (union) and are likewise surfaced.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use super::model::{db_file_from_root, Camera, Lens, Mount};
use super::xml::{line_col, parse_document, XmlError};
use crate::lensdb::lookup::normalize_name;

/// Reader error: file + failure class + position where known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DbError {
    /// Filesystem error (directory listing or file read).
    Io {
        /// Path involved.
        path: String,
        /// OS error text.
        msg: String,
    },
    /// The file is not valid UTF-8.
    Utf8 {
        /// Path involved.
        path: String,
    },
    /// Well-formedness error from the XML subset parser.
    Xml {
        /// Path involved.
        path: String,
        /// The parse error (carries line/column).
        error: XmlError,
    },
    /// The XML is well-formed but violates the database schema.
    Schema {
        /// Path involved.
        path: String,
        /// Plain-words description.
        msg: String,
        /// 1-based line of the offending construct.
        line: usize,
        /// 1-based byte column of the offending construct.
        col: usize,
    },
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Io { path, msg } => write!(f, "{path}: I/O error: {msg}"),
            DbError::Utf8 { path } => write!(f, "{path}: not valid UTF-8"),
            DbError::Xml { path, error } => write!(f, "{path}: {error}"),
            DbError::Schema {
                path,
                msg,
                line,
                col,
            } => write!(f, "{path}: line {line}, column {col}: {msg}"),
        }
    }
}

/// The merged, indexed profile database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensDatabase {
    /// Database format version (max seen; mixed versions warn).
    pub version: u32,
    /// Mount declarations, sorted by canonical name.
    pub mounts: Vec<Mount>,
    /// Camera records in canonical order.
    pub cameras: Vec<Camera>,
    /// Lens records in canonical order.
    pub lenses: Vec<Lens>,
    /// Non-fatal findings (duplicate identities, mixed versions), in
    /// deterministic processing order. Empty on a clean corpus.
    pub warnings: Vec<String>,
}

/// Identity key of a camera: normalized (maker, model, variant).
fn camera_key(c: &Camera) -> (String, String, String) {
    (
        normalize_name(c.maker()),
        normalize_name(c.model()),
        normalize_name(c.variant.as_deref().unwrap_or("")),
    )
}

/// Identity key of a lens: normalized (maker, model, sorted mounts,
/// crop-factor bits — the same maker/model calibrated for another
/// sensor is a distinct record, per the corpus).
type LensKey = (String, String, Vec<String>, u64);

fn lens_key(l: &Lens) -> LensKey {
    let mut mounts: Vec<String> = l.mounts.iter().map(|m| normalize_name(m)).collect();
    mounts.sort();
    (
        normalize_name(l.maker()),
        normalize_name(l.model()),
        mounts,
        l.crop_factor.to_bits(),
    )
}

/// Read a profile data DIRECTORY (non-recursive; `*.xml` files only,
/// extension matched case-insensitively).
pub fn read_dir(dir: &Path) -> Result<LensDatabase, DbError> {
    let entries = std::fs::read_dir(dir).map_err(|e| DbError::Io {
        path: dir.display().to_string(),
        msg: e.to_string(),
    })?;
    let mut docs: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| DbError::Io {
            path: dir.display().to_string(),
            msg: e.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_xml = path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("xml"));
        if !is_xml {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = std::fs::read(&path).map_err(|e| DbError::Io {
            path: path.display().to_string(),
            msg: e.to_string(),
        })?;
        let text = String::from_utf8(bytes).map_err(|_| DbError::Utf8 {
            path: path.display().to_string(),
        })?;
        docs.push((name, text));
    }
    from_named_documents(&docs)
}

/// Merge named XML documents into one database. Input order is
/// IRRELEVANT: documents are processed in sorted-name order (the
/// shuffled-listing determinism proof rides on this).
pub fn from_named_documents(docs: &[(String, String)]) -> Result<LensDatabase, DbError> {
    let mut order: Vec<usize> = (0..docs.len()).collect();
    order.sort_by(|&a, &b| docs[a].0.cmp(&docs[b].0));

    let mut warnings: Vec<String> = Vec::new();
    let mut version: Option<u32> = None;
    // BTreeMap keyed by identity: deterministic and override-proof.
    let mut mounts: BTreeMap<String, (Mount, String)> = BTreeMap::new();
    let mut cameras: BTreeMap<(String, String, String), (Camera, String)> = BTreeMap::new();
    let mut lenses: BTreeMap<LensKey, (Lens, String)> = BTreeMap::new();

    for &idx in &order {
        let (name, text) = &docs[idx];
        let root = parse_document(text).map_err(|error| DbError::Xml {
            path: name.clone(),
            error,
        })?;
        let file = db_file_from_root(&root).map_err(|e| {
            let (line, col) = line_col(text, e.offset);
            DbError::Schema {
                path: name.clone(),
                msg: e.msg,
                line,
                col,
            }
        })?;

        match version {
            None => version = Some(file.version),
            Some(v) if v != file.version => {
                warnings.push(format!(
                    "{name}: database version {} differs from {v}; keeping the maximum",
                    file.version
                ));
                version = Some(v.max(file.version));
            }
            Some(_) => {}
        }

        for m in file.mounts {
            let key = normalize_name(m.name());
            match mounts.get_mut(&key) {
                None => {
                    mounts.insert(key, (m, name.clone()));
                }
                Some((existing, first_file)) => {
                    // Merge compat by union; never drop earlier data.
                    let mut added = false;
                    for c in m.compat {
                        if !existing.compat.contains(&c) {
                            existing.compat.push(c);
                            added = true;
                        }
                    }
                    warnings.push(format!(
                        "{name}: duplicate mount {:?} (first defined in {first_file}); \
                         compat lists merged{}",
                        existing.name(),
                        if added { " with additions" } else { "" }
                    ));
                }
            }
        }
        for c in file.cameras {
            let key = camera_key(&c);
            match cameras.get(&key) {
                None => {
                    cameras.insert(key, (c, name.clone()));
                }
                Some((first, first_file)) => {
                    warnings.push(format!(
                        "{name}: duplicate camera {:?} / {:?}{} ignored \
                         (first defined in {first_file})",
                        first.maker(),
                        first.model(),
                        first
                            .variant
                            .as_deref()
                            .map(|v| format!(" (variant {v:?})"))
                            .unwrap_or_default(),
                    ));
                }
            }
        }
        for l in file.lenses {
            let key = lens_key(&l);
            match lenses.get(&key) {
                None => {
                    lenses.insert(key, (l, name.clone()));
                }
                Some((first, first_file)) => {
                    warnings.push(format!(
                        "{name}: duplicate lens {:?} / {:?} (crop {}) ignored \
                         (first defined in {first_file})",
                        first.maker(),
                        first.model(),
                        first.crop_factor,
                    ));
                }
            }
        }
    }

    let mut db = LensDatabase {
        version: version.unwrap_or(0),
        mounts: mounts.into_values().map(|(m, _)| m).collect(),
        cameras: cameras.into_values().map(|(c, _)| c).collect(),
        lenses: lenses.into_values().map(|(l, _)| l).collect(),
        warnings,
    };
    // Canonical compat order inside each mount (union order is
    // file-dependent only when duplicates exist; sort settles it).
    for m in &mut db.mounts {
        m.compat.sort();
        m.compat.dedup();
    }
    Ok(db)
}

impl LensDatabase {
    /// A canonical, deterministic text rendering of the whole database
    /// (counts + every record; floats via shortest-round-trip `{:?}`).
    /// Two equal databases produce byte-identical summaries — this is
    /// the golden-hash surface and the two-run determinism oracle.
    pub fn canonical_summary(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "lensdb v{} mounts={} cameras={} lenses={} warnings={}",
            self.version,
            self.mounts.len(),
            self.cameras.len(),
            self.lenses.len(),
            self.warnings.len()
        );
        for m in &self.mounts {
            let _ = writeln!(s, "mount {:?} compat={:?}", m.name(), m.compat);
        }
        for c in &self.cameras {
            let _ = writeln!(
                s,
                "camera {:?} {:?} variant={:?} mount={:?} crop={:?} ar={:?}",
                c.maker(),
                c.model(),
                c.variant,
                c.mount,
                c.crop_factor,
                c.aspect_ratio
            );
        }
        for l in &self.lenses {
            let _ = writeln!(
                s,
                "lens {:?} {:?} mounts={:?} focal={:?} aperture={:?} type={:?} \
                 crop={:?} ar={:?}",
                l.maker(),
                l.model(),
                l.mounts,
                l.focal,
                l.aperture,
                l.lens_type,
                l.crop_factor,
                l.aspect_ratio
            );
            if let Some(cal) = &l.calibration {
                for d in &cal.distortion {
                    let _ = writeln!(s, "  distortion {d:?}");
                }
                for t in &cal.tca {
                    let _ = writeln!(s, "  tca {t:?}");
                }
                for v in &cal.vignetting {
                    let _ = writeln!(s, "  vignetting {v:?}");
                }
            }
        }
        for w in &self.warnings {
            let _ = writeln!(s, "warning {w}");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::FULL_FIXTURE;
    use super::*;

    /// FNV-1a (64-bit) over bytes — the crate's pinned-golden hash form
    /// (same constants as `plane::plane_hash`).
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    fn second_file() -> String {
        "<lensdatabase version=\"2\">\n\
         <mount><name>Example FD</name><compat>Generic</compat></mount>\n\
         <camera><maker>Othercorp</maker><model>Othercorp Box 100</model>\n\
         <mount>Example FD</mount><cropfactor>1.5</cropfactor></camera>\n\
         </lensdatabase>\n"
            .to_owned()
    }

    fn docs() -> Vec<(String, String)> {
        vec![
            ("a-example.xml".to_owned(), FULL_FIXTURE.to_owned()),
            ("b-other.xml".to_owned(), second_file()),
        ]
    }

    #[test]
    fn merge_counts_and_clean_warnings() {
        let db = from_named_documents(&docs()).expect("db");
        assert_eq!(db.version, 2);
        assert_eq!(db.mounts.len(), 2);
        assert_eq!(db.cameras.len(), 2);
        assert_eq!(db.lenses.len(), 3);
        assert!(
            db.warnings.is_empty(),
            "clean input must not warn: {:?}",
            db.warnings
        );
    }

    #[test]
    fn shuffled_input_order_produces_identical_database() {
        let forward = from_named_documents(&docs()).expect("db");
        let mut shuffled = docs();
        shuffled.reverse();
        let backward = from_named_documents(&shuffled).expect("db");
        assert_eq!(forward, backward);
        assert_eq!(forward.canonical_summary(), backward.canonical_summary());
    }

    #[test]
    fn two_runs_are_equal() {
        let a = from_named_documents(&docs()).expect("db");
        let b = from_named_documents(&docs()).expect("db");
        assert_eq!(a, b);
        assert_eq!(a.canonical_summary(), b.canonical_summary());
    }

    #[test]
    fn duplicate_camera_identity_warns_and_first_wins() {
        // Same camera identity, different crop factor, in a
        // LATER-sorting file: must warn and keep the first value.
        let dup = "<lensdatabase version=\"2\">\
                   <camera><maker>Examplecorp</maker>\
                   <model>Examplecorp Snapper 5D</model><variant>mk II</variant>\
                   <mount>Example EF</mount><cropfactor>2.0</cropfactor>\
                   </camera></lensdatabase>";
        let mut d = docs();
        d.push(("z-dup.xml".to_owned(), dup.to_owned()));
        let db = from_named_documents(&d).expect("db");
        assert_eq!(db.cameras.len(), 2, "duplicate must not add a record");
        let cam = db
            .cameras
            .iter()
            .find(|c| c.model() == "Examplecorp Snapper 5D")
            .expect("camera present");
        assert_eq!(cam.crop_factor, 1.6, "first definition wins");
        assert_eq!(db.warnings.len(), 1);
        assert!(
            db.warnings[0].contains("duplicate camera"),
            "{}",
            db.warnings[0]
        );
        assert!(db.warnings[0].contains("z-dup.xml"), "{}", db.warnings[0]);
        assert!(
            db.warnings[0].contains("a-example.xml"),
            "{}",
            db.warnings[0]
        );
    }

    #[test]
    fn duplicate_camera_matches_case_and_whitespace_insensitively() {
        let dup = "<lensdatabase version=\"2\">\
                   <camera><maker>EXAMPLECORP</maker>\
                   <model>examplecorp   snapper 5d</model><variant>MK II</variant>\
                   <mount>Example EF</mount><cropfactor>2.0</cropfactor>\
                   </camera></lensdatabase>";
        let mut d = docs();
        d.push(("z-dup.xml".to_owned(), dup.to_owned()));
        let db = from_named_documents(&d).expect("db");
        assert_eq!(db.cameras.len(), 2);
        assert_eq!(db.warnings.len(), 1);
    }

    #[test]
    fn duplicate_lens_identity_warns_and_first_wins() {
        let dup = "<lensdatabase version=\"2\">\
                   <lens><maker>Examplecorp</maker>\
                   <model>Examplecorp Prime 50mm f/1.8</model>\
                   <mount>Example EF</mount><mount>M42</mount>\
                   <cropfactor>1.0</cropfactor></lens></lensdatabase>";
        let mut d = docs();
        d.push(("z-dup.xml".to_owned(), dup.to_owned()));
        let db = from_named_documents(&d).expect("db");
        assert_eq!(db.lenses.len(), 3, "duplicate must not add a record");
        let lens = db
            .lenses
            .iter()
            .find(|l| l.model() == "Examplecorp Prime 50mm f/1.8")
            .expect("lens present");
        assert!(
            lens.calibration.is_some(),
            "first definition (with calibration) wins"
        );
        assert_eq!(db.warnings.len(), 1);
        assert!(
            db.warnings[0].contains("duplicate lens"),
            "{}",
            db.warnings[0]
        );
    }

    #[test]
    fn same_lens_with_different_crop_is_a_distinct_record() {
        let other_crop = "<lensdatabase version=\"2\">\
                          <lens><maker>Examplecorp</maker>\
                          <model>Examplecorp Prime 50mm f/1.8</model>\
                          <mount>Example EF</mount><mount>M42</mount>\
                          <cropfactor>1.6</cropfactor></lens></lensdatabase>";
        let mut d = docs();
        d.push(("z-crop.xml".to_owned(), other_crop.to_owned()));
        let db = from_named_documents(&d).expect("db");
        assert_eq!(db.lenses.len(), 4);
        assert!(db.warnings.is_empty(), "{:?}", db.warnings);
    }

    #[test]
    fn duplicate_mount_merges_compat_union_and_warns() {
        let dup = "<lensdatabase version=\"2\">\
                   <mount><name>Example EF</name><compat>T2</compat>\
                   <compat>M42</compat></mount></lensdatabase>";
        let mut d = docs();
        d.push(("z-mount.xml".to_owned(), dup.to_owned()));
        let db = from_named_documents(&d).expect("db");
        assert_eq!(db.mounts.len(), 2);
        let m = db
            .mounts
            .iter()
            .find(|m| m.name() == "Example EF")
            .expect("mount");
        // Union of {M42, Generic} and {T2, M42}, sorted.
        assert_eq!(m.compat, vec!["Generic", "M42", "T2"]);
        assert_eq!(db.warnings.len(), 1);
        assert!(
            db.warnings[0].contains("duplicate mount"),
            "{}",
            db.warnings[0]
        );
    }

    #[test]
    fn mixed_versions_warn_and_keep_maximum() {
        let v3 = "<lensdatabase version=\"3\"><mount><name>Z</name></mount></lensdatabase>";
        let mut d = docs();
        d.push(("z-v3.xml".to_owned(), v3.to_owned()));
        let db = from_named_documents(&d).expect("db");
        assert_eq!(db.version, 3);
        assert_eq!(db.warnings.len(), 1);
        assert!(db.warnings[0].contains("version"), "{}", db.warnings[0]);
    }

    #[test]
    fn xml_error_names_the_file_and_position() {
        let broken = "<lensdatabase version=\"2\">\n<mount><name>X</name>\n</lensdatabase>";
        let d = vec![("broken.xml".to_owned(), broken.to_owned())];
        match from_named_documents(&d).expect_err("must fail") {
            DbError::Xml { path, error } => {
                assert_eq!(path, "broken.xml");
                assert_eq!(error.line, 3);
            }
            other => panic!("wrong error class: {other:?}"),
        }
    }

    #[test]
    fn schema_error_names_the_file_and_position() {
        let bad = "<lensdatabase version=\"2\">\n  <gadget/>\n</lensdatabase>";
        let d = vec![("bad.xml".to_owned(), bad.to_owned())];
        match from_named_documents(&d).expect_err("must fail") {
            DbError::Schema {
                path, msg, line, ..
            } => {
                assert_eq!(path, "bad.xml");
                assert!(msg.contains("<gadget>"), "{msg}");
                assert_eq!(line, 2);
            }
            other => panic!("wrong error class: {other:?}"),
        }
    }

    #[test]
    fn read_dir_matches_in_memory_merge_regardless_of_creation_order() {
        // Hermetic temp dir; files created in NON-sorted order.
        let dir = std::env::temp_dir().join(format!(
            "appname-lensdb-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("b-other.xml"), second_file()).expect("write");
        std::fs::write(dir.join("a-example.xml"), FULL_FIXTURE).expect("write");
        // Non-XML entries are ignored.
        std::fs::write(dir.join("timestamp.txt"), "1577948414").expect("write");
        let from_fs = read_dir(&dir).expect("read_dir");
        let in_memory = from_named_documents(&docs()).expect("db");
        assert_eq!(from_fs, in_memory);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    // ------------------------------------------------------------------
    // Tester round (2026-08-17): reader-contract pins on hostile /
    // degenerate directory content.
    // ------------------------------------------------------------------

    /// Hermetic per-test temp dir (unique name; caller cleans up).
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "appname-lensdb-tester-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    /// PINNED CONTRACT: zero documents (empty dir / no *.xml files) is
    /// Ok-EMPTY — version 0, no records, no warnings — not an error.
    #[test]
    fn empty_input_is_ok_empty_not_an_error() {
        let db = from_named_documents(&[]).expect("empty set is Ok");
        assert_eq!(db.version, 0);
        assert!(db.mounts.is_empty());
        assert!(db.cameras.is_empty());
        assert!(db.lenses.is_empty());
        assert!(db.warnings.is_empty());

        let dir = temp_dir("empty");
        let from_fs = read_dir(&dir).expect("empty dir is Ok");
        assert_eq!(from_fs, db);
        // A dir with only non-XML files is equally empty.
        std::fs::write(dir.join("readme.txt"), "not xml").expect("write");
        assert_eq!(read_dir(&dir).expect("non-xml ignored"), db);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// PINNED CONTRACT (observation flagged in the round log): one
    /// malformed file fails the WHOLE read — all-or-nothing, no
    /// partial database alongside per-file errors.
    #[test]
    fn one_malformed_file_fails_the_whole_read() {
        let d = vec![
            ("a-good.xml".to_owned(), FULL_FIXTURE.to_owned()),
            (
                "b-bad.xml".to_owned(),
                "<lensdatabase version=\"2\">".to_owned(),
            ),
        ];
        match from_named_documents(&d).expect_err("whole read must fail") {
            DbError::Xml { path, .. } => assert_eq!(path, "b-bad.xml"),
            other => panic!("wrong error class: {other:?}"),
        }
        // The good file alone parses — the failure above is strictly
        // the malformed file's doing.
        assert!(from_named_documents(&d[..1]).is_ok());
    }

    /// A non-UTF-8 profile file (windows-1252 bytes, e.g. 0x92 curly
    /// quote) is a typed Utf8 error naming the file — not a panic, not
    /// a lossy parse.
    #[test]
    fn non_utf8_file_is_a_typed_utf8_error_naming_the_file() {
        let dir = temp_dir("cp1252");
        let mut bytes = b"<lensdatabase version=\"2\"><mount><name>O".to_vec();
        bytes.push(0x92); // windows-1252 right single quote
        bytes.extend_from_slice(b"Corp</name></mount></lensdatabase>");
        std::fs::write(dir.join("cp1252.xml"), &bytes).expect("write");
        match read_dir(&dir).expect_err("must fail") {
            DbError::Utf8 { path } => assert!(path.ends_with("cp1252.xml"), "{path}"),
            other => panic!("wrong error class: {other:?}"),
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// Extension matching is case-insensitive: `.XML` files are read.
    #[test]
    fn uppercase_xml_extension_is_read() {
        let dir = temp_dir("upperext");
        std::fs::write(dir.join("A-EXAMPLE.XML"), FULL_FIXTURE).expect("write");
        let db = read_dir(&dir).expect("read_dir");
        assert_eq!(db.lenses.len(), 3);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// Duplicate identities WITHIN one file are surfaced exactly like
    /// cross-file duplicates: warning + first definition wins.
    #[test]
    fn duplicate_identities_within_a_single_file_warn_and_first_wins() {
        let doc = "<lensdatabase version=\"2\">\
                   <camera><maker>A</maker><model>A One</model>\
                   <mount>M</mount><cropfactor>1.5</cropfactor></camera>\
                   <camera><maker>A</maker><model>A One</model>\
                   <mount>M</mount><cropfactor>2.0</cropfactor></camera>\
                   <lens><maker>A</maker><model>A 50mm</model>\
                   <mount>M</mount><cropfactor>1.0</cropfactor></lens>\
                   <lens><maker>a</maker><model>a  50MM</model>\
                   <mount>M</mount><cropfactor>1.0</cropfactor></lens>\
                   </lensdatabase>";
        let db = from_named_documents(&[("one.xml".to_owned(), doc.to_owned())]).expect("db");
        assert_eq!(db.cameras.len(), 1, "in-file duplicate camera collapses");
        assert_eq!(db.cameras[0].crop_factor, 1.5, "first wins");
        assert_eq!(
            db.lenses.len(),
            1,
            "normalized in-file duplicate lens collapses"
        );
        assert_eq!(db.warnings.len(), 2, "{:?}", db.warnings);
        assert!(db.warnings.iter().any(|w| w.contains("duplicate camera")));
        assert!(db.warnings.iter().any(|w| w.contains("duplicate lens")));
        // Both warnings name one.xml as both the duplicate and the
        // first-definition file.
        for w in &db.warnings {
            assert!(w.contains("one.xml"), "{w}");
        }
    }

    /// Valid XML that is not a lens database (wrong root) is a Schema
    /// error naming the file — pinned through the READER so the
    /// user-facing path is covered, not just the model layer.
    #[test]
    fn wrong_root_element_is_a_schema_error_through_the_reader() {
        let d = vec![(
            "notdb.xml".to_owned(),
            "<catalog version=\"2\"><item/></catalog>".to_owned(),
        )];
        match from_named_documents(&d).expect_err("must fail") {
            DbError::Schema { path, msg, .. } => {
                assert_eq!(path, "notdb.xml");
                assert!(msg.contains("expected <lensdatabase>"), "{msg}");
            }
            other => panic!("wrong error class: {other:?}"),
        }
    }

    /// PINNED PROPERTY: the same records distributed DIFFERENTLY across
    /// files merge to an identical database (equal `==` and equal
    /// canonical summary) as long as no identity is duplicated — the
    /// per-file grouping is not observable in the merged result.
    #[test]
    fn redistributing_records_across_files_merges_identically() {
        // Baseline: fixture + second file as shipped.
        let combined = from_named_documents(&docs()).expect("db");
        // Redistribution: second_file()'s mount moves in with the
        // fixture; its camera stands alone in a third file.
        let mount_only = "<lensdatabase version=\"2\">\n\
                          <mount><name>Example FD</name><compat>Generic</compat></mount>\n\
                          </lensdatabase>\n";
        let camera_only = "<lensdatabase version=\"2\">\n\
                           <camera><maker>Othercorp</maker><model>Othercorp Box 100</model>\n\
                           <mount>Example FD</mount><cropfactor>1.5</cropfactor></camera>\n\
                           </lensdatabase>\n";
        let redistributed = from_named_documents(&[
            ("q-mounts.xml".to_owned(), mount_only.to_owned()),
            ("r-example.xml".to_owned(), FULL_FIXTURE.to_owned()),
            ("s-cams.xml".to_owned(), camera_only.to_owned()),
        ])
        .expect("db");
        assert_eq!(combined, redistributed);
        assert_eq!(
            combined.canonical_summary(),
            redistributed.canonical_summary()
        );
    }

    #[test]
    fn missing_directory_is_an_io_error() {
        let err =
            read_dir(Path::new("/nonexistent/appname-lensdb-nowhere")).expect_err("must fail");
        assert!(matches!(err, DbError::Io { .. }));
    }

    /// Pinned golden over the canonical serialization of the synthetic
    /// merge (oracle-first: the shape assertions above pin the content;
    /// this hash pins byte stability).
    #[test]
    fn pinned_golden_canonical_summary_hash() {
        let db = from_named_documents(&docs()).expect("db");
        let hash = fnv1a(db.canonical_summary().as_bytes());
        // Pinned from a real run AFTER the shape oracles above passed
        // (oracle-first discipline; never hand-invented).
        const PINNED_LENSDB_GOLDEN: u64 = 0x0cde_43b1_f2cd_6642;
        assert_eq!(
            hash,
            PINNED_LENSDB_GOLDEN,
            "canonical summary drifted:\n{}",
            db.canonical_summary()
        );
    }

    /// The REAL vendored corpus (read-only external path outside this
    /// build tree — the acquisition staged in `appname-proto`), so the
    /// default suite stays hermetic. Run manually:
    /// `cargo test -p appname-algo -- --ignored`.
    #[test]
    #[ignore = "reads the read-only vendored corpus outside the build tree"]
    fn real_corpus_parses_completely_and_deterministically() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../appname-proto/vendor/lensfun-db/db");
        assert!(
            dir.is_dir(),
            "vendored corpus not found at {} — this ignored test only runs \
             in the full Projects checkout",
            dir.display()
        );
        let db = read_dir(&dir).expect("the whole corpus must parse");
        let db2 = read_dir(&dir).expect("second read");
        assert_eq!(db, db2, "two corpus reads must be identical");
        assert_eq!(db.version, 2);
        // Corpus shape at the census pin (56 profile files).
        assert_eq!(db.mounts.len(), 52, "distinct mount declarations");
        assert!(db.cameras.len() > 1000, "cameras: {}", db.cameras.len());
        assert!(db.lenses.len() > 1400, "lenses: {}", db.lenses.len());
        eprintln!(
            "corpus: version={} mounts={} cameras={} lenses={} warnings={}",
            db.version,
            db.mounts.len(),
            db.cameras.len(),
            db.lenses.len(),
            db.warnings.len()
        );
        for w in &db.warnings {
            eprintln!("corpus warning: {w}");
        }
        eprintln!(
            "corpus canonical summary fnv1a = {:#018x}",
            fnv1a(db.canonical_summary().as_bytes())
        );
    }
}
