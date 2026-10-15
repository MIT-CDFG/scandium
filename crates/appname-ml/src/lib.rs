// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-ml` — model runtime slots and pack manifest pipeline, flag-gated.
//!
//! Spec: ES Part 53 §53.9 (pack registry, offline packs), ES §16.4 rule 5 /
//! ALGORITHMS.md §1.4.4 (determinism class N), ALGORITHMS.md §24 (one
//! inference substrate: ONNX Runtime via `ort`, candle sanctioned
//! alternative), ES Part 48 dependency row (feature `ml-models`, dynamic
//! EPs); DD D-45 (weights licensing), D-46 (runtime election), D-48 (value
//! capture — replay never re-runs a model), D-49 (offline default,
//! manifest-verified). Contract: `contracts/schemas/ai-model-pack.v1.json`.
//!
//! # Posture (normative for this crate)
//!
//! - **Offline default (D-49):** nothing in this crate initiates network
//!   access, ever. Packs load from local files only; `download_url` fields in
//!   manifests are provenance metadata, never dereferenced here. The crate
//!   has no network-capable dependency.
//! - **Manifest-verified (D-49):** a model artifact is usable only through
//!   [`verify::VerifiedArtifact`], which exists only after the artifact's
//!   bytes hash to the manifest-pinned sha256. Mismatch, unpinned conversion
//!   hashes (`pending-packaging`), and `not-distributed` packs are typed
//!   refusals — never a bypass.
//! - **Pins are never re-derived (plan §1 rule 8):** manifest values under
//!   `packs/` are copied mechanically from
//!   `research/pins/models.v5.6-candidate.yaml`; this crate checks and
//!   enforces them, it does not invent them.
//! - **Value capture (D-48, class N):** a model invocation produces a
//!   [`capture::CapturedModelOutput`] carrying the output raster and the full
//!   golden-key identity (slot, pack id + version, artifact sha256, execution
//!   provider). The captured raster — not the model — is what enters a
//!   document: [`pipeline::MaskAssetCapture`] encodes it as `appname.mask.v1`
//!   asset bytes for the D-ML-1 ingress (`Engine::add_mask_asset` → the
//!   §13.2 combine doors), and core applies it under an ordinary envelope.
//!   This crate has NO `appname-core` LIBRARY dependency and no
//!   document-mutation API: replay can never re-run a model from here by
//!   construction. (`appname-core` appears as a DEV-dependency only —
//!   `tests/door_pipeline.rs` proves the cross-crate pipeline; test binaries
//!   ship nothing and core still has no path to this crate.)
//! - **Compile-flag-gated runtime (D-46, ES:16944):** the ONNX Runtime
//!   backend lives behind the `ml-models` cargo feature, off by default.
//!   Until the `ort` dependency record lands (drafted in
//!   `reports/O-ML-log.md`, routes via O-INT to O-BUILD), the feature builds
//!   a typed-NI stub; every ML slot degrades to its ALGORITHMS.md classical
//!   ladder when the runtime is absent — the core binary never requires it.

pub mod capture;
pub mod manifest;
pub mod pipeline;
pub mod runtime;
pub mod sha256;
pub mod verify;

/// Crate name marker (kept from the bootstrap stub; exercised by the smoke
/// test).
pub const CRATE_NAME: &str = "appname-ml";

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-ml");
    }
}
