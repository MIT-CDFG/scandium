// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! `appname-io` — codec matrix, package interop (CP-0.5 "Codecs v2",
//! slice 1).
//!
//! Spec: ES §7–§8 (package/codecs; §8.1 trait+registry+errors, §8.2
//! built-ins, §8.6 metadata/orientation), §29 (output subsystem, later
//! rows), Parts 42–43, 48; DD ⑩⑪⑫㉞. Interop truth:
//! `bogos/tests/interop/*.yaml` (none of the current corpora cover the
//! slice-1 formats; ORA arrives at slice 2).
//!
//! This slice: JPEG, TIFF, WebP, GIF, BMP import/export with ICC profile
//! CARRIAGE and EXIF orientation (kamadak-exif), behind the §8.1 `Codec`
//! trait + fixed-order registry (PNG's slot wraps core's boundary —
//! ownership migration is a recorded handoff).
//!
//! **Color seam (documented interim):** decodes produce linear-f32
//! premultiplied working buffers under the SAME interim sRGB assumption
//! core documents (`appname_core::io::srgb` module doc), by riding core's
//! public CP-0.1 ingestion door (see [`plane`] module doc). Embedded ICC
//! profiles are EXTRACTED, PRESERVED, and re-ATTACHED (`Imported.profile`
//! / `Metadata.icc` → re-embedded on export) but NOT converted — O-COLOR's
//! CMM (§5) replaces the assumption without touching the codec surface.
//!
//! **Determinism (D-48 discipline):** identical input bytes → identical
//! output bytes. All elected codecs are pure-Rust, single-threaded (the
//! `image` facade rides `default-features = false`, so no rayon), with
//! fixed encoder parameters; palette quantization is in-crate with
//! deterministic ordering ([`quant`]); the §8.6 orientation baked into
//! pixels is value-captured twice over — recorded on
//! `Imported.orientation_applied` AND materialized in the `ingress_png`
//! asset bytes the journaled `edit.paste` envelope references, so replay
//! reproduces the baked pixels byte-identically without re-reading EXIF.
//!
//! **Mutation spine (D-48):** this crate mutates NO document state — it
//! decodes to buffers and encodes from planes. Document ingestion is
//! JOURNALED (`Engine::add_raster_asset` + `edit.paste` envelopes, see
//! [`import_into_engine`] — the landed MAJOR-2 reconciliation); no
//! side-door mutation APIs, ever, and replay reproduces imports
//! byte-identically.

pub mod codecs;
pub mod error;
pub mod export;
pub mod ingress;
pub mod meta;
pub mod opts;
pub mod plane;
pub mod quant;
pub mod raw;
pub mod registry;
pub mod xmlmini;

pub use codecs::ora::export_ora_document;

pub use codecs::psd::{import_psd, rejected_report};
pub use error::{CodecError, IoDirection, IoFailure};
pub use export::pdf::{write_raster_pdf, PdfExportOptions, PdfImageEncoding};
pub use export::pipeline::{encode_variant, estimate_file_size, export_file, write_atomic};
pub use export::policy::apply_policy;
pub use export::preset::{ExportPreset, ProfilePolicy, ResizeSpec};
pub use export::slices::{
    export_slices, ManifestStamp, SliceDef, SliceExportReport, SliceFailure, SliceJob, SliceOutput,
};
pub use export::ExportSource;
pub use ingress::{decode_for_ingress, IngressError, INGRESS_ALLOWED};
pub use meta::Metadata;
pub use opts::{
    BigTiff, BmpDepth, BmpOptions, ChromaSubsampling, CodecOptions, DecodeOpts, Dither, EncodeOpts,
    GifOptions, IccPolicy, JpegOptions, MetadataPolicy, OraOptions, PngDepth, PngOptions,
    SelectiveMeta, TiffCompression, TiffOptions, WebpOptions,
};
pub use plane::{linear_plane_to_srgb8, linear_plane_to_srgb8_over_matte, RgbaPlane};
pub use raw::params::{Demosaic, RawDevelopParams, WhiteBalance};
pub use registry::{
    Codec, CodecCaps, CodecId, CodecRegistry, CompatAction, CompatItem, CompatReport, ExportImage,
    ExportLayer, Imported, ImportedLayer, LayerLocks, LicenseStatus,
};

use appname_core::{ContentHash, Engine, EngineError};

/// Crate name marker kept from the bootstrap smoke test.
pub const CRATE_NAME: &str = "appname-io";

/// Derive a well-formed, deterministic v4-shaped UUID string from a seed
/// (BLAKE3 over the seed, version/variant nibbles forced). Journaled
/// import ids come from content, not clocks or RNGs, so the SAME decode
/// sequence submits byte-identical envelopes — replay identity holds not
/// just per document but across repeat imports of the same bytes.
pub(crate) fn derive_uuid(seed: &str) -> String {
    let h = ContentHash::of_bytes(seed.as_bytes());
    let mut b = *h.as_bytes();
    b[6] = (b[6] & 0x0F) | 0x40; // version 4
    b[8] = (b[8] & 0x3F) | 0x80; // RFC variant
    let hex: Vec<String> = b[..16].iter().map(|v| format!("{v:02x}")).collect();
    format!(
        "{}{}{}{}-{}{}-{}{}-{}{}-{}{}{}{}{}{}",
        hex[0],
        hex[1],
        hex[2],
        hex[3],
        hex[4],
        hex[5],
        hex[6],
        hex[7],
        hex[8],
        hex[9],
        hex[10],
        hex[11],
        hex[12],
        hex[13],
        hex[14],
        hex[15]
    )
}

/// Land a decode into a document through core's JOURNALED ingestion doors
/// (the MAJOR-2 reconciliation, landed on main: `import_layer` is retired;
/// imports are `Engine::add_raster_asset` — content-addressed,
/// decode-validated — plus journaled envelopes). Per surface, in journal
/// order:
/// 1. `edit.paste` referencing the ingested asset, at the layer's own
///    offset (`stack.xml` x/y and friends; single images at [0, 0] — the
///    CLI `import` parity placement);
/// 2. `layer.rename` to the decoded layer name (layered containers) or
///    the caller's `name` (single images) — the wave-4 tail made names
///    applicable, retiring the "Pasted"-only interim;
/// 3. `layer.set_props` when the decoded layer carries non-default
///    opacity/visibility/blend (ORA `stack.xml` fidelity) — omitted
///    entirely for all-default layers, so simple imports stay
///    two-envelope.
///
/// Envelope ids and `newLayerId`s are derived deterministically from the
/// asset id + journal position ([`derive_uuid`]), so replay reproduces
/// imports byte-identically — the property the retired out-of-journal
/// door could not offer.
///
/// The engine must carry the canonical kind assembly
/// (`appname_core::engine::kinds::register`) — a bare `Engine::new()`
/// answers `error.command.not_implemented` for every kind used here.
/// Returns the created layer ids, bottom-to-top.
pub fn import_into_engine(
    engine: &mut Engine,
    name: &str,
    imported: Imported,
) -> Result<Vec<appname_core::LayerId>, EngineError> {
    struct Carrier {
        png: Vec<u8>,
        at: appname_core::IVec2,
        name: String,
        opacity: f32,
        fill_opacity: f32,
        visible: bool,
        blend: appname_core::BlendMode,
        clip_to_below: bool,
        locks: LayerLocks,
    }
    let carriers: Vec<Carrier> = match imported.layers {
        Some(layers) => layers
            .into_iter()
            .map(|l| Carrier {
                png: l.ingress_png,
                at: l.offset,
                name: l.name,
                opacity: l.opacity,
                fill_opacity: l.fill_opacity,
                visible: l.visible,
                blend: l.blend,
                clip_to_below: l.clip_to_below,
                locks: l.locks,
            })
            .collect(),
        None => vec![Carrier {
            png: imported.ingress_png,
            at: appname_core::IVec2::new(0, 0),
            name: name.to_string(),
            opacity: 1.0,
            fill_opacity: 1.0,
            visible: true,
            blend: appname_core::BlendMode::Normal,
            clip_to_below: false,
            locks: LayerLocks::default(),
        }],
    };
    let mut ids = Vec::new();
    for c in carriers {
        let asset_id = engine.add_raster_asset(c.png)?;
        let ordinal = engine.journal().len();
        let envelope_id = derive_uuid(&format!("appname-io/import/envelope/{asset_id}/{ordinal}"));
        let layer_id = derive_uuid(&format!("appname-io/import/layer/{asset_id}/{ordinal}"));
        let paste = serde_json::json!({
            "schema": "appname.command.v1",
            "id": envelope_id,
            "kind": "edit.paste",
            "params": { "assetId": asset_id.to_string(), "at": [c.at.x, c.at.y],
                        "newLayerId": layer_id },
        });
        let receipt = engine.apply_json(&paste.to_string())?;
        ids.extend(
            receipt
                .created_ids
                .iter()
                .copied()
                .map(appname_core::LayerId),
        );

        // 1..=255 scalar values per the layer.rename contract; longer
        // names truncate on a char boundary (decoded names are untrusted).
        let clipped: String = c.name.chars().take(255).collect();
        if !clipped.is_empty() && clipped != "Pasted" {
            let rename_id = derive_uuid(&format!("appname-io/import/rename/{asset_id}/{ordinal}"));
            let rename = serde_json::json!({
                "schema": "appname.command.v1",
                "id": rename_id,
                "kind": "layer.rename",
                "target": { "layerId": layer_id },
                "params": { "name": clipped },
            });
            engine.apply_json(&rename.to_string())?;
        }

        let any_lock = c.locks != LayerLocks::default();
        if c.opacity < 1.0
            || c.fill_opacity < 1.0
            || !c.visible
            || c.blend != appname_core::BlendMode::Normal
            || c.clip_to_below
            || any_lock
        {
            let props_id = derive_uuid(&format!("appname-io/import/props/{asset_id}/{ordinal}"));
            let mut params = serde_json::Map::new();
            if c.opacity < 1.0 {
                params.insert("opacity".into(), serde_json::json!(c.opacity));
            }
            if c.fill_opacity < 1.0 {
                params.insert("fillOpacity".into(), serde_json::json!(c.fill_opacity));
            }
            if c.clip_to_below {
                params.insert("clipToBelow".into(), serde_json::json!(true));
            }
            if any_lock {
                params.insert(
                    "locks".into(),
                    serde_json::json!({
                        "transparency": c.locks.transparency,
                        "pixels": c.locks.pixels,
                        "position": c.locks.position,
                    }),
                );
            }
            if !c.visible {
                params.insert("visible".into(), serde_json::json!(false));
            }
            if c.blend != appname_core::BlendMode::Normal {
                params.insert(
                    "blend".into(),
                    serde_json::to_value(c.blend).unwrap_or_default(),
                );
            }
            let props = serde_json::json!({
                "schema": "appname.command.v1",
                "id": props_id,
                "kind": "layer.set_props",
                "target": { "layerId": layer_id },
                "params": params,
            });
            engine.apply_json(&props.to_string())?;
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    /// Bootstrap smoke test: proves the crate compiles and `cargo test`
    /// exercises it.
    #[test]
    fn bootstrap_smoke() {
        assert_eq!(super::CRATE_NAME, "appname-io");
    }
}
