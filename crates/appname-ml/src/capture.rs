// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Value capture — the D-48 discipline for class-N model ops.
//!
//! Spec: ES §16.4 rule 5 taxonomy + ALGORITHMS.md §1.4.4 (class N: golden
//! tests pin CPU-backend outputs PER MODEL VERSION; model version is part of
//! the golden key), ALGORITHMS.md §9 ("the *selection command* records the
//! mask raster, never a model re-run"), ES §53.10 / §39.4 ML clause
//! ("replays, actions, and history never re-invoke a model"), D-48.
//!
//! # How replay never re-runs a model
//!
//! A model invocation happens OUTSIDE the command spine: caller stages an
//! input raster, runs a session (both in [`crate::runtime`]), and receives a
//! [`CapturedModelOutput`]. What enters the document is the captured OUTPUT
//! RASTER, ingested as an ordinary content-addressed package asset
//! (`appname_asset_create`, ES §9.2) and applied by an ordinary envelope —
//! exactly the `samples`-assetId mechanism brush.stroke already uses. The
//! journal therefore replays an asset apply, byte-identical, whether or not
//! any model pack is installed. This crate cannot violate that by
//! construction: it has no `appname-core` dependency and no document API.
//!
//! The capture record additionally carries the full golden-key identity so
//! O-QA's class-N goldens can pin (slot, pack, version, artifact hash, EP)
//! → output-hash rows without re-running anything.

use serde::Serialize;

use crate::runtime::{BackendId, ExecutionProvider, RasterPayload, SlotId};
use crate::sha256;
use crate::verify::VerifiedArtifact;

/// The complete record of one model invocation. The `output` raster is the
/// document-bound value; everything else is capture provenance.
#[derive(Debug, Clone)]
pub struct CapturedModelOutput {
    pub slot: SlotId,
    pub pack_id: String,
    pub pack_version: String,
    /// Verified sha256 of the exact model artifact that ran.
    pub artifact_sha256: String,
    pub backend: BackendId,
    pub execution_provider: ExecutionProvider,
    /// Content hash of the staged model input (caller-supplied, from the
    /// engine's content-addressing; recorded verbatim).
    pub input_content_hash: String,
    /// sha256 of `output.data`, computed at capture time.
    pub output_sha256: String,
    pub output: RasterPayload,
}

impl CapturedModelOutput {
    /// Seal a capture record. The only constructor: output hash is computed
    /// here, so a record's `output_sha256` always matches its raster.
    pub fn seal(
        slot: SlotId,
        artifact: &VerifiedArtifact,
        backend: BackendId,
        execution_provider: ExecutionProvider,
        input_content_hash: String,
        output: RasterPayload,
    ) -> CapturedModelOutput {
        let output_sha256 = sha256::hex_digest(&output.data);
        CapturedModelOutput {
            slot,
            pack_id: artifact.pack_id().to_string(),
            pack_version: artifact.pack_version().to_string(),
            artifact_sha256: artifact.sha256().to_string(),
            backend,
            execution_provider,
            input_content_hash,
            output_sha256,
            output,
        }
    }

    /// The class-N golden key (ALGORITHMS.md §1.4.4): pack version and
    /// artifact identity are IN the key, so a pack upgrade can never collide
    /// with an older golden row.
    pub fn golden_key(&self) -> GoldenKey {
        GoldenKey {
            slot: self.slot.as_str(),
            pack_id: self.pack_id.clone(),
            pack_version: self.pack_version.clone(),
            artifact_sha256: self.artifact_sha256.clone(),
            execution_provider: self.execution_provider.as_str(),
        }
    }
}

/// Golden-key identity for class-N golden corpora (O-QA surface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GoldenKey {
    pub slot: &'static str,
    pub pack_id: String,
    pub pack_version: String,
    pub artifact_sha256: String,
    pub execution_provider: &'static str,
}

impl GoldenKey {
    /// Canonical single-line form, stable by construction (used as the
    /// golden corpus row key).
    pub fn canonical(&self) -> String {
        format!(
            "{}/{}@{}/{}/{}",
            self.slot,
            self.pack_id,
            self.pack_version,
            self.artifact_sha256,
            self.execution_provider
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PackRegistry;
    use crate::runtime::RasterFormat;
    use crate::verify::load_verified;

    /// Build a real VerifiedArtifact through the load gate (no bypass
    /// constructor exists — this test doubles as proof of that posture).
    fn verified_fixture() -> (VerifiedArtifact, std::path::PathBuf) {
        let payload = b"capture-fixture-model-bytes";
        let sha = sha256::hex_digest(payload);
        let json = format!(
            r#"{{
  "packs": [{{
    "id": "capture-fixture",
    "name": "Capture fixture",
    "version": "3.2",
    "task": "matting",
    "source": {{
      "repo": "https://example.invalid/repo",
      "revision": "0000000000000000000000000000000000000000",
      "files": [{{"name": "w.ckpt", "bytes": 27, "sha256": "{src}"}}],
      "code_license": "Apache-2.0",
      "weights_license": "Apache-2.0",
      "license_url": "https://example.invalid/LICENSE"
    }},
    "converted": {{
      "recipe": "none — upstream artifact used directly",
      "files": [{{"path": "m/w.onnx", "sha256": "{sha}"}}]
    }},
    "distribution": {{"mode": "bundled-optional-pack", "redistribution_allowed": true}},
    "legal": {{"rights_ref": "test"}},
    "revocable": false
  }}]
}}"#,
            src = "a".repeat(64),
            sha = sha,
        );
        let reg = PackRegistry::parse(&json).unwrap();
        let root = std::env::temp_dir().join(format!("appname-ml-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("m")).unwrap();
        std::fs::write(root.join("m/w.onnx"), payload).unwrap();
        let pack = reg.pack("capture-fixture").unwrap();
        let art = load_verified(pack, &pack.converted.files[0], &root).unwrap();
        (art, root)
    }

    #[test]
    fn seal_records_output_hash_and_golden_key() {
        let (art, root) = verified_fixture();
        let output = RasterPayload {
            width: 2,
            height: 2,
            format: RasterFormat::MaskF32Le,
            data: vec![0u8; 16],
        };
        let cap = CapturedModelOutput::seal(
            SlotId::PortraitMatte,
            &art,
            BackendId("test-backend"),
            ExecutionProvider::Cpu,
            "blake3:deadbeef".into(),
            output.clone(),
        );
        assert_eq!(cap.output_sha256, sha256::hex_digest(&output.data));
        assert_eq!(cap.pack_version, "3.2");
        let key = cap.golden_key().canonical();
        assert_eq!(
            key,
            format!("portrait_matte/capture-fixture@3.2/{}/cpu", art.sha256())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn golden_key_serialization_is_deterministic() {
        let k = GoldenKey {
            slot: "portrait_matte",
            pack_id: "matting-modnet".into(),
            pack_version: "1.1".into(),
            artifact_sha256: "f".repeat(64),
            execution_provider: "cpu",
        };
        let a = serde_json::to_string(&k).unwrap();
        let b = serde_json::to_string(&k).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            k.canonical(),
            format!("portrait_matte/matting-modnet@1.1/{}/cpu", "f".repeat(64))
        );
    }
}
