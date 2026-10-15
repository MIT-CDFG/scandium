// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.5 worked example `vector/{pathId}.json` (`"schema":
//! "appname.vector.v1"`; straight edges = collinear one-third-handle
//! cubics; `p0` MUST equal previous `p1`, readers MAY tolerate ≤ 1e−6 px
//! drift and re-weld); §2.8 serialization conventions (camelCase, schema
//! discriminator, finite numbers, unknown-enum preservation via `Maybe`);
//! document.v1.json (`pathRef` → this sidecar, shape layers and vector
//! masks).
//!
//! VERIFIED against contracts (lane-log schema-gap flags, cropRect-class):
//! contracts/schemas/ has NO JSON Schema for `appname.vector.v1` — the
//! §7.5 worked example is the only published shape, so this module follows
//! it verbatim. The example writes `"fillRule": "nonzero"` while §20.4's
//! `appname.shape.v1` custom-shape example writes `"non_zero"` (§2.8
//! snake_case rule) — an in-spec inconsistency. Pin here: EMIT the §7.5
//! sidecar spelling (`"nonzero"`/`"evenodd"`), ACCEPT both spellings on
//! read; unknown strings are preserved for round-trip via core's `Maybe`
//! and reported as a `LoadNote` (never a silent default). Both flags are
//! logged for the fold-in register.

use appname_core::{Maybe, PathId};
use serde::{Deserialize, Serialize};

use crate::geom::Vec2;
use crate::path::{CubicSeg, FillRule, PathError, SubPath, VectorPath};

/// §7.5 schema discriminator.
pub const SIDECAR_SCHEMA: &str = "appname.vector.v1";

/// Non-fatal reader observations (§2.8 lenient-reader side).
#[derive(Clone, Debug, PartialEq)]
pub enum LoadNote {
    /// `fillRule` value was not a known spelling; preserved raw and the
    /// in-memory path defaults to `nonzero` (§20.1 default rule for
    /// unknown-rule rendering — deterministic, and the raw value round-
    /// trips back out unchanged).
    UnknownFillRule { raw: String },
    /// Contiguity drift ≤ 1e−6 was re-welded (count of welded joints).
    Rewelded { joints: usize },
}

impl Serialize for FillRule {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // §7.5 worked-example spellings.
        match self {
            FillRule::NonZero => s.serialize_str("nonzero"),
            FillRule::EvenOdd => s.serialize_str("evenodd"),
        }
    }
}

impl<'de> Deserialize<'de> for FillRule {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        match raw.as_str() {
            // §7.5 spelling + the §20.4/§2.8 snake_case spelling.
            "nonzero" | "non_zero" => Ok(FillRule::NonZero),
            "evenodd" | "even_odd" => Ok(FillRule::EvenOdd),
            other => Err(serde::de::Error::custom(format!(
                "unknown fillRule: {other}"
            ))),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SegWire {
    p0: [f64; 2],
    c1: [f64; 2],
    c2: [f64; 2],
    p1: [f64; 2],
}

#[derive(Serialize, Deserialize)]
struct SubPathWire {
    closed: bool,
    segments: Vec<SegWire>,
}

/// The full sidecar object (§7.5). `fillRule` rides core's `Maybe` so an
/// unknown future rule survives a round trip byte-preserved (§2.8).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarWire {
    schema: String,
    fill_rule: Maybe<FillRule>,
    subpaths: Vec<SubPathWire>,
}

fn to_wire(path: &VectorPath) -> SidecarWire {
    SidecarWire {
        schema: SIDECAR_SCHEMA.to_owned(),
        fill_rule: Maybe::Known(path.fill_rule),
        subpaths: path
            .subpaths
            .iter()
            .map(|s| SubPathWire {
                closed: s.closed,
                segments: s
                    .segments
                    .iter()
                    .map(|g| SegWire {
                        p0: [g.p0.x, g.p0.y],
                        c1: [g.c1.x, g.c1.y],
                        c2: [g.c2.x, g.c2.y],
                        p1: [g.p1.x, g.p1.y],
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Serialize a path to its `vector/{pathId}.json` sidecar bytes. Strict
/// WRITER: the path must validate (§20.1 invariants; finite numbers per
/// the document.v1 x-spec-note — never emit NaN/∞).
pub fn path_to_sidecar_json(path: &VectorPath) -> Result<Vec<u8>, PathError> {
    path.validate()?;
    serde_json::to_vec_pretty(&to_wire(path)).map_err(|e| PathError::Json {
        message: e.to_string(),
    })
}

/// Parse sidecar bytes into a normalized path (lenient READER: §7.5
/// re-weld ≤ 1e−6, §20.1 degenerate removal; beyond-tolerance drift and
/// non-finite numbers are typed errors). `id` comes from the package entry
/// name (the sidecar body carries no id — §7.5 worked example).
pub fn path_from_sidecar_json(
    id: PathId,
    bytes: &[u8],
) -> Result<(VectorPath, Vec<LoadNote>), PathError> {
    let wire: SidecarWire = serde_json::from_slice(bytes).map_err(|e| PathError::Json {
        message: e.to_string(),
    })?;
    if wire.schema != SIDECAR_SCHEMA {
        return Err(PathError::SchemaMismatch { got: wire.schema });
    }
    let mut notes = Vec::new();
    let fill_rule = match wire.fill_rule.known() {
        Some(r) => *r,
        None => {
            let raw = match &wire.fill_rule {
                Maybe::Unknown(v) => v.to_string(),
                Maybe::Known(_) => unreachable!(),
            };
            notes.push(LoadNote::UnknownFillRule { raw });
            FillRule::NonZero
        }
    };
    let mut path = VectorPath::new(id, fill_rule);
    for sw in wire.subpaths {
        path.subpaths.push(SubPath {
            closed: sw.closed,
            segments: sw
                .segments
                .into_iter()
                .map(|g| {
                    CubicSeg::new(
                        Vec2::new(g.p0[0], g.p0[1]),
                        Vec2::new(g.c1[0], g.c1[1]),
                        Vec2::new(g.c2[0], g.c2[1]),
                        Vec2::new(g.p1[0], g.p1[1]),
                    )
                })
                .collect(),
        });
    }
    // Count joints the weld pass will touch (for the note) BEFORE
    // normalize rewrites them.
    let mut drifted = 0usize;
    for sub in &path.subpaths {
        for w in sub.segments.windows(2) {
            if w[0].p1 != w[1].p0 {
                drifted += 1;
            }
        }
        if sub.closed {
            if let (Some(f), Some(l)) = (sub.segments.first(), sub.segments.last()) {
                if l.p1 != f.p0 {
                    drifted += 1;
                }
            }
        }
    }
    path.normalize()?;
    if drifted > 0 {
        notes.push(LoadNote::Rewelded { joints: drifted });
    }
    Ok((path, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §7.5 worked example, verbatim (rounded rect (10,10)–(110,70),
    /// radius 8, κ ≈ 0.5523 → handle offset ≈ 4.42).
    const WORKED_EXAMPLE: &str = r#"{
  "schema": "appname.vector.v1",
  "fillRule": "nonzero",
  "subpaths": [
    { "closed": true, "segments": [
      { "p0": [18.0, 10.0],  "c1": [46.0, 10.0],   "c2": [74.0, 10.0],   "p1": [102.0, 10.0] },
      { "p0": [102.0, 10.0], "c1": [106.42, 10.0], "c2": [110.0, 13.58], "p1": [110.0, 18.0] },
      { "p0": [110.0, 18.0], "c1": [110.0, 32.67], "c2": [110.0, 47.33], "p1": [110.0, 62.0] },
      { "p0": [110.0, 62.0], "c1": [110.0, 66.42], "c2": [106.42, 70.0], "p1": [102.0, 70.0] },
      { "p0": [102.0, 70.0], "c1": [74.0, 70.0],   "c2": [46.0, 70.0],   "p1": [18.0, 70.0] },
      { "p0": [18.0, 70.0],  "c1": [13.58, 70.0],  "c2": [10.0, 66.42],  "p1": [10.0, 62.0] },
      { "p0": [10.0, 62.0],  "c1": [10.0, 47.33],  "c2": [10.0, 32.67],  "p1": [10.0, 18.0] },
      { "p0": [10.0, 18.0],  "c1": [10.0, 13.58],  "c2": [13.58, 10.0],  "p1": [18.0, 10.0] }
    ] }
  ]
}"#;

    #[test]
    fn worked_example_loads_validates_and_round_trips() {
        let (path, notes) =
            path_from_sidecar_json(PathId::nil(), WORKED_EXAMPLE.as_bytes()).unwrap();
        assert!(notes.is_empty());
        assert_eq!(path.fill_rule, FillRule::NonZero);
        assert_eq!(path.subpaths.len(), 1);
        assert!(path.subpaths[0].closed);
        assert_eq!(path.subpaths[0].segments.len(), 8);
        assert_eq!(path.validate(), Ok(()));

        // Round trip: emit, re-load, structurally identical.
        let bytes = path_to_sidecar_json(&path).unwrap();
        let (again, notes2) = path_from_sidecar_json(PathId::nil(), &bytes).unwrap();
        assert!(notes2.is_empty());
        assert_eq!(path, again);

        // Emitted JSON uses the §7.5 spellings and the schema tag.
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"appname.vector.v1\""));
        assert!(text.contains("\"nonzero\""));
    }

    #[test]
    fn snake_case_spelling_accepted_on_read() {
        let json = WORKED_EXAMPLE.replace("\"nonzero\"", "\"non_zero\"");
        let (path, notes) = path_from_sidecar_json(PathId::nil(), json.as_bytes()).unwrap();
        assert!(notes.is_empty());
        assert_eq!(path.fill_rule, FillRule::NonZero);
        let eo = WORKED_EXAMPLE.replace("\"nonzero\"", "\"even_odd\"");
        let (path, _) = path_from_sidecar_json(PathId::nil(), eo.as_bytes()).unwrap();
        assert_eq!(path.fill_rule, FillRule::EvenOdd);
    }

    #[test]
    fn unknown_fill_rule_preserved_and_noted_never_silent() {
        let json = WORKED_EXAMPLE.replace("\"nonzero\"", "\"quadtree\"");
        let (path, notes) = path_from_sidecar_json(PathId::nil(), json.as_bytes()).unwrap();
        assert_eq!(path.fill_rule, FillRule::NonZero); // deterministic default
        assert_eq!(
            notes,
            vec![LoadNote::UnknownFillRule {
                raw: "\"quadtree\"".into()
            }]
        );
    }

    #[test]
    fn reader_rewelds_within_tolerance_and_notes_it() {
        let json = WORKED_EXAMPLE.replace("\"p0\": [102.0, 10.0]", "\"p0\": [102.0000001, 10.0]");
        let (path, notes) = path_from_sidecar_json(PathId::nil(), json.as_bytes()).unwrap();
        assert_eq!(path.validate(), Ok(()));
        assert!(notes.contains(&LoadNote::Rewelded { joints: 1 }));
    }

    #[test]
    fn reader_rejects_drift_beyond_tolerance_and_bad_schema() {
        let json = WORKED_EXAMPLE.replace("\"p0\": [102.0, 10.0]", "\"p0\": [102.001, 10.0]");
        assert!(matches!(
            path_from_sidecar_json(PathId::nil(), json.as_bytes()),
            Err(PathError::Discontiguous { .. })
        ));
        let json = WORKED_EXAMPLE.replace("appname.vector.v1", "appname.vector.v9");
        assert!(matches!(
            path_from_sidecar_json(PathId::nil(), json.as_bytes()),
            Err(PathError::SchemaMismatch { .. })
        ));
    }

    #[test]
    fn writer_refuses_invalid_paths() {
        let (mut path, _) =
            path_from_sidecar_json(PathId::nil(), WORKED_EXAMPLE.as_bytes()).unwrap();
        path.subpaths[0].segments[3].p0.x += 0.5;
        assert!(matches!(
            path_to_sidecar_json(&path),
            Err(PathError::Discontiguous { .. })
        ));
    }
}
