// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §7.1/§7.5 (package assets), §10.2 (`samples` — assets are
//! never inline in JSON); integration-log wave-2 binding arbitration 4
//! (samples as package assets, `appname.samples.v1` bytes contract).
//!
//! Session/package asset store: opaque byte blobs addressed by id, the
//! captured VALUES that envelopes reference (`samples` field). Core never
//! decodes them (the consuming [`super::kind_impl::KindImpl`] owns the
//! format); replay carries them alongside the journal (D-48 value
//! capture), and the package writer persists exactly the blobs the journal
//! references (GC — arbitration 4). Persistence/GC land with the package
//! wiring; this module is the store itself.
//!
//! Ids are CONTENT-DERIVED (UUIDv5 over the blob's BLAKE3 hex, the fixed
//! engine namespace): the same bytes always address the same asset —
//! dedup is inherent, script authors can precompute ids, and §7.1's
//! content-addressed `assets/` layout is honored in uuid form.

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::hash::ContentHash;

/// Asset kind tag (wave-1 CP-0.5: the samples pipeline generalizes to
/// raster image assets — the journaled-import reconciliation's carrier).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AssetKind {
    /// `appname.samples.v1` stroke recordings (CP-0.3).
    #[default]
    Samples,
    /// Encoded raster images (PNG bytes at CP-0.5) — `edit.paste`'s
    /// "ingested raster asset" (§9.2/§10.3).
    RasterImage,
    /// Document-embedded font faces (§23.5 subset embedding; §44.2 step 2
    /// — TEXTVEC Door 10). Bytes are the face blob VERBATIM; the TextLayout
    /// service owns parsing (core stays font-blind).
    FontFace,
    /// Single-channel f32 coverage planes (the D-ML-1 ingress carrier —
    /// O-ML filing, reports/O-ML-log.md 2026-08-16: "an ingested
    /// single-channel f32 raster asset"): `appname.mask.v1` bytes per
    /// [`encode_mask_plane`]/[`decode_mask_plane`], applied to the
    /// selection surface or a layer raster mask through the §13.2 combine
    /// doors ([`super::kind_impl::KindContext::apply_mask_asset_to_selection`]).
    MaskPlane,
}

/// The `appname.mask.v1` wire layout (engine-internal asset format; the
/// package carries assets as OPAQUE content-addressed blobs per §7.1, so
/// this is an asset-format definition, not a package-schema change —
/// fold-in candidate flagged in the lane log): one JSON header line
/// `{"schema":"appname.mask.v1","x":…,"y":…,"w":…,"h":…}` + `\n` +
/// exactly `w*h` little-endian f32 coverage values, row-major over the
/// rect, each finite and in `[0, 1]` (§13.1 single-representation rule).
/// Bit-exact: encode∘decode ≡ id, so content addressing is stable.
pub const MASK_PLANE_SCHEMA: &str = "appname.mask.v1";

/// Encode a coverage plane as `appname.mask.v1` bytes (test/producer
/// helper; O-ML's export side mirrors this layout).
pub fn encode_mask_plane(rect: crate::geom::IRect, values: &[f32]) -> Vec<u8> {
    assert_eq!(
        values.len(),
        (rect.width().max(0) as usize) * (rect.height().max(0) as usize),
        "encode_mask_plane: values must cover the rect exactly"
    );
    let header = format!(
        "{{\"schema\":\"{MASK_PLANE_SCHEMA}\",\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}\n",
        rect.min.x,
        rect.min.y,
        rect.width(),
        rect.height()
    );
    let mut out = header.into_bytes();
    out.reserve(values.len() * 4);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode + validate `appname.mask.v1` bytes. Typed per the §40.1 rows:
/// malformed header/payload → `error.schema.out_of_range` against the
/// offending part; wrong schema stamp → `error.schema.wrong_schema`.
pub fn decode_mask_plane(
    bytes: &[u8],
) -> Result<(crate::geom::IRect, Vec<f32>), crate::engine::error::EngineError> {
    use crate::engine::error::{EngineError, ErrorCode};
    let nl = bytes
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| EngineError::range("mask", "appname.mask.v1: missing header line"))?;
    let header: serde_json::Value = serde_json::from_slice(&bytes[..nl])
        .map_err(|e| EngineError::range("mask", format!("appname.mask.v1 header: {e}")))?;
    if header.get("schema").and_then(serde_json::Value::as_str) != Some(MASK_PLANE_SCHEMA) {
        return Err(EngineError::new(
            ErrorCode::WrongSchema,
            "mask asset schema must be appname.mask.v1",
        ));
    }
    let geti = |k: &str| -> Result<i64, EngineError> {
        header
            .get(k)
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| EngineError::range("mask", format!("header `{k}` must be an integer")))
    };
    let (x, y, w, h) = (geti("x")?, geti("y")?, geti("w")?, geti("h")?);
    if w < 1 || h < 1 || w > 1 << 20 || h > 1 << 20 {
        return Err(EngineError::range(
            "mask",
            format!("plane extent {w}x{h} out of range (floor 1x1)"),
        ));
    }
    let count = (w as usize)
        .checked_mul(h as usize)
        .filter(|n| *n <= (1usize << 28))
        .ok_or_else(|| EngineError::range("mask", "plane pixel count overflows the cap"))?;
    let payload = &bytes[nl + 1..];
    if payload.len() != count * 4 {
        return Err(EngineError::range(
            "mask",
            format!(
                "payload carries {} bytes, expected {} (w*h*4)",
                payload.len(),
                count * 4
            ),
        ));
    }
    let mut values = Vec::with_capacity(count);
    for chunk in payload.chunks_exact(4) {
        let v = f32::from_le_bytes(chunk.try_into().expect("chunks_exact(4)"));
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(EngineError::range(
                "mask",
                "coverage values must be finite and in [0, 1] (§13.1)",
            ));
        }
        values.push(v);
    }
    Ok((
        crate::geom::IRect::new(x as i32, y as i32, (x + w) as i32, (y + h) as i32),
        values,
    ))
}

/// Derive the content-addressed asset id for a byte blob.
pub fn asset_id_for(bytes: &[u8]) -> Uuid {
    let hex = ContentHash::of_bytes(bytes).to_hex();
    Uuid::new_v5(
        &crate::engine::doc_new::APPNAME_NS,
        format!("asset/{hex}").as_bytes(),
    )
}

/// Opaque asset blobs by id (Arc'd: journal replay and packages share).
///
/// GC discipline (arbitration 4): a blob persists iff it is referenced by
/// a journal envelope OR it was registered THIS session (`pending` — the
/// `asset add` → stroke pipeline registers before the referencing envelope
/// exists). Loading clears pending, so an asset that never gets referenced
/// survives exactly one save and is collected on the next save after a
/// reopen (bounded orphan lifetime).
#[derive(Default, Clone)]
pub struct AssetStore {
    blobs: HashMap<Uuid, (Arc<Vec<u8>>, AssetKind)>,
    /// Session-registered ids awaiting their referencing envelope.
    pending: std::collections::HashSet<Uuid>,
}

impl AssetStore {
    pub fn new() -> AssetStore {
        AssetStore::default()
    }

    pub fn bytes(&self, id: Uuid) -> Option<&[u8]> {
        self.blobs.get(&id).map(|(b, _)| b.as_slice())
    }

    pub fn kind_of(&self, id: Uuid) -> Option<AssetKind> {
        self.blobs.get(&id).map(|(_, k)| *k)
    }

    pub fn contains(&self, id: Uuid) -> bool {
        self.blobs.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    /// Ids in deterministic (sorted) order — package writer input.
    pub fn ids_sorted(&self) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self.blobs.keys().copied().collect();
        ids.sort();
        ids
    }

    /// Insert content-addressed; returns the derived id. Identical bytes
    /// land on the same id (inherent dedup).
    #[cfg_attr(not(test), allow(dead_code))] // wave-2 commit-2 consumers (package persistence + CLI asset door)
    pub(crate) fn insert(&mut self, bytes: Vec<u8>) -> Uuid {
        self.insert_kind(bytes, AssetKind::Samples)
    }

    pub(crate) fn insert_kind(&mut self, bytes: Vec<u8>, kind: AssetKind) -> Uuid {
        let id = asset_id_for(&bytes);
        self.blobs
            .entry(id)
            .or_insert_with(|| (Arc::new(bytes), kind));
        self.pending.insert(id);
        id
    }

    /// True while the blob awaits its first referencing envelope (persists
    /// through the next save regardless of references).
    pub fn is_pending(&self, id: Uuid) -> bool {
        self.pending.contains(&id)
    }

    /// Re-arm the pending grace over every blob currently held, so a save
    /// keeps them even though no envelope references them yet.
    ///
    /// q-tutorial/Q-5 (MAJOR): the grace is per SESSION, and loading clears
    /// it — correct for a long-lived shell, wrong for a door where each
    /// registration is its own process. `appname asset add` loads, inserts
    /// and saves, so the previous registration was neither referenced nor
    /// pending by the time the next add saved, and the save-time GC dropped
    /// it. Three adds left one asset, each `ok`, and the loss surfaced later
    /// at the stroke as "samples asset does not resolve". The CLI's asset
    /// door calls this after loading, which says exactly what it means: the
    /// blobs in this package are registered and awaiting their first use.
    pub fn rearm_pending_for_all(&mut self) {
        let ids: Vec<Uuid> = self.blobs.keys().copied().collect();
        self.pending.extend(ids);
    }

    /// Loader door: install a blob under its recorded id, verifying the
    /// content address (typed mismatch is the caller's to report).
    #[cfg_attr(not(test), allow(dead_code))] // wave-2 commit-2 consumers (package persistence + CLI asset door)
    pub(crate) fn insert_loaded(&mut self, id: Uuid, bytes: Vec<u8>) -> Result<(), Uuid> {
        self.insert_loaded_kind(id, bytes, AssetKind::Samples)
    }

    pub(crate) fn insert_loaded_kind(
        &mut self,
        id: Uuid,
        bytes: Vec<u8>,
        kind: AssetKind,
    ) -> Result<(), Uuid> {
        let derived = asset_id_for(&bytes);
        if derived != id {
            return Err(derived);
        }
        self.blobs
            .entry(id)
            .or_insert_with(|| (Arc::new(bytes), kind));
        Ok(())
    }

    /// Drop every blob that is neither referenced nor session-pending
    /// (save-time GC, arbitration 4).
    #[cfg_attr(not(test), allow(dead_code))] // in-session GC consumer arrives with history limits
    pub(crate) fn retain_referenced(&mut self, referenced: &std::collections::HashSet<Uuid>) {
        let pending = &self.pending;
        self.blobs
            .retain(|id, _| referenced.contains(id) || pending.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_content_derived_and_deduped() {
        let mut store = AssetStore::new();
        let a = store.insert(b"blob-one".to_vec());
        let b = store.insert(b"blob-one".to_vec());
        let c = store.insert(b"blob-two".to_vec());
        assert_eq!(a, b, "same bytes, same id");
        assert_ne!(a, c);
        assert_eq!(store.len(), 2);
        assert_eq!(store.bytes(a).unwrap(), b"blob-one");
        assert_eq!(
            a,
            asset_id_for(b"blob-one"),
            "precomputable by script authors"
        );
    }

    #[test]
    fn loaded_blobs_verify_their_content_address() {
        let mut store = AssetStore::new();
        let id = asset_id_for(b"payload");
        assert!(store.insert_loaded(id, b"payload".to_vec()).is_ok());
        let err = store.insert_loaded(id, b"tampered".to_vec()).unwrap_err();
        assert_ne!(err, id, "mismatch reports the derived id");
    }

    #[test]
    fn mask_plane_encode_decode_is_bit_exact() {
        let rect = crate::geom::IRect::new(-8, 4, 24, 20);
        // Awkward f32s included: encode∘decode must be the identity
        // (content addressing stability rides on it).
        let values: Vec<f32> = (0..(32 * 16))
            .map(|i| ((i % 97) as f32 / 96.0).min(1.0))
            .collect();
        let bytes = encode_mask_plane(rect, &values);
        let (r2, v2) = decode_mask_plane(&bytes).unwrap();
        assert_eq!(r2, rect);
        assert_eq!(
            v2.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "bit-exact round trip"
        );
        // Out-of-range coverage refuses (§13.1).
        let bad = encode_mask_plane(crate::geom::IRect::new(0, 0, 1, 1), &[1.0]);
        let mut bad = bad;
        let n = bad.len();
        bad[n - 4..].copy_from_slice(&1.5f32.to_le_bytes());
        assert!(decode_mask_plane(&bad).is_err());
        // Truncated payload refuses.
        let mut short = encode_mask_plane(crate::geom::IRect::new(0, 0, 2, 1), &[0.0, 0.0]);
        short.truncate(short.len() - 1);
        assert!(decode_mask_plane(&short).is_err());
    }

    #[test]
    fn gc_retains_referenced_and_session_pending() {
        let mut store = AssetStore::new();
        // Loaded (non-pending) blobs: only references keep them.
        let keep = asset_id_for(b"keep");
        let dropped = asset_id_for(b"drop");
        store.insert_loaded(keep, b"keep".to_vec()).unwrap();
        store.insert_loaded(dropped, b"drop".to_vec()).unwrap();
        // Session-registered blob: pending grace holds it unreferenced.
        let pending = store.insert(b"pending".to_vec());
        assert!(store.is_pending(pending));
        assert!(!store.is_pending(keep));

        let referenced: std::collections::HashSet<Uuid> = [keep].into_iter().collect();
        store.retain_referenced(&referenced);
        assert_eq!(store.len(), 2);
        assert!(store.contains(keep));
        assert!(store.contains(pending));
        assert!(!store.contains(dropped));
    }

    /// q-tutorial/Q-5 (MAJOR): `appname asset add` loads, inserts and saves,
    /// so a blob registered by an EARLIER invocation arrives unreferenced and
    /// unpending, and this save's GC dropped it — three adds left one asset,
    /// each returning ok, and the loss surfaced later at the stroke as
    /// "samples asset does not resolve". Re-arming the grace over what the
    /// package already holds is what the CLI door now does after loading.
    #[test]
    fn rearming_the_grace_keeps_blobs_an_earlier_session_registered() {
        let mut store = AssetStore::new();
        let earlier = asset_id_for(b"registered earlier");
        store
            .insert_loaded(earlier, b"registered earlier".to_vec())
            .unwrap();
        // Exactly the state the second `asset add` loads into: present, not
        // referenced by anything, and no longer pending.
        assert!(!store.is_pending(earlier));
        let none: std::collections::HashSet<Uuid> = std::collections::HashSet::new();

        // CONTROL — without the re-arm, the save GC drops it. That is the bug.
        let mut control = store.clone();
        control.retain_referenced(&none);
        assert!(!control.contains(earlier), "the control must show the loss");

        store.rearm_pending_for_all();
        assert!(store.is_pending(earlier));
        let this_session = store.insert(b"registered now".to_vec());
        store.retain_referenced(&none);
        assert!(store.contains(earlier), "the earlier registration survives");
        assert!(store.contains(this_session), "and so does this one");
        assert_eq!(store.len(), 2);
    }
}
