// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! Engine-side export through the ABI (ABI 1.2 additive block — O-UI
//! ABI-growth request 2; the rejected alternative was shell-side Qt
//! encoding of readbacks, a semantics fork: metadata/profile/depth would
//! diverge from the engine's codec matrix).
//!
//! Spec: ES §8.1 (codec trait/registry, `CodecError` taxonomy with its
//! FIXED status + MessageId table), §8.2 (per-format options + defaults),
//! §29 (output subsystem, later rows); plan §2.3 "Codecs v2" row (O-IO's
//! engine capability — this module WRAPS it, §3.11 single-door rule).
//!
//! Composite dispatch mirrors the CLI's `export_bytes` (integration-log
//! wave-2 arbitration 3) exactly: single-raster documents keep core's §4.4
//! reference PNG path byte-for-byte (the goldens bar); everything else
//! composites through render's compositor (`composite_document_with` over
//! the engine's §12.1 op registry, the §34.3 parity-gated path — CLI
//! parity, adjustment/smart-chain documents serve) and encodes through
//! `Codec::encode` over one linear premultiplied f32 plane. Formats = the §8.1 built-in registry ids with
//! write capability (`png`, `jpeg`, `tiff`, `webp`, `gif`, `bmp`, `ora`);
//! `psd` is import-only (§8.5) and answers `error.codec.write_unsupported`.
//!
//! ICC carriage: an embedded working profile exports under the default
//! `IccPolicy::Embed`; builtin/untagged working states embed nothing (the
//! §5.1 builtins are derivable, not byte assets). Metadata: the document
//! carries no §8.6 metadata layer at this checkpoint — `Metadata::default`
//! rides until the document-meta wave (divergence-noted).

use appname_core::{Engine, EngineError, ErrorCode};
use appname_io::{
    Codec, CodecError, CodecOptions, CodecRegistry, EncodeOpts, ExportImage, JpegOptions, Metadata,
};

use crate::registry::ErrorBody;
use crate::AppnameStatus;

/// `CodecError` → stable status, the FIXED ES §8.1 table (verbatim rows):
/// UnrecognizedFormat / UnsupportedFeature / LimitExceeded / Depth / Model
/// / WriteUnsupported → UNSUPPORTED; Truncated / Corrupt / Io → IO;
/// OptionOutOfRange / FrameMapping → INVALID_ARG; Internal → INTERNAL.
fn codec_status(e: &CodecError) -> AppnameStatus {
    match e {
        CodecError::UnrecognizedFormat
        | CodecError::UnsupportedFeature { .. }
        | CodecError::LimitExceeded { .. }
        | CodecError::DepthUnsupported { .. }
        | CodecError::ModelUnsupported { .. }
        | CodecError::WriteUnsupported => AppnameStatus::AppnameErrUnsupported,
        CodecError::Truncated { .. } | CodecError::Corrupt { .. } | CodecError::Io(_) => {
            AppnameStatus::AppnameErrIo
        }
        CodecError::OptionOutOfRange { .. } | CodecError::FrameMapping { .. } => {
            AppnameStatus::AppnameErrInvalidArg
        }
        CodecError::Internal(_) => AppnameStatus::AppnameErrInternal,
    }
}

/// Wire an §8.1 codec failure: the taxonomy MessageId (`error.codec.*`)
/// rides as `code`, the display text as `message` — same error-body shape
/// as the engine taxonomy.
pub(crate) fn codec_body(e: &CodecError) -> ErrorBody {
    ErrorBody::new(
        codec_status(e),
        serde_json::to_string(&serde_json::json!({
            "code": e.message_id(),
            "message": e.to_string(),
        }))
        .expect("literal object serializes"),
    )
}

fn invalid_option(detail: String) -> ErrorBody {
    codec_body(&CodecError::OptionOutOfRange {
        field: "options",
        detail,
    })
}

/// Default per-codec options for a registry id (every §8.2 built-in).
fn default_options(id: &str) -> Option<CodecOptions> {
    Some(match id {
        "png" => CodecOptions::Png(Default::default()),
        "jpeg" => CodecOptions::Jpeg(Default::default()),
        "tiff" => CodecOptions::Tiff(Default::default()),
        "webp" => CodecOptions::Webp(Default::default()),
        "gif" => CodecOptions::Gif(Default::default()),
        "bmp" => CodecOptions::Bmp(Default::default()),
        "ora" => CodecOptions::Ora(Default::default()),
        _ => return None,
    })
}

/// Parse the ABI options JSON (NULL/empty = defaults) onto the codec's
/// option struct. Deliberately minimal at this checkpoint: the one dial
/// the chrome wave's export dialog drives is `quality` (jpeg 0..=100 int;
/// webp 0..=100, selecting lossy). Every other key — including `quality`
/// on a format without that dial — answers `error.codec.bad_option`
/// typed, never silently: dials land with their §8.2 rows.
fn apply_options(id: &str, opts: &mut CodecOptions, text: &str) -> Result<(), ErrorBody> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| invalid_option(format!("options must be a JSON object: {e}")))?;
    let obj = v
        .as_object()
        .ok_or_else(|| invalid_option("options must be a JSON object".into()))?;
    for (key, value) in obj {
        match (key.as_str(), &mut *opts) {
            ("quality", CodecOptions::Jpeg(o)) => {
                let q = value.as_u64().filter(|q| *q <= 100).ok_or_else(|| {
                    invalid_option("jpeg quality must be an integer 0..=100".into())
                })?;
                *o = JpegOptions {
                    quality: q as u8,
                    ..o.clone()
                };
            }
            ("quality", CodecOptions::Webp(o)) => {
                let q = value
                    .as_f64()
                    .filter(|q| (0.0..=100.0).contains(q))
                    .ok_or_else(|| {
                        invalid_option("webp quality must be a number 0..=100".into())
                    })?;
                o.quality = q as f32;
                // An explicit quality selects the lossy arm (the lossless
                // default ignores distortion quality).
                o.lossless = false;
            }
            _ => {
                return Err(invalid_option(format!(
                    "unsupported export option `{key}` for format `{id}` at this checkpoint"
                )))
            }
        }
    }
    Ok(())
}

/// Encode the document composite as `format` bytes — the CLI-parity
/// dispatch (module doc).
pub(crate) fn export_encoded_bytes(
    engine: &Engine,
    format: &str,
    options_json: &str,
) -> Result<Vec<u8>, ErrorBody> {
    let registry = CodecRegistry::builtin();
    let codec: &dyn Codec = registry
        .by_id(format)
        .ok_or_else(|| codec_body(&CodecError::UnrecognizedFormat))?;
    if !codec.capabilities().write {
        return Err(codec_body(&CodecError::WriteUnsupported));
    }
    let mut format_opts = default_options(format).ok_or_else(|| {
        // A registry id without an options constructor is a wiring gap in
        // THIS module, not caller misuse.
        crate::internal(&format!("no options constructor for codec `{format}`"))
    })?;
    apply_options(format, &mut format_opts, options_json)?;

    let doc = engine.document().ok_or_else(|| {
        crate::body_of(&EngineError::bad_target(
            "document not initialized; apply doc.new first",
        ))
    })?;

    // CLI-parity fast path (arbitration 3): a NON-PARAMETRIC single-raster
    // PNG export keeps core's §4.4 reference bytes byte-for-byte — the
    // goldens bar. Gated on `!has_visible_parametric_content` exactly like
    // the CLI's `export_plane`: any parametric content (adjustment, smart,
    // AND parametric shape/text) routes through render below, so every
    // shipped export door serves every layer kind UNIFORMLY through the
    // one compositing truth (the composite-legs slice collapsed the old
    // readback-vs-export divergence; the pure-raster fast path stays only
    // where render is byte-identical to core — pinned by the render-vs-core
    // cross-check suite).
    if format == "png" && !appname_core::has_visible_parametric_content(doc) {
        let rasters = doc
            .z_order_walk()
            .iter()
            .filter_map(|id| doc.layer(*id))
            .filter(|n| n.pixels().is_some())
            .count();
        if rasters <= 1 {
            return appname_core::export_png_bytes(engine).map_err(|e| crate::body_of(&e));
        }
    }

    // The general path: render's parity-gated compositor over the ENGINE'S
    // FULL assembly (§12.1 ops + the Door-6 text / Door-1 vector / fill-math
    // seams — the same `engine.assembly()` the readback door and the CLI's
    // export dispatch pass; parametric shape/text/adjustment/smart all serve)
    // -> one linear premultiplied f32 plane -> Codec::encode.
    // §19.1 one compositing truth: frame on `image_rect()` (the crop
    // sub-region while a crop is active, else the full canvas) — the SAME
    // frame core's reference door and the CLI export dispatch honor.
    let frame = doc.geometry().image_rect();
    let composite = appname_render::composite_document_with(doc, engine.assembly(), frame)
        .map_err(|e| {
            crate::body_of(&EngineError::new(
                ErrorCode::NotImplemented,
                format!("render composite refused: {e}"),
            ))
        })?;
    let plane = composite.region_rgba_f32(frame);

    // ICC carriage: embedded working profile bytes ride under the default
    // Embed policy; builtins/untagged embed nothing (module doc).
    use appname_core::graph::document::DocProfileSource;
    let profile_bytes: Option<Vec<u8>> =
        doc.profile(doc.color().working)
            .and_then(|p| match &p.source {
                DocProfileSource::Icc(bytes) => Some(bytes.as_ref().clone()),
                _ => None,
            });

    let meta = Metadata::default();
    let img = ExportImage {
        width: frame.width() as u32,
        height: frame.height() as u32,
        plane: &plane,
        layers: None,
        profile: profile_bytes.as_deref(),
        meta: &meta,
    };
    let opts = EncodeOpts {
        ppi: Some(doc.geometry().resolution_ppi),
        ..EncodeOpts::new(format_opts)
    };
    codec.encode(&img, &opts).map_err(|e| codec_body(&e))
}

// ---------------------------------------------------------------------------
// ABI symbol.
// ---------------------------------------------------------------------------

use std::ffi::c_char;

use crate::doc::AppnameDoc;
use crate::mem::AppnameError;
use crate::{guarded, read_cstr, registry};

/// ABI 1.2 additive block (gate on `appname_version().abi_minor >= 2`).
/// Export the flattened document composite to `path` as `format` — an
/// §8.1 built-in registry id: `"png"`, `"jpeg"`, `"tiff"`, `"webp"`,
/// `"gif"`, `"bmp"`, `"ora"` (`"psd"` is import-only per §8.5 and answers
/// `error.codec.write_unsupported`). Encoding is ENGINE-side over
/// `Codec::encode` — profile carriage, matte, and depth semantics are the
/// engine matrix's, identical to the CLI's export dispatch (single-raster
/// PNG byte-identical to core's reference export; everything else through
/// the §34.3 parity-gated compositor).
///
/// `options_json` is NULL/empty for the §8.2 defaults, else a JSON object.
/// Supported at this checkpoint: `{"quality": 0..=100}` for `jpeg`
/// (integer) and `webp` (number, selects the lossy arm). Any other key —
/// or `quality` on another format — rejects `error.codec.bad_option`;
/// further dials land with their §8.2 rows, typed until then. Codec
/// failures ride the FIXED §8.1 table (`error.codec.*` codes with their
/// pinned statuses); an unknown format is `error.codec.unrecognized`
/// (`APPNAME_ERR_UNSUPPORTED`); a pre-init handle is
/// `error.command.bad_target`; a filesystem failure writing `path` is
/// `error.io.write_failed`. The write is plain (not temp+rename — the
/// export target is not the document of record; CLI parity).
///
/// # Safety
/// `doc` a handle from this library; `path`/`format` valid NUL-terminated
/// C strings; `options_json` NULL or a valid NUL-terminated C string;
/// `err` NULL or writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_export(
    doc: *mut AppnameDoc,
    path: *const c_char,
    format: *const c_char,
    options_json: *const c_char,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract — all three strings are valid
        // NUL-terminated C strings for the duration of the call.
        let path = unsafe { read_cstr(path, "path") }?;
        let format = unsafe { read_cstr(format, "format") }?;
        let options = if options_json.is_null() {
            String::new()
        } else {
            // SAFETY: non-NULL branch of the same forwarded contract.
            unsafe { read_cstr(options_json, "options_json") }?
        };
        let bytes = registry::with_doc(doc, |state| {
            export_encoded_bytes(&state.engine, &format, &options)
        })?;
        std::fs::write(std::path::Path::new(&path), &bytes).map_err(|e| {
            crate::body_of(&EngineError::new(
                ErrorCode::IoWriteFailed,
                format!("writing {path}: {e}"),
            ))
        })?;
        Ok(AppnameStatus::AppnameOk)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine_with_fill() -> Engine {
        let mut e = crate::doc::abi_assembly().build();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "e0000000-0000-4000-8000-000000000001",
                "kind": "doc.new",
                "params": {"width": 32, "height": 16, "background": "white"}
            })
            .to_string(),
        )
        .unwrap();
        let layer = e.document().unwrap().z_order_walk()[0].as_uuid();
        e.apply_json(
            &json!({
                "schema": "appname.command.v1",
                "id": "e0000000-0000-4000-8000-000000000002",
                "kind": "fill",
                "target": {"layerId": layer.to_string()},
                "params": {"content": "color",
                            "color": {"model": "rgb", "comps": [0.8, 0.2, 0.1, 0.0]}}
            })
            .to_string(),
        )
        .unwrap();
        e
    }

    #[test]
    fn png_single_raster_is_core_reference_bytes() {
        // The CLI-parity fast path: byte-for-byte core §4.4 export.
        let e = engine_with_fill();
        let mine = export_encoded_bytes(&e, "png", "").unwrap();
        let core = appname_core::export_png_bytes(&e).unwrap();
        assert_eq!(mine, core, "single-raster PNG rides core's bytes");
    }

    #[test]
    fn typed_rejects_unknown_format_psd_write_and_bad_options() {
        let e = engine_with_fill();
        for (format, options, want_code) in [
            ("tga", "", "error.codec.unrecognized"),
            ("psd", "", "error.codec.write_unsupported"),
            ("png", "{\"quality\": 90}", "error.codec.bad_option"),
            ("jpeg", "{\"quality\": 101}", "error.codec.bad_option"),
            ("jpeg", "{\"smoothing\": 1}", "error.codec.bad_option"),
            ("jpeg", "not json", "error.codec.bad_option"),
        ] {
            let err = export_encoded_bytes(&e, format, options).unwrap_err();
            let payload: serde_json::Value =
                serde_json::from_str(err.json.to_str().unwrap()).unwrap();
            assert_eq!(payload["code"], want_code, "{format} {options}");
        }
    }

    #[test]
    fn jpeg_quality_changes_the_bytes_deterministically() {
        let e = engine_with_fill();
        let q90a = export_encoded_bytes(&e, "jpeg", "{\"quality\": 90}").unwrap();
        let q90b = export_encoded_bytes(&e, "jpeg", "{\"quality\": 90}").unwrap();
        let q10 = export_encoded_bytes(&e, "jpeg", "{\"quality\": 10}").unwrap();
        assert_eq!(q90a, q90b, "deterministic encode (D-48 discipline)");
        assert_ne!(q90a, q10, "quality is live");
        assert!(q90a.starts_with(&[0xFF, 0xD8]), "JPEG SOI");
    }
}
