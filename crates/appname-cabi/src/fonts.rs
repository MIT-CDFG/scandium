// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 the Appname contributors

//! The font-provisioning door — ABI 1.6 additive block.
//!
//! Spec: ES §44.1 (host-provisioned faces feed layout as INPUT — the
//! engine stays platform-blind; a face is `(bytes, source class,
//! host-derived FontRef metadata)`), §44.2 (the FontDb resolution
//! order + the loader's asset-index contract), §23.5 (fsType embedding
//! policy — surfaced as `embeddable`); §9.1/Appendix N (additive-block
//! discipline); TEXTVEC Door 10 (the enumeration surface). The
//! Door-10-mirror shape is O-INT-ruled; the engine half
//! (`register_provided_face` + `register_with_handle`) is TEXTVEC's,
//! landed with zero core-trait change — this door reaches the concrete
//! `TextLayoutService` through the handle `DocState` retains at
//! construction (see `doc::abi_assembly_with_text_handle`).
//!
//! # Appendix-N justification
//!
//! Providing a host face and reading the FontDb are NOT expressible by
//! composing existing symbols: the concrete `register_provided_face`
//! lives on the text service, not the `TextLayout` trait the rest of
//! the ABI sees, and the §44.1 metadata + source-class + idempotence +
//! §23.5 dedup rules are normative. One implementation lives in
//! `appname-text`; this door hands it bytes + metadata and reads back
//! the deterministic enumeration — a per-front-end reimplementation
//! would fork §44 semantics.
//!
//! # Call-order is the door's invariant [CONFORMANCE]
//!
//! The door PRESERVES caller order: providing faces A, B, C in that
//! sequence registers them in that sequence, and the enumeration
//! reflects the service's deterministic §44.2 view over the resulting
//! set. Whether a *loader* calls in document order is the loader's
//! §44.2 asset-index contract, NOT this door's concern — the boundary
//! is: the door is faithful to ITS caller's order; the document-order
//! guarantee is the loader's, upstream of here.
//!
//! # What this door is NOT
//!
//! `DocEmbedded` faces (the document's own embedded fonts) register
//! through the package-loader door (`appname_asset_create` /
//! `register_embedded_face`), never here — the engine refuses a
//! `DocEmbedded` source on this surface, and so does this door
//! (typed, before touching the service).

use std::ffi::c_char;

use serde_json::json;

use appname_core::engine::assets::asset_id_for;
// `enumerate_faces` / `fonts_stamp` are `TextLayout` trait methods
// (`register_provided_face` is inherent on the concrete service); the
// trait must be in scope to call them on the retained handle.
use appname_core::engine::text_ops::TextLayout;
use appname_text::fontdb::FaceSource;

use crate::doc::AppnameDoc;
use crate::mem::{AppnameError, AppnameString};
use crate::registry::{self, ErrorBody};
use crate::{body_of, guarded, invalid_arg, read_bytes, read_cstr, AppnameStatus};

/// Parse the wire source token into the engine enum. `docEmbedded` is a
/// valid token but REFUSED on this surface (it belongs to the loader
/// door) — a typed refusal, not an unknown-token error, so the message
/// names the right door.
fn parse_source(token: &str) -> Result<FaceSource, ErrorBody> {
    match token {
        "user" => Ok(FaceSource::User),
        "system" => Ok(FaceSource::System),
        "bundled" => Ok(FaceSource::Bundled),
        "docEmbedded" => Err(invalid_arg(
            "source \"docEmbedded\" is not provisionable here — document-embedded \
             faces register through the package loader (appname_asset_create), \
             never as provided faces (ES §44.2)",
        )),
        other => Err(invalid_arg(&format!(
            "unknown face source {other:?} (expected user|system|bundled)"
        ))),
    }
}

/// ABI 1.6 additive block (gate on `appname_version().abi_minor >= 6`).
/// Provide a HOST face to the document's layout FontDb (ES §44.1): the
/// `bytes` are validated (a face BOTH HarfBuzz and FreeType accept, else
/// typed refusal), keyed by their content-addressed asset id, and
/// registered with the given `source` (`user`|`system`|`bundled`) and
/// host-derived `family`/`style`/`postscript_name`.
///
/// **Idempotent per (asset, bytes).** Re-providing identical bytes with
/// the same id is a pure no-op (first host metadata wins within this
/// door); providing DIFFERENT bytes under an id already registered, or a
/// `postscript_name` already held by a DIFFERENT face, refuses typed
/// (§23.5 dedup would otherwise silently shadow one face and hand tofu to
/// text while enumeration lists the covering face as available — a host
/// bug surfaces instead). `docEmbedded` as a source refuses typed (that
/// class is the loader's door).
///
/// On `APPNAME_OK`, `*out_id` (when non-NULL) receives the
/// content-addressed asset id as a UUID string (free with
/// `appname_string_free`) — the stable handle for this face.
///
/// # Safety
/// `doc` a live handle; `bytes` points to `bytes_len` readable bytes when
/// `bytes_len > 0`; `source`/`family`/`style`/`postscript_name` valid
/// NUL-terminated C strings; `out_id` NULL or writable; `err` NULL or
/// writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn appname_doc_provide_face(
    doc: *mut AppnameDoc,
    bytes: *const u8,
    bytes_len: usize,
    source: *const c_char,
    family: *const c_char,
    style: *const c_char,
    postscript_name: *const c_char,
    out_id: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        // SAFETY: forwarded caller contract — `bytes` covers `bytes_len`
        // readable bytes; consumed within this call only.
        let blob = unsafe { read_bytes(bytes, bytes_len, "bytes") }?;
        // SAFETY: the four token/string params are valid NUL-terminated
        // C strings per the documented contract.
        let source = parse_source(&unsafe { read_cstr(source, "source") }?)?;
        let family = unsafe { read_cstr(family, "family") }?;
        let style = unsafe { read_cstr(style, "style") }?;
        let psname = unsafe { read_cstr(postscript_name, "postscript_name") }?;

        let asset = asset_id_for(blob);
        registry::with_doc(doc, |state| {
            let svc = state.text_handle.as_ref().ok_or_else(|| {
                crate::internal(
                    "this document has no text-layout service (bare assembly); \
                     provide-face is unavailable",
                )
            })?;
            svc.register_provided_face(asset, blob, source, &family, &style, &psname)
                .map_err(|e| body_of(&e))
        })?;

        if !out_id.is_null() {
            // SAFETY: non-NULL checked; caller contract says writable.
            unsafe { *out_id = registry::mint_string(asset.to_string()) };
        }
        Ok(AppnameStatus::AppnameOk)
    })
}

/// ABI 1.6 additive block. Read the document's layout FontDb — the
/// deterministic Door-10 enumeration plus the font-environment
/// generation stamp (ES §44.2 step-5 order; §44.1 step-4 "installing a
/// missing font re-resolves"). On `APPNAME_OK`, `*out_json` (when
/// non-NULL) receives (free with `appname_string_free`):
///
/// ```json
/// { "schema": "appname.fontfaces.v1",
///   "stamp": 3,
///   "faces": [ { "family": "...", "style": "...",
///                "postscriptName": "...", "embeddable": true }, … ] }
/// ```
///
/// `stamp` bumps whenever the face set changes (a provide, an embed, an
/// install/uninstall watch) — a consumer keys caches on it. `embeddable`
/// is the §23.5 fsType verdict (restricted / bitmap-only refuse). A
/// document whose engine has no text service (bare assembly) reports an
/// empty set with `stamp` 0.
///
/// # Safety
/// `doc` a live handle; `out_json` NULL or writable; `err` NULL or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn appname_doc_font_faces(
    doc: *mut AppnameDoc,
    out_json: *mut *mut AppnameString,
    err: *mut *mut AppnameError,
) -> AppnameStatus {
    guarded(err, || {
        let wire = registry::with_doc(doc, |state| {
            let (stamp, faces) = match state.text_handle.as_ref() {
                Some(svc) => (svc.fonts_stamp(), svc.enumerate_faces()),
                None => (0, Vec::new()),
            };
            Ok(json!({
                "schema": "appname.fontfaces.v1",
                "stamp": stamp,
                "faces": faces
                    .into_iter()
                    .map(|f| json!({
                        "family": f.family,
                        "style": f.style,
                        "postscriptName": f.postscript_name,
                        "embeddable": f.embeddable,
                    }))
                    .collect::<Vec<_>>(),
            }))
        })?;
        if !out_json.is_null() {
            // SAFETY: non-NULL checked; caller contract says writable.
            unsafe { *out_json = registry::mint_string(wire.to_string()) };
        }
        Ok(AppnameStatus::AppnameOk)
    })
}
