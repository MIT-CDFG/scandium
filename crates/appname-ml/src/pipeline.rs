// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Model-output → `appname.mask.v1` asset pipeline — the D-ML-1 consumption
//! leg (CP-1.0 slice 2).
//!
//! Spec: ALGORITHMS.md §9 ("the *selection command* records the mask
//! raster, never a model re-run"), ES §13.1 (single mask representation:
//! f32 coverage in [0,1]), §13.2 (combine ops), D-48 value capture; the
//! D-ML-1 door (core `engine/assets.rs` + `engine/kind_impl.rs`, landed
//! door slice 2, main @ 3ada087) and its `appname.mask.v1` asset format.
//!
//! # The pipeline (end to end)
//!
//! 1. A subject-select/matte session produces a single-channel f32 mask
//!    ([`RasterFormat::MaskF32Le`]) which the caller seals into a
//!    [`CapturedModelOutput`] — the class-N value capture.
//! 2. [`MaskAssetCapture::from_capture`] (the only constructor) validates
//!    the sealed output as a coverage plane and encodes the
//!    **`appname.mask.v1`** wire bytes: one JSON header line
//!    `{"schema":"appname.mask.v1","x":…,"y":…,"w":…,"h":…}` + `\n` +
//!    exactly `w*h` little-endian f32 values, row-major, each finite and in
//!    `[0, 1]`. This mirrors core's `engine::assets::encode_mask_plane`
//!    BYTE-FOR-BYTE (pinned by unit test here and by cross-crate parity in
//!    `tests/door_pipeline.rs`), so the engine's content-derived asset id
//!    is stable across the boundary.
//! 3. The bytes are ingested through the door's SANCTIONED engine-side
//!    ingress — `Engine::add_mask_asset` (validates at ingestion) — and
//!    applied to the selection surface or a layer raster mask by core's
//!    `KindContext::{apply_mask_asset_to_selection,
//!    apply_mask_asset_to_layer_mask}` with the §13.2 op set. Application
//!    is core's act, under a registered kind: this crate produces bytes and
//!    never touches a document (no `appname-core` dependency — the library
//!    link-graph posture stands; the cross-crate pipeline test uses a
//!    DEV-dependency, which ships nothing and leaves replay structurally
//!    unable to reach a model).
//!
//! # Wire-arm posture (recorded, §3.13 fold-in gate)
//!
//! The door's wire arms — `selection.set` `source:"raster"` and `mask.add`
//! `from:"asset"` — are UNSANCTIONED at v5.5 and sit on the fold-in
//! register (O-CORE log, door slice 2). NOTHING in this module emits,
//! names, or serves a wire spelling: there is no envelope construction
//! here, only asset bytes. The engine's stage-5 validation refuses those
//! spellings today (pinned in `tests/door_pipeline.rs`).
//!
//! # Golden coverage (dispatch: "the CAPTURED artifact is the mask asset")
//!
//! The asset bytes are a deterministic pure function of the sealed output
//! payload + placement origin, and [`MaskAssetCapture::golden_row`] binds
//! the full class-N golden key to the ASSET sha256 — so O-QA's golden rows
//! pin the exact document-bound artifact, not merely the raw payload.

use crate::capture::CapturedModelOutput;
use crate::runtime::RasterFormat;
use crate::sha256;

/// The `appname.mask.v1` schema stamp (mirror of core's
/// `engine::assets::MASK_PLANE_SCHEMA`; parity pinned cross-crate).
pub const MASK_PLANE_SCHEMA: &str = "appname.mask.v1";

/// Extent cap per axis — mirror of the door's decode gate (`w > 1 << 20`
/// refuses), so a plane this crate emits is never door-refused for size.
pub const MAX_PLANE_EXTENT: u32 = 1 << 20;

/// Total-pixel cap — mirror of the door's `1 << 28` decode cap.
pub const MAX_PLANE_PIXELS: usize = 1 << 28;

/// Typed refusals of the capture→asset leg. Every variant is terminal:
/// nothing malformed is ever encoded (the door would refuse it anyway —
/// this crate refuses FIRST, with the model-side context).
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineError {
    /// The sealed output is not a single-channel f32 mask plane.
    NotAMaskPlane { format: &'static str },
    /// data length does not equal `w * h * 4`.
    MalformedPayload { expected: usize, actual: usize },
    /// Zero/oversize extent, pixel-count overflow, or placement such that
    /// `origin + extent` leaves i32 range (door decode computes max = x + w).
    ExtentOutOfRange {
        origin_x: i32,
        origin_y: i32,
        width: u32,
        height: u32,
    },
    /// A coverage value is not finite or not in `[0, 1]` (§13.1).
    NonCoverageValue { index: usize, value: f32 },
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineError::NotAMaskPlane { format } => write!(
                f,
                "captured output is {format}, not a mask-f32-le coverage plane"
            ),
            PipelineError::MalformedPayload { expected, actual } => write!(
                f,
                "captured payload carries {actual} bytes, expected {expected} (w*h*4)"
            ),
            PipelineError::ExtentOutOfRange {
                origin_x,
                origin_y,
                width,
                height,
            } => write!(
                f,
                "plane {width}x{height} at ({origin_x},{origin_y}) out of range \
                 (floor 1x1, cap {MAX_PLANE_EXTENT} per axis / {MAX_PLANE_PIXELS} px, \
                 origin+extent must fit i32)"
            ),
            PipelineError::NonCoverageValue { index, value } => write!(
                f,
                "coverage value [{index}] = {value} not finite in [0, 1] (§13.1)"
            ),
        }
    }
}

impl std::error::Error for PipelineError {}

/// A sealed model capture together with its encoded `appname.mask.v1`
/// asset bytes — the complete value-capture-to-door record. Single
/// constructor ([`MaskAssetCapture::from_capture`]), same discipline as
/// [`CapturedModelOutput::seal`]: the asset bytes and their sha256 always
/// derive from the held capture, never from caller-supplied bytes.
#[derive(Debug, Clone)]
pub struct MaskAssetCapture {
    capture: CapturedModelOutput,
    origin_x: i32,
    origin_y: i32,
    asset_bytes: Vec<u8>,
    asset_sha256: String,
}

impl MaskAssetCapture {
    /// Validate a sealed capture as a §13.1 coverage plane placed at
    /// `(origin_x, origin_y)` document px and encode its `appname.mask.v1`
    /// asset bytes. Refusal order: format → payload shape → extent/
    /// placement → per-value coverage.
    pub fn from_capture(
        capture: CapturedModelOutput,
        origin_x: i32,
        origin_y: i32,
    ) -> Result<MaskAssetCapture, PipelineError> {
        let out = &capture.output;
        if out.format != RasterFormat::MaskF32Le {
            return Err(PipelineError::NotAMaskPlane {
                format: out.format.as_str(),
            });
        }
        let extent_err = PipelineError::ExtentOutOfRange {
            origin_x,
            origin_y,
            width: out.width,
            height: out.height,
        };
        if out.width < 1
            || out.height < 1
            || out.width > MAX_PLANE_EXTENT
            || out.height > MAX_PLANE_EXTENT
        {
            return Err(extent_err);
        }
        let pixels = (out.width as usize)
            .checked_mul(out.height as usize)
            .filter(|n| *n <= MAX_PLANE_PIXELS)
            .ok_or_else(|| extent_err.clone())?;
        // The door's decode computes max = origin + extent; keep it in i32.
        if i64::from(origin_x) + i64::from(out.width) > i64::from(i32::MAX)
            || i64::from(origin_y) + i64::from(out.height) > i64::from(i32::MAX)
        {
            return Err(extent_err);
        }
        let expected = pixels * 4;
        if out.data.len() != expected {
            return Err(PipelineError::MalformedPayload {
                expected,
                actual: out.data.len(),
            });
        }
        for (index, chunk) in out.data.chunks_exact(4).enumerate() {
            let value = f32::from_le_bytes(chunk.try_into().expect("chunks_exact(4)"));
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(PipelineError::NonCoverageValue { index, value });
            }
        }
        // Encode: header line mirrors core's encode_mask_plane exactly;
        // the payload is the captured bytes VERBATIM (already f32 LE
        // row-major — bit-exact, so content addressing is stable).
        let header = format!(
            "{{\"schema\":\"{MASK_PLANE_SCHEMA}\",\"x\":{origin_x},\"y\":{origin_y},\"w\":{},\"h\":{}}}\n",
            out.width, out.height
        );
        let mut asset_bytes = header.into_bytes();
        asset_bytes.extend_from_slice(&out.data);
        let asset_sha256 = sha256::hex_digest(&asset_bytes);
        Ok(MaskAssetCapture {
            capture,
            origin_x,
            origin_y,
            asset_bytes,
            asset_sha256,
        })
    }

    /// The sealed capture record (golden-key identity + raw output).
    pub fn capture(&self) -> &CapturedModelOutput {
        &self.capture
    }

    /// Placement origin in document px (the plane's min corner).
    pub fn origin(&self) -> (i32, i32) {
        (self.origin_x, self.origin_y)
    }

    /// The `appname.mask.v1` bytes — feed these to the door's ingress
    /// (`Engine::add_mask_asset`); the engine derives the content id.
    pub fn asset_bytes(&self) -> &[u8] {
        &self.asset_bytes
    }

    /// sha256 (lowercase hex) of the asset bytes — the CAPTURED artifact's
    /// hash, what class-N golden rows pin.
    pub fn asset_sha256(&self) -> &str {
        &self.asset_sha256
    }

    /// Canonical class-N golden row: full golden key → the mask asset it
    /// captured. Stable by construction (corpus row form for O-QA).
    pub fn golden_row(&self) -> String {
        format!(
            "{} => {}:{}",
            self.capture.golden_key().canonical(),
            MASK_PLANE_SCHEMA,
            self.asset_sha256
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::CapturedModelOutput;
    use crate::manifest::PackRegistry;
    use crate::runtime::{BackendId, ExecutionProvider, RasterPayload, SlotId};
    use crate::verify::load_verified;

    /// A real sealed capture through the load gate (no bypass constructor
    /// exists anywhere on this path).
    fn sealed(output: RasterPayload) -> CapturedModelOutput {
        let payload = b"pipeline-fixture-model-bytes";
        let sha = crate::sha256::hex_digest(payload);
        let json = format!(
            r#"{{"packs":[{{"id":"pipeline-fixture","name":"P","version":"2.0","task":"matting",
"source":{{"repo":"https://example.invalid/r","revision":"0","files":[{{"name":"w","bytes":28,"sha256":"{src}"}}],
"code_license":"Apache-2.0","weights_license":"Apache-2.0","license_url":"https://example.invalid/L"}},
"converted":{{"recipe":"none — upstream artifact used directly","files":[{{"path":"m/w.onnx","sha256":"{sha}"}}]}},
"distribution":{{"mode":"bundled-optional-pack","redistribution_allowed":true}},
"legal":{{"rights_ref":"test"}},"revocable":false}}]}}"#,
            src = "a".repeat(64),
            sha = sha,
        );
        let reg = PackRegistry::parse(&json).unwrap();
        let root = std::env::temp_dir().join(format!(
            "appname-ml-pipeline-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("m")).unwrap();
        std::fs::write(root.join("m/w.onnx"), payload).unwrap();
        let pack = reg.pack("pipeline-fixture").unwrap();
        let art = load_verified(pack, &pack.converted.files[0], &root).unwrap();
        let cap = CapturedModelOutput::seal(
            SlotId::PortraitMatte,
            &art,
            BackendId("test-backend"),
            ExecutionProvider::Cpu,
            "blake3:cafe".into(),
            output,
        );
        let _ = std::fs::remove_dir_all(&root);
        cap
    }

    fn mask_payload(width: u32, height: u32, values: &[f32]) -> RasterPayload {
        RasterPayload {
            width,
            height,
            format: RasterFormat::MaskF32Le,
            data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    /// THE wire pin: exact bytes and sha256 of a known plane. Any layout
    /// drift (header spelling, field order, payload encoding) turns this
    /// red before the cross-crate parity test even runs.
    #[test]
    fn wire_layout_pinned() {
        let cap = sealed(mask_payload(2, 2, &[0.0, 0.25, 0.5, 1.0]));
        let mac = MaskAssetCapture::from_capture(cap, 3, -4).unwrap();
        let mut expected =
            b"{\"schema\":\"appname.mask.v1\",\"x\":3,\"y\":-4,\"w\":2,\"h\":2}\n".to_vec();
        for v in [0.0f32, 0.25, 0.5, 1.0] {
            expected.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(mac.asset_bytes(), expected.as_slice());
        assert_eq!(expected.len(), 70);
        // Pinned independently (python3 hashlib over the same layout).
        assert_eq!(
            mac.asset_sha256(),
            "82ebd06ca2e8d84ac530fc3c23567c49b8078d2b21f36e3bba7cc935d8017987"
        );
        assert_eq!(mac.origin(), (3, -4));
    }

    #[test]
    fn golden_row_binds_key_to_asset() {
        let cap = sealed(mask_payload(1, 1, &[1.0]));
        let art_sha = cap.artifact_sha256.clone();
        let mac = MaskAssetCapture::from_capture(cap, 0, 0).unwrap();
        assert_eq!(
            mac.golden_row(),
            format!(
                "portrait_matte/pipeline-fixture@2.0/{art_sha}/cpu => appname.mask.v1:{}",
                mac.asset_sha256()
            )
        );
    }

    #[test]
    fn deterministic_encoding() {
        let a = MaskAssetCapture::from_capture(sealed(mask_payload(3, 1, &[0.0, 0.5, 1.0])), 7, 9)
            .unwrap();
        let b = MaskAssetCapture::from_capture(sealed(mask_payload(3, 1, &[0.0, 0.5, 1.0])), 7, 9)
            .unwrap();
        assert_eq!(a.asset_bytes(), b.asset_bytes());
        assert_eq!(a.asset_sha256(), b.asset_sha256());
    }

    #[test]
    fn non_mask_format_refused() {
        let cap = sealed(RasterPayload {
            width: 1,
            height: 1,
            format: RasterFormat::Rgba8,
            data: vec![0; 4],
        });
        assert_eq!(
            MaskAssetCapture::from_capture(cap, 0, 0).unwrap_err(),
            PipelineError::NotAMaskPlane { format: "rgba8" }
        );
    }

    #[test]
    fn malformed_payload_refused() {
        let mut p = mask_payload(2, 1, &[0.0, 0.0]);
        p.data.truncate(7);
        assert_eq!(
            MaskAssetCapture::from_capture(sealed(p), 0, 0).unwrap_err(),
            PipelineError::MalformedPayload {
                expected: 8,
                actual: 7
            }
        );
    }

    #[test]
    fn extent_refusals() {
        // Zero extent.
        assert!(matches!(
            MaskAssetCapture::from_capture(sealed(mask_payload(0, 0, &[])), 0, 0).unwrap_err(),
            PipelineError::ExtentOutOfRange { .. }
        ));
        // Per-axis cap (door mirror): data check never reached, so the
        // oversize payload need not be materialized.
        let huge = RasterPayload {
            width: MAX_PLANE_EXTENT + 1,
            height: 1,
            format: RasterFormat::MaskF32Le,
            data: Vec::new(),
        };
        assert!(matches!(
            MaskAssetCapture::from_capture(sealed(huge), 0, 0).unwrap_err(),
            PipelineError::ExtentOutOfRange { .. }
        ));
        // Placement overflow: origin + extent must stay in i32 (door
        // decode computes max = x + w).
        assert!(matches!(
            MaskAssetCapture::from_capture(sealed(mask_payload(1, 1, &[0.5])), i32::MAX, 0)
                .unwrap_err(),
            PipelineError::ExtentOutOfRange { .. }
        ));
    }

    #[test]
    fn non_coverage_values_refused() {
        for (bad, what) in [(1.5f32, "over"), (-0.1, "under"), (f32::NAN, "nan")] {
            let err = MaskAssetCapture::from_capture(sealed(mask_payload(2, 1, &[0.5, bad])), 0, 0)
                .unwrap_err();
            match err {
                PipelineError::NonCoverageValue { index, .. } => {
                    assert_eq!(index, 1, "{what}: refusal names the offending index")
                }
                other => panic!("{what}: expected NonCoverageValue, got {other:?}"),
            }
        }
    }
}
