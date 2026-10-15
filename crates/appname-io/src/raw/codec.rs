// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The RAW registry slot — §8.1 registration row 9 (…, PSD, ORA, **RAW**),
//! feature-gated (module doc).
//!
//! Spec: ES §8.1 sniff table ("per-vendor via LibRaw probe; e.g. CR2 =
//! TIFF magic + `43 52` @8, eff. 10; NEF/ARW/DNG = TIFF magic, eff. 4"),
//! §8.3, Part 43 §43.5; DD ⑫.
//!
//! **Binding posture (the record consumed — deps/libraw.yaml
//! `preAuthorizedFor`, this slice):** `decode` now runs the REAL staged
//! path: §43.3 param validation → module resolution (the replaceable
//! dylib via [`crate::raw::stage1`]; ABSENT ⇒ the §43.5 row-1 typed
//! refusal `error.unsupported.raw_disabled`) → LibRaw `open_buffer` +
//! `unpack` through the sanitized wrapper (identification failure ⇒
//! `error.codec.raw.unsupported_camera`; mid-unpack failure ⇒
//! `error.codec.corrupt`) → §8.1 bomb gates → CFA classification →
//! level/multiplier/matrix sanitization (the bound degenerate-levels
//! advisory) → the RECORDED mosaic-egress gap (typed; see the stage1
//! module doc and the lane log's finding — the documented C API cannot
//! egress the CFA plane, resolution escalated, never improvised).
//! `sniff` keeps the documented per-vendor magic subset below — still a
//! strict SUBSET of the LibRaw probe, superseded when the egress
//! resolution makes full decodes real.

use appname_core::ChannelKind;

use crate::error::CodecError;
use crate::opts::{DecodeOpts, EncodeOpts};
use crate::raw::{stage1, RAW_EXTENSIONS};
use crate::registry::{Codec, CodecCaps, CodecId, ExportImage, Imported, LicenseStatus};

/// The license-gated RAW reader (import-only: §8.3 has no RAW encode).
///
/// `module` is the test/assembly override; `None` (the registry's
/// default) resolves the process-global module
/// ([`stage1::install_module`] / `APPNAME_LIBRAW_DYLIB`).
#[derive(Default)]
pub struct RawCodec {
    module: Option<libraw_sys::RawLib>,
}

impl RawCodec {
    /// A codec bound to an explicit module (conformance-stub tests; the
    /// packaging channel uses the global door instead).
    pub fn with_module(lib: libraw_sys::RawLib) -> RawCodec {
        RawCodec { module: Some(lib) }
    }
}

fn tiff_magic(head: &[u8]) -> bool {
    head.len() >= 4
        && (head[..4] == [0x49, 0x49, 0x2A, 0x00] || head[..4] == [0x4D, 0x4D, 0x00, 0x2A])
}

/// The documented skeleton sniff subset and each match's effective magic
/// length (§8.1 table shape). Order: most-specific first.
fn effective_len(head: &[u8]) -> Option<u32> {
    // RAF: "FUJIFILMCCD-RAW" @0 (eff. 15).
    if head.len() >= 15 && &head[..15] == b"FUJIFILMCCD-RAW" {
        return Some(15);
    }
    // CR3: ISO-BMFF "ftyp" @4 + brand "crx " @8 (eff. 12).
    if head.len() >= 12 && &head[4..8] == b"ftyp" && &head[8..12] == b"crx " {
        return Some(12);
    }
    // CR2: TIFF magic + "CR" @8 (eff. 10, the §8.1 worked example).
    if head.len() >= 10 && tiff_magic(head) && head[8] == 0x43 && head[9] == 0x52 {
        return Some(10);
    }
    // RW2: Panasonic TIFF-variant magic II 55 00 (eff. 4).
    if head.len() >= 4 && head[..4] == [0x49, 0x49, 0x55, 0x00] {
        return Some(4);
    }
    // ORF: Olympus TIFF-variant magic IIRO / IIRS (eff. 4).
    if head.len() >= 4
        && head[..2] == [0x49, 0x49]
        && head[2] == 0x52
        && (head[3] == 0x4F || head[3] == 0x53)
    {
        return Some(4);
    }
    // NEF/ARW/DNG (and other TIFF-carried vendor formats): TIFF magic
    // (eff. 4) — the §8.1 extension tiebreak separates them from plain
    // TIFF, and plain .tif keeps TIFF by registration order.
    if tiff_magic(head) {
        return Some(4);
    }
    None
}

impl Codec for RawCodec {
    fn id(&self) -> CodecId {
        CodecId("raw")
    }

    fn capabilities(&self) -> CodecCaps {
        CodecCaps {
            read: true,
            write: false,
            layered: false,
            max_depth: ChannelKind::U16,
            alpha: false,
            animation: false,
            license: LicenseStatus::LicenseGatedModule,
        }
    }

    fn sniff(&self, head: &[u8]) -> bool {
        effective_len(head).is_some()
    }

    fn magic_len(&self) -> u32 {
        // The TIFF-magic floor; content-dependent lengths ride
        // `effective_magic_len` (CR2 = 10 beats TIFF = 4 on content).
        4
    }

    fn effective_magic_len(&self, head: &[u8]) -> u32 {
        effective_len(head).unwrap_or(0)
    }

    fn extensions(&self) -> &'static [&'static str] {
        RAW_EXTENSIONS
    }

    fn decode(&self, bytes: &[u8], opts: &DecodeOpts) -> Result<Imported, CodecError> {
        // §43.3 staged validation FIRST — bad raw.* options are
        // `error.codec.bad_option` before any module or pixel work.
        if let Some(params) = &opts.raw {
            params.validate()?;
        }
        // Module resolution (module doc): explicit binding, else the
        // process-global door; absent ⇒ §43.5 row 1, typed.
        let lib = match &self.module {
            Some(l) => l,
            None => stage1::global_module()?,
        };
        // The sanitized stage-1 wrapper: open/unpack, bomb gates, CFA
        // classification, level/multiplier/matrix sanitization (incl.
        // the bound degenerate-levels advisory).
        let meta = stage1::stage1_open(lib, bytes, opts)?;
        // The recorded egress gap: the mosaic plane (with per-channel
        // black levels, orientation, EXIF) has no documented-C-API
        // route — typed refusal, resolution escalated (stage1 doc +
        // lane log). When it lands, this point feeds
        // develop::stage2_normalize (levels re-checked there) →
        // develop::develop() and builds `Imported` per the §8.3 result
        // contract.
        let _ = meta;
        Err(stage1::mosaic_egress_gap())
    }

    fn encode(&self, _img: &ExportImage<'_>, _opts: &EncodeOpts) -> Result<Vec<u8>, CodecError> {
        Err(CodecError::WriteUnsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::params::RawDevelopParams;
    use crate::registry::CodecRegistry;

    fn cr2_head() -> Vec<u8> {
        let mut v = vec![0x49, 0x49, 0x2A, 0x00, 0x10, 0x00, 0x00, 0x00, 0x43, 0x52];
        v.resize(16, 0);
        v
    }

    #[test]
    fn cr2_beats_tiff_on_effective_length() {
        let reg = CodecRegistry::builtin();
        assert_eq!(reg.resolve(&cr2_head(), Some("tif")).unwrap().id().0, "raw");
    }

    #[test]
    fn nef_ties_tiff_and_wins_by_extension() {
        let reg = CodecRegistry::builtin();
        let head = b"II\x2A\x00\x08\x00\x00\x00\x00\x00";
        assert_eq!(reg.resolve(head, Some("nef")).unwrap().id().0, "raw");
        assert_eq!(reg.resolve(head, Some("NEF")).unwrap().id().0, "raw");
        assert_eq!(reg.resolve(head, Some("dng")).unwrap().id().0, "raw");
    }

    #[test]
    fn plain_tif_stays_tiff_by_registration_order() {
        let reg = CodecRegistry::builtin();
        let head = b"II\x2A\x00\x08\x00\x00\x00\x00\x00";
        assert_eq!(reg.resolve(head, Some("tif")).unwrap().id().0, "tiff");
        // No decisive extension at all: earliest registration also wins.
        assert_eq!(reg.resolve(head, None).unwrap().id().0, "tiff");
    }

    #[test]
    fn vendor_magics_claimed() {
        let raw = RawCodec::default();
        assert!(raw.sniff(b"FUJIFILMCCD-RAW 0201"));
        assert_eq!(raw.effective_magic_len(b"FUJIFILMCCD-RAW 0201"), 15);
        let cr3 = [
            0u8, 0, 0, 0x18, b'f', b't', b'y', b'p', b'c', b'r', b'x', b' ',
        ];
        assert!(raw.sniff(&cr3));
        assert_eq!(raw.effective_magic_len(&cr3), 12);
        assert!(raw.sniff(&[0x49, 0x49, 0x55, 0x00, 0, 0])); // RW2
        assert!(raw.sniff(&[0x49, 0x49, 0x52, 0x4F, 0, 0])); // ORF IIRO
        assert!(raw.sniff(&[0x49, 0x49, 0x52, 0x53, 0, 0])); // ORF IIRS
        assert!(!raw.sniff(b"plain text"));
        assert!(!raw.sniff(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn decode_without_module_is_typed_raw_disabled() {
        // No module installed and no env override in the test
        // environment: the §43.5 row-1 arm, typed, never a crash.
        let err = RawCodec::default()
            .decode(&cr2_head(), &DecodeOpts::default())
            .unwrap_err();
        match &err {
            CodecError::UnsupportedFeature { feature } => {
                assert!(feature.starts_with("raw.module_unavailable"), "{feature}");
            }
            other => panic!("expected typed refusal, got {other:?}"),
        }
        assert_eq!(err.message_id(), "error.unsupported.raw_disabled");
    }

    #[test]
    fn decode_validates_params_before_module_resolution() {
        let opts = DecodeOpts {
            raw: Some(RawDevelopParams {
                exposure_ev: 42.0,
                ..Default::default()
            }),
            ..Default::default()
        };
        let err = RawCodec::default().decode(&cr2_head(), &opts).unwrap_err();
        assert_eq!(err.message_id(), "error.codec.bad_option");
    }

    // ------ stub-module boundary arms (the sanitized wrapper, live) ------

    use libraw_sys::stub::{stub_bytes, stub_lib, with_prefix, StubCfa, StubSpec};

    fn stub_codec() -> RawCodec {
        RawCodec::with_module(stub_lib())
    }

    /// The full live path up to the RECORDED egress gap: params → module
    /// → open → unpack → gates → CFA → sanitize all PASS, then the typed
    /// egress refusal (never silent, never a stand-in decode).
    #[test]
    fn decode_with_module_reaches_the_recorded_egress_gap() {
        let bytes = with_prefix(&cr2_head(), &stub_bytes(&StubSpec::bayer_rggb(64, 48)));
        let err = stub_codec()
            .decode(&bytes, &DecodeOpts::default())
            .unwrap_err();
        match &err {
            CodecError::UnsupportedFeature { feature } => {
                assert!(feature.contains("stage1_mosaic_egress"), "{feature}");
            }
            other => panic!("expected the egress gap, got {other:?}"),
        }
        assert_eq!(err.message_id(), "error.codec.unsupported_feature");
    }

    /// §43.5 row 2: identification failure surfaces the RAW-scoped id
    /// with the decoder's own message as developer detail.
    #[test]
    fn decode_open_rejection_is_unsupported_camera() {
        // Bytes without the stub's own format: the fake LibRaw cannot
        // identify them (mirrors a real non-RAW/unknown-camera buffer).
        let err = stub_codec()
            .decode(&cr2_head(), &DecodeOpts::default())
            .unwrap_err();
        assert_eq!(err.message_id(), "error.codec.raw.unsupported_camera");
        assert!(err.to_string().contains("stub:"), "{err}");
    }

    /// §43.5 row 4: identified but undecodable ⇒ corrupt (call-site
    /// mapping — numeric LibRaw codes are undocumented).
    #[test]
    fn decode_unpack_failure_is_corrupt() {
        let spec = StubSpec {
            unpack_result: -4,
            ..StubSpec::bayer_rggb(16, 16)
        };
        let err = stub_codec()
            .decode(&stub_bytes(&spec), &DecodeOpts::default())
            .unwrap_err();
        assert_eq!(err.message_id(), "error.codec.corrupt");
    }

    /// §8.1 bomb gates fire from decoder header dims BEFORE any further
    /// work (limit_exceeded, not the egress gap).
    #[test]
    fn decode_bomb_gate_fires_before_egress() {
        let spec = StubSpec::bayer_rggb(70_000, 70_000);
        let err = stub_codec()
            .decode(&stub_bytes(&spec), &DecodeOpts::default())
            .unwrap_err();
        assert_eq!(err.message_id(), "error.codec.limit_exceeded");
        // Dimension (not pixel-count) gate too.
        let spec = StubSpec::bayer_rggb(1_048_577, 1);
        let err = stub_codec()
            .decode(&stub_bytes(&spec), &DecodeOpts::default())
            .unwrap_err();
        assert_eq!(err.message_id(), "error.codec.limit_exceeded");
    }

    /// Non-Bayer sensors classify to the typed delegation arm (X-Trans
    /// pattern through the documented probe).
    #[test]
    fn decode_non_bayer_is_typed_delegation_pending() {
        let spec = StubSpec {
            cfa: StubCfa::XTrans,
            ..StubSpec::bayer_rggb(24, 24)
        };
        let err = stub_codec()
            .decode(&stub_bytes(&spec), &DecodeOpts::default())
            .unwrap_err();
        match &err {
            CodecError::UnsupportedFeature { feature } => {
                assert!(feature.contains("non_bayer_delegation"), "{feature}");
            }
            other => panic!("expected non-Bayer arm, got {other:?}"),
        }
    }

    /// The bound degenerate-levels advisory at the codec boundary: a
    /// stub serving a non-positive white level is a TYPED corrupt
    /// refusal, not a downstream panic.
    #[test]
    fn decode_degenerate_white_level_is_typed_corrupt() {
        let spec = StubSpec {
            color_maximum: 0,
            ..StubSpec::bayer_rggb(16, 16)
        };
        let err = stub_codec()
            .decode(&stub_bytes(&spec), &DecodeOpts::default())
            .unwrap_err();
        assert_eq!(err.message_id(), "error.codec.corrupt");
        assert!(err.to_string().contains("levels_degenerate"), "{err}");
    }

    /// stage1_open sanitization detail: garbage as-shot multipliers are
    /// neutralized-and-flagged, never a failed import; all four Bayer
    /// phases classify to the right pattern.
    #[test]
    fn stage1_sanitizes_multipliers_and_classifies_patterns() {
        let lib = stub_lib();
        let spec = StubSpec {
            cam_mul: [f32::NAN, 1.0, 1.5, 1.0],
            ..StubSpec::bayer_rggb(16, 16)
        };
        let meta = stage1::stage1_open(&lib, &stub_bytes(&spec), &DecodeOpts::default()).unwrap();
        assert!(meta.cam_mul_neutralized);
        assert_eq!(meta.cam_mul, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(meta.libraw_version, libraw_sys::PINNED_VERSION);

        use crate::raw::develop::CfaPattern;
        for (tile, want) in [
            ([0u8, 1, 3, 2], CfaPattern::Rggb),
            ([2u8, 1, 3, 0], CfaPattern::Bggr),
            ([1u8, 0, 2, 3], CfaPattern::Grbg),
            ([3u8, 2, 0, 1], CfaPattern::Gbrg),
        ] {
            let spec = StubSpec {
                cfa: StubCfa::Bayer(tile),
                ..StubSpec::bayer_rggb(16, 16)
            };
            let meta =
                stage1::stage1_open(&lib, &stub_bytes(&spec), &DecodeOpts::default()).unwrap();
            assert_eq!(meta.pattern, want, "tile {tile:?}");
        }
    }

    /// Matrix derivation feeds §43.2: a usable rgb_cam records
    /// LibRawTable; an all-zero one falls to the rule-3 assumption.
    #[test]
    fn stage1_matrix_source_resolution() {
        use crate::raw::develop::MatrixSource;
        let lib = stub_lib();
        let meta = stage1::stage1_open(
            &lib,
            &stub_bytes(&StubSpec::bayer_rggb(16, 16)),
            &DecodeOpts::default(),
        )
        .unwrap();
        assert_eq!(meta.matrix_source, MatrixSource::LibRawTable);
        assert!(meta.cam_to_xyz_d65.is_some());

        let spec = StubSpec {
            rgb_cam: [[0.0; 3]; 3],
            ..StubSpec::bayer_rggb(16, 16)
        };
        let meta = stage1::stage1_open(&lib, &stub_bytes(&spec), &DecodeOpts::default()).unwrap();
        assert_eq!(meta.matrix_source, MatrixSource::AssumedSrgb);
        assert!(meta.cam_to_xyz_d65.is_none());
    }

    #[test]
    fn encode_is_write_unsupported() {
        let meta = crate::meta::Metadata::default();
        let img = ExportImage {
            width: 1,
            height: 1,
            plane: &[0.0, 0.0, 0.0, 1.0],
            layers: None,
            profile: None,
            meta: &meta,
        };
        let opts = EncodeOpts {
            format: crate::opts::CodecOptions::Bmp(crate::opts::BmpOptions::default()),
            metadata: crate::opts::MetadataPolicy::Preserve,
            icc: crate::opts::IccPolicy::Embed,
            matte: [1.0, 1.0, 1.0],
            ppi: None,
        };
        assert!(matches!(
            RawCodec::default().encode(&img, &opts),
            Err(CodecError::WriteUnsupported)
        ));
    }
}
