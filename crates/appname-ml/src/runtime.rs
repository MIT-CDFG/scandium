// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Model runtime slots — D-46, flag-gated, offline-default.
//!
//! Spec: ALGORITHMS.md §24 (one inference substrate: ONNX Runtime via `ort`,
//! CPU EP everywhere, candle sanctioned alternative), ALGORITHMS.md §9
//! (subject-select ladder: BiRefNet → IS-Net/U2-Net → §8 classical),
//! ES Part 48 row (feature `ml-models`), ES Part 53 §53.9; INTEGRATION-NOTES
//! §6 rows 13a/13c (slot targets).
//!
//! Every slot is OPTIONAL: when no backend is compiled/linked or no pack is
//! bound, the caller gets a typed error and degrades to the slot's
//! ALGORITHMS.md classical ladder. Nothing in this module mutates a
//! document; sessions consume [`VerifiedArtifact`]s and produce rasters that
//! the caller VALUE-CAPTURES (see [`crate::capture`]).

use crate::verify::{PackRefusal, VerifiedArtifact};

/// The v1 slot set (first packs, INTEGRATION-NOTES 13a/13c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotId {
    /// `selection.ml_subject` salient tier — u2net (13a).
    SubjectSelectSalient,
    /// `selection.ml_subject` dichotomous tier — dis (13a).
    SubjectSelectDichotomous,
    /// `selection.ml_subject` high-quality tier — birefnet (13a).
    SubjectSelectHighQuality,
    /// `alpha_matting.model_options` portrait matte — modnet (13c).
    PortraitMatte,
}

impl SlotId {
    /// Stable wire/golden-key token.
    pub fn as_str(self) -> &'static str {
        match self {
            SlotId::SubjectSelectSalient => "subject_select.salient",
            SlotId::SubjectSelectDichotomous => "subject_select.dichotomous",
            SlotId::SubjectSelectHighQuality => "subject_select.high_quality",
            SlotId::PortraitMatte => "portrait_matte",
        }
    }

    pub const ALL: [SlotId; 4] = [
        SlotId::SubjectSelectSalient,
        SlotId::SubjectSelectDichotomous,
        SlotId::SubjectSelectHighQuality,
        SlotId::PortraitMatte,
    ];
}

/// Slot → pack binding, or the typed reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotBinding {
    /// Bound to a registry pack id (the pack may still refuse to load — the
    /// binding is identity, not loadability).
    Pack { pack_id: &'static str },
    /// No sanctioned pin exists — plan §1 rule 8 forbids inventing one.
    NoPinnedPack { reason: &'static str },
}

/// The v1 binding table.
///
/// GAP RECORD (2026-08-16, O-ML slice 1): `models.v5.6-candidate.yaml` — the
/// EXCLUSIVE pins feed — carries NO u2net row and NO dis row (19 pack ids,
/// none of them u2net/dis; the frozen spec `spec/ai/models.yaml` matches).
/// INTEGRATION-NOTES §6 rows for u2net/dis say "weights per models.yaml row",
/// but no such rows exist. Those two tiers are therefore typed-unbound until
/// the pins feed grows sanctioned rows (v5.6 fold-in territory) — routed to
/// O-INT via reports/O-ML-log.md. The ALGORITHMS.md §9 ladder still serves:
/// BiRefNet high-quality tier → §8 classical fallback.
pub fn slot_binding(slot: SlotId) -> SlotBinding {
    match slot {
        SlotId::SubjectSelectSalient => SlotBinding::NoPinnedPack {
            reason: "no u2net row in models.v5.6-candidate.yaml (exclusive pins feed); \
                     pin gap routed to O-INT — rule 8 forbids re-derivation",
        },
        SlotId::SubjectSelectDichotomous => SlotBinding::NoPinnedPack {
            reason: "no dis row in models.v5.6-candidate.yaml (exclusive pins feed); \
                     pin gap routed to O-INT — rule 8 forbids re-derivation",
        },
        SlotId::SubjectSelectHighQuality => SlotBinding::Pack {
            pack_id: "bgremove-birefnet",
        },
        SlotId::PortraitMatte => SlotBinding::Pack {
            pack_id: "matting-modnet",
        },
    }
}

/// Execution provider identity — part of the class-N golden key. v1 golden
/// path is CPU-EP only (ALGORITHMS.md §1.4.4: CPU-EP pinned goldens; GPU EPs
/// are statistical-parity surfaces, SSIM >= 0.995, and arrive with the real
/// backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionProvider {
    Cpu,
}

impl ExecutionProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecutionProvider::Cpu => "cpu",
        }
    }
}

/// Typed errors of the runtime tier. These are crate-level errors: no
/// `error.command.*` mapping exists yet because NO command kind in the 102
/// carries a model op — the door filing is in reports/O-ML-log.md; when a
/// door lands, the door owns the wire taxonomy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MlError {
    /// Built without the `ml-models` feature (the default, offline-lean
    /// build): no runtime is compiled in at all.
    RuntimeNotCompiled,
    /// `ml-models` is compiled but the ONNX Runtime binding has not landed —
    /// the `ort` deps record (drafted, reports/O-ML-log.md) must be accepted
    /// by O-BUILD before the dependency exists (plan §1 rule 6 / D-46).
    RuntimeNotLinked { detail: &'static str },
    /// Slot has no sanctioned pack pin (see [`slot_binding`]).
    SlotUnbound { slot: SlotId, reason: &'static str },
    /// The pack refused to load (hash, pending stamp, distribution gate...).
    Refused(PackRefusal),
}

impl std::fmt::Display for MlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MlError::RuntimeNotCompiled => write!(
                f,
                "model runtime not compiled (feature `ml-models` off) — use the classical ladder"
            ),
            MlError::RuntimeNotLinked { detail } => {
                write!(f, "model runtime not linked: {detail}")
            }
            MlError::SlotUnbound { slot, reason } => {
                write!(f, "slot {} unbound: {reason}", slot.as_str())
            }
            MlError::Refused(r) => write!(f, "pack refusal: {r}"),
        }
    }
}

impl std::error::Error for MlError {}

impl From<PackRefusal> for MlError {
    fn from(r: PackRefusal) -> Self {
        MlError::Refused(r)
    }
}

/// Raster payload wire formats for model inputs/outputs. v1 slots emit
/// single-channel coverage/matte planes; the f32 little-endian plane matches
/// the engine's f32 mask buffer convention (ES §13.2 selection surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterFormat {
    /// Interleaved RGBA, 8-bit, sRGB — model INPUT staging form.
    Rgba8,
    /// Single-channel f32 coverage in [0,1], little-endian — mask/matte
    /// OUTPUT form.
    MaskF32Le,
}

impl RasterFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            RasterFormat::Rgba8 => "rgba8",
            RasterFormat::MaskF32Le => "mask-f32-le",
        }
    }

    pub fn bytes_per_pixel(self) -> usize {
        match self {
            RasterFormat::Rgba8 => 4,
            RasterFormat::MaskF32Le => 4,
        }
    }
}

/// A raster buffer crossing the model boundary (either direction).
#[derive(Debug, Clone, PartialEq)]
pub struct RasterPayload {
    pub width: u32,
    pub height: u32,
    pub format: RasterFormat,
    pub data: Vec<u8>,
}

impl RasterPayload {
    /// Structural sanity: data length must equal w*h*bpp.
    pub fn well_formed(&self) -> bool {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|px| px.checked_mul(self.format.bytes_per_pixel()))
            .is_some_and(|len| len == self.data.len())
    }
}

/// Backend identity token (joins capture records).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendId(pub &'static str);

/// A loaded, runnable model session. Implementations exist only behind
/// `ml-models` once the runtime binding lands.
pub trait ModelSession {
    fn backend(&self) -> BackendId;
    fn execution_provider(&self) -> ExecutionProvider;
    /// Run the model. Deterministic per (artifact, EP) — class-N discipline
    /// pins CPU-EP outputs per model version.
    fn run(&mut self, input: &RasterPayload) -> Result<RasterPayload, MlError>;
}

/// A model runtime backend: turns verified artifacts into sessions.
pub trait ModelBackend {
    fn id(&self) -> BackendId;
    fn create_session(
        &self,
        artifact: &VerifiedArtifact,
        ep: ExecutionProvider,
    ) -> Result<Box<dyn ModelSession>, MlError>;
}

/// The compiled-in backend, if any. Default build: none — every acquisition
/// is a typed [`MlError::RuntimeNotCompiled`].
pub fn compiled_backend() -> Result<&'static dyn ModelBackend, MlError> {
    #[cfg(feature = "ml-models")]
    {
        Ok(&crate::runtime::ort_stub::ORT_STUB)
    }
    #[cfg(not(feature = "ml-models"))]
    {
        Err(MlError::RuntimeNotCompiled)
    }
}

/// ONNX Runtime backend slot — typed-NI stub until the `ort` deps record
/// lands (D-46; drafted census-style in reports/O-ML-log.md, routed via
/// O-INT to O-BUILD). Compiles under `--features ml-models` with ZERO new
/// dependencies; `create_session` refuses, so no code path can pretend a
/// runtime exists.
#[cfg(feature = "ml-models")]
pub mod ort_stub {
    use super::*;

    pub struct OrtStubBackend;

    pub static ORT_STUB: OrtStubBackend = OrtStubBackend;

    impl ModelBackend for OrtStubBackend {
        fn id(&self) -> BackendId {
            BackendId("ort-unlinked")
        }

        fn create_session(
            &self,
            _artifact: &VerifiedArtifact,
            _ep: ExecutionProvider,
        ) -> Result<Box<dyn ModelSession>, MlError> {
            Err(MlError::RuntimeNotLinked {
                detail: "the `ort` dependency record has not landed (deps/ record required \
                         before the dependency, plan §1 rule 6); draft routed via O-INT to \
                         O-BUILD — until acceptance this backend is a typed-NI stub",
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_bindings() {
        assert_eq!(
            slot_binding(SlotId::SubjectSelectHighQuality),
            SlotBinding::Pack {
                pack_id: "bgremove-birefnet"
            }
        );
        assert_eq!(
            slot_binding(SlotId::PortraitMatte),
            SlotBinding::Pack {
                pack_id: "matting-modnet"
            }
        );
        for slot in [
            SlotId::SubjectSelectSalient,
            SlotId::SubjectSelectDichotomous,
        ] {
            match slot_binding(slot) {
                SlotBinding::NoPinnedPack { reason } => {
                    assert!(reason.contains("models.v5.6-candidate.yaml"), "{reason}")
                }
                other => panic!("expected typed pin gap for {slot:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn default_build_has_no_runtime() {
        // Default features: acquiring a backend is a typed refusal — the
        // core binary never requires a model runtime (ES Part 48 row).
        #[cfg(not(feature = "ml-models"))]
        match compiled_backend() {
            Err(e) => assert_eq!(e, MlError::RuntimeNotCompiled),
            Ok(_) => panic!("no backend may exist in the default build"),
        }
        // With `ml-models` (no ort record landed): backend exists but every
        // session acquisition is a typed RuntimeNotLinked refusal.
        #[cfg(feature = "ml-models")]
        {
            let b = compiled_backend().expect("stub backend compiled");
            assert_eq!(b.id(), BackendId("ort-unlinked"));
        }
    }

    /// With `ml-models` on but no ort record landed, session acquisition is
    /// a typed RuntimeNotLinked refusal — never a fake session.
    #[cfg(feature = "ml-models")]
    #[test]
    fn unlinked_stub_refuses_sessions_typed() {
        use crate::manifest::PackRegistry;
        let payload = b"stub-refusal-fixture";
        let sha = crate::sha256::hex_digest(payload);
        let json = format!(
            r#"{{"packs":[{{"id":"stub-fixture","name":"S","version":"1.0","task":"matting",
"source":{{"repo":"https://example.invalid/r","revision":"0","files":[{{"name":"w","bytes":20,"sha256":"{src}"}}],
"code_license":"Apache-2.0","weights_license":"Apache-2.0","license_url":"https://example.invalid/L"}},
"converted":{{"recipe":"none — upstream artifact used directly","files":[{{"path":"m/w.onnx","sha256":"{sha}"}}]}},
"distribution":{{"mode":"bundled-optional-pack","redistribution_allowed":true}},
"legal":{{"rights_ref":"test"}},"revocable":false}}]}}"#,
            src = "a".repeat(64),
            sha = sha,
        );
        let reg = PackRegistry::parse(&json).unwrap();
        let root = std::env::temp_dir().join(format!("appname-ml-stub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("m")).unwrap();
        std::fs::write(root.join("m/w.onnx"), payload).unwrap();
        let pack = reg.pack("stub-fixture").unwrap();
        let art = crate::verify::load_verified(pack, &pack.converted.files[0], &root).unwrap();
        let backend = compiled_backend().unwrap();
        match backend.create_session(&art, ExecutionProvider::Cpu) {
            Err(MlError::RuntimeNotLinked { detail }) => {
                assert!(detail.contains("deps"), "{detail}")
            }
            other => panic!("expected typed RuntimeNotLinked, got {:?}", other.err()),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn raster_payload_well_formedness() {
        let ok = RasterPayload {
            width: 4,
            height: 2,
            format: RasterFormat::MaskF32Le,
            data: vec![0u8; 32],
        };
        assert!(ok.well_formed());
        let bad = RasterPayload {
            data: vec![0u8; 31],
            ..ok
        };
        assert!(!bad.well_formed());
    }

    #[test]
    fn slot_tokens_stable() {
        // Golden-key components must never drift silently.
        let tokens: Vec<&str> = SlotId::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            tokens,
            [
                "subject_select.salient",
                "subject_select.dichotomous",
                "subject_select.high_quality",
                "portrait_matte"
            ]
        );
    }
}
