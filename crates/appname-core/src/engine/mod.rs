// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §9.6 (command-as-JSON boundary), §10.1–§10.4 (commands, schema,
//! catalog, COW undo), §40.1/§40.2 (taxonomy + doc.new contract); D-48
//! determinism spine; DD ⑰; plan §2.1 rows "Command engine" +
//! "doc.new + journal + undo".
//!
//! **The single mutation door.** Every document mutation is an
//! `appname.command.v1` envelope through [`Engine::apply_json`] /
//! [`Engine::apply_bytes`] (raw text/bytes at the public boundary —
//! binding arbitration WD-1: the conformance corpus is fed byte-for-byte,
//! never pre-parsed). The slice-1 tile/graph doors are `pub(crate)`; this
//! module is their only caller.
//!
//! **Pinned validation order** (binding arbitration OQ-3; where v5.5 is
//! silent the proto's REPORT-A-proven taxonomy wins, OQ-2). Stages, each
//! with its error family; the FIRST failing stage reports:
//!
//! | stage | check | codes |
//! |---|---|---|
//! | 0 | API boundary: ≤ 1 MiB (§9.6), UTF-8, JSON parses, top level is an object | `error.api.invalid_arg` |
//! | 1 | envelope schema: known keys; `schema` const; `id` uuid non-nil; `kind` string; typed `target`/`params`/`samples`/`mergeKey`/`reversible`/`timestamp` | `error.schema.wrong_schema` / `error.schema.missing_field` / `error.schema.out_of_range` / `error.schema.duplicate_id` (nil id) |
//! | 2 | `id` unused in the journal (checked before kind — proto order) | `error.schema.duplicate_id` |
//! | 3 | `kind` registered (generated registry == `_index.yaml` == 102) | `error.command.unknown_kind` |
//! | 4 | kind implemented at this checkpoint | `error.command.not_implemented` (OQ-1: emitted although v5.5 message_ids lacks it — upstream mint list) |
//! | 5 | params vs the kind's generated `paramsSchema` (defaults filled; required order = schema order; unknown keys last) | `error.schema.missing_field` / `error.schema.out_of_range` |
//! | 6 | target/live-state, per kind. doc.new: (6a) fresh handle, (6b) profile space, (6c) paused depth, (6d) unsupported mode (`background_color` resolves — [`doc_new::background_value`]) | `error.command.bad_target` / `error.color.profile_space_mismatch` / `error.command.not_implemented` |
//! | 7 | execute through the §10.1 recorder, then commit: prune touched coords (§2.4) → capture afters → per-(kind,model) dedup intern (§3.8; review scoping rule) → journal append → history push | — |
//!
//! **Journal + undo.** The journal is the append-only envelope list; the
//! cursor marks the in-effect prefix; applying on an undone tail truncates
//! it (linear history, DD ⑰). Undo restores via recorded state — for
//! `doc.new` the §40.2 handle inverse (back to the empty pre-init shell,
//! the whole `Document` MOVED aside, zero pixel copies); pixel-scope kinds
//! restore per-tile `before` Arcs ([`delta::DeltaSet`], §10.4 — never the
//! proto's whole-layer snapshots). Replay of a journal is deterministic:
//! engine-generated ids are UUIDv5 over the creating envelope id (§9.6) and
//! [`Engine::state_hash`] is byte-stable across replays.

pub mod assets;
pub mod color_glue;
pub mod delta;
pub mod doc_new;
pub mod envelope;
pub mod error;
pub mod fill_reg;
pub mod gradient;
pub mod history_spill;
pub mod kind_impl;
pub mod kinds;
pub mod kinds_apply_image;
pub mod kinds_channels;
pub mod kinds_guides;
pub mod kinds_layer;
pub mod kinds_layer_geom;
pub mod kinds_merge;
pub mod kinds_mode;
pub mod kinds_ops;
pub mod kinds_slices_meta;
pub mod kinds_text;
pub mod kinds_vector;
pub mod mask_ops;
pub mod op_eval;
pub mod op_impl;
pub mod registry;
pub mod registry_gen;
pub mod sampling;
pub mod text_ops;
pub mod vector_ops;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use crate::geom::IRect;
use crate::graph::document::Document;
use crate::graph::id::LayerId;
use crate::hash::ContentHash;

use assets::AssetStore;
use delta::{DedupIndex, DeltaRecorder, DeltaSet};
use envelope::Envelope;
use error::{EngineError, ErrorCode};
use kind_impl::{EngineAssembly, KindContext, KindImpl};

/// One receipt-side invalidation (integration-log wave-2 arbitration 2,
/// per O-RENDER's spec): the document-space rect (lod 0) this command
/// changed on one layer. Tile-writing kinds report each touched
/// `TileCoord::pixel_rect()`; property/structural kinds report the node's
/// full bounds (before ∪ after). Undo/redo rows carry the SAME rects.
/// Render's `DirtyTracker::feed` consumes these (type unification is an
/// O-RENDER CP-0.5 cleanup).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirtyRect {
    pub layer: LayerId,
    pub rect: IRect,
}

/// Receipt returned by the apply entry (proto shape + wave-2 dirty rects).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReceipt {
    pub id: Uuid,
    pub kind: String,
    /// Journal length after this command.
    pub revision: u64,
    /// Ids of objects this command created (deterministic across replay,
    /// §9.6 — for doc.new: `[root group, background layer]`).
    pub created_ids: Vec<Uuid>,
    /// Receipt-side invalidations (arbitration 2).
    pub dirty: Vec<DirtyRect>,
    /// §10.5 class-M coalescing: `true` when this command MERGED into the
    /// previous step (one undo step; the §9.6 history event's `coalesced`
    /// flag). Serialized only when set — every non-merged receipt stays
    /// byte-identical to its pre-§10.5 form.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub coalesced: bool,
    /// §6.2 warning MessageIds surfaced with the result (mode-convert's
    /// warnings-before-execute row; door C5). Serialized only when
    /// non-empty — every warning-free receipt stays byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// One row of the history list (proto shape).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub index: usize,
    pub id: Uuid,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// False for entries behind the cursor (undone, redoable).
    pub active: bool,
    /// The step's receipt dirty rects — identical on apply, undo, and redo
    /// (arbitration 2). Empty for rows derived from a reopened journal
    /// (history is not persisted, §10.4).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dirty: Vec<DirtyRect>,
    /// The filter this row applied, when `kind` alone cannot name it
    /// (A-18; B-06/Q-D24 ruled ENGINE PROJECTION). `filter.apply` is ONE
    /// kind over the EIGHT filters registered in §16.1, so a row bearing
    /// only `kind` projects all eight onto the single label
    /// `history.filter.apply` ("Apply Filter") and the shell cannot
    /// recover which ran. This carries the filter's own id (e.g.
    /// `filter.gaussian_blur`), which is a REQUIRED param of the kind.
    ///
    /// `None` for every kind that needs no such subject, and skipped on
    /// the wire when absent — so 101 of the 102 kinds serialize exactly
    /// as they did before this field existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_id: Option<String>,
}

/// The subject a generic `kind` cannot name on its own (A-18).
///
/// Read from the envelope at row-build time rather than stored when the
/// step is recorded: the engine ALREADY holds the answer, so this stays a
/// projection fix — no new recorded state, no journal format change, no
/// migration. `HistoryStep` deliberately keeps no params; the journal
/// does, and both row builders have the envelope in hand.
fn envelope_filter_id(env: &Envelope) -> Option<String> {
    env.params
        .as_ref()?
        .get("filterId")?
        .as_str()
        .map(str::to_string)
}

/// Recorded inverse of one journal step.
enum StepDelta {
    /// doc.new: §40.2 handle inverse — undo moves the whole document aside
    /// (an O(1) move, no pixel copy), redo moves it back. The sealed
    /// `DeltaSet` (background tiles + tree ops) is kept for accounting and
    /// for the tile-delta consumers of later slices.
    DocInit {
        stash: Option<Document>,
        #[cfg_attr(not(test), allow(dead_code))]
        // §10.4 accounting consumer arrives with history limits
        deltas: DeltaSet,
    },
    /// Assembled kinds (§10.1/§10.4): undo swaps the `before` Arcs back in,
    /// redo the `after` Arcs — O(deltas) pointer swaps, the real COW-delta
    /// path the slice-2 substrate was built for.
    Deltas { deltas: DeltaSet },
    /// §10.4 history spill: a cold below-cursor step whose `before`
    /// payloads live on the scratch disk (compressed, re-accounted at
    /// compressed size). Undo revives it back to `Deltas` transparently.
    Spilled(history_spill::SpilledDeltaSet),
    /// CheckpointOnly kinds (§10.3 class C, §10.4 `inverse: snap`): the
    /// OTHER document state rides the step — undo/redo are O(1) swaps of
    /// the live document with the stash (Arc-shared tiles, no pixel copy).
    Snapshot { other: Document },
    /// †-marker journal entries (`history.snapshot`, §10.3/§40.8): a NO-OP
    /// placeholder that keeps the `journal[base+i] ↔ history[i]`
    /// correspondence exact (fix-round B1) while staying OUTSIDE the undo
    /// stack — undo/redo skip over it, [`Engine::history`] filters it, and
    /// it never occupies a §40.8 position slot.
    Marker,
}

struct HistoryStep {
    env_id: Uuid,
    kind: String,
    timestamp: Option<String>,
    delta: StepDelta,
    /// Receipt dirty rects, echoed on undo/redo rows (arbitration 2).
    dirty: Vec<DirtyRect>,
}

/// The command engine: document slot + append-only journal + COW-delta
/// history + the per-context dedup index (commit pipeline).
pub struct Engine {
    doc: Option<Document>,
    journal: Vec<Envelope>,
    journal_ids: HashSet<Uuid>,
    /// Entries at `cursor..journal.len()` are undone (redoable); applying a
    /// new command truncates them (linear history, DD ⑰).
    cursor: usize,
    history: Vec<HistoryStep>,
    /// Journal offset where in-process history begins (wave-2b reviewer
    /// item, SUFFIX-ALIGNED history): entries below the base are
    /// persisted-journal rows with NO deltas (history is not persisted,
    /// §10.4) — fresh engines use base 0; reopening sets the base to the
    /// loaded journal length, so POST-open commands undo/redo in-process
    /// while the persisted prefix stays typed-refused at the boundary.
    /// Invariant: `history.len() == journal.len() - history_base`.
    history_base: usize,
    dedup: DedupIndex,
    /// Construction-time kind assembly (arbitration 1); kept for
    /// [`Engine::replay_assembled`] and loader re-assembly.
    assembly: EngineAssembly,
    /// Kind → implementation, built once at construction (no runtime door).
    impls: HashMap<&'static str, Arc<dyn KindImpl>>,
    /// Opaque captured-value blobs referenced by envelopes (arbitration 4).
    assets: AssetStore,
    /// §10.4 history limits (bytes / depth); `usize::MAX` = unbounded (the
    /// pre-spill behavior). The preference plumbing (§30.1
    /// `history_bytes_mb` / `history_states`) sets these at the assembly
    /// points.
    history_bytes_limit: usize,
    history_depth_limit: usize,
    /// Scratch-disk location (§3.4: a preference; default OS temp) and the
    /// lazily created per-document scratch file.
    scratch_dir: std::path::PathBuf,
    scratch: Option<crate::io::scratch::ScratchDisk>,
    spill_stats: history_spill::HistoryStats,
    /// Per-engine CMM (§5.2 per-engine context; O-COLOR slice 1b).
    cms: appname_color::Cms,
    /// §10.4 named snapshots (session state; history.snapshot †).
    snapshots: Vec<NamedSnapshot>,
    /// D-48 vector value-captures (the bound `vector.shape_add` posture:
    /// replay never re-runs constructor trig): envelope id → the
    /// captured geometry result. Persisted with the journal
    /// (`journal/captures/…`), consumed on journal replay.
    vector_captures: HashMap<Uuid, Arc<crate::engine::vector_ops::GeometryResult>>,
    /// §20.3 evaluation cache (derived state — never hashed, never
    /// persisted).
    vector_cache: crate::engine::vector_ops::VectorEvalCache,
}

struct NamedSnapshot {
    name: String,
    doc: Document,
    /// Journal index of the snapshot's † marker envelope. Journal indices
    /// are stable for the engine's lifetime (the journal only ever loses
    /// its TAIL, on branch truncation — never its bottom), so this index
    /// answers "is the marker still in the journal?" as `marker_index <
    /// cursor` after any truncation. A snapshot whose marker was truncated
    /// is pruned (fix-round truncation MAJOR): a `goto{snapshot}` naming it
    /// must answer `bad_target` in-session exactly as a fresh replay of the
    /// saved journal would (D-48 in-session ≡ replay).
    marker_index: usize,
    /// True iff `cursor == journal.len()` at capture time — the captured
    /// document IS the state a fresh replay reaches at the † marker.
    /// A MID-STACK capture (cursor below the tip, §10.4 snapshot-anytime)
    /// stores the CURSOR state, but its marker journals at the TIP
    /// († never truncates the redo tail), so replay recreates the snapshot
    /// from tip state. The asymmetry (re-review residual MAJOR, ruling a′):
    /// an untargeted mid-stack marker is BENIGN at replay — the recreated
    /// snapshot is simply never used — so mid-stack capture stays legal;
    /// only the `goto{snapshot}` door refuses when this flag is false,
    /// because committing that goto journals a restore no fresh replay
    /// satisfies (state-hash gate red forever). Faithful mid-stack goto
    /// needs capture-position carriage in the journal treatment (reviewer
    /// option (b)) — a recorded spec-side fold-in candidate; this flag is
    /// the interim posture.
    captured_at_matches_marker: bool,
}

impl Engine {
    /// Door C6, present-pass wiring point 1 (§5.4/§5.7): bake the display
    /// LUT for THIS document's working space against a monitor profile
    /// (ICC bytes; `None` = the sRGB display assumption). Read-side —
    /// mutates only the CMM cache, never document state (D-48-safe);
    /// per-monitor `MonitorId` keying and the §5.4 re-bake trigger list
    /// are the CONSUMER's (O-UI via the ABI). Payload:
    /// [`appname_color::DisplayLutData`] (rgb, 33³ per LUT_SIZE).
    pub fn bake_display_lut(
        &mut self,
        display_icc: Option<Vec<u8>>,
        intent: appname_color::Intent,
        bpc: bool,
    ) -> Result<appname_color::DisplayLutData, EngineError> {
        let (working, display) = self.display_chain_uids(display_icc)?;
        appname_color::bake_display_lut(&mut self.cms, working, display, intent, bpc)
            .map_err(color_glue::map_color_error)
    }

    /// Door C6, present-pass wiring point 2 (§5.6): bake the soft-proof
    /// LUT ("the proof LUT replaces the display LUT"; rgba — a carries
    /// the gamut flag, warn where a > 0.5 when `gamut_warning`). Payload:
    /// [`appname_color::ProofLut`]; params: [`appname_color::SoftProof`].
    pub fn bake_proof_lut(
        &mut self,
        display_icc: Option<Vec<u8>>,
        proof: &appname_color::SoftProof,
    ) -> Result<appname_color::ProofLut, EngineError> {
        let (working, display) = self.display_chain_uids(display_icc)?;
        appname_color::bake_proof_lut(&mut self.cms, working, display, proof)
            .map_err(color_glue::map_color_error)
    }

    /// Open an ICC blob in the engine CMM and return its uid (door C6
    /// helper: consumers hand proof/display profiles as bytes across the
    /// ABI; content-keyed — reopening is a dedup hit).
    pub fn open_profile_bytes(
        &mut self,
        bytes: Vec<u8>,
    ) -> Result<appname_color::ProfileUid, EngineError> {
        self.cms
            .open_profile(bytes, appname_color::ColorModel::Rgb)
            .map_err(color_glue::map_color_error)
    }

    /// A §5.1 builtin space's CMM uid (door C6 helper — proof/display
    /// selections over builtins without byte plumbing).
    pub fn builtin_profile_uid(
        &self,
        space: appname_color::BuiltinSpace,
    ) -> appname_color::ProfileUid {
        self.cms.builtin(space)
    }

    fn display_chain_uids(
        &mut self,
        display_icc: Option<Vec<u8>>,
    ) -> Result<(appname_color::ProfileUid, appname_color::ProfileUid), EngineError> {
        let doc = self.doc.as_ref().ok_or_else(|| {
            EngineError::bad_target(
                "no document — the display chain binds a document's working space",
            )
        })?;
        let entry = doc.profile(doc.color().working).cloned().ok_or_else(|| {
            EngineError::bad_target("the working profile does not resolve in the store (§5.1)")
        })?;
        let working = color_glue::resolve_uid(&mut self.cms, &entry)?;
        let display = match display_icc {
            Some(bytes) => self
                .cms
                .open_profile(bytes, appname_color::ColorModel::Rgb)
                .map_err(color_glue::map_color_error)?,
            None => self.cms.builtin(appname_color::BuiltinSpace::Srgb),
        };
        Ok((working, display))
    }

    /// An uninitialized BARE shell (no kind assembly): `doc.new` plus
    /// `not_implemented` for everything else.
    pub fn new() -> Engine {
        Engine::with_assembly(EngineAssembly::new())
    }

    /// Construction over an assembly (arbitration 1; use
    /// [`EngineAssembly::build`]).
    pub(crate) fn with_assembly(assembly: EngineAssembly) -> Engine {
        let impls = assembly
            .impls
            .iter()
            .map(|i| (i.kind(), Arc::clone(i)))
            .collect();
        Engine {
            doc: None,
            journal: Vec::new(),
            journal_ids: HashSet::new(),
            cursor: 0,
            history: Vec::new(),
            history_base: 0,
            dedup: DedupIndex::new(),
            assembly,
            impls,
            assets: AssetStore::new(),
            history_bytes_limit: usize::MAX,
            history_depth_limit: usize::MAX,
            scratch_dir: std::env::temp_dir(),
            scratch: None,
            spill_stats: history_spill::HistoryStats::default(),
            cms: appname_color::Cms::new(),
            snapshots: Vec::new(),
            vector_captures: HashMap::new(),
            vector_cache: crate::engine::vector_ops::VectorEvalCache::new(),
        }
    }

    /// The assembly this engine was built over (clonable for replay/load).
    pub fn assembly(&self) -> &EngineAssembly {
        &self.assembly
    }

    /// The asset store (opaque captured values, arbitration 4).
    pub fn assets(&self) -> &AssetStore {
        &self.assets
    }

    /// D-48 vector capture table (persisted beside the journal; the
    /// package writer enumerates it).
    pub(crate) fn vector_captures(
        &self,
    ) -> &HashMap<Uuid, Arc<crate::engine::vector_ops::GeometryResult>> {
        &self.vector_captures
    }

    /// Loader-side capture re-entry (idempotent; the journal-replay path
    /// consumes these so `vector.shape_add` never re-runs constructor
    /// trig, D-48).
    pub(crate) fn insert_loaded_vector_capture(
        &mut self,
        env_id: Uuid,
        g: crate::engine::vector_ops::GeometryResult,
    ) {
        self.vector_captures
            .entry(env_id)
            .or_insert_with(|| Arc::new(g));
    }

    /// §20.3 evaluation cache stats (hits, misses) — observability pin.
    pub fn vector_cache_stats(&self) -> (u64, u64) {
        self.vector_cache.stats()
    }

    /// §20.3: the shape layer's COMPOSITE path — the left fold of its
    /// `bool_ops` over its base path via the geometry seam, memoized in
    /// the engine-side evaluation cache (keyed by content hashes, so
    /// source edits re-evaluate lazily by construction).
    pub fn evaluate_shape_composite(
        &mut self,
        layer: crate::graph::id::LayerId,
    ) -> Result<Arc<crate::engine::vector_ops::GeometryResult>, EngineError> {
        let doc = self
            .doc
            .as_ref()
            .ok_or_else(|| EngineError::bad_target("document not initialized"))?;
        let sd = doc
            .layer(layer)
            .and_then(|n| n.shape_data())
            .ok_or_else(|| {
                EngineError::new(
                    ErrorCode::TargetKindMismatch,
                    "evaluate_shape_composite targets a Shape layer (§20.2)",
                )
            })?;
        let svc =
            self.assembly.vector_geometry().cloned().ok_or_else(|| {
                crate::engine::vector_ops::geometry_unregistered("shape composite")
            })?;
        let base = doc.path(sd.path()).ok_or_else(|| {
            EngineError::bad_target(format!("shape base path {} does not resolve", sd.path()))
        })?;
        let mut ops = Vec::new();
        for (op, pid) in sd.bool_ops() {
            let p = doc.path(*pid).ok_or_else(|| {
                EngineError::bad_target(format!("bool_ops operand {pid} does not resolve"))
            })?;
            ops.push((*op, Arc::clone(p.sidecar()), p.content_hash()));
        }
        let (base_sidecar, base_hash) = (Arc::clone(base.sidecar()), base.content_hash());
        self.vector_cache
            .evaluate_composite(&*svc, &base_sidecar, base_hash, &ops)
    }

    /// Loader-side asset re-entry under a recorded id (package loader +
    /// §10.6 autosave recovery). `Err` returns the occupying id when the
    /// slot is already taken (idempotent recovery keeps the existing blob).
    pub(crate) fn insert_loaded_asset(
        &mut self,
        id: Uuid,
        bytes: Vec<u8>,
        kind: assets::AssetKind,
    ) -> Result<(), Uuid> {
        self.assets.insert_loaded_kind(id, bytes, kind)
    }

    /// Test-only document door (in-crate fixtures that pre-date a command
    /// kind for the state under test — e.g. mask round trips before the
    /// mask.* kinds land).
    #[cfg(test)]
    pub(crate) fn doc_mut_for_test(&mut self) -> &mut Document {
        self.doc.as_mut().expect("initialized")
    }

    pub fn is_initialized(&self) -> bool {
        self.doc.is_some()
    }

    pub fn document(&self) -> Option<&Document> {
        self.doc.as_ref()
    }

    pub fn journal(&self) -> &[Envelope] {
        &self.journal
    }

    pub fn journal_cursor(&self) -> usize {
        self.cursor
    }

    // ------------------------------------------------------------------
    // The apply boundary (stages 0-7).
    // ------------------------------------------------------------------

    /// Apply one command from raw bytes (stage 0 upward). WD-1: this and
    /// [`Engine::apply_json`] are the corpus-facing entries.
    pub fn apply_bytes(&mut self, bytes: &[u8]) -> Result<ApplyReceipt, EngineError> {
        let text = std::str::from_utf8(bytes).map_err(|e| {
            EngineError::new(
                ErrorCode::InvalidArg,
                format!("command document is not UTF-8: {e}"),
            )
        })?;
        self.apply_json(text)
    }

    /// Apply one command from raw JSON text (stage 0 upward).
    pub fn apply_json(&mut self, text: &str) -> Result<ApplyReceipt, EngineError> {
        let env = Envelope::from_json(text)?; // stages 0-1
        self.apply_envelope(env)
    }

    /// Apply a validated envelope (stage 2 upward). Public for in-process
    /// callers and replay; the raw-bytes entries above are the API-boundary
    /// form (WD-1).
    pub fn apply_envelope(&mut self, env: Envelope) -> Result<ApplyReceipt, EngineError> {
        // Stage 2: duplicate command ids reject against the whole journal
        // (proto order: before kind resolution).
        if self.journal_ids.contains(&env.id) {
            return Err(EngineError::with_field(
                ErrorCode::DuplicateId,
                "id",
                format!("command id {} is already in the journal", env.id),
            ));
        }

        // Stage 3: kind known.
        let spec = registry::lookup(&env.kind).ok_or_else(|| {
            EngineError::new(
                ErrorCode::UnknownKind,
                format!("unknown command kind `{}` (§10.3 catalog)", env.kind),
            )
        })?;

        // Stage 4: kind implemented — built-in doc.new, the § dagger
        // built-ins (history.snapshot/goto — they manipulate the history
        // machinery itself, which no KindImpl can reach), assembled impls
        // (arbitration 1), or not_implemented on a bare engine.
        if spec.dagger && matches!(spec.kind, "history.snapshot" | "history.goto") {
            let effective = registry::validate_params(spec, env.params_map())?;
            return self.apply_dagger(env, &effective);
        }
        let imp = if spec.kind == "doc.new" {
            None
        } else {
            match self.impls.get(spec.kind) {
                Some(imp) => Some(Arc::clone(imp)),
                None => return Err(EngineError::not_implemented(spec.kind, spec.milestone)),
            }
        };

        // Stage 5: params against the generated schema (defaults filled).
        let effective = registry::validate_params(spec, env.params_map())?;

        // Stages 6-7 for assembled kinds: live-state checks + execution
        // inside the implementation, against the recorder-coupled doors.
        if let Some(imp) = imp {
            return self.apply_assembled(env, &*imp, &effective);
        }

        // Stage 6a: doc.new demands a fresh handle (§40.2).
        if self.doc.is_some() {
            return Err(EngineError::bad_target("document already initialized"));
        }
        let params = doc_new::decode_params(&effective);
        // Stages 6b-6e: live-state support checks, pinned order.
        doc_new::check_supported(&params)?;

        // Stage 7: execute through the recorder, then commit.
        let mut rec = DeltaRecorder::new();
        let (mut doc, created_ids) = doc_new::execute(env.id, &params, &mut rec)?;
        let deltas = rec.finish(&mut doc, &mut self.dedup);
        #[cfg(debug_assertions)]
        {
            for (surface, _, _) in &deltas.bounds {
                if let Some(buf) = doc
                    .layer(crate::graph::id::LayerId::from_uuid(*surface))
                    .and_then(|n| n.pixels())
                {
                    debug_assert!(
                        buf.validate_premultiplied().is_ok(),
                        "commit produced a premultiplied violation (§6.4)"
                    );
                }
            }
            debug_assert!(
                doc.validate_tree().is_ok(),
                "commit corrupted the tree (§2.7)"
            );
        }

        // Applying on an undone tail branches: drop the redoable entries
        // (linear history, DD ⑰).
        self.truncate_undone_tail();

        // Structural kind (arbitration 2): dirty = node bounds before∪after
        // per touched surface (empty for a transparent background — nothing
        // visible changed).
        let dirty: Vec<DirtyRect> = deltas
            .bounds
            .iter()
            .map(|(surface, before, after)| DirtyRect {
                layer: LayerId::from_uuid(*surface),
                rect: before.union(after),
            })
            .filter(|d| !d.rect.is_empty())
            .collect();

        self.journal.push(env.clone());
        self.journal_ids.insert(env.id);
        self.history.push(HistoryStep {
            env_id: env.id,
            kind: env.kind.clone(),
            timestamp: env.timestamp.clone(),
            delta: StepDelta::DocInit {
                stash: None,
                deltas,
            },
            dirty: dirty.clone(),
        });
        self.cursor += 1;
        self.doc = Some(doc);

        Ok(ApplyReceipt {
            id: env.id,
            kind: env.kind,
            revision: self.journal.len() as u64,
            created_ids,
            dirty,
            coalesced: false,
            warnings: Vec::new(),
        })
    }

    /// Stages 6-7 for an assembled kind: run the implementation against the
    /// recorder-coupled context, roll back cleanly on failure (§10.1 makes
    /// rollback a before-restore), commit (§2.4 prune → after-capture →
    /// per-context dedup), and derive the receipt dirty rects
    /// (arbitration 2).
    fn apply_assembled(
        &mut self,
        env: Envelope,
        imp: &dyn KindImpl,
        effective: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<ApplyReceipt, EngineError> {
        if self.doc.is_none() {
            return Err(EngineError::bad_target(format!(
                "document not initialized; the first command must be doc.new (got {})",
                env.kind
            )));
        }
        self.refuse_geometry_walk_on_smart(&env.kind, effective)?;
        // §10.3 class C (CheckpointOnly, inverse: snap): the pre-command
        // document clone IS the inverse (§10.4 snapshot semantics) — Arc
        // tiles make it O(nodes), not O(pixels).
        let checkpoint = registry::lookup(&env.kind)
            .map(|s| s.class == "C")
            .unwrap_or(false);
        let pre_snapshot = if checkpoint { self.doc.clone() } else { None };
        let mut rec = DeltaRecorder::new();
        // D-48 capture threading: a capture recorded for THIS envelope id
        // (journal replay) rides in; a freshly generated one rides out and
        // is committed with the step below.
        let capture_in = self.vector_captures.get(&env.id).cloned();
        let mut capture_out: Option<Arc<crate::engine::vector_ops::GeometryResult>> = None;
        let mut warnings: Vec<String> = Vec::new();
        let outcome = {
            let doc = self.doc.as_mut().expect("checked above");
            let mut cx = KindContext {
                env: &env,
                params: effective,
                doc,
                rec: &mut rec,
                assets: &mut self.assets,
                cms: &mut self.cms,
                ops: &self.assembly.ops,
                vector: self.assembly.vector.as_ref(),
                text: self.assembly.text.as_ref(),
                blend_kernels: self.assembly.blend_kernels,
                fill_convert: self.assembly.fill_convert,
                warnings: &mut warnings,
                capture_in,
                capture_out: &mut capture_out,
                checkpoint,
            };
            imp.apply(&mut cx)
        };
        if outcome.is_ok() {
            if let Some(g) = capture_out {
                self.vector_captures.insert(env.id, g);
            }
        }
        let doc = self.doc.as_mut().expect("checked above");
        if checkpoint {
            return match outcome {
                Ok(outcome) => {
                    let _ = rec.finish(doc, &mut self.dedup); // class C records no deltas
                                                              // D = full canvas per raster layer (§40.2 doc-scope
                                                              // dirty; divergence: no render-topic events headless).
                    let canvas = doc.geometry().canvas_rect();
                    let dirty: Vec<DirtyRect> = doc
                        .z_order_walk()
                        .into_iter()
                        .filter(|id| {
                            doc.layer(*id)
                                .map(|n| n.pixels().is_some())
                                .unwrap_or(false)
                        })
                        .map(|id| DirtyRect {
                            layer: id,
                            rect: canvas,
                        })
                        .collect();
                    self.truncate_undone_tail();
                    self.journal.push(env.clone());
                    self.journal_ids.insert(env.id);
                    self.history.push(HistoryStep {
                        env_id: env.id,
                        kind: env.kind.clone(),
                        timestamp: env.timestamp.clone(),
                        delta: StepDelta::Snapshot {
                            other: pre_snapshot.expect("cloned above"),
                        },
                        dirty: dirty.clone(),
                    });
                    self.cursor += 1;
                    self.enforce_history_limits();
                    Ok(ApplyReceipt {
                        id: env.id,
                        kind: env.kind,
                        revision: self.journal.len() as u64,
                        created_ids: outcome.created_ids,
                        dirty,
                        coalesced: false,
                        warnings,
                    })
                }
                Err(e) => {
                    // Snapshot rollback: discard the touched state wholesale.
                    self.doc = pre_snapshot;
                    Err(e)
                }
            };
        }
        match outcome {
            Ok(outcome) => {
                // Chain-edit D hints (receipt-side; §10.3 rows the deltas
                // cannot encode — an op edit changes pixels at re-render).
                let dirty_hints = rec.take_dirty_hints();
                let deltas = rec.finish(doc, &mut self.dedup);
                #[cfg(debug_assertions)]
                {
                    debug_assert!(
                        doc.validate_tree().is_ok(),
                        "commit corrupted the tree (§2.7)"
                    );
                    for (surface, _, _) in &deltas.bounds {
                        if let Some(buf) = doc
                            .layer(LayerId::from_uuid(*surface))
                            .and_then(|n| n.pixels())
                        {
                            debug_assert!(
                                buf.validate_premultiplied().is_ok(),
                                "commit produced a premultiplied violation (§6.4)"
                            );
                        }
                    }
                }
                // Tile-writing kinds: per-touched-tile pixel_rect(); plus
                // bounds-pair unions for surfaces with no tile deltas
                // (property-style effects). Arbitration 2.
                let mut dirty: Vec<DirtyRect> = deltas
                    .tiles
                    .iter()
                    .map(|d| DirtyRect {
                        layer: LayerId::from_uuid(d.surface),
                        rect: d.coord.pixel_rect(),
                    })
                    .collect();
                for (surface, before, after) in &deltas.bounds {
                    if !deltas.tiles.iter().any(|t| t.surface == *surface) {
                        let rect = before.union(after);
                        if !rect.is_empty() {
                            dirty.push(DirtyRect {
                                layer: LayerId::from_uuid(*surface),
                                rect,
                            });
                        }
                    }
                }
                for hint in dirty_hints {
                    if !dirty.contains(&hint) {
                        dirty.push(hint);
                    }
                }

                // §10.5 class-M coalescing [NORMATIVE letter]: same-
                // mergeKey CONSECUTIVE envelopes merge — "Merging replaces
                // the previous step in place: the merged command is the
                // newer one (its full-state params are the latest values)"
                // — so the JOURNAL tail is replaced (replay applies only
                // the final full-state command; D-48 byte-identity holds
                // by the full-state rule) and undo treats the run as ONE
                // step. Gates, all engine-decidable:
                //   · this kind is class M (§10.3 — class-R kinds with a
                //     client mergeKey do NOT merge in the journal; their
                //     coalescing is producer-side per §9.6 rule 3 — the
                //     vector.edit question is ESCALATED, lane log);
                //   · both envelopes carry the SAME non-null mergeKey;
                //   · the previous step is the live tail (no undone
                //     entries — "consecutive"), is in-process, same kind,
                //     and holds resident deltas (not doc-init/snapshot/
                //     marker/spilled).
                // Session boundaries are the PRODUCER's (§10.5 table): the
                // engine never invents one — a new sessionUuid in the key
                // breaks the run, exactly as specified.
                let mut deltas_slot = Some(deltas);
                let merged = 'merge: {
                    let Some(key) = env.merge_key.as_deref() else {
                        break 'merge false;
                    };
                    if registry::lookup(&env.kind).map(|s| s.class) != Some("M") {
                        break 'merge false;
                    }
                    if self.cursor != self.journal.len() || self.cursor <= self.history_base {
                        break 'merge false;
                    }
                    let prev_env = &self.journal[self.cursor - 1];
                    if prev_env.kind != env.kind || prev_env.merge_key.as_deref() != Some(key) {
                        break 'merge false;
                    }
                    let step = self
                        .history
                        .last_mut()
                        .expect("cursor > history_base implies a step");
                    let StepDelta::Deltas { deltas: prev } = &mut step.delta else {
                        break 'merge false;
                    };
                    prev.merge_from(deltas_slot.take().expect("deltas present"));
                    step.env_id = env.id;
                    step.timestamp = env.timestamp.clone();
                    for d in &dirty {
                        if !step.dirty.contains(d) {
                            step.dirty.push(*d);
                        }
                    }
                    dirty = step.dirty.clone();
                    // Journal replacement: the merged-away envelope id
                    // stays burned in journal_ids (stage-2 duplicate
                    // detection keeps catching client re-submissions).
                    self.journal.pop();
                    self.journal.push(env.clone());
                    self.journal_ids.insert(env.id);
                    true
                };
                if merged {
                    self.enforce_history_limits();
                    return Ok(ApplyReceipt {
                        id: env.id,
                        kind: env.kind,
                        revision: self.journal.len() as u64,
                        created_ids: outcome.created_ids,
                        dirty,
                        coalesced: true,
                        warnings,
                    });
                }
                let deltas = deltas_slot.take().expect("deltas present when not merged");
                self.truncate_undone_tail();
                self.journal.push(env.clone());
                self.journal_ids.insert(env.id);
                self.history.push(HistoryStep {
                    env_id: env.id,
                    kind: env.kind.clone(),
                    timestamp: env.timestamp.clone(),
                    delta: StepDelta::Deltas { deltas },
                    dirty: dirty.clone(),
                });
                self.cursor += 1;
                self.enforce_history_limits();
                Ok(ApplyReceipt {
                    id: env.id,
                    kind: env.kind,
                    revision: self.journal.len() as u64,
                    created_ids: outcome.created_ids,
                    dirty,
                    coalesced: false,
                    warnings,
                })
            }
            Err(e) => {
                // Roll back any partial mutation: the recorder's befores
                // swap back in, bounds re-tighten to their captures, and
                // recorded tree ops reverse out (added nodes removed).
                let mut deltas = rec.finish(doc, &mut self.dedup);
                deltas.restore_before(doc);
                doc.sync_selection_bounds();
                Err(e)
            }
        }
    }

    /// Reviewer MAJOR (CP-0.7 close, QA-F8 adjacent): the §40.2
    /// all-surface geometry walks enumerate via `raster_surfaces()`,
    /// which has no smart-object arm — a smart layer's §11.5 proxy would
    /// be silently EXCLUDED from `doc.resize` / `doc.rotate` / anchored
    /// `doc.canvas_resize` (left unscaled / unrotated / untranslated on
    /// the new canvas; on a square canvas `doc.rotate{90}` touches no
    /// door at all, so no door-level guard can catch it). RULED interim
    /// fix, same shape as the export serving arm: these kind shapes
    /// REFUSE TYPED — before execution, nothing mutated — on documents
    /// carrying smart-object layers (visible or not: a hidden stale
    /// proxy is still the banned silent shape) until the §18.2
    /// placement-transform row lands. `doc.crop`/`doc.trim` are
    /// geometry-only over unbounded buffers (§19.1) and stay unguarded;
    /// origin-anchor (`anchor:"top_left"`) canvas resize moves no
    /// document-space coordinate and passes.
    fn refuse_geometry_walk_on_smart(
        &self,
        kind: &str,
        effective: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), EngineError> {
        let guarded = match kind {
            "doc.resize" | "doc.rotate" => true,
            "doc.canvas_resize" => {
                effective
                    .get("anchor")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("center")
                    != "top_left"
            }
            _ => return Ok(()),
        };
        if !guarded {
            return Ok(());
        }
        let doc = self.doc.as_ref().expect("caller checked");
        let has_smart = doc.z_order_walk().into_iter().any(|id| {
            doc.layer(id)
                .map(|n| n.smart_object().is_some())
                .unwrap_or(false)
        });
        if has_smart {
            return Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!(
                    "{kind} on a document carrying smart-object layers needs the §18.2 \
                     placement-transform row (the §40.2 all-surface walk cannot transform \
                     the §11.5 proxy yet) — refused typed, never a silent exclusion \
                     (QA-F8 interim discipline)"
                ),
            ));
        }
        Ok(())
    }

    /// §10.3 † execution (history.snapshot R†, history.goto C†).
    fn apply_dagger(
        &mut self,
        env: Envelope,
        effective: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<ApplyReceipt, EngineError> {
        if self.doc.is_none() {
            return Err(EngineError::bad_target(format!(
                "document not initialized; the first command must be doc.new (got {})",
                env.kind
            )));
        }
        match env.kind.as_str() {
            "history.snapshot" => {
                // Contract `scope` (fix-round M7): `merged` (flattened
                // composite only) needs the compositor flatten — a typed
                // refusal, never a silent acceptance of the param.
                if effective.get("scope").and_then(serde_json::Value::as_str) == Some("merged") {
                    return Err(EngineError::new(
                        ErrorCode::NotImplemented,
                        "history.snapshot{scope:\"merged\"} (flattened-composite capture) \
                         arrives with the compositor flatten row; v1 captures scope:\"full\"",
                    ));
                }
                let name = effective
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    // Fix-round m13: the contract default is the LOCALIZED
                    // "Snapshot N" in the `panel.history.snapshot.new`
                    // domain (registered in contracts/message_ids.yaml).
                    // The engine is locale-free: it emits the MessageId +
                    // args in the house key-echo form (`id name=value`, the
                    // CLI loader's untranslated-fallback grammar) and the
                    // shell/CLI localizes at display; goto{snapshot}
                    // matches the stored string either way.
                    .unwrap_or_else(|| {
                        format!("panel.history.snapshot.new n={}", self.snapshots.len() + 1)
                    });
                // §10.4: a named COW capture; accounted like history (the
                // §10.4 budget folds it in via history_stats when limits
                // land for snapshots — noted).
                let doc = self.doc.as_ref().expect("checked").clone();
                // The marker envelope pushes at the CURRENT tip (below, after
                // this push bookkeeping) — even above an undone tail, since †
                // does not truncate. Its index keys the truncation prune.
                let marker_index = self.journal.len();
                // Ruling a′ flag: does the captured document match what a
                // fresh replay reaches at the marker? Only when the cursor
                // sits AT the tip — a mid-stack capture stores cursor
                // state while its marker journals above the redo tail
                // (see the field doc on `captured_at_matches_marker`).
                let captured_at_matches_marker = self.cursor == self.journal.len();
                self.snapshots.push(NamedSnapshot {
                    name,
                    doc,
                    marker_index,
                    captured_at_matches_marker,
                });
                // † MARKER journaling: the envelope journals (replay
                // recreates the snapshot) and the redo tail is NOT
                // truncated (outside the stack). Fix-round B1: the marker
                // pushes a corresponding NO-OP history step so the
                // journal↔history correspondence `journal[base+i] ↔
                // history[i]` holds exactly — the old base-advance shifted
                // every step below a tail marker (undo below a marker
                // restored the wrong state). Marker steps are invisible:
                // undo/redo skip them, [`Engine::history`] filters them
                // (†: never a stack slot, §40.1), and the §10.4
                // bottom-drop keeps 1:1 by dropping journal-aligned steps.
                self.journal.push(env.clone());
                self.journal_ids.insert(env.id);
                self.history.push(HistoryStep {
                    env_id: env.id,
                    kind: env.kind.clone(),
                    timestamp: env.timestamp.clone(),
                    delta: StepDelta::Marker,
                    dirty: Vec::new(),
                });
                if self.cursor == self.journal.len() - 1 {
                    // Marker applied at the tail: the cursor absorbs it.
                    self.cursor += 1;
                }
                Ok(ApplyReceipt {
                    id: env.id,
                    kind: env.kind,
                    revision: self.journal.len() as u64,
                    created_ids: Vec::new(),
                    dirty: Vec::new(),
                    coalesced: false,
                    warnings: Vec::new(),
                })
            }
            "history.goto" => {
                if let Some(pos) = effective
                    .get("position")
                    .and_then(serde_json::Value::as_u64)
                {
                    // Fix-round B1/M3: `position` is an UNDO-STACK index
                    // (§40.8: "absolute index in the undo stack",
                    // 0..=undoDepth) — † markers journal but never occupy
                    // a stack slot, so positions count non-marker entries.
                    // Out-of-range AND in-range-but-unreachable (history
                    // not live across the package boundary, §10.4) both
                    // answer `error.schema.out_of_range` per the contract
                    // targetResolution — the old journal-index clamp
                    // silently accepted out-of-range positions and leaked
                    // bad_target/nothing_to_undo.
                    let p = pos as usize;
                    let total = self.undo_depth_total();
                    if p > total {
                        return Err(EngineError::range(
                            "position",
                            format!(
                                "`position` {p} is outside the stack range 0..={total} (§40.8)"
                            ),
                        ));
                    }
                    let cur = self.undo_position();
                    if p != cur {
                        if self.cursor < self.history_base {
                            return Err(EngineError::range(
                                "position",
                                "the cursor sits below the live history base (history is not \
                                 persisted, §10.4); only the current position is reachable",
                            ));
                        }
                        if p < cur {
                            let floor = (0..self.history_base)
                                .filter(|&i| !self.is_marker_entry(i))
                                .count();
                            if p < floor {
                                return Err(EngineError::range(
                                    "position",
                                    format!(
                                        "`position` {p} lies below the live-state floor {floor} \
                                         (history is not persisted, §10.4); in-range-but-\
                                         unreachable positions answer out_of_range"
                                    ),
                                ));
                            }
                        }
                    }
                    // † position form: journals AS the equivalent
                    // undo/redo run (contract daggerSemantics) — the run
                    // moves the cursor and appends nothing; the cursor is
                    // the persisted §7.8 truth, and the CLI appends the
                    // goto envelope to the §7.8 autosave log so §10.6
                    // crash replay re-runs it (fix-round M5).
                    let mut dirty: Vec<DirtyRect> = Vec::new();
                    while self.undo_position() > p {
                        let row = self.undo().map_err(|e| {
                            EngineError::range(
                                "position",
                                format!("goto undo run stopped: {}", e.message),
                            )
                        })?;
                        dirty.extend(row.dirty);
                    }
                    while self.undo_position() < p {
                        let row = self.redo().map_err(|e| {
                            EngineError::range(
                                "position",
                                format!("goto redo run stopped: {}", e.message),
                            )
                        })?;
                        dirty.extend(row.dirty);
                    }
                    return Ok(ApplyReceipt {
                        id: env.id,
                        kind: env.kind,
                        revision: self.journal.len() as u64,
                        created_ids: Vec::new(),
                        // Contract: D = the union of the replayed steps'
                        // regions (§40.8).
                        dirty,
                        coalesced: false,
                        warnings: Vec::new(),
                    });
                }
                let name = effective
                    .get("snapshot")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| EngineError::missing("position|snapshot"))?;
                // §40.8: most-recent match on duplicate names — among
                // snapshots whose † marker is below the cursor.
                // A marker on the undone tail is invisible here (fix-round
                // truncation MAJOR): committing this goto truncates that
                // tail, so a fresh replay of the resulting journal would
                // find no marker before the goto — in-session must answer
                // the same `bad_target` (D-48 in-session ≡ replay). Redo
                // past the marker makes it reachable again.
                let snap = self
                    .snapshots
                    .iter()
                    .rev()
                    .filter(|s| s.marker_index < self.cursor)
                    .find(|s| s.name == name)
                    .ok_or_else(|| {
                        if self.snapshots.iter().any(|s| s.name == name) {
                            EngineError::bad_target(format!(
                                "no snapshot named {name:?} at this history position (§40.8): \
                                 its marker sits on the undone tail, which this goto would \
                                 truncate; redo past the marker to reach it"
                            ))
                        } else {
                            EngineError::bad_target(format!("no snapshot named {name:?} (§40.8)"))
                        }
                    })?;
                // Ruling a′ (re-review residual MAJOR), the goto-door
                // guard: a mid-stack-captured snapshot holds cursor state
                // while its † marker replays from tip state — restoring it
                // would journal an envelope the state-hash gate refuses on
                // every fresh replay (D-48 in-session ≡ replay). The
                // capture itself stays legal (§10.4 snapshot-anytime; an
                // untargeted marker is benign at replay) — only this door
                // refuses.
                if !snap.captured_at_matches_marker {
                    return Err(EngineError::bad_target(format!(
                        "snapshot {name:?} was captured mid-stack (cursor below the journal \
                         tip) and cannot be restored replay-faithfully (§40.8): its † marker \
                         replays from tip state, not the captured state; re-capture at the \
                         intended position with the redo tail resolved to target it"
                    )));
                }
                let snap = snap.doc.clone();
                // §10.4: restore executes as a NEW CheckpointOnly command
                // appended to the stack (single-step undoable; redo tail
                // clears like any push).
                let pre = self.doc.replace(snap);
                self.truncate_undone_tail();
                self.journal.push(env.clone());
                self.journal_ids.insert(env.id);
                self.history.push(HistoryStep {
                    env_id: env.id,
                    kind: env.kind.clone(),
                    timestamp: env.timestamp.clone(),
                    delta: StepDelta::Snapshot {
                        other: pre.expect("initialized"),
                    },
                    dirty: Vec::new(),
                });
                self.cursor += 1;
                self.enforce_history_limits();
                Ok(ApplyReceipt {
                    id: env.id,
                    kind: env.kind,
                    revision: self.journal.len() as u64,
                    created_ids: Vec::new(),
                    dirty: Vec::new(),
                    coalesced: false,
                    warnings: Vec::new(),
                })
            }
            other => Err(EngineError::new(
                ErrorCode::NotImplemented,
                format!("dagger kind `{other}` arrives with its row (§10.3)"),
            )),
        }
    }

    /// The named §10.4 snapshots (session state — die with the engine).
    pub fn snapshots(&self) -> impl Iterator<Item = &str> + '_ {
        self.snapshots.iter().map(|s| s.name.as_str())
    }

    // ------------------------------------------------------------------
    // Undo / redo / history (engine functions; the history.* dagger kinds
    // arrive at 0.7 per the index).
    // ------------------------------------------------------------------

    /// Undo the newest in-effect command. Unavailable on a reopened engine:
    /// history is not persisted (§10.4) — the CLI undoes across the package
    /// boundary via replay-as-recovery instead.
    pub fn undo(&mut self) -> Result<HistoryRow, EngineError> {
        // † markers below the cursor are OUTSIDE the undo stack (§10.3):
        // scan down past them WITHOUT mutating state, then undo the first
        // real step. A failed scan leaves the cursor untouched.
        let mut top = self.cursor;
        loop {
            if top == 0 {
                return Err(EngineError::new(
                    ErrorCode::NothingToUndo,
                    "nothing to undo",
                ));
            }
            if top <= self.history_base {
                // Suffix boundary: everything below the base is persisted
                // journal without deltas (§10.4).
                return Err(EngineError::new(
                    ErrorCode::NothingToUndo,
                    "history was not persisted in the package (§10.4); undo across the package \
                     boundary uses the replay path",
                ));
            }
            if matches!(
                self.history[top - 1 - self.history_base].delta,
                StepDelta::Marker
            ) {
                top -= 1;
                continue;
            }
            break;
        }
        let idx = top - 1;
        // §10.4: a spilled step revives transparently before its Arc swaps.
        self.revive_history_step(idx - self.history_base)?;
        match &mut self.history[idx - self.history_base].delta {
            StepDelta::DocInit { stash, .. } => {
                debug_assert!(stash.is_none());
                // §40.2: undo returns the handle to the empty pre-init state.
                *stash = self.doc.take();
            }
            StepDelta::Deltas { deltas } => {
                let doc = self.doc.as_mut().expect("delta steps require a document");
                deltas.restore_before(doc);
            }
            StepDelta::Spilled(_) => unreachable!("revived above"),
            StepDelta::Marker => unreachable!("markers are skipped by the scan above"),
            StepDelta::Snapshot { other } => {
                // §10.4 snap inverse: O(1) swap live ↔ stash (undo and redo
                // are the same operation).
                if let Some(live) = self.doc.as_mut() {
                    std::mem::swap(live, other);
                }
            }
        }
        if let Some(doc) = self.doc.as_mut() {
            doc.sync_selection_bounds(); // §13.1 (restores touching the selection surface)
        }
        self.cursor = idx;
        Ok(self.history_row(idx))
    }

    /// Redo the next undone command, if any (same reopened-engine rule as
    /// [`Engine::undo`]).
    pub fn redo(&mut self) -> Result<HistoryRow, EngineError> {
        // † markers at/above the cursor are OUTSIDE the undo stack (§10.3):
        // scan up past them WITHOUT mutating state, then redo the first
        // real step. A failed scan leaves the cursor untouched.
        let mut idx = self.cursor;
        loop {
            if idx >= self.journal.len() {
                return Err(EngineError::new(
                    ErrorCode::NothingToRedo,
                    "nothing to redo",
                ));
            }
            if idx < self.history_base {
                // A persisted redoable tail has no in-process deltas (§10.4).
                return Err(EngineError::new(
                    ErrorCode::NothingToRedo,
                    "history was not persisted in the package (§10.4); redo across the package \
                     boundary uses the replay path",
                ));
            }
            if matches!(
                self.history[idx - self.history_base].delta,
                StepDelta::Marker
            ) {
                idx += 1;
                continue;
            }
            break;
        }
        // Spill only ever targets below-cursor steps, but a set_history_limits
        // call could land between an undo and its redo — revive defensively.
        self.revive_history_step(idx - self.history_base)?;
        match &mut self.history[idx - self.history_base].delta {
            StepDelta::DocInit { stash, .. } => {
                debug_assert!(stash.is_some());
                self.doc = stash.take();
            }
            StepDelta::Deltas { deltas } => {
                let doc = self.doc.as_mut().expect("delta steps require a document");
                deltas.restore_after(doc);
            }
            StepDelta::Spilled(_) => unreachable!("revived above"),
            StepDelta::Marker => unreachable!("markers are skipped by the scan above"),
            StepDelta::Snapshot { other } => {
                if let Some(live) = self.doc.as_mut() {
                    std::mem::swap(live, other);
                }
            }
        }
        if let Some(doc) = self.doc.as_mut() {
            doc.sync_selection_bounds();
        }
        self.cursor = idx + 1;
        // Absorb † markers directly above (undo→redo stays involutive on
        // the cursor across markers; a tail marker never strands the
        // cursor below it).
        while self.cursor < self.journal.len()
            && matches!(
                self.history[self.cursor - self.history_base].delta,
                StepDelta::Marker
            )
        {
            self.cursor += 1;
        }
        Ok(self.history_row(idx))
    }

    /// The IN-PROCESS history list: one row per SUFFIX entry (post-open
    /// commands with live deltas), journal-indexed. Unchanged semantics for
    /// fresh engines (base 0 ⇒ every entry). The TOTAL per-journal view —
    /// prefix entries rendered as persisted rows — is [`Engine::journal_rows`]
    /// (what the CLI history verb shows); consumers inferring liveness from
    /// `history().len() == journal().len()` keep their pre-suffix behavior
    /// on reopened documents until they adopt
    /// [`Engine::undoable_in_process`] (the brokered cabi update).
    pub fn history(&self) -> Vec<HistoryRow> {
        (self.history_base..self.journal.len())
            // † marker steps are B1 correspondence placeholders — never a
            // stack slot, never shown (§40.1).
            .filter(|i| !matches!(self.history[i - self.history_base].delta, StepDelta::Marker))
            .map(|i| self.history_row(i))
            .collect()
    }

    /// True when journal index `i` is a †-marker entry (`history.snapshot`
    /// journals as a marker, §40.8) — decided from the ENVELOPE kind so the
    /// persisted prefix (no history steps, §10.4) classifies identically.
    fn is_marker_entry(&self, i: usize) -> bool {
        self.journal[i].kind == "history.snapshot"
    }

    /// The current §40.8 undo-stack position: non-marker journal entries
    /// below the cursor († markers never occupy a position slot).
    fn undo_position(&self) -> usize {
        (0..self.cursor)
            .filter(|&i| !self.is_marker_entry(i))
            .count()
    }

    /// The §40.8 `undoDepth` bound: non-marker entries across the whole
    /// journal (the redoable tail included).
    fn undo_depth_total(&self) -> usize {
        (0..self.journal.len())
            .filter(|&i| !self.is_marker_entry(i))
            .count()
    }

    // ------------------------------------------------------------------
    // §10.4 history limits: byte accounting, spill, drop (CP-0.5 wave 2).
    // ------------------------------------------------------------------

    /// Configure the §10.4 history limits (bytes / depth) and enforce them
    /// immediately. `usize::MAX` disables a limit (the default).
    pub fn set_history_limits(&mut self, bytes_limit: usize, depth_limit: usize) {
        self.history_bytes_limit = bytes_limit;
        self.history_depth_limit = depth_limit;
        self.enforce_history_limits();
    }

    /// Scratch-disk directory (§3.4: a preference; default OS temp). Takes
    /// effect for the NEXT scratch-file creation — the file is created
    /// lazily on first spill.
    pub fn set_scratch_dir(&mut self, dir: std::path::PathBuf) {
        self.scratch_dir = dir;
    }

    /// History-tier telemetry (§3.4 stats families that apply headless).
    pub fn history_stats(&self) -> history_spill::HistoryStats {
        let mut s = self.spill_stats;
        s.resident_bytes = self
            .history
            .iter()
            .map(|st| match &st.delta {
                StepDelta::Deltas { deltas } => history_spill::resident_accounted(deltas),
                StepDelta::DocInit { deltas, .. } => history_spill::resident_accounted(deltas),
                StepDelta::Spilled(_) | StepDelta::Snapshot { .. } | StepDelta::Marker => 0,
            })
            .sum();
        s.spilled_bytes = self
            .history
            .iter()
            .map(|st| match &st.delta {
                StepDelta::Spilled(sp) => sp.accounted,
                _ => 0,
            })
            .sum();
        s.disk_bytes = self.scratch.as_ref().map(|sc| sc.disk_bytes()).unwrap_or(0);
        s
    }

    fn accounted_total(&self) -> usize {
        let s = self.history_stats();
        s.resident_bytes + s.spilled_bytes
    }

    /// §10.4 enforcement order at push time: (1) compress + spill the
    /// coldest (oldest) resident below-cursor DeltaSets; (2) if still over
    /// bytes or depth, drop whole steps from the bottom of the undo stack;
    /// (3) the newest [`history_spill::NEVER_DROP_FLOOR`] steps are never
    /// dropped regardless of size.
    /// Branch truncation (linear history, DD ⑰): a command committing at a
    /// cursor below the journal tip drops the undone entries
    /// `journal[cursor..]` — envelope ids, envelopes, the aligned history
    /// steps — and PRUNES session snapshots whose † marker fell inside the
    /// truncated range (fix-round truncation MAJOR). Without the prune,
    /// `Engine::snapshots` kept entries whose markers were no longer in the
    /// journal, so an in-session `goto{snapshot}` succeeded and journaled an
    /// envelope no fresh replay could satisfy — the saved package then
    /// permanently refused replay-as-recovery. After the prune, the
    /// in-session goto answers `bad_target` exactly as the replay would
    /// (D-48 in-session ≡ replay). No-op when the cursor is at the tip.
    fn truncate_undone_tail(&mut self) {
        if self.cursor >= self.journal.len() {
            return;
        }
        for dropped in &self.journal[self.cursor..] {
            self.journal_ids.remove(&dropped.id);
        }
        self.journal.truncate(self.cursor);
        self.history
            .truncate(self.cursor.saturating_sub(self.history_base));
        self.history_base = self.history_base.min(self.cursor);
        self.snapshots.retain(|s| s.marker_index < self.cursor);
    }

    fn enforce_history_limits(&mut self) {
        if self.history_bytes_limit == usize::MAX && self.history_depth_limit == usize::MAX {
            return;
        }
        // Phase 1: spill oldest resident below-cursor steps.
        let mut demoted_this_pass = 0usize;
        while self.accounted_total() > self.history_bytes_limit {
            let below_cursor = self.cursor.saturating_sub(self.history_base);
            let Some(i) = self.history[..below_cursor.min(self.history.len())]
                .iter()
                .position(|st| {
                    matches!(&st.delta, StepDelta::Deltas { deltas }
                        if history_spill::resident_accounted(deltas) > 0
                            // Execution-time stashes (DetachMask nodes) pin
                            // their step resident — tiny and rare.
                            && deltas.tree_stash.iter().all(Option::is_none))
                })
            else {
                break; // nothing left to spill
            };
            if self.scratch.is_none() {
                let doc_id = self
                    .doc
                    .as_ref()
                    .map(|d| d.id().as_uuid())
                    .unwrap_or_else(uuid::Uuid::nil);
                match crate::io::scratch::ScratchDisk::create(&self.scratch_dir, doc_id) {
                    Ok(sc) => self.scratch = Some(sc),
                    Err(_) => break, // spill unavailable: stay resident (never corrupt)
                }
            }
            let scratch = self.scratch.as_mut().expect("created above");
            let StepDelta::Deltas { deltas } = std::mem::replace(
                &mut self.history[i].delta,
                StepDelta::Deltas {
                    deltas: DeltaSet {
                        tiles: Vec::new(),
                        tree_after: Vec::new(),
                        tree_stash: Vec::new(),
                        bounds: Vec::new(),
                    },
                },
            ) else {
                unreachable!("position() matched Deltas");
            };
            match history_spill::spill_step(
                deltas,
                scratch,
                &mut demoted_this_pass,
                &mut self.spill_stats,
            ) {
                Ok(spilled) => self.history[i].delta = StepDelta::Spilled(spilled),
                Err(_) => break, // disk trouble: keep the placeholder-free state and stop
            }
        }
        // Phase 2: drop whole steps from the bottom (respecting the floor
        // and never crossing the cursor).
        while (self.history.len() > self.history_depth_limit
            || self.accounted_total() > self.history_bytes_limit)
            && self.history.len() > history_spill::NEVER_DROP_FLOOR
            && self.cursor > self.history_base
        {
            let step = self.history.remove(0);
            if let StepDelta::Spilled(sp) = &step.delta {
                if let Some(scratch) = self.scratch.as_mut() {
                    history_spill::free_step(sp, scratch);
                }
            }
            self.history_base += 1;
            self.spill_stats.dropped_steps += 1;
        }
    }

    /// Revive a spilled step back to a resident `DeltaSet` (undo path).
    fn revive_history_step(&mut self, hist_idx: usize) -> Result<(), EngineError> {
        if !matches!(self.history[hist_idx].delta, StepDelta::Spilled(_)) {
            return Ok(());
        }
        let scratch = self.scratch.as_mut().ok_or_else(|| {
            EngineError::new(
                ErrorCode::IoReadFailed,
                "spilled history without a scratch file",
            )
        })?;
        let StepDelta::Spilled(sp) = std::mem::replace(
            &mut self.history[hist_idx].delta,
            StepDelta::Deltas {
                deltas: DeltaSet {
                    tiles: Vec::new(),
                    tree_after: Vec::new(),
                    tree_stash: Vec::new(),
                    bounds: Vec::new(),
                },
            },
        ) else {
            unreachable!("matched above");
        };
        let deltas = history_spill::revive_step(sp, scratch, &mut self.spill_stats)
            .map_err(|e| EngineError::new(ErrorCode::IoReadFailed, e))?;
        self.history[hist_idx].delta = StepDelta::Deltas { deltas };
        Ok(())
    }

    /// `index` is a JOURNAL index at or above the history base.
    fn history_row(&self, index: usize) -> HistoryRow {
        let step = &self.history[index - self.history_base];
        HistoryRow {
            index,
            id: step.env_id,
            kind: step.kind.clone(),
            timestamp: step.timestamp.clone(),
            active: index < self.cursor,
            dirty: step.dirty.clone(),
            // `index` is a JOURNAL index (see this fn's own contract), so
            // the envelope that carries `filterId` is directly in hand.
            filter_id: self.journal.get(index).and_then(envelope_filter_id),
        }
    }

    /// Package-loader construction (§7.8): loaded document + journal +
    /// cursor, with NO history steps (§10.4: history is not persisted) and
    /// a fresh dedup index.
    pub(crate) fn from_loaded_parts(
        assembly: EngineAssembly,
        doc: Document,
        journal: Vec<Envelope>,
        cursor: usize,
        assets: AssetStore,
    ) -> Engine {
        let journal_ids = journal.iter().map(|e| e.id).collect();
        let cursor = cursor.min(journal.len());
        let impls = assembly
            .impls
            .iter()
            .map(|i| (i.kind(), Arc::clone(i)))
            .collect();
        let history_base = journal.len();
        Engine {
            doc: Some(doc),
            journal,
            journal_ids,
            cursor,
            history: Vec::new(),
            history_base,
            dedup: DedupIndex::new(),
            assembly,
            impls,
            assets,
            history_bytes_limit: usize::MAX,
            history_depth_limit: usize::MAX,
            scratch_dir: std::env::temp_dir(),
            scratch: None,
            spill_stats: history_spill::HistoryStats::default(),
            cms: appname_color::Cms::new(),
            snapshots: Vec::new(),
            vector_captures: HashMap::new(),
            vector_cache: crate::engine::vector_ops::VectorEvalCache::new(),
        }
    }

    /// In-process undo availability: a NON-MARKER step sits between the
    /// history base and the cursor (suffix-aligned history; † markers are
    /// not undoable, §10.3; the persisted prefix undoes via the CLI replay
    /// path instead).
    pub fn undoable_in_process(&self) -> bool {
        (self.history_base..self.cursor)
            .any(|i| !matches!(self.history[i - self.history_base].delta, StepDelta::Marker))
    }

    /// In-process redo availability: an undone NON-MARKER suffix entry
    /// exists (a persisted redoable tail below the base is the replay
    /// path's).
    pub fn redoable_in_process(&self) -> bool {
        self.cursor >= self.history_base
            && (self.cursor..self.journal.len())
                .any(|i| !matches!(self.history[i - self.history_base].delta, StepDelta::Marker))
    }

    /// Rebuild through THIS engine's assembly and asset store (D-48: the
    /// assembly is a construction value; assets are captured values the
    /// journal references — identical inputs replay to an identical
    /// [`Engine::state_hash`]).
    pub fn replay_assembled(&self, journal: &[Envelope]) -> Result<Engine, EngineError> {
        let mut engine = self.fresh_like();
        for env in journal {
            engine.apply_envelope(env.clone())?;
        }
        Ok(engine)
    }

    /// A fresh uninitialized engine carrying THIS engine's assembly and
    /// asset store (the D-48 construction/captured values) — the base every
    /// replay-style rebuild starts from.
    pub fn fresh_like(&self) -> Engine {
        let mut engine = self.assembly.build();
        engine.assets = self.assets.clone();
        // D-48 vector captures are captured values exactly like assets:
        // a replay consumes them instead of re-running geometry.
        engine.vector_captures = self.vector_captures.clone();
        engine
    }

    // `Engine::import_layer` (the proto's IO-call import, the one documented
    // D-48 exception) is RETIRED at CP-0.5 wave 1: the ratified MAJOR-2
    // reconciliation routes imports through `add_raster_asset` + a journaled
    // `edit.paste` envelope, so replay equality holds for EVERY journal (no
    // out-of-journal mutations remain).

    /// Register an opaque asset blob (e.g. an `appname.samples.v1` stroke
    /// recording) and return its content-derived id — the value an
    /// envelope's `samples` field references. An IO-style door like
    /// [`Engine::import_layer`], NOT a command: assets are captured VALUES
    /// (D-48) that the referencing envelope replays against; they never
    /// mutate document state by themselves. Identical bytes yield the same
    /// id (content addressing — script authors can precompute via
    /// [`crate::engine::assets::asset_id_for`]).
    /// Re-arm the asset store's pending grace over every loaded blob
    /// (q-tutorial/Q-5). A door where every registration is its own process —
    /// `appname asset add` — calls this after loading so that assets
    /// registered by EARLIER invocations are not GC'd by this one's save.
    pub fn rearm_pending_assets(&mut self) {
        self.assets.rearm_pending_for_all();
    }

    pub fn add_samples_asset(&mut self, bytes: Vec<u8>) -> Uuid {
        self.assets.insert(bytes)
    }

    /// The `doc.duplicate` † door (§10.3: R†, inverse — — no history
    /// entry, NO JOURNAL RECORD on the source; the duplicate opens with
    /// empty in-process history and its own journal base, §10.6 — exactly
    /// the loaded-parts posture). Handle management is the shell's: the
    /// CLI's `duplicate` verb and cabi's future doc-open surface call
    /// this; the apply-path kind answers typed by design.
    pub fn duplicate(&self) -> Result<Engine, EngineError> {
        let doc = self
            .doc
            .as_ref()
            .ok_or_else(|| EngineError::bad_target("document not initialized"))?
            .clone();
        Ok(Engine::from_loaded_parts(
            self.assembly.clone(),
            doc,
            self.journal.clone(),
            self.cursor,
            self.assets.clone(),
        ))
    }

    /// Ingest ICC profile bytes into the DOCUMENT profile store (§5.2
    /// ladder through the per-engine CMM; §7.5 persistence). Content-derived
    /// id: identical bytes mint the identical `ProfileId`. IO-style door
    /// like asset ingestion — profiles are captured values the assign/
    /// convert envelopes reference (§9.6 rule-2 analog).
    pub fn add_icc_profile(
        &mut self,
        bytes: Vec<u8>,
    ) -> Result<crate::graph::id::ProfileId, EngineError> {
        if self.doc.is_none() {
            return Err(EngineError::bad_target(
                "document not initialized; apply doc.new first (profiles are document-scoped)",
            ));
        }
        // v1 documents are Rgba; the §5.5 space check runs against RGB.
        let uid = self
            .cms
            .open_profile(bytes.clone(), appname_color::ColorModel::Rgb)
            .map_err(color_glue::map_color_error)?;
        let pid = color_glue::icc_profile_id(uid);
        self.doc.as_mut().expect("checked").register_profile(
            pid,
            crate::graph::document::DocProfile {
                uid,
                source: crate::graph::document::DocProfileSource::Icc(std::sync::Arc::new(bytes)),
            },
        );
        Ok(pid)
    }

    /// Write one IMPORT-PROVENANCE key into the document's §7.3 open
    /// `meta` object — the §42.4 slot (`meta["compatReport"]` "persisted
    /// into the imported document … survives the save to `.appname`,
    /// travels with the document") and the §42.2 `meta["psd.*"]` carriage
    /// blobs. An IO-style door like [`Engine::add_icc_profile`], NOT a
    /// command: importers call it once at landing; the value is boundary
    /// provenance — outside undo history (§42.4 badge law: sticky until a
    /// future re-import), not journaled, and not folded into
    /// [`Engine::state_hash`] (a journal replay reconstructs edits, not
    /// import provenance; the loaded document carries it instead —
    /// recorded fold-in question, see the lane log).
    ///
    /// The §10.3 REGISTERED user-facing keys are refused here: those
    /// mutate through `meta.set` envelopes (CP-1.0) so the D-48 spine
    /// keeps every user-visible metadata edit journaled.
    pub fn set_import_meta(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), EngineError> {
        /// §10.3 registered `meta.set` key set (contracts/commands/
        /// meta.set.yaml `paramsSchema.properties.key.enum`).
        const REGISTERED_META_KEYS: [&str; 8] = [
            "title",
            "description",
            "author",
            "copyright",
            "keywords",
            "rating",
            "globalLight",
            "workspaceHints",
        ];
        if REGISTERED_META_KEYS.contains(&key) {
            return Err(EngineError::bad_target(format!(
                "meta key '{key}' is in the §10.3 registered set — it mutates \
                 through a `meta.set` envelope, not the import-provenance door"
            )));
        }
        let doc = self.doc.as_mut().ok_or_else(|| {
            EngineError::bad_target(
                "document not initialized; apply doc.new first (meta is document-scoped)",
            )
        })?;
        doc.meta_mut().insert(key.to_string(), value);
        Ok(())
    }

    /// Register an encoded raster image (PNG at CP-0.5) as an INGESTED
    /// raster asset — `edit.paste`'s carrier (§9.2/§10.3; the ratified
    /// MAJOR-2 reconciliation: imports enter as content-addressed assets
    /// and a journaled envelope references them). Decode is validated at
    /// ingestion so a garbage blob answers typed here rather than at every
    /// paste.
    pub fn add_raster_asset(&mut self, bytes: Vec<u8>) -> Result<Uuid, EngineError> {
        crate::io::png_io::decode_png_to_buffer(&bytes, crate::graph::id::ProfileId::nil())
            .map_err(|e| {
                EngineError::bad_target(format!(
                    "not an ingestible raster image (PNG at CP-0.5): {e}"
                ))
            })?;
        Ok(self
            .assets
            .insert_kind(bytes, assets::AssetKind::RasterImage))
    }

    /// Register a font face blob as a document-embedded asset (TEXTVEC
    /// Door 10, §44.2 step 2 — the document is its own first fallback
    /// resource). With a [`text_ops::TextLayout`] service registered the
    /// face validates + registers into its FontDb immediately (a garbage
    /// blob answers typed here); on a bare assembly the blob is carried
    /// OPAQUE (§7.7 lenient posture — the seam validates when a service
    /// exists, mirrors the vector-sidecar loader rule).
    pub fn add_font_asset(&mut self, bytes: Vec<u8>) -> Result<Uuid, EngineError> {
        let id = assets::asset_id_for(&bytes);
        if let Some(svc) = self.assembly.text.as_ref() {
            svc.register_embedded_face(id, &bytes)?;
        }
        Ok(self.assets.insert_kind(bytes, assets::AssetKind::FontFace))
    }

    /// Register a single-channel f32 coverage plane as a mask asset
    /// (D-ML-1 ingress — the O-ML filing's "ingested single-channel f32
    /// raster asset"). Decode validates at ingestion (`appname.mask.v1`
    /// per [`assets::decode_mask_plane`]) so a garbage blob answers typed
    /// HERE rather than at every application.
    pub fn add_mask_asset(&mut self, bytes: Vec<u8>) -> Result<Uuid, EngineError> {
        assets::decode_mask_plane(&bytes)?;
        Ok(self.assets.insert_kind(bytes, assets::AssetKind::MaskPlane))
    }

    /// Re-register every FontFace asset into the text service's FontDb
    /// (idempotent; the package loader calls this after the asset pass so
    /// document-embedded faces resolve at §44.2 step 2 before any layout).
    /// No-op on a bare assembly.
    pub fn sync_embedded_faces(&mut self) -> Result<(), EngineError> {
        let Some(svc) = self.assembly.text.as_ref() else {
            return Ok(());
        };
        for id in self.assets.ids_sorted() {
            if self.assets.kind_of(id) == Some(assets::AssetKind::FontFace) {
                let bytes = self.assets.bytes(id).expect("sorted ids resolve");
                svc.register_embedded_face(id, bytes)?;
            }
        }
        Ok(())
    }

    /// Door 10 enumeration surface (shell/engine font listing): the text
    /// service's deterministic FontDb view, `None` on bare assemblies.
    pub fn text_faces(&self) -> Option<Vec<text_ops::FontFaceInfo>> {
        self.assembly.text.as_ref().map(|svc| svc.enumerate_faces())
    }

    /// History rows derived from the journal + cursor — identical data to
    /// [`Engine::history`] but available on reopened engines too (whose
    /// in-process delta history is empty, §10.4).
    pub fn journal_rows(&self) -> Vec<HistoryRow> {
        self.journal
            .iter()
            .enumerate()
            .map(|(index, env)| HistoryRow {
                index,
                id: env.id,
                kind: env.kind.clone(),
                timestamp: env.timestamp.clone(),
                active: index < self.cursor,
                dirty: Vec::new(),
                filter_id: envelope_filter_id(env),
            })
            .collect()
    }

    // Rebuild-from-journal notes (D-48/§9.6): identical envelopes yield an
    // identical `Engine::state_hash`.
    // The CP-0.1-era bare `Engine::replay` (kindless `Engine::new()` under
    // the hood) is RETIRED (io re-review): a recovery path built on it
    // answers `not_implemented` for every assembled kind. The replay doors
    // are assembly-carrying only: [`Engine::replay_assembled`] (this
    // engine's assembly + assets) and [`Engine::replay_with`] below.

    /// Replay a journal through an explicit assembly with an empty asset
    /// store — for journals whose kinds capture no assets. Prefer
    /// [`Engine::replay_assembled`] when an engine (and its captured
    /// assets) is at hand.
    pub fn replay_with(
        assembly: &EngineAssembly,
        journal: &[Envelope],
    ) -> Result<Engine, EngineError> {
        let mut engine = assembly.build();
        for env in journal {
            engine.apply_envelope(env.clone())?;
        }
        Ok(engine)
    }

    /// Deterministic identity of the current document state (BLAKE3-256
    /// over the document's logical content: geometry, color tags, tree in
    /// z-order with per-node properties, per-raster `region_hash`,
    /// selection). The replay-equality anchor at the API layer (full
    /// CLI-level byte identity arrives with the package writer, slice 3).
    pub fn state_hash(&self) -> ContentHash {
        self.state_hash_with(true)
    }

    /// The PERSISTENT-state hash: everything the package carries — i.e.
    /// [`Engine::state_hash`] MINUS the live selection fold. §7.3/§14.2
    /// posture (tools finding a, wave 4): the live selection is SESSION
    /// state — document.v1.json has no selection key and persistence is the
    /// explicit `selection.save` → alpha-channel path (§14.2). Package-
    /// boundary equivalence checks (the CLI's replay-as-recovery gate)
    /// compare THIS hash, because a journal replay legitimately reproduces
    /// a live selection the loaded document does not carry. Recorded
    /// fold-in divergence: if the spec later persists the live selection,
    /// this hash and the loader grow it together.
    pub fn state_hash_persistent(&self) -> ContentHash {
        self.state_hash_with(false)
    }

    fn state_hash_with(&self, include_selection: bool) -> ContentHash {
        ContentHash::from_stream(|h| {
            h.update(b"appname.state.v1");
            let Some(doc) = &self.doc else {
                h.update(&[0]);
                return;
            };
            h.update(&[1]);
            h.update(doc.id().as_uuid().as_bytes());
            let g = doc.geometry();
            for v in [
                g.image_size.x,
                g.image_size.y,
                g.canvas_size.x,
                g.canvas_size.y,
            ] {
                h.update(&v.to_le_bytes());
            }
            h.update(&g.resolution_ppi.to_le_bytes());
            // §19.1 crop folds only when active (crop-free docs keep every
            // pinned hash).
            if let Some(r) = g.crop_rect {
                for v in [r.min.x, r.min.y, r.max.x, r.max.y] {
                    h.update(&v.to_le_bytes());
                }
            }
            let c = doc.color();
            h.update(&[c.mode.as_u8(), c.depth.as_u8()]);
            h.update(c.working.as_uuid().as_bytes());
            h.update(doc.root().as_uuid().as_bytes());

            let walk = doc.z_order_walk();
            h.update(&(walk.len() as u32).to_le_bytes());
            for id in walk {
                let n = doc.layer(id).expect("walk yields resolvable ids");
                h.update(n.id().as_uuid().as_bytes());
                let kind = n.kind().kind_str().as_bytes();
                h.update(&(kind.len() as u32).to_le_bytes());
                h.update(kind);
                h.update(&(n.name().len() as u32).to_le_bytes());
                h.update(n.name().as_bytes());
                h.update(&(n.blend() as u32).to_le_bytes());
                h.update(&n.opacity().to_le_bytes());
                h.update(&n.fill_opacity().to_le_bytes());
                h.update(&[n.visible() as u8, n.clip_to_below() as u8]);
                h.update(&n.locks().bits().to_le_bytes());
                for v in n.transform().0 {
                    h.update(&v.to_le_bytes());
                }
                h.update(&(n.children().len() as u32).to_le_bytes());
                match n.pixels() {
                    Some(buf) => {
                        h.update(&[1]);
                        h.update(buf.region_hash().as_bytes());
                    }
                    None => {
                        h.update(&[0]);
                    }
                }
                // §4.6 parametric hash inputs (fold only on the new kinds —
                // raster/group docs keep every pinned hash).
                if let Some(op) = n.adjustment_op() {
                    h.update(op.as_uuid().as_bytes());
                }
                if let Some(so) = n.smart_object() {
                    let crate::graph::node::SmartSource::Embedded(a) = so.source();
                    h.update(a.as_bytes());
                    for v in so.place_transform().0 {
                        h.update(&v.to_le_bytes());
                    }
                    h.update(&(so.smart_filters().len() as u32).to_le_bytes());
                    for f in so.smart_filters() {
                        h.update(f.as_uuid().as_bytes());
                    }
                    h.update(so.cached_composite().region_hash().as_bytes());
                }
                // §20.2 shape payload folds only on shape layers (vector-
                // free docs keep every pinned hash).
                if let Some(sd) = n.shape_data() {
                    h.update(sd.path().as_uuid().as_bytes());
                    let fill = serde_json::to_vec(sd.fill()).expect("fill serializes");
                    h.update(&(fill.len() as u32).to_le_bytes());
                    h.update(&fill);
                    let stroke = serde_json::to_vec(sd.stroke()).expect("stroke serializes");
                    h.update(&(stroke.len() as u32).to_le_bytes());
                    h.update(&stroke);
                    h.update(&(sd.bool_ops().len() as u32).to_le_bytes());
                    for (op, pid) in sd.bool_ops() {
                        h.update(op.as_wire().as_bytes());
                        h.update(pid.as_uuid().as_bytes());
                    }
                }
                // §22 fill payload folds only on fill layers (fill-free
                // docs keep every pinned hash — the shape/mask precedent).
                // Without this fold two fill layers differing ONLY in their
                // content hash EQUAL, which would make a content change
                // invisible to autosave/dirty detection and let §34.4
                // property 9 pass on a document that did not actually
                // return. The WIRE object is folded (not the decoded
                // content) for the same reason the shape row folds its
                // fill/stroke verbatim: it is the complete representation,
                // §7.7 unknown fields included.
                if let Some(fd) = n.fill_data() {
                    let fill = serde_json::to_vec(fd.wire()).expect("fill wire serializes");
                    h.update(&(fill.len() as u32).to_le_bytes());
                    h.update(&fill);
                }
                // §23.1 text payload folds only on text layers (text-free
                // docs keep every pinned hash — the shape/mask precedent).
                if let Some(td) = n.text_data() {
                    h.update(td.content_hash().as_bytes());
                    let b = td.bounds();
                    for v in [b.min.x, b.min.y, b.max.x, b.max.y] {
                        h.update(&v.to_le_bytes());
                    }
                }
                // Masks fold ONLY when present, so mask-free documents keep
                // their pinned CP-0.3 hashes (goldens/selftest references).
                if !n.masks().is_empty() {
                    h.update(&(n.masks().len() as u32).to_le_bytes());
                    for mid in n.masks() {
                        let m = doc.mask(*mid).expect("validated tree");
                        h.update(mid.as_uuid().as_bytes());
                        h.update(&[m.enabled() as u8, m.inverted() as u8, m.linked() as u8]);
                        h.update(&m.density().to_le_bytes());
                        h.update(&m.feather().to_le_bytes());
                        h.update(m.coverage().region_hash().as_bytes());
                    }
                }
            }
            // Embedded profiles fold ONLY when present (mask precedent:
            // builtin-only stores keep every pinned hash).
            if doc.has_embedded_profiles() {
                for (pid, entry) in doc.profiles() {
                    if let crate::graph::document::DocProfileSource::Icc(_) = entry.source {
                        h.update(pid.as_uuid().as_bytes());
                        h.update(&entry.uid.as_u128().to_le_bytes());
                    }
                }
            }
            // §12 op nodes fold only when present (op-free docs keep
            // pinned hashes). Sorted by uuid for determinism.
            if doc.op_count() > 0 {
                let mut ids: Vec<crate::graph::id::OpId> = Vec::new();
                for lid in doc.z_order_walk() {
                    let n = doc.layer(lid).expect("walk resolves");
                    ids.extend(n.adjustment_op());
                    if let Some(so) = n.smart_object() {
                        ids.extend(so.smart_filters().iter().copied());
                    }
                }
                h.update(&(ids.len() as u32).to_le_bytes());
                for oid in ids {
                    let op = doc.op(oid).expect("validated tree");
                    h.update(oid.as_uuid().as_bytes());
                    let k = op.op().as_bytes();
                    h.update(&(k.len() as u32).to_le_bytes());
                    h.update(k);
                    let params = serde_json::to_vec(op.params()).expect("params serialize");
                    h.update(&(params.len() as u32).to_le_bytes());
                    h.update(&params);
                    h.update(&[(op.blend() as u32) as u8, op.enabled() as u8]);
                    h.update(&op.opacity().to_le_bytes());
                    match op.mask() {
                        Some(m) => {
                            h.update(&[1]);
                            h.update(m.as_uuid().as_bytes());
                            let mask = doc.mask(m).expect("validated tree");
                            h.update(mask.coverage().region_hash().as_bytes());
                        }
                        None => {
                            h.update(&[0]);
                        }
                    }
                }
            }
            // §20 doc paths fold only when present (path-free docs keep
            // pinned hashes). Sorted by uuid for determinism; the CONTENT
            // hash stands in for the sidecar bytes (single truth).
            if doc.path_count() > 0 {
                let ids = doc.path_ids_sorted();
                h.update(&(ids.len() as u32).to_le_bytes());
                for pid in ids {
                    let p = doc.path(pid).expect("sorted ids resolve");
                    h.update(pid.as_uuid().as_bytes());
                    match p.name() {
                        Some(n) => {
                            h.update(&(n.len() as u32).to_le_bytes());
                            h.update(n.as_bytes());
                        }
                        None => {
                            h.update(&u32::MAX.to_le_bytes());
                        }
                    }
                    h.update(p.content_hash().as_bytes());
                }
            }
            // §24.2 furniture folds only when present (default keeps
            // pinned hashes).
            if !doc.guides().is_default() {
                let bytes = serde_json::to_vec(doc.guides()).expect("guides serialize");
                h.update(&(bytes.len() as u32).to_le_bytes());
                h.update(&bytes);
            }
            // §14.1 named channels fold only when present (channel-free
            // docs keep every pinned hash — the mask precedent). §14.1
            // list order IS the fold order.
            if doc.channel_count() > 0 {
                let ids = doc.channel_ids_ordered();
                h.update(&(ids.len() as u32).to_le_bytes());
                for cid in ids {
                    let c = doc.channel(*cid).expect("ordered ids resolve");
                    h.update(cid.as_uuid().as_bytes());
                    h.update(&(c.name().len() as u32).to_le_bytes());
                    h.update(c.name().as_bytes());
                    h.update(c.buffer().region_hash().as_bytes());
                    match c.spot() {
                        Some(s) => {
                            let bytes = serde_json::to_vec(s).expect("spot serializes");
                            h.update(&(bytes.len() as u32).to_le_bytes());
                            h.update(&bytes);
                        }
                        None => {
                            h.update(&[0]);
                        }
                    }
                }
            }
            // §29.2 slices fold only when present (slice-free docs keep
            // pinned hashes; the guides precedent).
            if !doc.slices().is_empty() {
                let bytes = serde_json::to_vec(doc.slices()).expect("slices serialize");
                h.update(&(bytes.len() as u32).to_le_bytes());
                h.update(&bytes);
            }
            // §10.3 REGISTERED meta keys fold only when present (meta.set
            // document state). Import-provenance carriage (compatReport,
            // psd.*) deliberately stays OUT of the fold — pre-slice
            // hashes over imported documents stay byte-identical, and
            // provenance is carriage, not command-reachable state.
            {
                const REGISTERED: [&str; 8] = [
                    "title",
                    "description",
                    "author",
                    "copyright",
                    "keywords",
                    "rating",
                    "globalLight",
                    "workspaceHints",
                ];
                for key in REGISTERED {
                    if let Some(v) = doc.meta().get(key) {
                        let bytes = serde_json::to_vec(v).expect("meta value serializes");
                        h.update(&(key.len() as u32).to_le_bytes());
                        h.update(key.as_bytes());
                        h.update(&(bytes.len() as u32).to_le_bytes());
                        h.update(&bytes);
                    }
                }
            }
            if include_selection {
                let sel = doc.selection();
                h.update(&[sel.active() as u8]);
                let b = sel.bounds();
                for v in [b.min.x, b.min.y, b.max.x, b.max.y] {
                    h.update(&v.to_le_bytes());
                }
                h.update(sel.mask().region_hash().as_bytes());
            }
        })
    }
}

impl Default for Engine {
    fn default() -> Engine {
        Engine::new()
    }
}

/// Summary Debug (ES §1.6 discipline; the document summarizes itself).
impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Engine")
            .field("initialized", &self.doc.is_some())
            .field("journal", &self.journal.len())
            .field("cursor", &self.cursor)
            .field("history_base", &self.history_base)
            .field("assembled_kinds", &self.impls.len())
            .field("assets", &self.assets.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{IRect, IVec2};
    use crate::graph::id::LayerId;
    use crate::tile::channel::Channel;
    use crate::tile::channel::ChannelKind;
    use crate::tile::map::TileCoord;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    fn envelope_text(id: &str, params: serde_json::Value) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": id,
            "kind": "doc.new",
            "params": params
        })
        .to_string()
    }

    fn fresh_id(n: u32) -> String {
        format!("b0000000-0000-4000-8000-0000000{n:05x}")
    }

    fn apply_new(
        engine: &mut Engine,
        n: u32,
        params: serde_json::Value,
    ) -> Result<ApplyReceipt, EngineError> {
        engine.apply_json(&envelope_text(&fresh_id(n), params))
    }

    // ---------------- pinned-order evidence ----------------

    #[test]
    fn stage2_duplicate_id_beats_stage3_unknown_kind() {
        let mut e = Engine::new();
        apply_new(&mut e, 1, json!({"width": 8, "height": 8})).unwrap();
        // Same id, unknown kind: the duplicate check reports (proto order).
        let text = json!({
            "schema": "appname.command.v1",
            "id": fresh_id(1),
            "kind": "zzz.not_a_kind"
        })
        .to_string();
        let err = e.apply_json(&text).unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);
    }

    #[test]
    fn stage3_unknown_kind() {
        let mut e = Engine::new();
        let text = json!({
            "schema": "appname.command.v1",
            "id": fresh_id(2),
            "kind": "doc.transmogrify"
        })
        .to_string();
        let err = e.apply_json(&text).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownKind); // G-ENV-kind-unknown
    }

    #[test]
    fn stage4_not_implemented_beats_stage5_params() {
        let mut e = Engine::new();
        // layer.add with garbage params: implementedness reports first.
        let text = json!({
            "schema": "appname.command.v1",
            "id": fresh_id(3),
            "kind": "layer.add",
            "params": {"definitely": "wrong"}
        })
        .to_string();
        let err = e.apply_json(&text).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    #[test]
    fn stage5_params_beats_stage6_state() {
        let mut e = Engine::new();
        apply_new(&mut e, 4, json!({"width": 8, "height": 8})).unwrap();
        // Initialized document AND width 0: schema violation reports first
        // (proto order: params before init gating).
        let err = apply_new(&mut e, 5, json!({"width": 0, "height": 8})).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
    }

    #[test]
    fn stage6_order_profile_before_unsupported_mode() {
        // The corpus's executed profile case: cmyk_a + explicit profile must
        // report profile_space_mismatch, not cmyk_a's not_implemented
        // (G-DN-contract-invalid-2 pins 6b before 6d).
        let mut e = Engine::new();
        let err = apply_new(
            &mut e,
            6,
            json!({
                "width": 64, "height": 64, "mode": "cmyk_a",
                "profile": "eeeeeeee-0000-4000-8000-000000000001"
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);
    }

    // ---------------- registry-wide not_implemented ----------------

    #[test]
    fn all_101_unimplemented_kinds_report_not_implemented() {
        // Fixture fidelity (review blocker 1): apply each kind's CONTRACT
        // validMinimal envelope byte-for-byte — the exact shapes the corpus
        // not_implemented cases are generated from, including the samples
        // assetId the seven stroke kinds carry.
        let mut count = 0usize;
        for spec in registry_gen::COMMANDS.iter() {
            if spec.kind == "doc.new" {
                continue;
            }
            let mut e = Engine::new();
            let err = e.apply_bytes(spec.valid_minimal.as_bytes()).unwrap_err();
            if matches!(spec.kind, "history.snapshot" | "history.goto") {
                // CP-0.7: the § daggers are engine-built-ins (doc.new
                // precedent); uninitialized bare engines answer the
                // doc-slot check for them.
                assert_eq!(err.code, ErrorCode::BadTarget, "dagger built-in doc gate");
                count += 1;
                continue;
            }
            assert_eq!(
                err.code,
                ErrorCode::NotImplemented,
                "kind {} must report not_implemented (OQ-1); got {}",
                spec.kind,
                err
            );
            count += 1;
        }
        assert_eq!(
            count, 101,
            "exactly 101 registered kinds are unimplemented at CP-0.1"
        );
    }

    #[test]
    fn stroke_kind_with_samples_reports_not_implemented() {
        // Review blocker 1 regression: the contract envelope for
        // brush.stroke carries samples — stage 1 must carry it through so
        // stage 4 answers (corpus G-NI-brush.stroke expectation, no hedge).
        let spec = registry::lookup("brush.stroke").unwrap();
        assert!(spec.valid_minimal.contains("\"samples\":"));
        let mut e = Engine::new();
        let err = e.apply_bytes(spec.valid_minimal.as_bytes()).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert!(err.message.contains("brush.stroke"));
    }

    #[test]
    fn samples_of_wrong_type_still_rejects_at_stage_1() {
        let mut e = Engine::new();
        let text = json!({
            "schema": "appname.command.v1",
            "id": fresh_id(130),
            "kind": "brush.stroke",
            "samples": 12345
        })
        .to_string();
        let err = e.apply_json(&text).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutOfRange);
        assert_eq!(err.field.as_deref(), Some("samples"));
    }

    #[test]
    fn sampled_not_implemented_details_name_kind_and_milestone() {
        let mut e = Engine::new();
        // history.goto left the sample at CP-0.7 (dagger built-in now);
        // channel.calculations is the 1.0-milestone representative.
        for (kind, milestone) in [
            ("layer.add", "0.5"),
            ("brush.stroke", "0.3"),
            ("channel.calculations", "1.0"),
        ] {
            let text = json!({
                "schema": "appname.command.v1",
                "id": Uuid::new_v4().to_string(),
                "kind": kind
            })
            .to_string();
            let err = e.apply_json(&text).unwrap_err();
            assert_eq!(err.code, ErrorCode::NotImplemented);
            assert!(
                err.message.contains(kind) && err.message.contains(milestone),
                "{}",
                err.message
            );
        }
    }

    // ---------------- doc.new contract rejects (corpus shape) ----------------

    #[test]
    fn doc_new_missing_required_params() {
        let mut e = Engine::new();
        let err = apply_new(&mut e, 10, json!({"height": 1080})).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField); // G-DN-missing-width
        assert_eq!(err.field.as_deref(), Some("width"));
        let err = apply_new(&mut e, 11, json!({"width": 1920})).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingField); // G-DN-missing-height
        assert_eq!(err.field.as_deref(), Some("height"));
    }

    #[test]
    fn doc_new_bounds_rejects() {
        let mut e = Engine::new();
        for (n, params) in [
            (12, json!({"width": 0, "height": 1080})), // G-DN-width-below-min
            (13, json!({"width": 1000001, "height": 1080})), // G-DN-width-above-max
            (14, json!({"width": 1920, "height": 0})), // G-DN-height-below-min
            (15, json!({"width": 1920, "height": 1000001})), // G-DN-height-above-max
            (
                16,
                json!({"width": 1920, "height": 1080, "resolutionPpi": 0}),
            ), // below-min
            (
                17,
                json!({"width": 1920, "height": 1080, "resolutionPpi": 10001}),
            ), // above-max
        ] {
            let err = apply_new(&mut e, n, params).unwrap_err();
            assert_eq!(err.code, ErrorCode::OutOfRange, "case {n}");
        }
    }

    #[test]
    fn doc_new_type_and_enum_rejects() {
        let mut e = Engine::new();
        for (n, params) in [
            (20, json!({"width": "1", "height": 1080})), // G-DN-width-bad-type
            (21, json!({"width": 1920, "height": "1"})), // G-DN-height-bad-type
            (22, json!({"width": 19.5, "height": 1080})), // integer with fraction
            (
                23,
                json!({"width": 1920, "height": 1080, "resolutionPpi": "1"}),
            ), // G-DN-resolutionPpi-bad-type
            (
                24,
                json!({"width": 1920, "height": 1080, "mode": "not_in_enum"}),
            ), // G-DN-mode-bad-enum
            (25, json!({"width": 1920, "height": 1080, "mode": 12345})), // G-DN-mode-bad-type
            (
                26,
                json!({"width": 1920, "height": 1080, "depth": "not_in_enum"}),
            ), // G-DN-depth-bad-enum
            (27, json!({"width": 1920, "height": 1080, "depth": 12345})), // G-DN-depth-bad-type
            (
                28,
                json!({"width": 1920, "height": 1080, "background": "not_in_enum"}),
            ), // G-DN-background-bad-enum
            (
                29,
                json!({"width": 1920, "height": 1080, "background": 12345}),
            ), // G-DN-background-bad-type
            (30, json!({"width": 1920, "height": 1080, "profile": 12345})), // G-DN-profile-bad-type
            (
                31,
                json!({"width": 1920, "height": 1080, "profile": "not-a-uuid"}),
            ),
            (32, json!({"width": 1920, "height": 1080, "extra": true})), // G-DN-unknown-param-field
        ] {
            let err = apply_new(&mut e, n, params).unwrap_err();
            assert_eq!(err.code, ErrorCode::OutOfRange, "case {n}");
        }
        assert!(
            !e.is_initialized(),
            "every reject leaves the shell untouched"
        );
    }

    #[test]
    fn doc_new_already_initialized_is_bad_target() {
        let mut e = Engine::new();
        apply_new(&mut e, 40, json!({"width": 64, "height": 64})).unwrap();
        let err = apply_new(&mut e, 41, json!({"width": 64, "height": 64})).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget); // G-DN-contract-invalid-3
        assert!(err.message.contains("already initialized"));
    }

    #[test]
    fn doc_new_valid_minimal_accepts_and_builds_the_document() {
        // G-DN-accept-valid-minimal (the contract's validMinimal payload).
        let mut e = Engine::new();
        let receipt = apply_new(&mut e, 50, json!({"width": 1920, "height": 1080})).unwrap();
        assert_eq!(receipt.kind, "doc.new");
        assert_eq!(receipt.revision, 1);
        assert_eq!(
            receipt.created_ids.len(),
            2,
            "[root group, background layer]"
        );

        let doc = e.document().unwrap();
        assert_eq!(doc.layer_count(), 2);
        let bg = LayerId::from_uuid(receipt.created_ids[1]);
        let layer = doc.layer(bg).unwrap();
        assert_eq!(layer.name(), "Background");
        let buf = layer.pixels().unwrap();
        assert_eq!(buf.bounds(), IRect::new(0, 0, 1920, 1080));
        assert_eq!(buf.kind(), ChannelKind::U8); // default depth
                                                 // Default background white, premultiplied.
        assert_eq!(buf.read_pixel_f32(IVec2::new(0, 0)), [1.0, 1.0, 1.0, 1.0]);
        assert!(buf.validate_premultiplied().is_ok());
        assert_eq!(doc.geometry().resolution_ppi, 300.0); // contract default
    }

    #[test]
    fn doc_new_min_boundary_1x1_accepts() {
        // G-DN-accept-min-boundary.
        let mut e = Engine::new();
        apply_new(&mut e, 51, json!({"width": 1, "height": 1})).unwrap();
        let doc = e.document().unwrap();
        assert_eq!(doc.geometry().canvas_size, crate::geom::IVec2::new(1, 1));
    }

    #[test]
    fn doc_new_valid_maximal_is_schema_valid_without_executing() {
        // G-DN-accept-valid-maximal-schema-only: the 10^6 x 10^6 payload
        // passes stage 5 (schema) — asserted at the validator layer without
        // allocating a quadrillion-pixel document.
        let spec = registry::lookup("doc.new").unwrap();
        let params = json!({
            "width": 1000000, "height": 1000000, "resolutionPpi": 10000,
            "mode": "lab_a", "depth": "f32", "background": "background_color",
            "profile": "eeeeeeee-0000-4000-8000-000000000001"
        });
        let map = params.as_object().unwrap();
        assert!(registry::validate_params(spec, Some(map)).is_ok());
    }

    #[test]
    fn doc_new_backgrounds_per_mode_write_the_2_2_values() {
        // white/black/transparent across the supported modes.
        let mut e = Engine::new();
        apply_new(
            &mut e,
            60,
            json!({"width": 8, "height": 8, "mode": "gray_a", "background": "white"}),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let buf = e.document().unwrap().layer(bg).unwrap().pixels().unwrap();
        assert_eq!(buf.read_pixel_f32(IVec2::new(3, 3)), [1.0, 1.0, 0.0, 0.0]);

        let mut e = Engine::new();
        apply_new(
            &mut e,
            61,
            json!({"width": 8, "height": 8, "mode": "lab_a", "background": "black"}),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let buf = e.document().unwrap().layer(bg).unwrap().pixels().unwrap();
        let px = buf.read_pixel_f32(IVec2::new(0, 0));
        // Lab neutral a/b encode as 128/255 exactly (ES §2.2) — the u8
        // storage default round-trips it.
        assert_eq!(px, [0.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);

        let mut e = Engine::new();
        apply_new(
            &mut e,
            62,
            json!({"width": 8, "height": 8, "background": "transparent"}),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let buf = e.document().unwrap().layer(bg).unwrap().pixels().unwrap();
        assert!(
            buf.tiles().is_empty(),
            "transparent background stores no tiles"
        );
        assert_eq!(buf.bounds(), IRect::EMPTY);
    }

    #[test]
    fn doc_new_black_rgba_background() {
        let mut e = Engine::new();
        apply_new(
            &mut e,
            63,
            json!({"width": 8, "height": 8, "background": "black"}),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let buf = e.document().unwrap().layer(bg).unwrap().pixels().unwrap();
        assert_eq!(buf.read_pixel_f32(IVec2::new(7, 7)), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn doc_new_paused_depths_are_typed_errors_not_panics() {
        // Review minor-6 slice-2 constraint: the paused-depth panics are
        // unreachable from the command surface — typed errors instead.
        let mut e = Engine::new();
        let err =
            apply_new(&mut e, 70, json!({"width": 8, "height": 8, "depth": "f16"})).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert_eq!(err.field.as_deref(), Some("depth"));

        #[cfg(not(feature = "depth-u16"))]
        {
            let err = apply_new(&mut e, 71, json!({"width": 8, "height": 8, "depth": "u16"}))
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotImplemented);
        }
        #[cfg(feature = "depth-u16")]
        {
            apply_new(&mut e, 72, json!({"width": 8, "height": 8, "depth": "u16"})).unwrap();
            assert_eq!(e.document().unwrap().color().depth, ChannelKind::U16);
        }
    }

    #[test]
    fn doc_new_unsupported_mode_is_typed() {
        let mut e = Engine::new();
        let err = apply_new(
            &mut e,
            73,
            json!({"width": 8, "height": 8, "mode": "cmyk_a"}),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
        assert_eq!(err.field.as_deref(), Some("mode"));
    }

    #[test]
    fn background_color_resolves_to_the_headless_default_swatch() {
        // Slice-2 open question CLOSED (doc_new::background_value reading):
        // headless tool state resolves the swatch at its startup default
        // (white), deterministically — replay reproduces it.
        let mut e = Engine::new();
        apply_new(
            &mut e,
            74,
            json!({"width": 8, "height": 8, "background": "background_color"}),
        )
        .unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        let buf = e.document().unwrap().layer(bg).unwrap().pixels().unwrap();
        assert_eq!(buf.read_pixel_f32(IVec2::new(4, 4)), [1.0, 1.0, 1.0, 1.0]);
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(replayed.state_hash(), e.state_hash());
    }

    #[test]
    fn doc_new_f32_depth_accepts() {
        let mut e = Engine::new();
        apply_new(&mut e, 75, json!({"width": 8, "height": 8, "depth": "f32"})).unwrap();
        assert_eq!(e.document().unwrap().color().depth, ChannelKind::F32);
    }

    #[test]
    fn rgba_profile_param_is_profile_space_mismatch_at_cp01() {
        // Divergence note: EVERY explicit profile is unverifiable pre-CMM.
        let mut e = Engine::new();
        let err = apply_new(
            &mut e,
            76,
            json!({"width": 8, "height": 8, "profile": "eeeeeeee-0000-4000-8000-000000000001"}),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ProfileSpaceMismatch);
    }

    #[test]
    fn non_finite_numbers_cannot_reach_params_validation() {
        // §40.1 non-finite note: RFC 8259 has no NaN/Inf literals and
        // serde_json refuses over-range magnitudes at parse — stage 0.
        let mut e = Engine::new();
        let text = format!(
            "{{\"schema\":\"appname.command.v1\",\"id\":\"{}\",\"kind\":\"doc.new\",\
             \"params\":{{\"width\":8,\"height\":8,\"resolutionPpi\":1e999}}}}",
            fresh_id(77)
        );
        let err = e.apply_json(&text).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArg);
    }

    #[test]
    fn apply_bytes_rejects_non_utf8_and_accepts_utf8() {
        let mut e = Engine::new();
        let err = e.apply_bytes(&[0xFF, 0xFE, 0x00]).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArg);
        let text = envelope_text(&fresh_id(80), json!({"width": 8, "height": 8}));
        e.apply_bytes(text.as_bytes()).unwrap();
        assert!(e.is_initialized());
    }

    // ---------------- journal / replay determinism ----------------

    #[test]
    fn replay_reaches_identical_state_hash() {
        let mut a = Engine::new();
        apply_new(
            &mut a,
            90,
            json!({"width": 640, "height": 480, "background": "black"}),
        )
        .unwrap();
        let b = a.replay_assembled(a.journal()).unwrap();
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "replay is byte-identical at the API layer"
        );
        // Created ids replay identically too (§9.6 v5 derivation).
        assert_eq!(a.document().unwrap().id(), b.document().unwrap().id());
        assert_eq!(
            a.document().unwrap().z_order_walk(),
            b.document().unwrap().z_order_walk()
        );
    }

    #[test]
    fn same_envelope_same_state_different_envelope_different_state() {
        let mut a = Engine::new();
        apply_new(&mut a, 91, json!({"width": 64, "height": 64})).unwrap();
        let mut b = Engine::new();
        apply_new(&mut b, 91, json!({"width": 64, "height": 64})).unwrap();
        assert_eq!(a.state_hash(), b.state_hash());

        // A different envelope id derives different object ids (§9.6).
        let mut c = Engine::new();
        apply_new(&mut c, 92, json!({"width": 64, "height": 64})).unwrap();
        assert_ne!(a.state_hash(), c.state_hash());

        // Different content differs too.
        let mut d = Engine::new();
        apply_new(
            &mut d,
            91,
            json!({"width": 64, "height": 64, "background": "black"}),
        )
        .unwrap();
        assert_ne!(a.state_hash(), d.state_hash());
    }

    #[test]
    fn journal_is_append_only_and_branch_truncates() {
        let mut e = Engine::new();
        apply_new(&mut e, 95, json!({"width": 8, "height": 8})).unwrap();
        assert_eq!(e.journal().len(), 1);
        assert_eq!(e.journal_cursor(), 1);

        e.undo().unwrap();
        assert_eq!(e.journal().len(), 1, "undo never removes journal entries");
        assert_eq!(e.journal_cursor(), 0);

        // An undone entry still occupies its id (it is journaled until
        // branched) — proto order: stage 2 precedes truncation.
        let err = apply_new(&mut e, 95, json!({"width": 16, "height": 16})).unwrap_err();
        assert_eq!(err.code, ErrorCode::DuplicateId);

        // Applying a NEW id on the undone tail branches: the redoable entry
        // drops and its id is released (linear history, DD ⑰).
        apply_new(&mut e, 96, json!({"width": 16, "height": 16})).unwrap();
        assert_eq!(e.journal().len(), 1);
        assert_eq!(e.journal_cursor(), 1);
        let err = e.redo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToRedo);
        assert_eq!(
            e.document().unwrap().geometry().canvas_size,
            crate::geom::IVec2::new(16, 16)
        );
        // Evidence the truncated entry's id (95) left the journal: re-using
        // it now passes stage 2 and fails at stage 6a instead.
        let err = apply_new(&mut e, 95, json!({"width": 8, "height": 8})).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::BadTarget,
            "id released by the branch truncation"
        );
    }

    // ---------------- undo / redo invariants ----------------

    #[test]
    fn undo_redo_round_trips_state_hash() {
        let mut e = Engine::new();
        let empty_hash = e.state_hash();
        apply_new(&mut e, 100, json!({"width": 320, "height": 200})).unwrap();
        let full_hash = e.state_hash();
        assert_ne!(empty_hash, full_hash);

        let row = e.undo().unwrap();
        assert!(!row.active);
        assert!(
            !e.is_initialized(),
            "doc.new undo returns the empty pre-init handle (§40.2)"
        );
        assert_eq!(e.state_hash(), empty_hash);

        let row = e.redo().unwrap();
        assert!(row.active);
        assert_eq!(e.state_hash(), full_hash, "redo restores byte identity");

        // Idempotent bounds of the stacks.
        e.undo().unwrap();
        let err = e.undo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToUndo);
        e.redo().unwrap();
        let err = e.redo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToRedo);
    }

    #[test]
    fn history_rows_track_cursor() {
        let mut e = Engine::new();
        apply_new(&mut e, 110, json!({"width": 8, "height": 8})).unwrap();
        let rows = e.history();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].active);
        assert_eq!(rows[0].kind, "doc.new");
        e.undo().unwrap();
        assert!(!e.history()[0].active);
    }

    #[test]
    fn nothing_to_undo_on_fresh_engine() {
        let mut e = Engine::new();
        assert_eq!(e.undo().unwrap_err().code, ErrorCode::NothingToUndo);
        assert_eq!(e.redo().unwrap_err().code, ErrorCode::NothingToRedo);
    }

    // ---------------- commit pipeline: dedup + recorder evidence ----------------

    #[test]
    fn background_tiles_are_interned_to_one_payload() {
        // §3.8 in the commit pipeline: a tile-aligned white 1024x512
        // background is 4x2 tiles of identical content — after commit they
        // share ONE Arc, and no slice-1 debug tripwire fired on the way
        // (this test runs under debug_assertions).
        let mut e = Engine::new();
        apply_new(&mut e, 120, json!({"width": 1024, "height": 512})).unwrap();
        let doc = e.document().unwrap();
        let bg = doc.z_order_walk()[0];
        let buf = doc.layer(bg).unwrap().pixels().unwrap();
        assert_eq!(buf.tiles().len(), 4 * 2);
        let first = buf.get_tile(TileCoord::new(0, 0, 0)).unwrap();
        for (c, arc) in buf.tiles().iter() {
            assert!(
                Arc::ptr_eq(first, arc),
                "tile {c:?} must share the interned payload (per-context dedup)"
            );
        }
    }

    #[test]
    fn unaligned_background_interns_per_content_class() {
        // 1920x1080 is NOT tile-aligned: interior tiles are one uniform
        // content class; right-column, bottom-row, and corner edge tiles are
        // three more (partial coverage, §2.3 edge zero rule). Dedup interns
        // each class to one payload — exactly 4 distinct Arcs across 40
        // tiles.
        let mut e = Engine::new();
        apply_new(&mut e, 122, json!({"width": 1920, "height": 1080})).unwrap();
        let doc = e.document().unwrap();
        let bg = doc.z_order_walk()[0];
        let buf = doc.layer(bg).unwrap().pixels().unwrap();
        assert_eq!(buf.tiles().len(), 8 * 5);

        let interior_a = buf.get_tile(TileCoord::new(0, 0, 0)).unwrap();
        let interior_b = buf.get_tile(TileCoord::new(0, 6, 3)).unwrap();
        assert!(
            Arc::ptr_eq(interior_a, interior_b),
            "interior tiles share one payload"
        );
        let right_a = buf.get_tile(TileCoord::new(0, 7, 0)).unwrap();
        let right_b = buf.get_tile(TileCoord::new(0, 7, 3)).unwrap();
        assert!(
            Arc::ptr_eq(right_a, right_b),
            "right-edge tiles share one payload"
        );
        let bottom_a = buf.get_tile(TileCoord::new(0, 0, 4)).unwrap();
        let bottom_b = buf.get_tile(TileCoord::new(0, 6, 4)).unwrap();
        assert!(
            Arc::ptr_eq(bottom_a, bottom_b),
            "bottom-edge tiles share one payload"
        );

        let mut distinct: Vec<*const crate::tile::storage::TileStorage> = buf
            .tiles()
            .iter()
            .map(|(_, arc)| Arc::as_ptr(arc))
            .collect();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            4,
            "interior + right + bottom + corner content classes"
        );
    }

    // ---------------- assembly seam (wave-2 arbitration 1) ----------------

    /// Test-only kind implementation over a REGISTERED kind (`fill` — real
    /// §10.3 row, unimplemented elsewhere at CP-0.3): paints an opaque
    /// 256×256 plane at a params-chosen tile through the context doors.
    /// Failure mode: `content: "pattern"` errors AFTER installing (rollback
    /// evidence).
    struct MockFill;

    impl kind_impl::KindImpl for MockFill {
        fn kind(&self) -> &'static str {
            "fill"
        }

        fn apply(
            &self,
            cx: &mut kind_impl::KindContext<'_>,
        ) -> Result<kind_impl::KindOutcome, EngineError> {
            let doc = cx.document();
            let layer = doc.z_order_walk()[0];
            let coord = TileCoord::new(0, 0, 0);
            let plane = vec![[0.25f32, 0.5, 0.125, 1.0]; crate::tile::TILE * crate::tile::TILE];
            cx.install_tile_plane(layer, coord, &plane)?;
            cx.grow_layer_bounds(layer, coord.pixel_rect())?;
            if cx
                .params()
                .get("content")
                .and_then(serde_json::Value::as_str)
                == Some("pattern")
            {
                return Err(EngineError::bad_target("mock failure after mutation"));
            }
            Ok(kind_impl::KindOutcome::default())
        }
    }

    fn assembled() -> Engine {
        kind_impl::EngineAssembly::new()
            .with_kind(Arc::new(MockFill))
            .build()
    }

    fn fill_envelope(n: u32, content: &str) -> String {
        json!({
            "schema": "appname.command.v1",
            "id": fresh_id(n),
            "kind": "fill",
            "params": { "content": content }
        })
        .to_string()
    }

    #[test]
    fn bare_engine_answers_not_implemented_assembled_engine_implements() {
        let mut bare = Engine::new();
        apply_new(&mut bare, 200, json!({"width": 300, "height": 300})).unwrap();
        let err = bare
            .apply_json(&fill_envelope(201, "foreground"))
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::NotImplemented,
            "bare engines keep the CP-0.1 answer"
        );

        let mut e = assembled();
        apply_new(
            &mut e,
            200,
            json!({"width": 300, "height": 300, "background": "transparent"}),
        )
        .unwrap();
        let receipt = e.apply_json(&fill_envelope(201, "foreground")).unwrap();
        assert_eq!(receipt.kind, "fill");
        let bg = e.document().unwrap().z_order_walk()[0];
        let buf = e.document().unwrap().layer(bg).unwrap().pixels().unwrap();
        // u8 storage re-encodes the plane values (64/255, 128/255, 32/255).
        let expect = [
            u8::from_f32(0.25).to_f32(),
            u8::from_f32(0.5).to_f32(),
            u8::from_f32(0.125).to_f32(),
            1.0,
        ];
        assert_eq!(buf.read_pixel_f32(IVec2::new(10, 10)), expect);
        assert_eq!(buf.bounds(), IRect::new(0, 0, 256, 256));
    }

    #[test]
    fn assembled_receipt_reports_per_tile_dirty_rects() {
        let mut e = assembled();
        apply_new(
            &mut e,
            210,
            json!({"width": 300, "height": 300, "background": "transparent"}),
        )
        .unwrap();
        let receipt = e.apply_json(&fill_envelope(211, "foreground")).unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        assert_eq!(
            receipt.dirty,
            vec![DirtyRect {
                layer: bg,
                rect: IRect::new(0, 0, 256, 256)
            }],
            "tile-writing kinds report each touched TileCoord::pixel_rect()"
        );
        // Undo and redo rows echo the SAME rects (arbitration 2).
        let undo_row = e.undo().unwrap();
        assert_eq!(undo_row.dirty, receipt.dirty);
        let redo_row = e.redo().unwrap();
        assert_eq!(redo_row.dirty, receipt.dirty);
    }

    #[test]
    fn doc_new_receipt_reports_structural_bounds_dirty() {
        let mut e = Engine::new();
        let receipt = apply_new(&mut e, 215, json!({"width": 300, "height": 200})).unwrap();
        let bg = e.document().unwrap().z_order_walk()[0];
        assert_eq!(
            receipt.dirty,
            vec![DirtyRect {
                layer: bg,
                rect: IRect::new(0, 0, 300, 200)
            }],
            "structural kinds report node bounds before∪after"
        );
        // Transparent background: nothing visible changed.
        let mut t = Engine::new();
        let receipt = apply_new(
            &mut t,
            216,
            json!({"width": 64, "height": 64, "background": "transparent"}),
        )
        .unwrap();
        assert!(receipt.dirty.is_empty());
    }

    #[test]
    fn assembled_undo_redo_restore_via_cow_deltas() {
        let mut e = assembled();
        apply_new(&mut e, 220, json!({"width": 300, "height": 300})).unwrap();
        let before_hash = e.state_hash();
        e.apply_json(&fill_envelope(221, "foreground")).unwrap();
        let after_hash = e.state_hash();
        assert_ne!(before_hash, after_hash);

        e.undo().unwrap();
        assert_eq!(
            e.state_hash(),
            before_hash,
            "undo = before-Arc swaps (§10.4)"
        );
        e.redo().unwrap();
        assert_eq!(e.state_hash(), after_hash, "redo = after-Arc swaps");
        // The document survives (Deltas steps never stash the doc).
        assert!(e.is_initialized());
    }

    #[test]
    fn assembled_replay_reproduces_state_hash() {
        let mut e = assembled();
        apply_new(&mut e, 230, json!({"width": 300, "height": 300})).unwrap();
        e.apply_json(&fill_envelope(231, "foreground")).unwrap();
        let replayed = e.replay_assembled(e.journal()).unwrap();
        assert_eq!(
            replayed.state_hash(),
            e.state_hash(),
            "assembly is a construction value (D-48)"
        );
        // An explicitly BARE assembly refuses the assembled kind — proving
        // the journal alone is not enough without the same assembly (the
        // kindless `Engine::replay` convenience is retired for exactly this
        // footgun; every replay door now carries an assembly).
        let err = Engine::replay_with(&EngineAssembly::new(), e.journal()).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotImplemented);
    }

    #[test]
    fn failed_assembled_apply_rolls_back_cleanly() {
        let mut e = assembled();
        apply_new(&mut e, 240, json!({"width": 300, "height": 300})).unwrap();
        let hash = e.state_hash();
        let journal_len = e.journal().len();
        // The mock mutates a tile, THEN fails: the engine must restore the
        // befores (§10.1 rollback) and journal nothing.
        let err = e.apply_json(&fill_envelope(241, "pattern")).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert_eq!(e.state_hash(), hash, "partial mutation rolled back");
        assert_eq!(
            e.journal().len(),
            journal_len,
            "failed applies never journal"
        );
        let rows = e.history();
        assert_eq!(rows.len(), 1, "no history step for the failed apply");
        assert_eq!(rows[0].kind, "doc.new");
    }

    #[test]
    fn assembled_kind_requires_an_initialized_document() {
        let mut e = assembled();
        let err = e.apply_json(&fill_envelope(250, "foreground")).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadTarget);
        assert!(err.message.contains("doc.new"));
    }

    #[test]
    #[should_panic(expected = "not in the §10.3 registry")]
    fn assembly_refuses_unregistered_kinds() {
        struct Bad;
        impl kind_impl::KindImpl for Bad {
            fn kind(&self) -> &'static str {
                "zzz.not_a_kind"
            }
            fn apply(
                &self,
                _cx: &mut kind_impl::KindContext<'_>,
            ) -> Result<kind_impl::KindOutcome, EngineError> {
                unreachable!()
            }
        }
        let _ = kind_impl::EngineAssembly::new().with_kind(Arc::new(Bad));
    }

    #[test]
    #[should_panic(expected = "doc.new is engine-built-in")]
    fn assembly_refuses_doc_new() {
        struct Bad;
        impl kind_impl::KindImpl for Bad {
            fn kind(&self) -> &'static str {
                "doc.new"
            }
            fn apply(
                &self,
                _cx: &mut kind_impl::KindContext<'_>,
            ) -> Result<kind_impl::KindOutcome, EngineError> {
                unreachable!()
            }
        }
        let _ = kind_impl::EngineAssembly::new().with_kind(Arc::new(Bad));
    }

    // ------------- suffix-aligned reopened history (wave-2b) -------------

    fn suffix_assembled() -> kind_impl::EngineAssembly {
        kind_impl::EngineAssembly::new().with_kind(Arc::new(MockFill))
    }

    fn reopened_with_fill_assembly(n: u32) -> Engine {
        let mut e = suffix_assembled().build();
        apply_new(
            &mut e,
            n,
            json!({"width": 300, "height": 300, "background": "transparent"}),
        )
        .unwrap();
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        crate::io::package::load_package_bytes_with(&suffix_assembled(), &bytes)
            .unwrap()
            .engine
    }

    #[test]
    fn suffix_history_post_open_commands_undo_redo_byte_identical() {
        let mut e = reopened_with_fill_assembly(300);
        assert!(
            !e.undoable_in_process(),
            "canUndo false immediately after open"
        );
        assert!(!e.redoable_in_process());
        let opened = e.state_hash();

        let receipt = e.apply_json(&fill_envelope(301, "foreground")).unwrap();
        assert!(
            e.undoable_in_process(),
            "canUndo TRUE after a post-open command"
        );
        let edited = e.state_hash();
        assert_ne!(opened, edited);

        let row = e.undo().unwrap();
        assert_eq!(
            row.dirty, receipt.dirty,
            "suffix undo echoes the receipt rects"
        );
        assert_eq!(e.state_hash(), opened, "suffix undo restores byte identity");
        assert!(!e.undoable_in_process(), "back at the base");
        assert!(e.redoable_in_process());

        e.redo().unwrap();
        assert_eq!(e.state_hash(), edited, "suffix redo restores byte identity");
    }

    #[test]
    fn suffix_history_undo_at_base_answers_typed() {
        let mut e = reopened_with_fill_assembly(310);
        let err = e.undo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToUndo);
        assert!(err.message.contains("not persisted"), "{}", err.message);
        // After exhausting the suffix the SAME texture answers at the base.
        e.apply_json(&fill_envelope(311, "foreground")).unwrap();
        e.undo().unwrap();
        let err = e.undo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToUndo);
        assert!(err.message.contains("not persisted"), "{}", err.message);
    }

    #[test]
    fn suffix_history_base_is_fresh_each_open() {
        let mut e = reopened_with_fill_assembly(320);
        e.apply_json(&fill_envelope(321, "foreground")).unwrap();
        assert!(e.undoable_in_process());
        // Save → reopen: the new base covers everything persisted.
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let mut e2 = crate::io::package::load_package_bytes_with(&suffix_assembled(), &bytes)
            .unwrap()
            .engine;
        assert_eq!(e2.journal().len(), 2);
        assert!(!e2.undoable_in_process(), "fresh base each open");
        let err = e2.undo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToUndo);
        assert!(err.message.contains("not persisted"));
        assert_eq!(
            e2.state_hash(),
            e.state_hash(),
            "reopen identity unchanged by the base"
        );
    }

    #[test]
    fn suffix_history_branch_truncation_over_the_suffix() {
        let mut e = reopened_with_fill_assembly(330);
        e.apply_json(&fill_envelope(331, "foreground")).unwrap();
        e.undo().unwrap();
        // Branch: a new suffix command drops the undone tail entry.
        e.apply_json(&fill_envelope(332, "foreground")).unwrap();
        assert_eq!(e.journal().len(), 2, "base entry + the branch");
        assert_eq!(e.redo().unwrap_err().code, ErrorCode::NothingToRedo);
        assert_eq!(e.history().len(), 1, "suffix rows only");
        assert_eq!(e.history()[0].index, 1, "journal-indexed suffix row");
        assert_eq!(
            e.journal_rows().len(),
            2,
            "the CLI's total view keeps every row"
        );
    }

    #[test]
    fn suffix_history_persisted_redoable_tail_stays_replay_path() {
        // A package whose CHECKPOINT cursor sits below the journal length:
        // in-process redo INTO the persisted tail refuses typed.
        let mut e = suffix_assembled().build();
        apply_new(
            &mut e,
            340,
            json!({"width": 300, "height": 300, "background": "transparent"}),
        )
        .unwrap();
        e.apply_json(&fill_envelope(341, "foreground")).unwrap();
        e.undo().unwrap(); // cursor 1, journal 2 — tail persists in the save
        let bytes = crate::io::package::save_package_bytes(&e).unwrap();
        let mut e2 = crate::io::package::load_package_bytes_with(&suffix_assembled(), &bytes)
            .unwrap()
            .engine;
        assert_eq!(e2.journal().len(), 2);
        assert_eq!(e2.journal_cursor(), 1);
        assert!(
            !e2.redoable_in_process(),
            "tail below the base is the replay path's"
        );
        let err = e2.redo().unwrap_err();
        assert_eq!(err.code, ErrorCode::NothingToRedo);
        assert!(err.message.contains("replay path"), "{}", err.message);
    }

    #[test]
    fn doc_new_records_tree_ops_and_tile_deltas() {
        let mut e = Engine::new();
        apply_new(&mut e, 121, json!({"width": 300, "height": 300})).unwrap();
        match &e.history[0].delta {
            StepDelta::DocInit { stash, deltas } => {
                assert!(stash.is_none(), "live document is not stashed");
                assert_eq!(
                    deltas.tree_after.len(),
                    2,
                    "AddNode(root) + AddNode(background)"
                );
                // 300x300 spans a 2x2 tile grid; every touched tile changed
                // from absent to content.
                assert_eq!(deltas.tiles.len(), 4);
                assert_eq!(deltas.bounds.len(), 1);
                assert!(
                    deltas
                        .accounted_bytes(ChannelKind::U8, crate::tile::model::ColorModelKind::Rgba)
                        > 0
                );
            }
            StepDelta::Deltas { .. }
            | StepDelta::Spilled(_)
            | StepDelta::Snapshot { .. }
            | StepDelta::Marker => {
                panic!("doc.new records a DocInit step")
            }
        }
    }
}
