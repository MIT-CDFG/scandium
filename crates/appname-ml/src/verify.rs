// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Load-time artifact verification — the D-49 gate.
//!
//! Spec: ES Part 53 §53.9 ("hash verification before first load"),
//! ALGORITHMS.md §24 ("packs verify sha256 before load"), D-49.
//!
//! A backend session can only be constructed from a [`VerifiedArtifact`],
//! and a `VerifiedArtifact` exists only on the success path of
//! [`load_verified`] — hash mismatch, unpinned conversion stamps,
//! distribution-gated packs, and path escapes are all TYPED REFUSALS. There
//! is no bypass constructor (the struct's fields are read-only outside this
//! module and its only constructor is private).
//!
//! Containment is enforced twice (slice-2 advisory close-out): LEXICALLY
//! (absolute paths, `..`, and non-normal components refused before any
//! I/O) and PHYSICALLY (the resolved path is canonicalized — symlinks
//! followed — and must remain under the canonicalized packs root; a
//! symlink inside the root pointing outside it is a typed
//! [`PackRefusal::UnsafeArtifactPath`]). Symlinks that stay WITHIN the
//! root are permitted. This lands ahead of the ES §53.9 archive-install
//! wave, whose extraction step must uphold the same property.
//!
//! Offline default: this module takes local paths only and never
//! dereferences any URL. `download_url` in manifests is provenance metadata.

use std::path::{Component, Path, PathBuf};

use crate::manifest::{ConvertedFile, DistributionMode, Pack, Sha256Stamp};
use crate::sha256;

/// A pack artifact whose bytes hashed to the manifest pin. The ONLY handle a
/// model backend accepts.
#[derive(Debug, Clone)]
pub struct VerifiedArtifact {
    pack_id: String,
    pack_version: String,
    path: PathBuf,
    sha256: String,
    bytes: Vec<u8>,
}

impl VerifiedArtifact {
    pub fn pack_id(&self) -> &str {
        &self.pack_id
    }
    pub fn pack_version(&self) -> &str {
        &self.pack_version
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// The verified (manifest-pinned) sha256, lowercase hex — this value
    /// joins the class-N golden key (ALGORITHMS.md §1.4.4).
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Typed refusals of the load gate. Every variant is terminal for the load —
/// callers surface it; nothing "loads anyway".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackRefusal {
    /// Converted artifact hash is `pending-packaging` — the packaging
    /// pipeline has not stamped it, so there is nothing to verify against.
    /// (V15: red at beta. The pins feed carries no ready ONNX artifact for
    /// this file; conversion happens under a recorded basis, never here.)
    ConversionUnpinned { pack_id: String, path: String },
    /// The bytes on disk do not hash to the manifest pin.
    HashMismatch {
        pack_id: String,
        path: String,
        expected: String,
        actual: String,
    },
    /// Artifact file absent under the packs root.
    MissingArtifact { pack_id: String, path: String },
    /// Manifest path escapes the packs root (absolute, `..`, or non-normal
    /// components) — refused before any I/O.
    UnsafeArtifactPath { pack_id: String, path: String },
    /// `distribution.mode: not-distributed` — the MI-GAN-class gate: the
    /// registry may describe such a pack, the loader refuses it.
    DistributionGated { pack_id: String },
}

impl std::fmt::Display for PackRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackRefusal::ConversionUnpinned { pack_id, path } => write!(
                f,
                "pack {pack_id:?}: converted artifact {path:?} is pending-packaging — no pinned hash, refusing load"
            ),
            PackRefusal::HashMismatch { pack_id, path, expected, actual } => write!(
                f,
                "pack {pack_id:?}: artifact {path:?} sha256 mismatch (manifest {expected}, file {actual}) — refusing load"
            ),
            PackRefusal::MissingArtifact { pack_id, path } => {
                write!(f, "pack {pack_id:?}: artifact {path:?} not present under packs root")
            }
            PackRefusal::UnsafeArtifactPath { pack_id, path } => {
                write!(f, "pack {pack_id:?}: artifact path {path:?} escapes the packs root")
            }
            PackRefusal::DistributionGated { pack_id } => {
                write!(f, "pack {pack_id:?} is not-distributed — loader refuses (lawful fallback per manifest)")
            }
        }
    }
}

impl std::error::Error for PackRefusal {}

/// Loadability of one converted file, decided from the manifest alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    /// A pinned hash exists; [`load_verified`] may be attempted.
    Verifiable { path: String, sha256: String },
    /// Typed refusal without touching the filesystem.
    Refused(PackRefusal),
}

/// Manifest-only loadability check (no I/O).
pub fn load_state(pack: &Pack, file: &ConvertedFile) -> LoadState {
    if pack.distribution.mode == DistributionMode::NotDistributed {
        return LoadState::Refused(PackRefusal::DistributionGated {
            pack_id: pack.id.clone(),
        });
    }
    match &file.sha256 {
        Sha256Stamp::Hex(h) => LoadState::Verifiable {
            path: file.path.clone(),
            sha256: h.clone(),
        },
        Sha256Stamp::Sentinel(_) => LoadState::Refused(PackRefusal::ConversionUnpinned {
            pack_id: pack.id.clone(),
            path: file.path.clone(),
        }),
    }
}

/// True iff every converted file of the pack is `Verifiable` (and the pack is
/// distributable) — the "installable" predicate for UI surfaces.
pub fn pack_loadable(pack: &Pack) -> bool {
    pack.converted
        .files
        .iter()
        .all(|f| matches!(load_state(pack, f), LoadState::Verifiable { .. }))
}

fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel_path = Path::new(rel);
    let mut out = root.to_path_buf();
    let mut pushed = false;
    for comp in rel_path.components() {
        match comp {
            Component::Normal(c) => {
                out.push(c);
                pushed = true;
            }
            // Absolute prefixes, `..`, `.`, drive prefixes: all refused.
            _ => return None,
        }
    }
    if pushed {
        Some(out)
    } else {
        None
    }
}

/// THE load gate: resolve `file` under `packs_root`, hash its bytes, compare
/// to the manifest pin, and return the only session-grade handle on success.
pub fn load_verified(
    pack: &Pack,
    file: &ConvertedFile,
    packs_root: &Path,
) -> Result<VerifiedArtifact, PackRefusal> {
    let (path, expected) = match load_state(pack, file) {
        LoadState::Refused(r) => return Err(r),
        LoadState::Verifiable { path, sha256 } => (path, sha256),
    };
    let abs = safe_join(packs_root, &path).ok_or_else(|| PackRefusal::UnsafeArtifactPath {
        pack_id: pack.id.clone(),
        path: path.clone(),
    })?;
    // Physical containment (symlink-aware): canonicalize both ends and
    // require the RESOLVED artifact to remain under the RESOLVED root.
    // Canonicalization requires existence, so resolution failure is the
    // missing-artifact refusal; an in-root symlink to an in-root file is
    // fine, one escaping the root is the typed path refusal.
    let canon_root = packs_root
        .canonicalize()
        .map_err(|_| PackRefusal::MissingArtifact {
            pack_id: pack.id.clone(),
            path: path.clone(),
        })?;
    let canon = abs
        .canonicalize()
        .map_err(|_| PackRefusal::MissingArtifact {
            pack_id: pack.id.clone(),
            path: path.clone(),
        })?;
    if !canon.starts_with(&canon_root) {
        return Err(PackRefusal::UnsafeArtifactPath {
            pack_id: pack.id.clone(),
            path: path.clone(),
        });
    }
    let bytes = std::fs::read(&canon).map_err(|_| PackRefusal::MissingArtifact {
        pack_id: pack.id.clone(),
        path: path.clone(),
    })?;
    let actual = sha256::hex_digest(&bytes);
    if actual != expected {
        return Err(PackRefusal::HashMismatch {
            pack_id: pack.id.clone(),
            path,
            expected,
            actual,
        });
    }
    Ok(VerifiedArtifact {
        pack_id: pack.id.clone(),
        pack_version: pack.version.clone(),
        // The canonical (physically resolved) path — what was verified is
        // what is recorded.
        path: canon,
        sha256: actual,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::PackRegistry;

    fn synthetic_registry(mode: &str, sha: &str) -> PackRegistry {
        let json = format!(
            r#"{{
  "packs": [{{
    "id": "test-pack",
    "name": "Test pack",
    "version": "9.9",
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
      "files": [{{"path": "models/test/w.onnx", "sha256": "{sha}"}}]
    }},
    "distribution": {{"mode": "{mode}", "redistribution_allowed": true{fallback}}},
    "legal": {{"rights_ref": "test"}},
    "revocable": false
  }}]
}}"#,
            src = "a".repeat(64),
            sha = sha,
            mode = mode,
            fallback = if mode == "not-distributed" {
                r#", "lawful_fallback": "user-fetched with license display""#
            } else {
                ""
            },
        );
        // not-distributed with redistribution_allowed true is nonsensical but
        // schema-legal; keep the synthetic simple.
        PackRegistry::parse(&json).expect("synthetic registry valid")
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("appname-ml-verify-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("models/test")).unwrap();
        dir
    }

    #[test]
    fn verified_load_round_trip_and_mismatch_refusal() {
        let payload = b"deterministic-fake-model-bytes";
        let sha = crate::sha256::hex_digest(payload);
        let reg = synthetic_registry("bundled-optional-pack", &sha);
        let pack = reg.pack("test-pack").unwrap();
        let file = &pack.converted.files[0];
        let root = temp_root("roundtrip");
        std::fs::write(root.join("models/test/w.onnx"), payload).unwrap();

        let art = load_verified(pack, file, &root).expect("hash matches");
        assert_eq!(art.sha256(), sha);
        assert_eq!(art.pack_version(), "9.9");
        assert_eq!(art.bytes(), payload);

        // Flip one byte: typed HashMismatch, never a load.
        let mut tampered = payload.to_vec();
        tampered[0] ^= 0x01;
        std::fs::write(root.join("models/test/w.onnx"), &tampered).unwrap();
        let err = load_verified(pack, file, &root).unwrap_err();
        assert!(
            matches!(err, PackRefusal::HashMismatch { ref expected, .. } if *expected == sha),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pending_packaging_is_a_refusal_without_io() {
        let reg = synthetic_registry("bundled-optional-pack", "pending-packaging");
        let pack = reg.pack("test-pack").unwrap();
        let state = load_state(pack, &pack.converted.files[0]);
        assert_eq!(
            state,
            LoadState::Refused(PackRefusal::ConversionUnpinned {
                pack_id: "test-pack".into(),
                path: "models/test/w.onnx".into()
            })
        );
        assert!(!pack_loadable(pack));
    }

    #[test]
    fn not_distributed_is_gated() {
        let sha = "c".repeat(64);
        let reg = synthetic_registry("not-distributed", &sha);
        let pack = reg.pack("test-pack").unwrap();
        let err =
            load_verified(pack, &pack.converted.files[0], Path::new("/nonexistent")).unwrap_err();
        assert_eq!(
            err,
            PackRefusal::DistributionGated {
                pack_id: "test-pack".into()
            }
        );
    }

    #[test]
    fn missing_artifact_refused() {
        let sha = "c".repeat(64);
        let reg = synthetic_registry("bundled-optional-pack", &sha);
        let pack = reg.pack("test-pack").unwrap();
        let root = temp_root("missing");
        let err = load_verified(pack, &pack.converted.files[0], &root).unwrap_err();
        assert!(
            matches!(err, PackRefusal::MissingArtifact { .. }),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Symlink-aware containment (slice-2 advisory): a lexically clean
    /// path whose SYMLINK RESOLUTION escapes the packs root is refused
    /// typed; a symlink that resolves WITHIN the root loads normally.
    #[cfg(unix)]
    #[test]
    fn symlink_escape_refused_in_root_symlink_allowed() {
        use std::os::unix::fs::symlink;
        let payload = b"symlink-containment-model-bytes";
        let sha = crate::sha256::hex_digest(payload);
        let reg = synthetic_registry("bundled-optional-pack", &sha);
        let root = temp_root("symlink");

        // The real bytes live OUTSIDE the packs root.
        let outside =
            std::env::temp_dir().join(format!("appname-ml-verify-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("escaped.onnx"), payload).unwrap();

        // models/test/w.onnx -> ../../../outside/escaped.onnx (a symlink
        // INSIDE the root pointing OUT). Lexical screening passes — the
        // manifest path is clean — so only physical containment catches it.
        symlink(
            outside.join("escaped.onnx"),
            root.join("models/test/w.onnx"),
        )
        .unwrap();
        let pack = reg.pack("test-pack").unwrap();
        let err = load_verified(pack, &pack.converted.files[0], &root).unwrap_err();
        assert!(
            matches!(err, PackRefusal::UnsafeArtifactPath { .. }),
            "escaping symlink must be a typed path refusal even with a matching hash: {err:?}"
        );

        // An in-root symlink to an in-root file verifies and loads.
        std::fs::remove_file(root.join("models/test/w.onnx")).unwrap();
        std::fs::write(root.join("models/test/real.onnx"), payload).unwrap();
        symlink(
            root.join("models/test/real.onnx"),
            root.join("models/test/w.onnx"),
        )
        .unwrap();
        let art = load_verified(pack, &pack.converted.files[0], &root).unwrap();
        assert_eq!(art.sha256(), sha);
        assert!(
            art.path().ends_with("models/test/real.onnx"),
            "recorded path is the canonical resolution: {:?}",
            art.path()
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn traversal_paths_refused_before_io() {
        for bad in ["../escape.onnx", "/abs/path.onnx", ""] {
            let sha = "c".repeat(64);
            let mut reg = synthetic_registry("bundled-optional-pack", &sha);
            reg.packs[0].converted.files[0].path = bad.to_string();
            let pack = &reg.packs[0];
            let err = load_verified(pack, &pack.converted.files[0], Path::new("/tmp")).unwrap_err();
            assert!(
                matches!(err, PackRefusal::UnsafeArtifactPath { .. }),
                "path {bad:?} gave {err:?}"
            );
        }
    }
}
