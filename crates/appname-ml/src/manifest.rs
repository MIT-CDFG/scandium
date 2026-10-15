// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Pack manifest pipeline — parse + discipline validation.
//!
//! Spec: `contracts/schemas/ai-model-pack.v1.json` (appname.aiModelPack.v1,
//! ES Part 53 §53.9, ALGORITHMS.md §1.4.3, review ML-001); D-45/D-49.
//!
//! The staged registry lives at `packs/packs.json`, fed EXCLUSIVELY from
//! `research/pins/models.v5.6-candidate.yaml` (plan §1 rule 8 — no pin is
//! ever invented or re-derived here; this module enforces shape and
//! discipline over values it does not originate). Every schema
//! `additionalProperties: false` is mirrored with `deny_unknown_fields`.

use serde::{Deserialize, Serialize};

/// Registry root — schema root shape `{ "packs": [...] }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRegistry {
    pub packs: Vec<Pack>,
}

/// One pack record (supply-chain row).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub version: String,
    pub task: Task,
    pub source: Source,
    pub converted: Converted,
    pub distribution: Distribution,
    pub legal: Legal,
    #[serde(rename = "provenanceNote", skip_serializing_if = "Option::is_none")]
    pub provenance_note: Option<String>,
    pub revocable: bool,
    #[serde(rename = "minRamMb", skip_serializing_if = "Option::is_none")]
    pub min_ram_mb: Option<u64>,
    #[serde(rename = "gpuTierMin", skip_serializing_if = "Option::is_none")]
    pub gpu_tier_min: Option<String>,
    #[serde(rename = "determinismClass", skip_serializing_if = "Option::is_none")]
    pub determinism_class: Option<DeterminismClass>,
}

// Wire names: the schema's four camelCase fields carry explicit renames
// above; every other schema property is snake_case and maps 1:1.
impl Pack {
    /// License + provenance display surface (Part 53 §53.9: shown BEFORE
    /// download and in the Downloads/About UI; D-49 "license shown in UI").
    pub fn license_display(&self) -> LicenseDisplay {
        let weights_lower = self.source.weights_license.to_ascii_lowercase();
        LicenseDisplay {
            pack_id: self.id.clone(),
            pack_name: self.name.clone(),
            pack_version: self.version.clone(),
            code_license: self.source.code_license.clone(),
            weights_license: self.source.weights_license.clone(),
            license_url: self.source.license_url.clone(),
            upstream_repo: self.source.repo.clone(),
            upstream_revision: self.source.revision.clone(),
            // GeoCalib gate recorded now (§3.13): CC-BY-family weights MUST
            // render attribution in the UI proper, not only in the manifest.
            attribution_required_in_ui: weights_lower.contains("cc-by"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    Inpaint,
    SegmentPromptable,
    BackgroundRemoval,
    Matting,
    Sky,
    Denoise,
    SuperResolution,
    Deblur,
    JpegRepair,
    FaceRestore,
    Colorize,
    OldPhoto,
    Dehaze,
    LowLight,
    Harmonize,
    AutoEnhance,
    Calibration,
    FeaturesMatching,
    OpticalFlow,
    GenerativeEdit,
    TextLlm,
}

/// Determinism class — models are class N, full stop (ALGORITHMS.md §1.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeterminismClass {
    N,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub repo: String,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_verified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    pub files: Vec<SourceFile>,
    pub code_license: String,
    pub weights_license: String,
    pub license_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub training_data_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    pub name: String,
    pub bytes: PendingOr<u64>,
    pub sha256: Sha256Pin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Converted {
    pub recipe: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversion_tool_license: Option<String>,
    pub files: Vec<ConvertedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertedFile {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization: Option<Quantization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<PendingOr<u64>>,
    pub sha256: Sha256Stamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantization {
    Fp32,
    Fp16,
    Int8,
    Q4,
    Q4KM,
    Q5,
    Q8,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    pub mode: DistributionMode,
    pub redistribution_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lawful_fallback: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline_archive: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistributionMode {
    #[serde(rename = "bundled-optional-pack")]
    BundledOptionalPack,
    #[serde(rename = "user-fetched")]
    UserFetched,
    #[serde(rename = "not-distributed")]
    NotDistributed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Legal {
    pub rights_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_scan: Option<SecurityScan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityScan {
    Complete,
    Pending,
}

/// Schema `oneOf: [integer, const "pending"]` — pending values are completed
/// only by `tools/fetch_model_pins.py`, never by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PendingOr<T> {
    Value(T),
    Sentinel(PendingSentinel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingSentinel {
    #[serde(rename = "pending")]
    Pending,
}

/// Source-file hash pin: 64 lowercase hex, or `pending`.
/// (Untagged order matters: the exact-string sentinel MUST be tried before
/// the catch-all Hex(String) arm.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Sha256Pin {
    Sentinel(PendingSentinel),
    Hex(String),
}

/// Converted-file hash stamp: 64 lowercase hex (stamped BY the packaging
/// pipeline), or `pending-packaging` (gates at beta, V15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Sha256Stamp {
    Sentinel(PendingPackagingSentinel),
    Hex(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingPackagingSentinel {
    #[serde(rename = "pending-packaging")]
    PendingPackaging,
}

/// License + provenance display record (UI surface; D-49).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LicenseDisplay {
    pub pack_id: String,
    pub pack_name: String,
    pub pack_version: String,
    pub code_license: String,
    pub weights_license: String,
    pub license_url: String,
    pub upstream_repo: String,
    pub upstream_revision: String,
    pub attribution_required_in_ui: bool,
}

/// Manifest discipline violations (parse succeeds structurally; these are the
/// registry-level typed rejections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// JSON parse / shape failure (message from serde).
    Malformed(String),
    /// id fails `^[a-z][a-z0-9_.-]*$`.
    BadId {
        id: String,
    },
    DuplicateId {
        id: String,
    },
    /// `revocable` MUST be `false` (schema `const: false`; packs are never
    /// remotely revoked and never phone home).
    RevocablePack {
        id: String,
    },
    /// sha256 field is neither 64 lowercase hex nor its sentinel.
    BadHash {
        id: String,
        field: String,
        value: String,
    },
    /// A `bytes` field carries an integer below the schema's `minimum: 1`
    /// (slice-2 advisory: serde's u64 already refuses negatives; zero is
    /// refused here so the Rust validation mirrors the schema exactly).
    BadBytes {
        id: String,
        field: String,
        value: u64,
    },
    /// source.files must carry at least one file (schema minItems 1).
    NoSourceFiles {
        id: String,
    },
    /// converted.files empty — nothing for the runtime to ever load.
    NoConvertedFiles {
        id: String,
    },
    /// `redistribution_allowed: false` requires a prose `lawful_fallback`.
    MissingLawfulFallback {
        id: String,
    },
    /// weights_license empty — D-45 display and allowlist check impossible.
    MissingWeightsLicense {
        id: String,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Malformed(m) => write!(f, "manifest malformed: {m}"),
            ManifestError::BadId { id } => write!(f, "pack id violates schema pattern: {id:?}"),
            ManifestError::DuplicateId { id } => write!(f, "duplicate pack id: {id:?}"),
            ManifestError::RevocablePack { id } => {
                write!(
                    f,
                    "pack {id:?} declares revocable != false (forbidden by schema const)"
                )
            }
            ManifestError::BadHash { id, field, value } => {
                write!(
                    f,
                    "pack {id:?} field {field}: not 64-lowercase-hex nor sentinel: {value:?}"
                )
            }
            ManifestError::BadBytes { id, field, value } => {
                write!(
                    f,
                    "pack {id:?} field {field}: bytes = {value} below schema minimum 1"
                )
            }
            ManifestError::NoSourceFiles { id } => write!(f, "pack {id:?} has no source files"),
            ManifestError::NoConvertedFiles { id } => {
                write!(f, "pack {id:?} has no converted files")
            }
            ManifestError::MissingLawfulFallback { id } => {
                write!(
                    f,
                    "pack {id:?}: redistribution_allowed=false without lawful_fallback"
                )
            }
            ManifestError::MissingWeightsLicense { id } => {
                write!(f, "pack {id:?}: empty weights_license")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

fn is_hex64_lower(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn id_ok(id: &str) -> bool {
    let mut bytes = id.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_lowercase() => {}
        _ => return false,
    }
    bytes.all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.' || b == b'-'
    })
}

impl PackRegistry {
    /// Parse + validate a registry JSON document.
    pub fn parse(json: &str) -> Result<PackRegistry, ManifestError> {
        let reg: PackRegistry =
            serde_json::from_str(json).map_err(|e| ManifestError::Malformed(e.to_string()))?;
        reg.validate()?;
        Ok(reg)
    }

    /// Discipline validation beyond serde shape.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let mut seen: Vec<&str> = Vec::new();
        for p in &self.packs {
            if !id_ok(&p.id) {
                return Err(ManifestError::BadId { id: p.id.clone() });
            }
            if seen.contains(&p.id.as_str()) {
                return Err(ManifestError::DuplicateId { id: p.id.clone() });
            }
            seen.push(&p.id);
            if p.revocable {
                return Err(ManifestError::RevocablePack { id: p.id.clone() });
            }
            if p.source.files.is_empty() {
                return Err(ManifestError::NoSourceFiles { id: p.id.clone() });
            }
            if p.converted.files.is_empty() {
                return Err(ManifestError::NoConvertedFiles { id: p.id.clone() });
            }
            if p.source.weights_license.trim().is_empty() {
                return Err(ManifestError::MissingWeightsLicense { id: p.id.clone() });
            }
            if !p.distribution.redistribution_allowed
                && p.distribution
                    .lawful_fallback
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                return Err(ManifestError::MissingLawfulFallback { id: p.id.clone() });
            }
            for sf in &p.source.files {
                if let Sha256Pin::Hex(h) = &sf.sha256 {
                    if !is_hex64_lower(h) {
                        return Err(ManifestError::BadHash {
                            id: p.id.clone(),
                            field: format!("source.files[{}].sha256", sf.name),
                            value: h.clone(),
                        });
                    }
                }
                // Schema `bytes: { minimum: 1 }` mirror (slice-2 advisory).
                if let PendingOr::Value(0) = sf.bytes {
                    return Err(ManifestError::BadBytes {
                        id: p.id.clone(),
                        field: format!("source.files[{}].bytes", sf.name),
                        value: 0,
                    });
                }
            }
            for cf in &p.converted.files {
                if let Sha256Stamp::Hex(h) = &cf.sha256 {
                    if !is_hex64_lower(h) {
                        return Err(ManifestError::BadHash {
                            id: p.id.clone(),
                            field: format!("converted.files[{}].sha256", cf.path),
                            value: h.clone(),
                        });
                    }
                }
                // Same `minimum: 1` mirror on the optional converted bytes.
                if let Some(PendingOr::Value(0)) = cf.bytes {
                    return Err(ManifestError::BadBytes {
                        id: p.id.clone(),
                        field: format!("converted.files[{}].bytes", cf.path),
                        value: 0,
                    });
                }
            }
        }
        Ok(())
    }

    pub fn pack(&self, id: &str) -> Option<&Pack> {
        self.packs.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pack_json(id: &str, revocable: bool, converted_sha: &str) -> String {
        format!(
            r#"{{
  "packs": [{{
    "id": "{id}",
    "name": "Test pack",
    "version": "1.1",
    "task": "matting",
    "source": {{
      "repo": "https://example.invalid/repo",
      "revision": "0000000000000000000000000000000000000000",
      "files": [{{"name": "w.ckpt", "bytes": 10, "sha256": "{src}"}}],
      "code_license": "Apache-2.0",
      "weights_license": "Apache-2.0",
      "license_url": "https://example.invalid/LICENSE"
    }},
    "converted": {{
      "recipe": "none — upstream artifact used directly",
      "files": [{{"path": "models/test/w.onnx", "quantization": "fp16", "sha256": "{conv}"}}]
    }},
    "distribution": {{"mode": "bundled-optional-pack", "redistribution_allowed": true}},
    "legal": {{"rights_ref": "test"}},
    "revocable": {revocable}
  }}]
}}"#,
            id = id,
            revocable = revocable,
            src = "a".repeat(64),
            conv = converted_sha,
        )
    }

    #[test]
    fn parses_minimal_valid() {
        let reg = PackRegistry::parse(&minimal_pack_json("test-pack", false, &"b".repeat(64)))
            .expect("valid registry");
        assert_eq!(reg.packs.len(), 1);
        assert_eq!(reg.packs[0].task, Task::Matting);
    }

    #[test]
    fn pending_packaging_sentinel_parses() {
        let reg = PackRegistry::parse(&minimal_pack_json("test-pack", false, "pending-packaging"))
            .expect("sentinel is schema-legal");
        assert_eq!(
            reg.packs[0].converted.files[0].sha256,
            Sha256Stamp::Sentinel(PendingPackagingSentinel::PendingPackaging)
        );
    }

    #[test]
    fn revocable_true_refused() {
        let err = PackRegistry::parse(&minimal_pack_json("test-pack", true, &"b".repeat(64)))
            .unwrap_err();
        assert_eq!(
            err,
            ManifestError::RevocablePack {
                id: "test-pack".into()
            }
        );
    }

    #[test]
    fn bad_hash_refused() {
        // Uppercase hex is NOT the wire form.
        let bad = format!("B{}", "b".repeat(63));
        let err = PackRegistry::parse(&minimal_pack_json("test-pack", false, &bad)).unwrap_err();
        assert!(matches!(err, ManifestError::BadHash { .. }), "{err:?}");
    }

    #[test]
    fn bad_id_refused() {
        let err = PackRegistry::parse(&minimal_pack_json("Test-Pack", false, &"b".repeat(64)))
            .unwrap_err();
        assert!(matches!(err, ManifestError::BadId { .. }), "{err:?}");
    }

    #[test]
    fn unknown_field_refused() {
        let json = minimal_pack_json("test-pack", false, &"b".repeat(64)).replace(
            "\"revocable\": false",
            "\"revocable\": false, \"phone_home\": true",
        );
        let err = PackRegistry::parse(&json).unwrap_err();
        assert!(matches!(err, ManifestError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn no_fallback_without_redistribution_refused() {
        let json = minimal_pack_json("test-pack", false, &"b".repeat(64)).replace(
            "\"redistribution_allowed\": true",
            "\"redistribution_allowed\": false",
        );
        let err = PackRegistry::parse(&json).unwrap_err();
        assert_eq!(
            err,
            ManifestError::MissingLawfulFallback {
                id: "test-pack".into()
            }
        );
    }

    /// Slice-2 advisory close-out: the schema's `bytes` `minimum: 1` is
    /// mirrored in Rust validation on both file lists.
    #[test]
    fn zero_bytes_refused_pending_sentinel_allowed() {
        // source.files[].bytes: 0 refused, "pending" sentinel still fine.
        let json = minimal_pack_json("test-pack", false, &"b".repeat(64))
            .replace("\"bytes\": 10", "\"bytes\": 0");
        let err = PackRegistry::parse(&json).unwrap_err();
        assert_eq!(
            err,
            ManifestError::BadBytes {
                id: "test-pack".into(),
                field: "source.files[w.ckpt].bytes".into(),
                value: 0
            }
        );
        let json = minimal_pack_json("test-pack", false, &"b".repeat(64))
            .replace("\"bytes\": 10", "\"bytes\": \"pending\"");
        assert!(
            PackRegistry::parse(&json).is_ok(),
            "sentinel is schema-legal"
        );

        // converted.files[].bytes (optional field): 0 refused when present.
        let json = minimal_pack_json("test-pack", false, &"b".repeat(64)).replace(
            "\"quantization\": \"fp16\",",
            "\"quantization\": \"fp16\", \"bytes\": 0,",
        );
        let err = PackRegistry::parse(&json).unwrap_err();
        assert!(
            matches!(err, ManifestError::BadBytes { ref field, .. }
                if field == "converted.files[models/test/w.onnx].bytes"),
            "{err:?}"
        );
    }

    #[test]
    fn cc_by_weights_flag_attribution() {
        let json = minimal_pack_json("test-pack", false, &"b".repeat(64)).replace(
            "\"weights_license\": \"Apache-2.0\"",
            "\"weights_license\": \"CC-BY-4.0\"",
        );
        let reg = PackRegistry::parse(&json).unwrap();
        assert!(reg.packs[0].license_display().attribution_required_in_ui);
    }
}
